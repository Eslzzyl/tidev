use crate::{app::Composer, config::ModelSummary};

#[derive(Clone, Debug)]
pub struct ModelPanelState {
    pub selected_index: usize,
    pub(crate) query: Composer,
}

impl Default for ModelPanelState {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelPanelState {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            query: Composer::new("Search connected models by provider or model name"),
        }
    }

    pub fn reset_selection(
        &mut self,
        items: &[ModelPanelItem],
        active_model: Option<(&str, &str)>,
    ) {
        if let Some((provider_id, model_id)) = active_model
            && let Some(index) = items.iter().position(|item| {
                matches!(item, ModelPanelItem::Model { summary }
                    if summary.provider_id == provider_id && summary.model_id == model_id)
            })
        {
            self.selected_index = index;
            return;
        }

        self.selected_index = first_selectable_index(items).unwrap_or(0);
    }

    pub fn move_selection(&mut self, items: &[ModelPanelItem], delta: isize) {
        let selectable = selectable_indices(items);
        if selectable.is_empty() {
            self.selected_index = 0;
            return;
        }

        let current_position = selectable
            .iter()
            .position(|index| *index == self.selected_index)
            .unwrap_or(0) as isize;
        let len = selectable.len() as isize;
        let next_position = (current_position + delta).rem_euclid(len) as usize;
        self.selected_index = selectable[next_position];
    }

    pub fn selected_model<'a>(&self, items: &'a [ModelPanelItem]) -> Option<&'a ModelSummary> {
        items
            .get(self.selected_index)
            .and_then(ModelPanelItem::as_model)
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
