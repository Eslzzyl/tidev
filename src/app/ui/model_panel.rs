use crate::{app::Composer, config::ModelSummary};

#[derive(Clone, Debug)]
pub struct ModelPanelTab {
    /// Agent type identifier, e.g. "general", "explorer", "librarian", etc.
    pub agent_type_str: String,
    /// Human-readable display name for the tab header.
    pub display_name: String,
    /// Currently selected item index within the filtered model list.
    pub selected_index: usize,
    /// Current model label shown on the tab, e.g. "openai/gpt-4o" or "<inherit>".
    pub current_label: String,
}

impl ModelPanelTab {
    pub fn new(agent_type_str: &str, display_name: &str, current_label: &str) -> Self {
        Self {
            agent_type_str: agent_type_str.to_string(),
            display_name: display_name.to_string(),
            selected_index: 0,
            current_label: current_label.to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModelPanelState {
    /// Shared search query across all tabs.
    pub(crate) query: Composer,
    /// Ordered list of tabs (General first, then Explorer, Librarian, etc.).
    pub tabs: Vec<ModelPanelTab>,
    /// Index into tabs for the currently active tab.
    pub selected_tab_index: usize,
}

impl Default for ModelPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelPanelState {
    pub fn new() -> Self {
        Self {
            query: Composer::new("Search connected models by provider or model name"),
            tabs: Vec::new(),
            selected_tab_index: 0,
        }
    }

    /// Resolve the active tab's selected_index given a filtered item list.
    pub(crate) fn current_tab_mut(&mut self) -> Option<&mut ModelPanelTab> {
        self.tabs.get_mut(self.selected_tab_index)
    }

    /// Resolve the active tab.
    pub(crate) fn current_tab(&self) -> Option<&ModelPanelTab> {
        self.tabs.get(self.selected_tab_index)
    }

    pub fn reset_selection(
        &mut self,
        items: &[ModelPanelItem],
        active_model: Option<(&str, &str)>,
    ) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        if let Some((provider_id, model_id)) = active_model
            && let Some(index) = items.iter().position(|item| {
                matches!(item, ModelPanelItem::Model { summary }
                    if summary.provider_id == provider_id && summary.model_id == model_id)
            })
        {
            tab.selected_index = index;
            return;
        }

        tab.selected_index = first_selectable_index(items).unwrap_or(0);
    }

    pub fn move_selection(&mut self, items: &[ModelPanelItem], delta: isize) {
        let Some(tab) = self.current_tab_mut() else {
            return;
        };
        let selectable = selectable_indices(items);
        if selectable.is_empty() {
            tab.selected_index = 0;
            return;
        }

        let current_position = selectable
            .iter()
            .position(|index| *index == tab.selected_index)
            .unwrap_or(0) as isize;
        let len = selectable.len() as isize;
        let next_position = (current_position + delta).rem_euclid(len) as usize;
        tab.selected_index = selectable[next_position];
    }

    pub fn selected_model<'a>(&self, items: &'a [ModelPanelItem]) -> Option<&'a ModelSummary> {
        let tab = self.current_tab()?;
        items
            .get(tab.selected_index)
            .and_then(ModelPanelItem::as_model)
    }

    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.selected_tab_index = index;
        }
    }

    /// Move to the next tab, wrapping around.
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.selected_tab_index = (self.selected_tab_index + 1) % self.tabs.len();
        }
    }

    /// Move to the previous tab, wrapping around.
    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.selected_tab_index = if self.selected_tab_index == 0 {
                self.tabs.len() - 1
            } else {
                self.selected_tab_index - 1
            };
        }
    }

    /// Is the active tab the "general" (main session) tab?
    pub fn is_general_tab(&self) -> bool {
        self.current_tab()
            .is_some_and(|t| t.agent_type_str == "general")
    }
}

#[derive(Clone, Debug)]
pub enum ModelPanelItem {
    ProviderHeader {
        provider_id: String,
        display_name: String,
    },
    Model {
        summary: ModelSummary,
    },
}

impl ModelPanelItem {
    pub fn as_model(&self) -> Option<&ModelSummary> {
        match self {
            Self::Model { summary } => Some(summary),
            Self::ProviderHeader { .. } => None,
        }
    }

    pub fn is_selectable(&self) -> bool {
        matches!(self, Self::Model { .. })
    }
}

pub fn first_selectable_index(items: &[ModelPanelItem]) -> Option<usize> {
    items.iter().position(ModelPanelItem::is_selectable)
}

pub fn selectable_indices(items: &[ModelPanelItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| item.is_selectable().then_some(index))
        .collect()
}

use super::App;

impl App {
    pub(crate) fn model_panel_items(&self, panel: &ModelPanelState) -> Vec<ModelPanelItem> {
        let query = panel.query.text().trim().to_ascii_lowercase();
        let mut items = Vec::new();
        let mut current_provider_id: Option<String> = None;

        for summary in self.config.connected_models(&self.auth) {
            if !model_panel_matches_query(&query, &summary) {
                continue;
            }

            if current_provider_id.as_deref() != Some(summary.provider_id.as_str()) {
                current_provider_id = Some(summary.provider_id.clone());
                items.push(ModelPanelItem::ProviderHeader {
                    provider_id: summary.provider_id.clone(),
                    display_name: summary.provider_display_name.clone(),
                });
            }

            items.push(ModelPanelItem::Model { summary });
        }

        items
    }
}

fn model_panel_matches_query(query: &str, summary: &ModelSummary) -> bool {
    if query.is_empty() {
        return true;
    }

    let provider_id = summary.provider_id.to_ascii_lowercase();
    let provider_display_name = summary.provider_display_name.to_ascii_lowercase();
    let model_id = summary.model_id.to_ascii_lowercase();
    let model_display_name = summary.model_display_name.to_ascii_lowercase();

    provider_id.contains(query)
        || provider_display_name.contains(query)
        || model_id.contains(query)
        || model_display_name.contains(query)
}
