use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::{Arc, atomic::AtomicBool};
use tokio::runtime::Runtime;

use crate::agent::runtime::ApprovedTool;
use crate::agent::{AgentDefinition, AgentType};
use crate::prompts::SessionMode;
use crate::session::{ToolCall, ToolExecutionResult};
use crate::tooling::{QuestionArgs, TaskArgs, canonical_tool_name, execute_shell_tool_call};

use super::App;

#[derive(Clone, Debug)]
pub(crate) struct PendingToolExecution {
    tool_calls: Vec<ToolCall>,
    execution_mode: SessionMode,
    next_index: usize,
    ready_tool_calls: Vec<ToolCall>,
}

impl PendingToolExecution {
    pub(crate) fn new(tool_calls: Vec<ToolCall>, execution_mode: SessionMode) -> Self {
        Self {
            tool_calls,
            execution_mode,
            next_index: 0,
            ready_tool_calls: Vec::new(),
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

    pub(crate) fn mode(&self) -> SessionMode {
        self.execution_mode
    }

    pub(crate) fn advance(&mut self) {
        self.next_index = self.next_index.saturating_add(1);
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.next_index >= self.tool_calls.len() && self.ready_tool_calls.is_empty()
    }

    pub(crate) fn add_ready(&mut self, tool_call: ToolCall) {
        self.ready_tool_calls.push(tool_call);
    }

    pub(crate) fn take_ready(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.ready_tool_calls)
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

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub(crate) enum RunningStatus {
    Running,
    Completed,
}

#[derive(Clone, Debug)]
pub(crate) struct RunningToolExecution {
    pub request_id: u64,
    pub tool_call: ToolCall,
    pub cancel_requested: Arc<AtomicBool>,
    pub _status: RunningStatus,
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
            _status: RunningStatus::Running,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum SubagentStatus {
    #[default]
    Thinking,
    Working,
    Tool,
    WritingOutput,
}

impl SubagentStatus {
    pub(crate) fn display(&self) -> &'static str {
        match self {
            Self::Thinking => "Thinking",
            Self::Working => "Working",
            Self::Tool => "Tool",
            Self::WritingOutput => "Writing output",
        }
    }

    pub(crate) fn from_status_text(text: &str) -> Self {
        match text {
            "Thinking" => Self::Thinking,
            "Working" => Self::Working,
            "Tool" => Self::Tool,
            "Writing output" => Self::WritingOutput,
            _ => Self::Thinking,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RunningSubagentExecution {
    pub request_id: u64,
    pub parent_session_id: uuid::Uuid,
    pub tool_call: ToolCall,
    pub child_session_id: uuid::Uuid,
    pub task_description: String,
    pub subagent_type: String,
    pub status: SubagentStatus,
    pub current_tool_call: Option<ToolCall>,
    pub cancel_requested: Arc<AtomicBool>,
}

impl RunningSubagentExecution {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        request_id: u64,
        parent_session_id: uuid::Uuid,
        tool_call: ToolCall,
        child_session_id: uuid::Uuid,
        task_description: String,
        subagent_type: String,
        cancel_requested: Arc<AtomicBool>,
    ) -> Self {
        Self {
            request_id,
            parent_session_id,
            tool_call,
            child_session_id,
            task_description,
            subagent_type,
            status: SubagentStatus::Thinking,
            current_tool_call: None,
            cancel_requested,
        }
    }
}

impl App {
    pub(crate) fn begin_tool_execution(
        &mut self,
        tool_calls: Vec<ToolCall>,
        execution_mode: SessionMode,
        runtime: &Runtime,
    ) -> Result<()> {
        self.pending_tool_execution = Some(PendingToolExecution::new(tool_calls, execution_mode));
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

        if !self.running_tool_executions.is_empty() {
            crate::log_info!("process_pending_tool_execution: waiting for running_tool_executions");
            return Ok(());
        }

        let is_permission_channel = self.pending_permission_response.is_some();
        let mut rejected: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();

        let mut question_opened = false;

        loop {
            let Some((tool_call, current_index, total, effective_mode)) =
                self.pending_tool_snapshot()
            else {
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

            if !self.tools.can_execute(&tool_call.name, effective_mode) {
                let output = format!(
                    "Tool '{}' is disabled in {} mode",
                    tool_call.name,
                    effective_mode.as_str()
                );
                if is_permission_channel {
                    rejected.push((tool_call, ToolExecutionResult::new(output)));
                } else {
                    self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                }
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
                    self.pending_tool_execution
                        .as_mut()
                        .unwrap()
                        .add_ready(tool_call);
                    self.advance_pending_tool_execution();
                    continue;
                } else {
                    let output = format!(
                        "Tool '{}' was denied by remembered permission",
                        permission_label
                    );
                    if is_permission_channel {
                        rejected.push((tool_call, ToolExecutionResult::new(output)));
                    } else {
                        self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                    }
                    self.advance_pending_tool_execution();
                    continue;
                }
            }

            let Some(definition) = self.tools.definition_for(&tool_call.name) else {
                let output = format!("Tool '{}' is unknown", tool_call.name);
                if is_permission_channel {
                    rejected.push((tool_call, ToolExecutionResult::new(output)));
                } else {
                    self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                }
                self.advance_pending_tool_execution();
                continue;
            };

            // Check for workspace boundary violations before proceeding
            if let Some(violation_path) =
                crate::tui::ui::workspace_boundary::extract_boundary_violation_path(
                    &self.workspace_root,
                    &tool_call,
                )
            {
                let path_str = violation_path.display().to_string();

                // Check stored permissions in memory
                if let Some(allowed) = self.is_workspace_boundary_allowed(&path_str) {
                    if !allowed {
                        // Previously denied, record denial and continue
                        let output = format!(
                            "[User denied access] The path '{}' is outside the workspace.",
                            path_str
                        );
                        if is_permission_channel {
                            rejected.push((tool_call, ToolExecutionResult::new(output)));
                        } else {
                            self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                        }
                        self.advance_pending_tool_execution();
                        continue;
                    }
                    // Previously allowed — execute with allow_outside=true
                    if is_permission_channel {
                        // In channel mode, don't execute synchronously.
                        // The runtime will execute it with allow_outside tracked
                        // via workspace_boundary_approved.
                        self.workspace_boundary_approved
                            .insert(tool_call.id.clone(), true);
                        self.pending_tool_execution
                            .as_mut()
                            .unwrap()
                            .add_ready(tool_call);
                        self.advance_pending_tool_execution();
                        continue;
                    }
                    if Self::is_readonly_tool(&tool_call.name) {
                        // Record boundary approval for async dispatch
                        self.workspace_boundary_approved
                            .insert(tool_call.id.clone(), true);
                        self.pending_tool_execution
                            .as_mut()
                            .unwrap()
                            .add_ready(tool_call);
                        self.advance_pending_tool_execution();
                        continue;
                    }
                    let handle = self.tools.execute_call_spawned(
                        runtime.handle().clone(),
                        self.store.clone(),
                        self.conversation.session_id,
                        tool_call.clone(),
                        self.mode,
                        true,
                    );
                    let mut result = runtime.block_on(handle).unwrap_or_else(|join_err| {
                        ToolExecutionResult::new(format!("Tool failed: {join_err}"))
                    });
                    if !result.output.starts_with("Tool failed:") {
                        result
                            .output
                            .push_str("\n\n[User approved access to path outside the workspace]");
                    }
                    self.record_tool_result(tool_call, result)?;
                    self.advance_pending_tool_execution();
                    continue;
                } else {
                    // No stored permission - show dialog
                    self.workspace_boundary_dialog = Some(
                        crate::tui::ui::workspace_boundary::WorkspaceBoundaryDialogState {
                            pending:
                                crate::tui::ui::workspace_boundary::PendingWorkspaceBoundaryCheck {
                                    tool_call: tool_call.clone(),
                                    requested_path: violation_path,
                                    workspace_root: self.workspace_root.clone(),
                                },
                            current_index,
                            total,
                        },
                    );
                    return Ok(());
                }
            }

            if tool_call.name == "question" {
                let args = match serde_json::from_str::<QuestionArgs>(&tool_call.arguments) {
                    Ok(args) => args,
                    Err(error) => {
                        let output =
                            format!("Tool failed: failed to decode question arguments: {error}");
                        if is_permission_channel {
                            rejected.push((tool_call, ToolExecutionResult::new(output)));
                        } else {
                            self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                        }
                        self.advance_pending_tool_execution();
                        continue;
                    }
                };

                if args.questions.is_empty() {
                    let output =
                        "Tool failed: question tool requires at least one question".to_string();
                    if is_permission_channel {
                        rejected.push((tool_call, ToolExecutionResult::new(output)));
                    } else {
                        self.record_tool_result(tool_call, ToolExecutionResult::new(output))?;
                    }
                    self.advance_pending_tool_execution();
                    continue;
                }

                self.begin_question_dialog(tool_call, args)?;
                question_opened = true;
                break;
            }

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

            self.pending_tool_execution
                .as_mut()
                .unwrap()
                .add_ready(tool_call);
            self.advance_pending_tool_execution();
            continue;
        }

        let ready_calls = self
            .pending_tool_execution
            .as_mut()
            .map(|p| p.take_ready())
            .unwrap_or_default();

        if !ready_calls.is_empty() {
            if is_permission_channel {
                return self.send_permission_approval(ready_calls, rejected, runtime);
            }
            return self.start_parallel_execution(ready_calls, runtime);
        }

        if question_opened {
            return Ok(());
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
            if is_permission_channel {
                // All tools rejected — send empty approval to continue the loop
                return self.send_permission_approval(ready_calls, rejected, runtime);
            }
            if self.running_subagent_executions.is_empty() {
                // Capture an intermediate snapshot after tool execution and before the
                // next LLM step, enabling per-step patch computation at round end.
                self.capture_step_snapshot(runtime);
                crate::log_info!("process_pending_tool_execution: old flow — no permission channel");
                self.pending_request = false;
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

    /// Send the permission approval response and clear state.
    fn send_permission_approval(
        &mut self,
        mut ready_calls: Vec<ToolCall>,
        mut rejected: Vec<(ToolCall, ToolExecutionResult)>,
        _runtime: &Runtime,
    ) -> Result<()> {
        crate::log_info!(
            "send_permission_approval: ready_calls={}, rejected={}",
            ready_calls.len(),
            rejected.len(),
        );
        let response_tx = match self.pending_permission_response.take() {
            Some(tx) => tx,
            None => return Ok(()),
        };
        self.pending_tool_execution = None;

        // Merge any rejected tools that were added outside the main loop
        // (e.g. from resolve_permission_prompt or question dialog resolution)
        rejected.append(&mut self.pending_rejected_tools);

        // Build approved list: None = execute, Some(error) = reject
        let mut approvals: Vec<ApprovedTool> = rejected
            .into_iter()
            .map(|(tc, result)| ApprovedTool {
                tool_call: tc,
                rejection: Some(result),
                child_session_id: None,
            })
            .collect();
        for tc in ready_calls.drain(..) {
            let child_session_id = if tc.name == "task" {
                Some(uuid::Uuid::new_v4())
            } else {
                None
            };
            approvals.push(ApprovedTool {
                tool_call: tc,
                rejection: None,
                child_session_id,
            });
        }

        crate::log_info!(
            "send_permission_approval: sending {} approvals ({} approved)",
            approvals.len(),
            approvals.iter().filter(|a| a.rejection.is_none()).count()
        );

        // Record approved tool calls as running for UI display.
        // For "task" tools, also create RunningSubagentExecution entries
        // so the TUI can show subagent cards with status updates.
        for approval in &approvals {
            if approval.rejection.is_none() {
                self.running_tool_executions.push(RunningToolExecution::new(
                    self.active_request_id,
                    approval.tool_call.clone(),
                    Arc::new(std::sync::atomic::AtomicBool::new(false)),
                ));

                // If this is a task/subagent tool, also track it as a
                // RunningSubagentExecution so the runtime's SubagentStatus
                // events can update the subagent card in the UI.
                if approval.tool_call.name == "task"
                    && let Ok(args) = serde_json::from_str::<crate::tooling::TaskArgs>(
                        &approval.tool_call.arguments,
                    ) {
                        let child_session_id = approval.child_session_id.unwrap_or_else(uuid::Uuid::new_v4);
                        let subagent_type_str = args.subagent_type.unwrap_or_default();
                        let description = args.description.trim().to_string();
                        self.running_subagent_executions.push(
                            RunningSubagentExecution::new(
                                self.active_request_id,
                                self.conversation.session_id,
                                approval.tool_call.clone(),
                                child_session_id,
                                description,
                                subagent_type_str,
                                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                            ),
                        );
                    }
            }
        }

        // Send response — the runtime loop will continue automatically
        let _ = response_tx.send(approvals);

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
            if let Some(p) = self.pending_tool_execution.as_mut() {
                p.add_ready(dialog.tool_call);
            }
            return self.process_pending_tool_execution(runtime);
        }

        let output = if remember {
            format!("Tool '{}' was denied and remembered", dialog.display_name)
        } else {
            format!("Tool '{}' was denied", dialog.display_name)
        };

        if self.pending_permission_response.is_some() {
            self.pending_rejected_tools
                .push((dialog.tool_call, ToolExecutionResult::new(output)));
        } else {
            self.record_tool_result(dialog.tool_call, ToolExecutionResult::new(output))?;
        }
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }

    #[allow(dead_code)]
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

        // Execute on blocking thread with catch_unwind protection,
        // block synchronously for the result via runtime.block_on.
        let handle = self.tools.execute_call_spawned(
            runtime.handle().clone(),
            self.store.clone(),
            self.conversation.session_id,
            tool_call.clone(),
            self.mode,
            false, // allow_outside: normal execution doesn't allow outside workspace
        );
        let result = runtime.block_on(handle).unwrap_or_else(|join_err| {
            ToolExecutionResult::new(format!("Tool failed: {join_err}"))
        });
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

        if description.is_empty() {
            anyhow::bail!("task description cannot be empty");
        }
        if prompt.is_empty() {
            anyhow::bail!("task prompt cannot be empty");
        }

        let subagent_type_str = args
            .subagent_type
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "subagent_type is required: specify one of explorer, librarian, oracle, designer, fixer"
                )
            })?;

        // Resolve agent type and create the agent definition
        let agent_type = AgentType::parse(subagent_type_str).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown subagent type '{subagent_type_str}': expected one of explorer, librarian, oracle, designer, fixer"
            )
        })?;
        let agent_type_name = agent_type.display_name().to_string();
        let agent_definition = AgentDefinition::new(agent_type);

        // Resolve model override for this agent type from config
        let model = self
            .config
            .resolve_agent_active_model(&self.auth, &agent_type_name)
            .ok()
            .flatten()
            .unwrap_or_else(|| self.active_model.clone());

        let request_id = self.active_request_id;
        let parent_session_id = self.conversation.session_id;
        let child_session_id = uuid::Uuid::new_v4();
        crate::log_info!(
            "start_subagent_task_execution: request_id={}, parent_session_id={}, child_session_id={}, tool_call_id={}, agent_type={}",
            request_id,
            parent_session_id,
            child_session_id,
            tool_call.id,
            agent_type.display_name()
        );
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.running_subagent_executions
            .push(RunningSubagentExecution::new(
                request_id,
                parent_session_id,
                tool_call.clone(),
                child_session_id,
                description.clone(),
                agent_type_name,
                cancel_requested.clone(),
            ));
        self.subagent_task_map
            .insert(tool_call.id.clone(), child_session_id);
        self.last_notice = Some(format!(
            "Running {} subagent(s)...",
            self.running_subagent_executions.len()
        ));

        let store_path = self.store.path().to_path_buf();
        let runtime_handle = runtime.handle().clone();
        let tx = self.backend_tx.clone();
        let llm = self.llm.clone();
        let tools = self.tools.clone();
        let workspace_root = self.workspace_root.clone();
        let task_call = tool_call.clone();

        runtime.spawn(async move {
            crate::log_info!(
                "subagent task spawned: request_id={}, child_session_id={}, agent_type={}",
                request_id,
                child_session_id,
                agent_definition.agent_type.display_name()
            );
            let context = crate::tui::subagent::SubagentTaskContext {
                parent_request_id: request_id,
                parent_session_id,
                child_session_id,
                description,
                prompt,
                agent_definition,
                llm,
                tools,
                model,
                workspace_root,
                store_path,
                tx: tx.clone(),
                cancel_requested,
                runtime_handle,
            };

            let output = match crate::tui::subagent::run_subagent_task(context).await {
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

    pub(crate) fn is_readonly_tool(name: &str) -> bool {
        matches!(
            canonical_tool_name(name),
            Some("read" | "list" | "glob" | "grep" | "websearch" | "webfetch")
        )
    }

    fn should_run_tool_async(&self, tool_call: &ToolCall) -> bool {
        self.tools
            .definition_for(&tool_call.name)
            .is_some_and(|def| {
                def.name == "bash"
                    || Self::is_readonly_tool(&def.name)
                    || matches!(
                        def.permission,
                        crate::tooling::ToolPermission::Read
                            | crate::tooling::ToolPermission::Search
                    )
            })
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
        self.running_tool_executions.push(RunningToolExecution::new(
            request_id,
            tool_call.clone(),
            cancel_requested.clone(),
        ));
        self.last_notice = Some(format!("Running {}...", tool_call.name));

        let tx = self.backend_tx.clone();
        let workspace_root = self.tools.workspace_root().to_path_buf();
        let max_output_bytes = self.tools.max_output_bytes();
        let rtk_enabled = self.tools.rtk_enabled();

        // Send initial ShellOutput to create a streaming placeholder in the UI
        let _ = tx.send(crate::session::BackendEvent::ShellOutput {
            session_id,
            content: String::new(),
            finished: false,
            exit_code: None,
        });

        runtime.spawn_blocking(move || {
            let result = execute_shell_tool_call(
                &workspace_root,
                &tool_call,
                max_output_bytes,
                rtk_enabled,
                cancel_requested,
                session_id,
                Some(tx.clone()),
            )
            .unwrap_or_else(|error| ToolExecutionResult::new(format!("Tool failed: {error}")));

            let _ = tx.send(crate::session::BackendEvent::ToolCompleted {
                session_id,
                request_id,
                tool_call,
                result,
            });
        });

        Ok(())
    }

    fn start_readonly_tool_execution(
        &mut self,
        tool_call: ToolCall,
        runtime: &Runtime,
    ) -> Result<()> {
        let session_id = self.conversation.session_id;
        let request_id = self.active_request_id;
        let allow_outside = self
            .workspace_boundary_approved
            .remove(&tool_call.id)
            .unwrap_or(false);
        let cancel_requested = Arc::new(AtomicBool::new(false));
        self.running_tool_executions.push(RunningToolExecution::new(
            request_id,
            tool_call.clone(),
            cancel_requested.clone(),
        ));
        self.last_notice = Some(format!("Running {}...", tool_call.name));

        let tx = self.backend_tx.clone();
        let tools = self.tools.clone();
        let store = self.store.clone();
        let mode = self.mode;

        // Use execute_call_spawned for catch_unwind protection and
        // offloading to the blocking thread pool.
        let handle = tools.execute_call_spawned(
            runtime.handle().clone(),
            store,
            session_id,
            tool_call.clone(),
            mode,
            allow_outside,
        );

        runtime.spawn(async move {
            let result = handle.await.unwrap_or_else(|join_err| {
                ToolExecutionResult::new(format!("Tool failed: {join_err}"))
            });
            let _ = tx.send(crate::session::BackendEvent::ToolCompleted {
                session_id,
                request_id,
                tool_call,
                result,
            });
        });

        Ok(())
    }

    pub(crate) fn start_parallel_execution(
        &mut self,
        tool_calls: Vec<ToolCall>,
        runtime: &Runtime,
    ) -> Result<()> {
        let count = tool_calls.len();
        crate::log_info!("start_parallel_execution: {} tools", count);

        if count == 1 {
            self.last_notice = Some(format!("Running {}...", tool_calls[0].name));
        } else {
            let tool_names: Vec<_> = tool_calls.iter().map(|t| t.name.as_str()).collect();
            self.last_notice = Some(format!(
                "Running {} tools ({})...",
                count,
                tool_names.join(", ")
            ));
        }

        // Phase 1: Dispatch async tools (bash, read-only, subagent) concurrently
        // These start immediately via spawn_blocking and report completion via events.
        for tool_call in &tool_calls {
            match tool_call.name.as_str() {
                "task" => {
                    if let Err(error) =
                        self.start_subagent_task_execution(tool_call.clone(), runtime)
                    {
                        crate::log_error!("start_subagent_task_execution failed: {}", error);
                        self.record_tool_result(
                            tool_call.clone(),
                            ToolExecutionResult::new(format!("Tool failed: {error}")),
                        )?;
                    }
                }
                "question" => {
                    let parsed: Result<QuestionArgs, _> =
                        serde_json::from_str(&tool_call.arguments);
                    match parsed {
                        Ok(args) if !args.questions.is_empty() => {
                            self.begin_question_dialog(tool_call.clone(), args)?;
                        }
                        _ => {
                            self.record_tool_result(
                                tool_call.clone(),
                                ToolExecutionResult::new(
                                    "Tool failed: failed to decode question arguments or empty questions",
                                ),
                            )?;
                        }
                    };
                }
                _ => {
                    if self.should_run_tool_async(tool_call) {
                        if tool_call.name == "bash" {
                            self.start_shell_tool_execution(tool_call.clone(), runtime)?;
                        } else {
                            self.start_readonly_tool_execution(tool_call.clone(), runtime)?;
                        }
                    }
                    // Sync tools handled in Phase 2 below
                }
            }
        }

        // Phase 2: Execute may-write tools synchronously, in order.
        // These complete inline before the next turn starts.
        for tool_call in tool_calls {
            match tool_call.name.as_str() {
                "task" | "question" => {
                    // Already handled in Phase 1
                }
                _ => {
                    if !self.should_run_tool_async(&tool_call) {
                        // Execute on blocking thread with catch_unwind protection,
                        // block synchronously for the result via runtime.block_on.
                        let handle = self.tools.execute_call_spawned(
                            runtime.handle().clone(),
                            self.store.clone(),
                            self.conversation.session_id,
                            tool_call.clone(),
                            self.mode,
                            false, // allow_outside: normal execution doesn't allow outside workspace
                        );
                        let result = runtime.block_on(handle).unwrap_or_else(|join_err| {
                            ToolExecutionResult::new(format!("Tool failed: {join_err}"))
                        });
                        self.record_tool_result(tool_call, result)?;
                    }
                }
            }
        }

        if self.running_tool_executions.is_empty()
            && self
                .pending_tool_execution
                .as_ref()
                .is_some_and(|p| p.is_finished())
        {
            self.pending_tool_execution = None;
            if self.running_subagent_executions.is_empty() {
                // Capture step snapshot before next LLM step (start_parallel_execution path)
                self.capture_step_snapshot(runtime);
                self.pending_request = false;
            } else {
                self.last_notice = Some(format!(
                    "Waiting for {} subagent(s)...",
                    self.running_subagent_executions.len()
                ));
            }
        }

        // Clean up any stale workspace_boundary_approved entries (should already be consumed)
        if !self.workspace_boundary_approved.is_empty() {
            self.workspace_boundary_approved.clear();
        }

        Ok(())
    }

    pub(crate) fn record_tool_result(
        &mut self,
        tool_call: ToolCall,
        result: ToolExecutionResult,
    ) -> Result<()> {
        let is_runtime_flow = self.pending_permission_rx.is_some();
        let display_result = if tool_call.name == "task" {
            // Subagent (task) results should not be preview-truncated;
            // the caller expects the complete output for correct decision-making.
            result.clone()
        } else {
            result.preview_for_storage(Some(tool_call.name.as_str()))
        };
        let output_for_tool_event = display_result.output.clone();
        let message = crate::session::Message::tool_result(
            tool_call.id,
            tool_call.name.clone(),
            display_result,
        );

        // In runtime flow, persistence is handled by AgentRuntime::persist_tool_result.
        // We only need to append the tool event for lookup purposes.
        if !is_runtime_flow {
            self.store.append_tool_event(
                self.conversation.session_id,
                message.id,
                &tool_call.name,
                &tool_call.arguments,
                &output_for_tool_event,
            )?;
        }

        if !result.instruction_sources.is_empty() {
            self.update_loaded_instruction_sources(&result.instruction_sources)
                .ok();
        }

        self.conversation.push(message.clone());
        if !is_runtime_flow {
            self.store
                .append_message(self.conversation.session_id, &message)?;
        }

        // Invalidate layout index and render cache since we added a new message
        self.message_layout_index.borrow_mut().valid = false;
        self.clear_message_render_cache();

        if tool_call.name == "todowrite" {
            self.todos = self.store.load_todos(self.conversation.session_id)?;
        }

        Ok(())
    }

    pub(crate) fn advance_pending_tool_execution(&mut self) {
        if let Some(execution) = self.pending_tool_execution.as_mut() {
            execution.advance();
        }
    }

    fn pending_tool_snapshot(&self) -> Option<(ToolCall, usize, usize, SessionMode)> {
        let execution = self.pending_tool_execution.as_ref()?;
        let tool_call = execution.current()?.clone();
        Some((
            tool_call,
            execution.current_index(),
            execution.total(),
            execution.mode(),
        ))
    }

    pub(crate) fn try_start_parallel_execution(&mut self, runtime: &Runtime) -> Result<()> {
        if self.running_tool_executions.is_empty()
            && self
                .pending_tool_execution
                .as_ref()
                .is_some_and(|p| p.is_finished())
        {
            self.pending_tool_execution = None;
            if self.running_subagent_executions.is_empty() {
                // Capture step snapshot before next LLM step (try_start_parallel_execution path)
                self.capture_step_snapshot(runtime);
                self.pending_request = false;
            } else {
                self.last_notice = Some(format!(
                    "Waiting for {} subagent(s)...",
                    self.running_subagent_executions.len()
                ));
            }
        }
        Ok(())
    }
}
