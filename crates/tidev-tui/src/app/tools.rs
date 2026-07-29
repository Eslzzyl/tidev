use super::*;

use std::collections::HashMap;
use std::path::Path;

use tidev_core::TuiResponse;
use tidev_types::message::ToolExecutionResult;
use tidev_types::tools::QuestionArgs;
use uuid::Uuid;

use crate::action::{BoundaryDecision, SensitiveFileDecision};
use crate::components::overlays::question::QuestionDialog;
use crate::components::overlays::sensitive::SensitiveFileDialog;
use crate::components::overlays::workspace::WorkspaceBoundaryDialog;

impl App {
    /// Handle a pending tool approval request from the agent loop.
    /// Stores the request per-session.
    pub(crate) fn handle_tui_request(&mut self, request: tidev_core::TuiRequest) {
        let session_id = request.session_id;
        match request.kind {
            tidev_core::TuiRequestKind::ToolApproval(tools_with_violations) => {
                log::info!(
                    "handle_tui_request: session {session_id}, {} tool(s) pending approval",
                    tools_with_violations.len()
                );
                self.pending_approvals.insert(
                    session_id,
                    PendingApproval {
                        response_tx: request.response_tx,
                        tools: tools_with_violations,
                        tool_index: 0,
                        approved_tools: Vec::new(),
                    },
                );
                // If no approval dialog is currently active, activate this one.
                if self.active_approval_session.is_none() {
                    self.active_approval_session = Some(session_id);
                    self.process_next_tool();
                }
            }
        }
    }

    /// Run the approval pipeline for the currently active session.
    /// Opens the appropriate dialog (workspace boundary, sensitive file,
    /// question, or permission) for the tool at `tool_index`. When all tools
    /// are processed, sends the approval response back to the runtime.
    pub(super) fn process_next_tool(&mut self) {
        let session_id = match self.active_approval_session {
            Some(sid) => sid,
            None => return,
        };
        let Some(approval) = self.pending_approvals.get_mut(&session_id) else {
            self.active_approval_session = None;
            return;
        };

        while approval.tool_index < approval.tools.len() {
            let (boundary_path, sensitive_path, is_question, args, tc) = {
                let twv = &approval.tools[approval.tool_index];
                let tc = &twv.tool_call;
                (
                    twv.workspace_boundary_violation.clone(),
                    twv.sensitive_file_violation.clone(),
                    tc.name == "question",
                    tc.arguments.clone(),
                    tc.clone(),
                )
            };
            let current_index = approval.tool_index + 1;
            let total = approval.tools.len();

            // Step 1: Workspace boundary violation check
            if let Some(ref path) = boundary_path {
                let path_str = path.to_string_lossy().to_string();
                match Self::is_path_allowed(&self.boundary_permissions, &path_str) {
                    Some(true) => {
                        log::info!("Boundary path already allowed: {path_str}");
                    }
                    Some(false) => {
                        log::info!("Boundary path previously denied: {path_str}");
                        let reason = self.boundary_reasons.remove(&path_str);
                        let msg = if let Some(ref r) = reason {
                            format!("Error: Path '{}' was denied. Reason: {}", path_str, r)
                        } else {
                            format!("Error: Path '{}' was denied.", path_str)
                        };
                        approval.approved_tools.push(ApprovedTool {
                            tool_call: tc,
                            rejection: Some(ToolExecutionResult::new(msg)),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                            user_reason: None,
                        });
                        approval.tool_index += 1;
                        continue;
                    }
                    None => {
                        log::info!("Opening WorkspaceBoundaryDialog for: {path_str}");
                        self.set_notice("Workspace boundary violation — please make a decision");
                        self.overlays.push(Box::new(WorkspaceBoundaryDialog::new(
                            path.clone(),
                            self.runtime.workspace_root().clone(),
                            current_index,
                            total,
                        )));
                        return;
                    }
                }
            }

            // Step 2: Sensitive file violation check
            if let Some(ref path) = sensitive_path {
                let path_str = path.to_string_lossy().to_string();
                match Self::is_path_allowed(&self.sensitive_permissions, &path_str) {
                    Some(true) => {
                        log::info!("Sensitive file already allowed: {path_str}");
                    }
                    Some(false) => {
                        log::info!("Sensitive file previously denied: {path_str}");
                        let reason = self.sensitive_reasons.remove(&path_str);
                        let msg = if let Some(ref r) = reason {
                            format!(
                                "Error: Sensitive file '{}' was denied. Reason: {}",
                                path_str, r
                            )
                        } else {
                            format!("Error: Sensitive file '{}' was denied.", path_str)
                        };
                        approval.approved_tools.push(ApprovedTool {
                            tool_call: tc,
                            rejection: Some(ToolExecutionResult::new(msg)),
                            child_session_id: None,
                            allow_outside: false,
                            sensitive_file_approved: false,
                            user_reason: None,
                        });
                        approval.tool_index += 1;
                        continue;
                    }
                    None => {
                        log::info!("Opening SensitiveFileDialog for: {path_str}");
                        self.set_notice("Sensitive file access — please make a decision");
                        self.overlays.push(Box::new(SensitiveFileDialog::new(
                            path.clone(),
                            self.runtime.workspace_root().clone(),
                            current_index,
                            total,
                        )));
                        return;
                    }
                }
            }

            // Step 3: 'question' tool — always show approval dialog
            if is_question {
                if let Ok(qa) = serde_json::from_str::<QuestionArgs>(&args) {
                    log::info!("Opening QuestionDialog ({} questions)", qa.questions.len());
                    self.set_notice("LLM has questions — please provide answers");
                    self.overlays
                        .push(Box::new(QuestionDialog::new(qa.questions)));
                    return;
                } else {
                    log::warn!("Invalid or empty question tool call arguments");
                    approval.approved_tools.push(ApprovedTool {
                        tool_call: tc,
                        rejection: Some(ToolExecutionResult::new(
                            "Tool 'question' was rejected: invalid or empty arguments.",
                        )),
                        child_session_id: None,
                        allow_outside: false,
                        sensitive_file_approved: false,
                        user_reason: None,
                    });
                    approval.tool_index += 1;
                    continue;
                }
            }

            // Step 4: Auto-approve (permission check already done by mode filter)
            {
                let boundary_str = boundary_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let sensitive_str = sensitive_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string());
                let allow_outside = boundary_str
                    .as_deref()
                    .and_then(|p| Self::is_path_allowed(&self.boundary_permissions, p))
                    .unwrap_or(false);
                let sensitive_approved = sensitive_str
                    .as_deref()
                    .and_then(|p| Self::is_path_allowed(&self.sensitive_permissions, p))
                    .unwrap_or(false);

                log::info!(
                    "Auto-approving tool ({}/{}) allow_outside={} sensitive_approved={}",
                    current_index,
                    total,
                    allow_outside,
                    sensitive_approved,
                );
                approval.approved_tools.push(ApprovedTool {
                    tool_call: tc,
                    rejection: None,
                    child_session_id: None,
                    allow_outside,
                    sensitive_file_approved: sensitive_approved,
                    user_reason: None,
                });
                approval.tool_index += 1;
                continue;
            }
        }

