//! Sandbox policy selection panel.
//!
//! Allows the user to view and change the sandbox policy at runtime.
//! This overrides the config file value for the current session only.
//!
//! In Plan mode the panel shows a locked notice (sandbox is always read-only).
//! In Build mode the panel offers workspace-write and full-access options.

use crate::sandbox::SandboxPolicy;
use crate::prompts::SessionMode;

use super::App;

/// Panel state for the sandbox policy selector.
#[derive(Clone, Debug)]
pub struct SandboxPanelState {
    /// Index into the list of available policies.
    pub selected_index: usize,
}

impl SandboxPanelState {
    pub fn new(current_mode: SessionMode) -> Self {
        // For Plan mode, show that sandbox is locked
        if current_mode.is_read_only() {
            return Self { selected_index: 0 };
        }

        // For Build mode, find the current policy index
        let current_policy = Self::build_items()
            .first()
            .map(|item| item.policy.label())
            .unwrap_or("");
        let current_index = Self::build_items()
            .iter()
            .position(|item| item.policy.label() == current_policy)
            .unwrap_or(0);

        Self {
            selected_index: current_index,
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        let len = Self::build_items().len() as isize;
        if len == 0 {
            return;
        }
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    /// Build the list of available policies for Build mode.
    pub fn build_items() -> Vec<PolicyItem> {
        vec![
            PolicyItem {
                policy: SandboxPolicy::WorkspaceWrite {
                    writable_roots: vec![],
                },
                label: "workspace-write",
                description: "Read access everywhere, writes restricted to workspace and /tmp",
            },
            PolicyItem {
                policy: SandboxPolicy::DangerFullAccess,
                label: "full access",
                description: "No filesystem restrictions",
            },
        ]
    }
}

/// A selectable sandbox policy item in the panel.
#[derive(Clone, Debug)]
pub struct PolicyItem {
    pub policy: SandboxPolicy,
    pub label: &'static str,
    pub description: &'static str,
}

impl App {
    /// Open the sandbox policy panel.
    pub(crate) fn open_sandbox_panel(&mut self) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.settings_panel = None;
        self.agents_panel = None;
        self.message_panel = None;
        self.rename_dialog = None;
        self.memory_panel = None;
        *self.balance_panel.lock().unwrap() = None;
        self.stats_panel = None;
        self.skills_panel = None;

        self.sandbox_panel = Some(SandboxPanelState::new(self.mode));
    }

    /// Apply the currently selected sandbox policy.
    pub(crate) fn apply_sandbox_policy(&mut self) {
        let Some(ref panel) = self.sandbox_panel else {
            return;
        };

        // In Plan mode, sandbox is locked — do nothing
        if self.mode.is_read_only() {
            self.last_notice = Some(
                "Sandbox is locked to read-only in Plan mode. Switch to Build to change.".to_string(),
            );
            self.sandbox_panel = None;
            return;
        }

        let items = SandboxPanelState::build_items();
        let Some(item) = items.get(panel.selected_index) else {
            return;
        };

        self.tools.set_sandbox_policy(Some(item.policy.clone()));

        self.last_notice = Some(format!(
            "Sandbox policy changed to: {}",
            item.label
        ));
        self.sandbox_panel = None;
    }
}
