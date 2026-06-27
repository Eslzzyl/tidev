use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use tokio::runtime::Runtime;

use tidev_agent::ApprovedTool;
use tidev_tools::QuestionArgs;
use tidev_session::session::{ToolCall, ToolExecutionResult};
use tidev_types::prompts::SessionMode;

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
            log::info!("process_pending_tool_execution: waiting for running_tool_executions");
            return Ok(());
        }

        let mut rejected: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();

        let mut question_opened = false;

        loop {
            let Some((tool_call, current_index, total, effective_mode)) =
                self.pending_tool_snapshot()
            else {
                log::info!("process_pending_tool_execution: no more tool_calls in snapshot");
                break;
            };
            log::info!(
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
                    "Tool '{}' is disabled in {} mode. \
                     If you need to modify files, you must explain your intent to the user and ask them to switch to Build mode.",
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
                    log::info!(
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
                crate::ui::workspace_boundary::extract_boundary_violation_path(
                    &self.workspace_root,
                    &tool_call,
                )
            {
                let path_str = violation_path.display().to_string();

                // If the path doesn't exist, don't ask for permission — the
                // operation will fail anyway. Skip this for tools that can
                // create files (write, apply_patch).
                let needs_existing = matches!(
                    tidev_tools::canonical_tool_name(&tool_call.name),
                    Some("read" | "edit" | "glob" | "grep")
                );
                if needs_existing && !violation_path.exists() {
                    let output =
                        format!("[Path not found] The path '{}' does not exist.", path_str);
                    rejected.push((tool_call, ToolExecutionResult::new(output)));
                    self.advance_pending_tool_execution();
                    continue;
                }

                // Check access control config for skip
                if self.config.read().unwrap().access_control.allow_outside_workspace_access {
                    self.workspace_boundary_approved
                        .insert(tool_call.id.clone(), true);
                    self.pending_tool_execution
                        .as_mut()
                        .unwrap()
                        .add_ready(tool_call);
                    self.advance_pending_tool_execution();
                    continue;
                }

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
                        crate::ui::workspace_boundary::WorkspaceBoundaryDialogState {
                            pending: crate::ui::workspace_boundary::PendingWorkspaceBoundaryCheck {
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
            if tidev_tools::canonical_tool_name(&tool_call.name) == Some("read") {
                // Extract the file path from arguments
                let file_path: Option<String> =
                    serde_json::from_str::<serde_json::Value>(&tool_call.arguments)
                        .ok()
                        .and_then(|v| v.get("file_path")?.as_str().map(|s| s.to_string()));

                if let Some(ref path_str) = file_path
                    && let Ok(resolved_path) =
                        tidev_tools::builtin::utils::resolve_workspace_path(
                            &self.workspace_root,
                            std::path::Path::new(path_str),
                            false,
                        )
                {
                    let patterns =
                        tidev_tools::builtin::sensitive::load_sensitive_patterns(
                            &self.workspace_root,
                        );
                    if tidev_tools::builtin::sensitive::is_path_sensitive(
                        &self.workspace_root,
                        &resolved_path,
                        &patterns,
                    ) {
                        let path_str = resolved_path.display().to_string();

                // Check access control config for skip
                if self.config.read().unwrap().access_control.allow_sensitive_file_access {
                    self.sensitive_file_approved
                        .insert(tool_call.id.clone(), true);
                    self.pending_tool_execution
                        .as_mut()
                        .unwrap()
                        .add_ready(tool_call);
                    self.advance_pending_tool_execution();
                    continue;
                }

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
                                Some(crate::ui::sensitive::SensitiveFileDialogState {
                                    pending: crate::ui::sensitive::PendingSensitiveFileCheck {
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
            log::info!("process_pending_tool_execution: finished");
            self.pending_tool_execution = None;
            // All tools rejected — send empty approval to continue the loop
            return self.send_permission_approval(ready_calls, rejected, runtime);
        } else {
            log::info!(
                "process_pending_tool_execution: loop ended but not finished, pending_tool_execution={}",
                self.pending_tool_execution.is_some()
            );
        }

        Ok(())
    }

    /// Send the permission approval response and clear state.
    fn send_permission_approval(
        &mut self,
        mut ready_calls: Vec<ToolCall>,
        mut rejected: Vec<(ToolCall, ToolExecutionResult)>,
        runtime: &Runtime,
    ) -> Result<()> {
        log::info!(
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

        // Immediately record rejected tool results in the in-memory conversation
        // so the TUI can render the rejection without waiting for the async
        // ToolCompleted event from the agent loop. Rejected tools are never
        // added to running_tool_executions, so ToolCompleted is silently
        // dropped — without this early record the rejection would never appear.
        for (tc, result) in &rejected {
            self.record_tool_result(tc.clone(), result.clone(), runtime)?;
        }

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

        log::info!(
            "send_permission_approval: sending {} approvals ({} approved)",
            approvals.len(),
            approvals.iter().filter(|a| a.rejection.is_none()).count()
        );

        // Record approved tool calls as running for UI display.
        for approval in &approvals {
            if approval.rejection.is_none() {
                self.running_tool_executions.push(RunningToolExecution::new(
                    self.active_request_id,
                    approval.tool_call.clone(),
                ));

                // Register tool_call_id → child_session_id mapping for navigation.
                if approval.tool_call.name == "task"
                    && let Some(child_session_id) = approval.child_session_id
                {
                    self.subagent_task_map
                        .insert(approval.tool_call.id.clone(), child_session_id);
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
            runtime.block_on(self.agent.remember_tool_permission(
                self.conversation.session_id,
                &dialog.permission_key,
                allow,
            ))?;
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
            self.record_tool_result(dialog.tool_call, ToolExecutionResult::new(output), runtime)?;
        }
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }

    pub(crate) fn record_tool_result(
        &mut self,
        tool_call: ToolCall,
        mut result: ToolExecutionResult,
        runtime: &Runtime,
    ) -> Result<()> {
        let is_task = tool_call.name == "task";
        let tool_call_id = tool_call.id.clone();

        // For task (subagent) results: inject child_session_id into metadata
        // so it persists in the stored message for click navigation after restart.
        if is_task && let Some(&child_session_id) = self.subagent_task_map.get(&tool_call_id) {
            result.metadata.child_session_id = Some(child_session_id);
        }

        let display_result = if is_task {
            // Subagent (task) results should not be preview-truncated;
            // the caller expects the complete output for correct decision-making.
            result.clone()
        } else {
            result.preview_for_storage(Some(tool_call.name.as_str()))
        };
        let message = tidev_session::session::Message::tool_result(
            tool_call.id,
            tool_call.name.clone(),
            display_result,
        );

        // Save the full (untruncated) output to tool_outputs table so the
        // TUI can display the complete content when the card is expanded.
        // The task tool stores the full output inside the subagent session
        // rather than here, so we skip it.
        if tool_call.name != "task"
            && let Err(e) = runtime.block_on(self.agent.save_tool_output(
                self.conversation.session_id,
                message.id,
                &tool_call.name,
                &result.output,
            ))
        {
            log::warn!("Failed to save full tool output: {e}");
        }

        // Persistence is handled by tidev_agent::SessionManager::persist_tool_result.

        if !result.instruction_sources.is_empty() {
            self.update_loaded_instruction_sources(&result.instruction_sources)
                .ok();
        }

        self.conversation.push(message.clone());

        // For task (subagent) results, also register the message_id →
        // child_session_id mapping so click navigation works reliably.
        if is_task && let Some(&child_session_id) = self.subagent_task_map.get(&tool_call_id) {
            self.subagent_result_message_map
                .insert(message.id, child_session_id);
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
}

/// A subagent overlay rendered inline in the parent conversation.
///
/// Each overlay subscribes directly to the child session's BackendEvent channel,
/// eliminating the need for aggregate subagent events (SubagentStatus, etc.).
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) struct SubagentOverlay {
    pub child_session_id: uuid::Uuid,
    pub parent_session_id: uuid::Uuid,
    pub agent_type: tidev_types::agent::AgentType,
    pub description: String,
    /// The tool_call ID from the parent conversation that spawned this subagent.
    pub tool_call_id: String,
    /// The tool_call name (always "task").
    pub tool_call_name: String,
    /// Direct event stream from the child AgentLoop.
    pub event_rx: tokio::sync::mpsc::UnboundedReceiver<tidev_session::session::BackendEvent>,
    /// The accumulated assistant message content for this subagent.
    pub assistant_content: String,
    /// Whether this subagent has completed.
    pub finished: bool,
}
