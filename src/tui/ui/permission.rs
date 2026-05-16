use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

use crate::agent::runtime::ApprovedTool;
use crate::prompts::SessionMode;
use crate::session::{ToolCall, ToolExecutionResult};
use crate::tooling::QuestionArgs;

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

/// Dialog state for sandbox elevation requests.
///
/// Shown when a sandboxed command is denied. User can choose to
/// retry with full access or cancel (which lets the error through).
#[derive(Clone, Debug)]
pub(crate) struct SandboxElevationDialog {
    /// The oneshot sender wrapped for clonability.
    pub(crate) response_tx: Arc<Mutex<Option<tokio::sync::oneshot::Sender<bool>>>>,
}

impl SandboxElevationDialog {
    pub(crate) fn new(response_tx: Option<tokio::sync::oneshot::Sender<bool>>) -> Self {
        Self {
            response_tx: Arc::new(Mutex::new(response_tx)),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RunningToolExecution {
    pub request_id: u64,
    pub tool_call: ToolCall,
}

impl RunningToolExecution {
    pub(crate) fn new(request_id: u64, tool_call: ToolCall) -> Self {
        Self {
            request_id,
            tool_call,
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
                rejected.push((tool_call, ToolExecutionResult::new(output)));
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
                    rejected.push((tool_call, ToolExecutionResult::new(output)));
                    self.advance_pending_tool_execution();
                    continue;
                }
            }

            let Some(definition) = self.tools.definition_for(&tool_call.name) else {
                let output = format!("Tool '{}' is unknown", tool_call.name);
                rejected.push((tool_call, ToolExecutionResult::new(output)));
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
                        rejected.push((tool_call, ToolExecutionResult::new(output)));
                        self.advance_pending_tool_execution();
                        continue;
                    }
                    // Previously allowed — execute with allow_outside=true
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

            // Check for sensitive file reads (only for the read tool)
            if crate::tooling::canonical_tool_name(&tool_call.name) == Some("read") {
                // Extract the file path from arguments
                let file_path: Option<String> =
                    serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                        .ok()
                        .and_then(|v| v.get("file_path")?.as_str().map(|s| s.to_string()));

                if let Some(ref path_str) = file_path
                    && let Ok(resolved_path) =
                        crate::tooling::builtin::utils::resolve_workspace_path(
                            &self.workspace_root,
                            std::path::Path::new(path_str),
                            false,
                        )
                {
                    let patterns = crate::tooling::builtin::sensitive::load_sensitive_patterns(
                        &self.workspace_root,
                    );
                    if crate::tooling::builtin::sensitive::is_path_sensitive(
                        &self.workspace_root,
                        &resolved_path,
                        &patterns,
                    ) {
                        let path_str = resolved_path.display().to_string();

                        // Check stored permissions in memory
                        if let Some(allowed) = self.is_sensitive_file_allowed(&path_str) {
                            if !allowed {
                                let output = format!(
                                    "[User denied access] The path '{}' is listed in sensitive.txt.",
                                    path_str
                                );
                                rejected.push((tool_call, ToolExecutionResult::new(output)));
                                self.advance_pending_tool_execution();
                                continue;
                            }
                            // Previously allowed — execute with sensitive_file_approved=true
                            self.sensitive_file_approved
                                .insert(tool_call.id.clone(), true);
                            self.pending_tool_execution
                                .as_mut()
                                .unwrap()
                                .add_ready(tool_call);
                            self.advance_pending_tool_execution();
                            continue;
                        } else {
                            // No stored permission - show dialog
                            self.sensitive_file_dialog =
                                Some(crate::tui::ui::sensitive::SensitiveFileDialogState {
                                    pending: crate::tui::ui::sensitive::PendingSensitiveFileCheck {
                                        tool_call: tool_call.clone(),
                                        sensitive_path: resolved_path,
                                        workspace_root: self.workspace_root.clone(),
                                    },
                                    current_index,
                                    total,
                                });
                            return Ok(());
                        }
                    }
                }
            }

            if tool_call.name == "question" {
                let args = match serde_json::from_str::<QuestionArgs>(&tool_call.arguments) {
                    Ok(args) => args,
                    Err(error) => {
                        let output =
                            format!("Tool failed: failed to decode question arguments: {error}");
                        rejected.push((tool_call, ToolExecutionResult::new(output)));
                        self.advance_pending_tool_execution();
                        continue;
                    }
                };

                if args.questions.is_empty() {
                    let output =
                        "Tool failed: question tool requires at least one question".to_string();
                    rejected.push((tool_call, ToolExecutionResult::new(output)));
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
            return self.send_permission_approval(ready_calls, rejected, runtime);
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
            // All tools rejected — send empty approval to continue the loop
            return self.send_permission_approval(ready_calls, rejected, runtime);
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
                allow_outside: false,
                sensitive_file_approved: false,
            })
            .collect();
        for tc in ready_calls.drain(..) {
            let child_session_id = if tc.name == "task" {
                Some(uuid::Uuid::new_v4())
            } else {
                None
            };
            let allow_outside = self
                .workspace_boundary_approved
                .remove(&tc.id)
                .unwrap_or(false);
            let sensitive_file_approved =
                self.sensitive_file_approved.remove(&tc.id).unwrap_or(false);
            approvals.push(ApprovedTool {
                tool_call: tc,
                rejection: None,
                child_session_id,
                allow_outside,
                sensitive_file_approved,
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
                ));

                // If this is a task/subagent tool, also track it as a
                // RunningSubagentExecution so the runtime's SubagentStatus
                // events can update the subagent card in the UI.
                if approval.tool_call.name == "task"
                    && let Ok(args) = serde_json::from_str::<crate::tooling::TaskArgs>(
                        &approval.tool_call.arguments,
                    )
                {
                    let child_session_id =
                        approval.child_session_id.unwrap_or_else(uuid::Uuid::new_v4);
                    let subagent_type_str = args.subagent_type.clone();
                    let description = args.description.trim().to_string();
                    self.running_subagent_executions
                        .push(RunningSubagentExecution::new(
                            self.active_request_id,
                            self.conversation.session_id,
                            approval.tool_call.clone(),
                            child_session_id,
                            description,
                            subagent_type_str,
                        ));
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

    pub(crate) fn record_tool_result(
        &mut self,
        tool_call: ToolCall,
        result: ToolExecutionResult,
    ) -> Result<()> {
        let display_result = if tool_call.name == "task" {
            // Subagent (task) results should not be preview-truncated;
            // the caller expects the complete output for correct decision-making.
            result.clone()
        } else {
            result.preview_for_storage(Some(tool_call.name.as_str()))
        };
        let message = crate::session::Message::tool_result(
            tool_call.id,
            tool_call.name.clone(),
            display_result,
        );

        // Persistence is handled by AgentRuntime::persist_tool_result.

        if !result.instruction_sources.is_empty() {
            self.update_loaded_instruction_sources(&result.instruction_sources)
                .ok();
        }

        self.conversation.push(message.clone());

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
}
