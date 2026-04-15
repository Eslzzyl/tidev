use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::{Arc, atomic::AtomicBool};
use tokio::runtime::Runtime;

use crate::session::{ToolCall, ToolExecutionResult};
use crate::tooling::{QuestionArgs, TaskArgs, execute_shell_tool_call};

use super::App;

#[derive(Clone, Debug)]
pub(crate) struct PendingToolExecution {
    tool_calls: Vec<ToolCall>,
    next_index: usize,
}

impl PendingToolExecution {
    pub(crate) fn new(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            tool_calls,
            next_index: 0,
        }
    }

    pub(crate) fn current(&self) -> Option<&ToolCall> {
        self.tool_calls.get(self.next_index)
    }

    pub(crate) fn current_index(&self) -> usize {
        self.next_index + 1
    }

    pub(crate) fn total(&self) -> usize {
        self.tool_calls.len()
    }

    pub(crate) fn advance(&mut self) {
        self.next_index = self.next_index.saturating_add(1);
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.next_index >= self.tool_calls.len()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PermissionDialogState {
    pub tool_call: ToolCall,
    pub permission_key: String,
    pub display_name: String,
    pub current_index: usize,
    pub total: usize,
}

impl PermissionDialogState {
    pub(crate) fn title(&self) -> String {
        format!(
            "Approve tool call {} of {} · {}",
            self.current_index, self.total, self.display_name
        )
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RunningToolExecution {
    pub request_id: u64,
    pub tool_call: ToolCall,
    pub cancel_requested: Arc<AtomicBool>,
}

impl RunningToolExecution {
    pub(crate) fn new(
        request_id: u64,
        tool_call: ToolCall,
        cancel_requested: Arc<AtomicBool>,
    ) -> Self {
        Self {
            request_id,
            tool_call,
            cancel_requested,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RunningSubagentExecution {
    pub request_id: u64,
    pub parent_session_id: uuid::Uuid,
    pub tool_call: ToolCall,
    pub child_session_id: uuid::Uuid,
    pub current_tool_call: Option<ToolCall>,
    pub status_text: String,
    pub cancel_requested: Arc<AtomicBool>,
}

impl RunningSubagentExecution {
    pub(crate) fn new(
        request_id: u64,
        parent_session_id: uuid::Uuid,
        tool_call: ToolCall,
        child_session_id: uuid::Uuid,
        cancel_requested: Arc<AtomicBool>,
        status_text: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            parent_session_id,
            tool_call,
            child_session_id,
            current_tool_call: None,
            status_text: status_text.into(),
            cancel_requested,
        }
    }
}

impl App {
    pub(crate) fn begin_tool_execution(
        &mut self,
        tool_calls: Vec<ToolCall>,
        runtime: &Runtime,
    ) -> Result<()> {
        self.pending_tool_execution = Some(PendingToolExecution::new(tool_calls));
        self.process_pending_tool_execution(runtime)
    }

    pub(crate) fn handle_permission_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.resolve_permission_prompt(true, false, runtime)?;
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.resolve_permission_prompt(true, true, runtime)?;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.resolve_permission_prompt(false, false, runtime)?;
            }
            KeyCode::Char('x') | KeyCode::Char('X') => {
                self.resolve_permission_prompt(false, true, runtime)?;
            }
            _ => {}
        }

        Ok(())
    }

    pub(crate) fn process_pending_tool_execution(&mut self, runtime: &Runtime) -> Result<()> {
        let Some(_) = self.pending_tool_execution.as_ref() else {
            return Ok(());
        };

        if self.running_tool_execution.is_some() {
            crate::log_info!("process_pending_tool_execution: waiting for running_tool_execution");
            return Ok(());
        }

        loop {
            let Some((tool_call, current_index, total)) = self.pending_tool_snapshot() else {
                crate::log_info!("process_pending_tool_execution: no more tool_calls in snapshot");
                break;
            };
            crate::log_info!(
                "process_pending_tool_execution: processing tool {} ({}/{}) id={}",
                tool_call.name,
                current_index,
                total,
                tool_call.id
            );
            let permission_key = self.tools.permission_key_for_call(&tool_call);
            let permission_label = self.tools.permission_label_for_call(&tool_call);

            if !self.tools.can_execute(&tool_call.name, self.mode) {
                let output = format!(
                    "Tool '{}' is not available in {} mode",
                    tool_call.name,
                    self.mode.as_str()
                );
                self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                self.advance_pending_tool_execution();
                continue;
            }

            if let Some(remembered) = self
                .store
                .load_tool_permission(self.conversation.session_id, &permission_key)?
            {
                if remembered {
                    crate::log_info!(
                        "process_pending_tool_execution: remembered permission allowed for {}",
                        tool_call.name
                    );
                    if self.execute_pending_tool_call(tool_call, runtime)? {
                        return Ok(());
                    }
                } else {
                    let output = format!(
                        "Tool '{}' was denied by remembered permission",
                        permission_label
                    );
                    self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                    self.advance_pending_tool_execution();
                }
                continue;
            }

            let Some(definition) = self.tools.definition_for(&tool_call.name) else {
                let output = format!("Tool '{}' is unknown", tool_call.name);
                self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                self.advance_pending_tool_execution();
                continue;
            };

            if definition.needs_confirmation() {
                self.last_notice = Some(format!(
                    "Approve tool call {} of {}: {}",
                    current_index, total, permission_label
                ));
                self.permission_dialog = Some(PermissionDialogState {
                    permission_key,
                    display_name: permission_label,
                    tool_call,
                    current_index,
                    total,
                });
                return Ok(());
            }

            if self.execute_pending_tool_call(tool_call, runtime)? {
                return Ok(());
            }
        }

        if self
            .pending_tool_execution
            .as_ref()
            .is_some_and(PendingToolExecution::is_finished)
        {
            crate::log_info!(
                "process_pending_tool_execution: finished, running_subagent_executions={}",
                self.running_subagent_executions.len()
            );
            self.pending_tool_execution = None;
            if self.running_subagent_executions.is_empty() {
                crate::log_info!("process_pending_tool_execution: calling start_assistant_turn");
                self.start_assistant_turn(runtime)?;
            } else {
                self.last_notice = Some(format!(
                    "Waiting for {} subagent(s)...",
                    self.running_subagent_executions.len()
                ));
            }
        } else {
            crate::log_info!(
                "process_pending_tool_execution: loop ended but not finished, pending_tool_execution={}, running_subagent_executions={}",
                self.pending_tool_execution.is_some(),
                self.running_subagent_executions.len()
            );
        }

        Ok(())
    }

    fn resolve_permission_prompt(
        &mut self,
        allow: bool,
        remember: bool,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(dialog) = self.permission_dialog.take() else {
            return Ok(());
        };

        if remember {
            self.store.remember_tool_permission(
                self.conversation.session_id,
                &dialog.permission_key,
                allow,
            )?;
        }

        if allow {
            if self.execute_pending_tool_call(dialog.tool_call, runtime)? {
                return Ok(());
            }
            return self.process_pending_tool_execution(runtime);
        }

        let output = if remember {
            format!("Tool '{}' was denied and remembered", dialog.display_name)
        } else {
            format!("Tool '{}' was denied", dialog.display_name)
        };

        self.record_tool_result(dialog.tool_call, ToolExecutionResult::new(output))?;
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }

    fn execute_pending_tool_call(
        &mut self,
        tool_call: ToolCall,
        runtime: &Runtime,
    ) -> Result<bool> {
        crate::log_info!(
            "execute_pending_tool_call: {} id={}",
            tool_call.name,
            tool_call.id
        );
        if tool_call.name == "task" {
            if let Err(error) = self.start_subagent_task_execution(tool_call.clone(), runtime) {
                crate::log_error!("start_subagent_task_execution failed: {}", error);
                self.record_tool_result(
                    tool_call,
                    ToolExecutionResult::new(format!("Tool failed: {error}")),
                )?;
                self.advance_pending_tool_execution();
                return Ok(false);
            }
            crate::log_info!(
                "execute_pending_tool_call: task started, advancing and returning false"
            );
            self.advance_pending_tool_execution();
            return Ok(false);
        }

        if tool_call.name == "question" {
            let args = match serde_json::from_str::<QuestionArgs>(&tool_call.arguments) {
                Ok(args) => args,
                Err(error) => {
                    self.record_tool_result(
                        tool_call,
                        ToolExecutionResult::new(format!(
                            "Tool failed: failed to decode question arguments: {error}"
                        )),
                    )?;
                    self.advance_pending_tool_execution();
                    return Ok(false);
                }
            };

            if args.questions.is_empty() {
                self.record_tool_result(
                    tool_call,
                    ToolExecutionResult::new(
                        "Tool failed: question tool requires at least one question",
                    ),
                )?;
                self.advance_pending_tool_execution();
                return Ok(false);
            }

            self.begin_question_dialog(tool_call, args)?;
            return Ok(true);
        }

        if self.should_run_shell_async(&tool_call) {
            self.start_shell_tool_execution(tool_call, runtime)?;
            return Ok(true);
        }

        let result = self
            .tools
            .execute_call(
                runtime.handle(),
                &self.store,
                self.conversation.session_id,
                &tool_call,
            )
            .unwrap_or_else(|error| ToolExecutionResult::new(format!("Tool failed: {error}")));
        self.record_tool_result(tool_call, result)?;
        self.advance_pending_tool_execution();
        Ok(false)
    }

    fn start_subagent_task_execution(
        &mut self,
        tool_call: ToolCall,
        runtime: &Runtime,
    ) -> Result<()> {
        let args = serde_json::from_str::<TaskArgs>(&tool_call.arguments)?;
        let description = args.description.trim().to_string();
        let prompt = args.prompt.trim().to_string();
        let subagent_type = args
            .subagent_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("general")
            .to_string();

        if description.is_empty() {
            anyhow::bail!("task description cannot be empty");
        }
        if prompt.is_empty() {
            anyhow::bail!("task prompt cannot be empty");
        }

        let request_id = self.active_request_id;
        let parent_session_id = self.conversation.session_id;
        let child_session_id = uuid::Uuid::new_v4();
        crate::log_info!(
            "start_subagent_task_execution: request_id={}, parent_session_id={}, child_session_id={}, tool_call_id={}",
            request_id,
            parent_session_id,
            child_session_id,
            tool_call.id
        );
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.running_subagent_executions
            .push(RunningSubagentExecution::new(
                request_id,
                parent_session_id,
                tool_call.clone(),
                child_session_id,
                cancel_requested.clone(),
                "Starting subagent...",
            ));
        self.last_notice = Some(format!(
            "Running {} subagent(s)...",
            self.running_subagent_executions.len()
        ));

        let store_path = self.store.path().to_path_buf();
        let runtime_handle = runtime.handle().clone();
        let tx = self.backend_tx.clone();
        let llm = self.llm.clone();
        let tools = self.tools.clone();
        let model = self.active_model.clone();
        let workspace_root = self.workspace_root.clone();
        let mode = self.mode;
        let task_call = tool_call.clone();

        runtime.spawn(async move {
            crate::log_info!(
                "subagent task spawned: request_id={}, child_session_id={}",
                request_id,
                child_session_id
            );
            let context = crate::app::subagent::SubagentTaskContext {
                parent_request_id: request_id,
                parent_session_id,
                child_session_id,
                description,
                prompt,
                subagent_type,
                llm,
                tools,
                model,
                workspace_root,
                store_path,
                tx: tx.clone(),
                cancel_requested,
                runtime_handle,
                mode,
            };

            let output = match crate::app::subagent::run_subagent_task(context).await {
                Ok(output) => {
                    crate::log_info!(
                        "subagent task succeeded: request_id={}, child_session_id={}, output_len={}",
                        request_id,
                        child_session_id,
                        output.len()
                    );
                    output
                }
                Err(error) => {
                    crate::log_error!(
                        "subagent task failed: request_id={}, child_session_id={}, error={}",
                        request_id,
                        child_session_id,
                        error
                    );
                    format!("Subagent failed: {error}")
                }
            };

            crate::log_info!(
                "sending SubagentCompleted: request_id={}, child_session_id={}",
                request_id,
                child_session_id
            );
            let _ = tx.send(crate::session::BackendEvent::SubagentCompleted {
                session_id: parent_session_id,
                request_id,
                tool_call: task_call,
                child_session_id,
                result: ToolExecutionResult::new(output),
            });
        });

        Ok(())
    }

    fn should_run_shell_async(&self, tool_call: &ToolCall) -> bool {
        self.tools
            .definition_for(&tool_call.name)
            .is_some_and(|definition| definition.name == "bash")
    }

    fn start_shell_tool_execution(&mut self, tool_call: ToolCall, runtime: &Runtime) -> Result<()> {
        let session_id = self.conversation.session_id;
        let request_id = self.active_request_id;
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.running_tool_execution = Some(RunningToolExecution::new(
            request_id,
            tool_call.clone(),
            cancel_requested.clone(),
        ));
        self.last_notice = Some(format!("Running {}...", tool_call.name));

        let tx = self.backend_tx.clone();
        let workspace_root = self.tools.workspace_root().to_path_buf();
        let max_output_bytes = self.tools.max_output_bytes();

        runtime.spawn_blocking(move || {
            let output = execute_shell_tool_call(
                &workspace_root,
                &tool_call,
                max_output_bytes,
                cancel_requested,
            )
            .unwrap_or_else(|error| format!("Tool failed: {error}"));

            let _ = tx.send(crate::session::BackendEvent::ToolCompleted {
                session_id,
                request_id,
                tool_call,
                result: ToolExecutionResult::new(output),
            });
        });

        Ok(())
    }

    pub(crate) fn record_tool_result(
        &mut self,
        tool_call: ToolCall,
        result: ToolExecutionResult,
    ) -> Result<()> {
        self.store.append_tool_event(
            self.conversation.session_id,
            &tool_call.name,
            &tool_call.arguments,
            &result.output,
        )?;

        let message = crate::session::Message::tool_result(tool_call.id, tool_call.name, result);
        self.conversation.push(message.clone());
        self.store
            .append_message(self.conversation.session_id, &message)?;
        Ok(())
    }

    pub(crate) fn advance_pending_tool_execution(&mut self) {
        if let Some(execution) = self.pending_tool_execution.as_mut() {
            execution.advance();
        }
    }

    fn pending_tool_snapshot(&self) -> Option<(ToolCall, usize, usize)> {
        let execution = self.pending_tool_execution.as_ref()?;
        let tool_call = execution.current()?.clone();
        Some((tool_call, execution.current_index(), execution.total()))
    }
}
