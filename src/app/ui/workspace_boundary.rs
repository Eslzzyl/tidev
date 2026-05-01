use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tokio::runtime::Runtime;

use crate::session::{ToolCall, ToolExecutionResult};

use super::App;

/// Represents a pending workspace boundary check for a tool call.
/// When a tool tries to access a path outside the workspace, this holds
/// the information needed to prompt the user for confirmation.
#[derive(Clone, Debug)]
pub(crate) struct PendingWorkspaceBoundaryCheck {
    pub tool_call: ToolCall,
    pub requested_path: PathBuf,
    pub workspace_root: PathBuf,
}

/// Dialog state for workspace boundary violation confirmation.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceBoundaryDialogState {
    pub pending: PendingWorkspaceBoundaryCheck,
    pub current_index: usize,
    pub total: usize,
}

impl WorkspaceBoundaryDialogState {
    pub(crate) fn title(&self) -> String {
        format!(
            "Security Warning {} of {}",
            self.current_index, self.total
        )
    }

    pub(crate) fn path_display(&self) -> String {
        self.pending.requested_path.display().to_string()
    }

    pub(crate) fn workspace_display(&self) -> String {
        self.pending.workspace_root.display().to_string()
    }

    /// Calculate the height needed for the dialog.
    pub(crate) fn dialog_height(&self, _width: u16) -> u16 {
        // Title + warning + path info + help text + borders
        8
    }
}

/// Represents the user's decision for a workspace boundary check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BoundaryDecision {
    /// Allow once (don't remember)
    AllowOnce,
    /// Allow and remember until tidev exits
    AllowUntilExit,
    /// Deny once (don't remember)
    DenyOnce,
    /// Deny and remember until tidev exits
    DenyUntilExit,
}

/// Extract paths from tool call arguments that might be outside the workspace.
/// Returns the first path that would violate workspace boundaries, or None if all paths are valid.
pub(crate) fn extract_boundary_violation_path(
    workspace_root: &std::path::Path,
    tool_call: &ToolCall,
) -> Option<PathBuf> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.arguments).ok()?;

    // List of tools that have path arguments
    let canonical_name = crate::tooling::canonical_tool_name(&tool_call.name)?;

    match canonical_name {
        "read" | "write" | "edit" | "list" | "glob" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                let path_buf = std::path::Path::new(path);
                if crate::tooling::builtin::utils::is_path_outside_workspace(workspace_root, path_buf) {
                    return Some(path_buf.to_path_buf());
                }
            }
        }
        "apply_patch" => {
            // For apply_patch, we need to extract the file path from the patch
            if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
                if let Some(file_path) = crate::tooling::extract_file_path_from_patch(patch) {
                    let path_buf = std::path::Path::new(&file_path);
                    if crate::tooling::builtin::utils::is_path_outside_workspace(workspace_root, path_buf) {
                        return Some(path_buf.to_path_buf());
                    }
                }
            }
        }
        "grep" => {
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                let path_buf = std::path::Path::new(path);
                if crate::tooling::builtin::utils::is_path_outside_workspace(workspace_root, path_buf) {
                    return Some(path_buf.to_path_buf());
                }
            }
        }
        "bash" => {
            // For bash, we don't check here - the RTK system handles security
            return None;
        }
        _ => {
            // For other tools (including MCP tools), we don't check paths
            return None;
        }
    }

    None
}

impl App {
    /// Check if a path has been allowed in memory.
    pub(crate) fn is_workspace_boundary_allowed(&self, path: &str) -> Option<bool> {
        self.workspace_boundary_permissions.get(path).copied()
    }

    /// Store a workspace boundary permission in memory.
    pub(crate) fn remember_workspace_boundary_permission(&mut self, path: String, allowed: bool) {
        self.workspace_boundary_permissions.insert(path, allowed);
    }