        // All tools processed — send response
        self.send_approval_response(session_id);
    }

    /// Send the accumulated approval response back to the runtime for a session.
    fn send_approval_response(&mut self, session_id: Uuid) {
        let Some(approval) = self.pending_approvals.remove(&session_id) else {
            log::warn!("send_approval_response: no pending approval for session {session_id}");
            return;
        };

        let tools = approval.approved_tools;
        log::info!(
            "send_approval_response: session {session_id}, {} tool(s) approved/rejected",
            tools.len()
        );

        let _ = approval.response_tx.send(TuiResponse::ToolApproval(tools));

        // Clear active approval if this was the active session.
        if self.active_approval_session == Some(session_id) {
            self.active_approval_session = None;
            // Check if any other session has pending approvals.
            if let Some(&next_sid) = self.pending_approvals.keys().next() {
                self.active_approval_session = Some(next_sid);
                self.process_next_tool();
            }
        }
    }

    /// Check whether a path is in an allowlist, using prefix matching so that
    /// allowing a directory also allows all files under it.
    pub(super) fn is_path_allowed(cache: &HashMap<String, bool>, path: &str) -> Option<bool> {
        let target = Path::new(path);
        let mut result: Option<bool> = None;
        let mut longest_prefix: usize = 0;
        for (stored, allowed) in cache {
            let stored_path = Path::new(stored);
            if target.starts_with(stored_path) {
                let components = stored_path.components().count();
                if components > longest_prefix {
                    longest_prefix = components;
                    result = Some(*allowed);
                }
            }
        }
        result
    }

    /// Record a workspace boundary decision in the in-memory cache.
    pub(super) fn record_boundary_decision(&mut self, path: &Path, decision: &BoundaryDecision) {
        match decision {
            BoundaryDecision::AllowOnce => {}
            BoundaryDecision::AllowUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.boundary_permissions.insert(path_str, true);
            }
            BoundaryDecision::DenyOnce => {}
            BoundaryDecision::DenyUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.boundary_permissions.insert(path_str, false);
            }
        }
    }

    /// Record a sensitive file decision in the in-memory cache.
    pub(super) fn record_sensitive_decision(
        &mut self,
        path: &Path,
        decision: &SensitiveFileDecision,
    ) {
        match decision {
            SensitiveFileDecision::AllowOnce => {}
            SensitiveFileDecision::AllowUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.sensitive_permissions.insert(path_str, true);
            }
            SensitiveFileDecision::DenyOnce => {}
            SensitiveFileDecision::DenyUntilExit => {
                let path_str = path.to_string_lossy().to_string();
                self.sensitive_permissions.insert(path_str, false);
            }
        }
    }
}
