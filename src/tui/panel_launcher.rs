/// Enum identifying which panel to open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanelAction {
    Agents,
    Balance,
    Mcp,
    Memory,
    Message,
    Model,
    Sandbox,
    Session,
    Settings,
    Skills,
    Stats,
    Theme,
}

/// A registered panel entry in the launcher.
#[derive(Clone, Debug)]
pub(crate) struct PanelEntry {
    pub description: &'static str,
    pub action: PanelAction,
}

/// All panels that the launcher can open.
pub(crate) static PANEL_ENTRIES: &[PanelEntry] = &[
    PanelEntry {
        description: "Switch AI model provider",
        action: PanelAction::Model,
    },
    PanelEntry {
        description: "Manage chat sessions",
        action: PanelAction::Session,
    },
    PanelEntry {
        description: "Change color theme",
        action: PanelAction::Theme,
    },
    PanelEntry {
        description: "Configure application settings",
        action: PanelAction::Settings,
    },
    PanelEntry {
        description: "Browse stored memories",
        action: PanelAction::Memory,
    },
    PanelEntry {
        description: "Manage MCP servers",
        action: PanelAction::Mcp,
    },
    PanelEntry {
        description: "List available sub-agent types",
        action: PanelAction::Agents,
    },
    PanelEntry {
        description: "Browse and preview available skills",
        action: PanelAction::Skills,
    },
    PanelEntry {
        description: "View and change sandbox policy for shell commands",
        action: PanelAction::Sandbox,
    },
    PanelEntry {
        description: "Token usage statistics",
        action: PanelAction::Stats,
    },
    PanelEntry {
        description: "API provider balance",
        action: PanelAction::Balance,
    },
    PanelEntry {
        description: "View message details in the current session",
        action: PanelAction::Message,
    },
];

#[derive(Clone, Debug, Default)]
pub(crate) struct PanelLauncherState {
    pub visible: bool,
    pub query: String,
    pub selected_index: usize,
    /// Filtered entry references, sorted by relevance.
    pub filtered: Vec<&'static PanelEntry>,
}

impl PanelLauncherState {
    /// Re‑filter entries based on the current query.
    pub fn sync(&mut self) {
        let query = self.query.trim().to_ascii_lowercase();
        let previous = self.selected_action();

        let mut candidates: Vec<&'static PanelEntry> = if query.is_empty() {
            PANEL_ENTRIES.iter().collect()
        } else {
            PANEL_ENTRIES
                .iter()
                .filter(|entry| {
                    let desc = entry.description.to_ascii_lowercase();
                    desc.contains(&query)
                })
                .collect()
        };

        // Stable sort: maintain the original order for equal scores.
        candidates.sort_by(|a, b| {
            let a_score = fuzzy_score(a.description, &query);
            let b_score = fuzzy_score(b.description, &query);
            b_score.cmp(&a_score)
        });

        self.filtered = candidates;

        // Restore previous selection if still present.
        if let Some(prev) = previous
            && let Some(idx) = self.filtered.iter().position(|e| e.action == prev)
        {
            self.selected_index = idx;
            return;
        }
        self.selected_index = 0;
    }

    pub fn clear(&mut self) {
        self.visible = false;
        self.query.clear();
        self.selected_index = 0;
        self.filtered.clear();
    }

    /// Open the launcher with an empty query.
    pub fn open(&mut self) {
        self.visible = true;
        self.query.clear();
        self.selected_index = 0;
        self.sync();
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        let current = self.selected_index as isize;
        let next = (current + delta).rem_euclid(len);
        self.selected_index = next as usize;
    }

    pub fn selected(&self) -> Option<&'static PanelEntry> {
        self.filtered.get(self.selected_index).copied()
    }

    fn selected_action(&self) -> Option<PanelAction> {
        self.selected().map(|e| e.action)
    }

    /// Consume the currently selected action (clearing the launcher).
    /// Returns `None` if there is no selection.
    pub fn take_selected_action(&mut self) -> Option<PanelAction> {
        let action = self.selected().map(|e| e.action);
        self.clear();
        action
    }
}

/// Simple substring + prefix scoring to match the existing palette style.
fn fuzzy_score(description: &str, query: &str) -> i32 {
    if query.is_empty() {
        return 0;
    }
    let lower = description.to_ascii_lowercase();
    if lower == query {
        10_000
    } else if lower.starts_with(query) {
        8_000
    } else if let Some(pos) = lower.find(query) {
        4_500 - (pos as i32 * 20)
    } else {
        -1
    }
}
