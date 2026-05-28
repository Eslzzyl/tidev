use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tokio::runtime::Runtime;

use tidev_session::session::{ToolCall, ToolExecutionResult};

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
        format!("Security Warning {} of {}", self.current_index, self.total)
    }

    pub(crate) fn path_display(&self) -> String {
        self.pending.requested_path.display().to_string()
    }

    pub(crate) fn workspace_display(&self) -> String {
        self.pending.workspace_root.display().to_string()
    }

    /// Calculate the height needed for the dialog.
    pub(crate) fn dialog_height(&self, _width: u16) -> u16 {
        // Inner: title(1) + message(2) + paths(2) + help(2) + bottom(1) = 8
        // Outer margin: top(1) + bottom(1) = 2
        10
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

/// Confirmation dialog shown when the user chooses "allow until exit" or
/// "deny until exit", asking them to confirm the remembered permission.
/// Maintains the same size and position as the workspace boundary dialog.
#[derive(Clone, Debug)]
pub(crate) struct WorkspaceBoundaryConfirmDialogState {
    /// The original pending boundary check.
    pub pending: PendingWorkspaceBoundaryCheck,
    /// The action being confirmed (AllowUntilExit or DenyUntilExit).
    pub action: BoundaryDecision,
    /// Which option is highlighted: 0 = confirm, 1 = cancel.
    pub selected_index: usize,
    pub current_index: usize,
    pub total: usize,
}

impl WorkspaceBoundaryConfirmDialogState {
    pub(crate) fn title(&self) -> String {
        format!("Confirm {} of {}", self.current_index, self.total)
    }

    pub(crate) fn path_display(&self) -> String {
        self.pending.requested_path.display().to_string()
    }

    pub(crate) fn workspace_display(&self) -> String {
        self.pending.workspace_root.display().to_string()
    }

    /// Calculate the height needed for the dialog (same as boundary dialog).
    pub(crate) fn dialog_height(&self, _width: u16) -> u16 {
        // Inner: title(1) + message(1) + paths(2) + options(2) + help(1) + bottom(1) = 8
        // Outer margin: top(1) + bottom(1) = 2
        10
    }
}

/// Extract paths from tool call arguments that might be outside the workspace.
/// Returns the first resolved (normalized) path that would violate workspace
/// boundaries, or None if all paths are valid. The returned path is the
/// resolved absolute path, so it can be used as a consistent key for
/// permission lookups regardless of the path representation used by the tool.
pub(crate) fn extract_boundary_violation_path(
    workspace_root: &std::path::Path,
    tool_call: &ToolCall,
) -> Option<PathBuf> {
    let args: serde_json::Value = serde_json::from_str(&tool_call.arguments).ok()?;

    let canonical_name = tidev_engine::tooling::canonical_tool_name(&tool_call.name)?;

    let path_buf: PathBuf = match canonical_name {
        "read" | "write" | "edit" | "glob" | "grep" => {
            // read/write/edit use "file_path", glob/grep use "path" (optional)
            let path_str = args
                .get("file_path")
                .or_else(|| args.get("path"))?
                .as_str()?;
            PathBuf::from(path_str)
        }
        "apply_patch" => {
            let patch = args.get("patch_text")?.as_str()?;
            PathBuf::from(tidev_engine::tooling::extract_file_path_from_patch(patch)?)
        }
        "bash" => return None,
        _ => return None,
    };

    if !tidev_engine::tooling::builtin::utils::is_path_outside_workspace(workspace_root, &path_buf)
    {
        return None;
    }

    // Return the resolved path for consistent permission key.
    // Use canonicalize_for_comparison which handles both existing paths
    // (resolving symlinks) and non-existent paths (via parent-walk fallback).
    let resolved =
        tidev_engine::tooling::builtin::utils::resolve_path_unchecked(workspace_root, &path_buf)
            .unwrap_or_else(|_| path_buf.clone());

    Some(tidev_engine::tooling::builtin::utils::canonicalize_for_comparison(&resolved))
}

impl App {
    /// Check if a path has been allowed or denied in memory.
    /// Uses prefix matching (longest matching prefix wins) so that:
    /// - Allowing a directory also allows all files under it.
    /// - Denying a directory also denies all files under it.
    /// - If a specific path has a more specific rule, it takes precedence.
    pub(crate) fn is_workspace_boundary_allowed(&self, path: &str) -> Option<bool> {
        let target = std::path::Path::new(path);
        let mut result: Option<bool> = None;
        let mut longest_prefix: usize = 0;
        for (stored_path, allowed) in &self.workspace_boundary_permissions {
            let stored = std::path::Path::new(stored_path);
            if target.starts_with(stored) {
                let components = stored.components().count();
                if components > longest_prefix {
                    longest_prefix = components;
                    result = Some(*allowed);
                }
            }
        }
        result
    }

    /// Store a workspace boundary permission in memory.
    pub(crate) fn remember_workspace_boundary_permission(&mut self, path: String, allowed: bool) {
        self.workspace_boundary_permissions.insert(path, allowed);
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

        let allowed = matches!(
            decision,
            BoundaryDecision::AllowOnce | BoundaryDecision::AllowUntilExit
        );
        let remember = matches!(
            decision,
            BoundaryDecision::AllowUntilExit | BoundaryDecision::DenyUntilExit
        );

        // If remembering, store the permission in memory
        if remember {
            let path_pattern = dialog.pending.requested_path.display().to_string();
            self.remember_workspace_boundary_permission(path_pattern, allowed);
        }

        if allowed {
            // Handle "question" tool via dialog (needs TUI interaction)
            if dialog.pending.tool_call.name == "question" {
                let args = match serde_json::from_str::<tidev_engine::tooling::QuestionArgs>(
                    &dialog.pending.tool_call.arguments,
                ) {
                    Ok(args) => args,
                    Err(error) => {
                        self.record_tool_result(
                            dialog.pending.tool_call.clone(),
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
                        dialog.pending.tool_call.clone(),
                        ToolExecutionResult::new(
                            "Tool failed: question tool requires at least one question",
                        ),
                    )?;
                    self.advance_pending_tool_execution();
                    return self.process_pending_tool_execution(runtime);
                }

                self.begin_question_dialog(dialog.pending.tool_call, args)?;
                return Ok(());
            }

            // For all other tools, route through normal runtime flow
            // via send_permission_approval, which propagates allow_outside.
            self.workspace_boundary_approved
                .insert(dialog.pending.tool_call.id.clone(), true);
            self.pending_tool_execution
                .as_mut()
                .unwrap()
                .add_ready(dialog.pending.tool_call);
            self.advance_pending_tool_execution();
            return self.process_pending_tool_execution(runtime);
        } else {
            // Record the denial with a message that won't trigger error rendering
            let output = format!(
                "[User denied access] The path '{}' is outside the workspace.",
                dialog.pending.requested_path.display()
            );
            let result = ToolExecutionResult::new(output);
            // Add to in-memory conversation for TUI rendering
            self.record_tool_result(dialog.pending.tool_call.clone(), result.clone())?;
            // Add to pending_rejected_tools so runtime persists it to DB via
            // send_permission_approval → ApprovedTool.rejection → persist_tool_result.
            // Without this, the orphaned tool call causes "no matching tool result" error.
            self.pending_rejected_tools
                .push((dialog.pending.tool_call, result));
            self.advance_pending_tool_execution();
        }

        // Continue processing pending tools
        self.process_pending_tool_execution(runtime)
    }

    /// Handle keyboard input for the workspace boundary dialog.
    /// 'a' and 'd' now show a confirmation dialog first instead of resolving immediately.
    pub(crate) fn handle_workspace_boundary_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        // Intercept AllowUntilExit / DenyUntilExit to show confirmation dialog
        if matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A')) {
            if let Some(ref dialog) = self.workspace_boundary_dialog {
                self.workspace_boundary_confirm_dialog =
                    Some(WorkspaceBoundaryConfirmDialogState {
                        pending: dialog.pending.clone(),
                        action: BoundaryDecision::AllowUntilExit,
                        selected_index: 0,
                        current_index: dialog.current_index,
                        total: dialog.total,
                    });
            }
            return Ok(());
        }
        if matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) {
            if let Some(ref dialog) = self.workspace_boundary_dialog {
                self.workspace_boundary_confirm_dialog =
                    Some(WorkspaceBoundaryConfirmDialogState {
                        pending: dialog.pending.clone(),
                        action: BoundaryDecision::DenyUntilExit,
                        selected_index: 0,
                        current_index: dialog.current_index,
                        total: dialog.total,
                    });
            }
            return Ok(());
        }

        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(BoundaryDecision::AllowOnce),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(BoundaryDecision::DenyOnce),
            KeyCode::Esc => Some(BoundaryDecision::DenyOnce),
            _ => None,
        };

        if let Some(decision) = decision {
            self.resolve_workspace_boundary_dialog(decision, runtime)?;
        }

        Ok(())
    }

    /// Handle keyboard input for the workspace boundary confirmation dialog.
    /// Left/Right to select, Enter to confirm, Esc to go back.
    pub(crate) fn handle_workspace_boundary_confirm_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(ref mut confirm) = self.workspace_boundary_confirm_dialog else {
            return Ok(());
        };

        match key.code {
            KeyCode::Left => {
                confirm.selected_index = confirm.selected_index.saturating_sub(1);
            }
            KeyCode::Right => {
                confirm.selected_index = confirm.selected_index.saturating_add(1).min(1);
            }
            KeyCode::Enter => {
                if confirm.selected_index == 0 {
                    // Confirm — execute the remembered action
                    let action = confirm.action;
                    self.workspace_boundary_confirm_dialog = None;
                    self.resolve_workspace_boundary_dialog(action, runtime)?;
                } else {
                    // Cancel — go back to the original dialog
                    self.workspace_boundary_confirm_dialog = None;
                }
            }
            KeyCode::Esc => {
                // Cancel — go back to the original dialog
                self.workspace_boundary_confirm_dialog = None;
            }
            _ => {}
        }

        Ok(())
    }
}
