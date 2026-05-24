//! Sensitive file confirmation dialog for the `read` tool.
//!
//! When the Agent tries to read a file listed in `.tidev/sensitive.txt`,
//! this dialog prompts the user to confirm or deny the read.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use std::path::PathBuf;
use tokio::runtime::Runtime;

use tidev_session::session::{ToolCall, ToolExecutionResult};

use super::App;

/// Represents a pending sensitive file check for a tool call.
#[derive(Clone, Debug)]
pub(crate) struct PendingSensitiveFileCheck {
    pub tool_call: ToolCall,
    pub sensitive_path: PathBuf,
    pub workspace_root: PathBuf,
}

/// Dialog state for sensitive file confirmation.
#[derive(Clone, Debug)]
pub(crate) struct SensitiveFileDialogState {
    pub pending: PendingSensitiveFileCheck,
    pub current_index: usize,
    pub total: usize,
}

impl SensitiveFileDialogState {
    pub(crate) fn title(&self) -> String {
        format!(
            "Sensitive File Warning {} of {}",
            self.current_index, self.total
        )
    }

    pub(crate) fn path_display(&self) -> String {
        self.pending.sensitive_path.display().to_string()
    }

    pub(crate) fn workspace_display(&self) -> String {
        self.pending.workspace_root.display().to_string()
    }

    /// Calculate the height needed for the dialog.
    pub(crate) fn dialog_height(&self, _width: u16) -> u16 {
        8
    }
}

/// Represents the user's decision for a sensitive file check.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SensitiveFileDecision {
    /// Allow once (don't remember)
    AllowOnce,
    /// Allow and remember until tidev exits
    AllowUntilExit,
    /// Deny once (don't remember)
    DenyOnce,
    /// Deny and remember until tidev exits
    DenyUntilExit,
}

impl App {
    /// Check if a sensitive file path has been allowed in memory.
    pub(crate) fn is_sensitive_file_allowed(&self, path: &str) -> Option<bool> {
        self.sensitive_file_permissions.get(path).copied()
    }

    /// Store a sensitive file permission in memory.
    pub(crate) fn remember_sensitive_file_permission(&mut self, path: String, allowed: bool) {
        self.sensitive_file_permissions.insert(path, allowed);
    }

    /// Resolve the sensitive file dialog with the user's decision.
    fn resolve_sensitive_file_dialog(
        &mut self,
        decision: SensitiveFileDecision,
        runtime: &Runtime,
    ) -> Result<()> {
        let Some(dialog) = self.sensitive_file_dialog.take() else {
            return Ok(());
        };

        let allowed = matches!(
            decision,
            SensitiveFileDecision::AllowOnce | SensitiveFileDecision::AllowUntilExit
        );
        let remember = matches!(
            decision,
            SensitiveFileDecision::AllowUntilExit | SensitiveFileDecision::DenyUntilExit
        );

        // If remembering, store the permission in memory
        if remember {
            let path_pattern = dialog.pending.sensitive_path.display().to_string();
            self.remember_sensitive_file_permission(path_pattern, allowed);
        }

        if allowed {
            // Route through normal runtime flow via send_permission_approval
            self.sensitive_file_approved
                .insert(dialog.pending.tool_call.id.clone(), true);
            self.pending_tool_execution
                .as_mut()
                .unwrap()
                .add_ready(dialog.pending.tool_call);
            self.advance_pending_tool_execution();
            return self.process_pending_tool_execution(runtime);
        } else {
            // Record the denial
            let output = format!(
                "[User denied access] The path '{}' is listed in sensitive.txt.",
                dialog.pending.sensitive_path.display()
            );
            self.record_tool_result(dialog.pending.tool_call, ToolExecutionResult::new(output))?;
            self.advance_pending_tool_execution();
        }

        // Continue processing pending tools
        self.process_pending_tool_execution(runtime)
    }

    /// Handle keyboard input for the sensitive file dialog.
    pub(crate) fn handle_sensitive_file_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Some(SensitiveFileDecision::AllowOnce),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(SensitiveFileDecision::AllowUntilExit),
            KeyCode::Char('n') | KeyCode::Char('N') => Some(SensitiveFileDecision::DenyOnce),
            KeyCode::Char('d') | KeyCode::Char('D') => Some(SensitiveFileDecision::DenyUntilExit),
            KeyCode::Esc => Some(SensitiveFileDecision::DenyOnce),
            _ => None,
        };

        if let Some(decision) = decision {
            self.resolve_sensitive_file_dialog(decision, runtime)?;
        }

        Ok(())
    }
}