    /// Execute a tool that has been allowed to access outside workspace.
    fn execute_boundary_allowed_tool(
        &mut self,
        tool_call: ToolCall,
        runtime: &Runtime,
    ) -> Result<()> {
        // Handle special cases like question tool
        if tool_call.name == "question" {
            let args = match serde_json::from_str::<crate::tooling::QuestionArgs>(&tool_call.arguments) {
                Ok(args) => args,
                Err(error) => {
                    self.record_tool_result(
                        tool_call,
                        ToolExecutionResult::new(format!(
                            "Tool failed: failed to decode question arguments: {error}"
                        )),
                    )?;
                    self.advance_pending_tool_execution();
                    return self.process_pending_tool_execution(runtime);
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
                return self.process_pending_tool_execution(runtime);
            }

            self.begin_question_dialog(tool_call, args)?;
            return Ok(());
        }

        // Note: Shell commands are handled by RTK system for security,
        // so we don't need special handling here.

        // Execute the tool with allow_outside=true
        let mut result = self
            .tools
            .execute_call(
                runtime.handle(),
                &self.store,
                self.conversation.session_id,
                &tool_call,
                self.mode,
                true, // allow_outside: this tool has been allowed by user
            )
            .unwrap_or_else(|error| ToolExecutionResult::new(format!("Tool failed: {error}")));

        // Inject a note into the output indicating this was an outside-workspace access
        // that the user approved. Skip if the tool itself failed.
        if !result.output.starts_with("Tool failed:") {
            result
                .output
                .push_str("\n\n[User approved access to path outside the workspace]");
        }

        self.record_tool_result(tool_call, result)?;
        self.advance_pending_tool_execution();
        self.process_pending_tool_execution(runtime)
    }

    /// Handle keyboard input for the workspace boundary dialog.
    pub(crate) fn handle_workspace_boundary_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(BoundaryDecision::AllowOnce),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(BoundaryDecision::AllowUntilExit),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(BoundaryDecision::DenyOnce),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(BoundaryDecision::DenyUntilExit),
            KeyCode::Esc => Some(BoundaryDecision::DenyOnce),
            _ => None,
        };

        if let Some(decision) = decision {
            self.resolve_workspace_boundary_dialog(decision, runtime)?;
        }

        Ok(())
    }

    /// Resolve the workspace boundary dialog with the user's decision.
    fn resolve_workspace_boundary_dialog(
        &mut self,
        decision: BoundaryDecision,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(dialog) = self.workspace_boundary_dialog.take() else {
            return Ok(());
        };

        let allowed = matches!(decision, BoundaryDecision::AllowOnce | BoundaryDecision::AllowUntilExit);
        let remember = matches!(decision, BoundaryDecision::AllowUntilExit | BoundaryDecision::DenyUntilExit);

        // If remembering, store the permission in memory
        if remember {
            let path_pattern = dialog.pending.requested_path.display().to_string();
            self.remember_workspace_boundary_permission(path_pattern, allowed);
        }

        if allowed {
            // Check if the tool is read-only; dispatch async instead of inline execute
            if Self::is_readonly_tool(&dialog.pending.tool_call.name) {
                // If it's also a "question" tool, handle via dialog (fall through below)
                if dialog.pending.tool_call.name != "question" {
                    self.workspace_boundary_approved
                        .insert(dialog.pending.tool_call.id.clone(), true);
                    self.pending_tool_execution
                        .as_mut()
                        .unwrap()
                        .add_ready(dialog.pending.tool_call);
                    self.advance_pending_tool_execution();
                    return self.process_pending_tool_execution(runtime);
                }
            }
            // Execute the tool immediately with allow_outside=true
            self.execute_boundary_allowed_tool(dialog.pending.tool_call, runtime)?;
            return Ok(());
        } else {
            // Record the denial with a message that won't trigger error rendering
            let output = format!(
                "[User denied access] The path '{}' is outside the workspace.",
                dialog.pending.requested_path.display()
            );
            self.record_tool_result(dialog.pending.tool_call, ToolExecutionResult::new(output))?;
            self.advance_pending_tool_execution();
        }

        // Continue processing pending tools
        self.process_pending_tool_execution(runtime)
    }
}
