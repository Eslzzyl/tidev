//! ModelPanel component — model selection panel with per-agent tabs,
//! search filtering, and thinking level sub-menu.
//!
//! Mirrors the old `tidev_tui::ui::model_panel` module with a self-contained
//! Component implementation.

use anyhow::Result;
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};
use tidev_config::ThinkingMatcher;
use tidev_config::ThinkingLevelType;
use tidev_config::auth::{ActiveModel, ModelSummary};

use crate::action::{Action, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// Thinking level helpers
// ---------------------------------------------------------------------------

/// Return available thinking level option strings for the model at `index`.
fn thinking_options_for_model(items: &[ModelPanelItem], index: usize) -> Vec<String> {
    let Some(ModelPanelItem::Model { summary }) = items.get(index) else {
        return vec![];
    };
    // Match against request_model_id first, then display_name — model_id
    // (the TOML key) is an arbitrary user choice and may use dashes instead
    // of dots (e.g. "gpt-5-6-luna" vs "gpt-5.6-luna").
    let id = if !summary.request_model_id.is_empty() {
        &summary.request_model_id
    } else {
        &summary.model_display_name
    };
    ThinkingMatcher::supported_levels(id)
        .iter()
        .map(|tl| tl.to_string())
        .collect()
}

fn first_selectable_index(items: &[ModelPanelItem]) -> Option<usize> {
    items.iter().position(ModelPanelItem::is_selectable)
}

fn selectable_indices(items: &[ModelPanelItem]) -> Vec<usize> {
    items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| item.is_selectable().then_some(i))
        .collect()
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct ModelPanelTab {
    pub agent_type_str: String,
    pub display_name: String,
    pub selected_index: usize,
    pub current_label: String,
    pub thinking_level_expanded: bool,
    pub thinking_level_index: usize,
}

impl ModelPanelTab {
    pub fn new(agent_type_str: &str, display_name: &str, current_label: &str) -> Self {
        Self {
            agent_type_str: agent_type_str.to_string(),
            display_name: display_name.to_string(),
            selected_index: 0,
            current_label: current_label.to_string(),
            thinking_level_expanded: false,
            thinking_level_index: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum ModelPanelItem {
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
            Self::Model { summary, .. } => Some(summary),
            Self::ProviderHeader { .. } => None,
        }
    }

    pub fn is_selectable(&self) -> bool {
        matches!(self, Self::Model { .. })
    }
}

// ---------------------------------------------------------------------------
// ModelPanel component
// ---------------------------------------------------------------------------

pub(crate) struct ModelPanel {
    tabs: Vec<ModelPanelTab>,
    selected_tab_index: usize,
    query: String,
    items_cache: Vec<ModelPanelItem>,
    /// Snapshot of all connected models (for local filtering without runtime).
    connected_models: Vec<ModelSummary>,
    /// Snapshot of the current active model (for selection reset).
    active_model: ActiveModel,
}

impl ModelPanel {
    /// Create a new ModelPanel with pre-built tabs, connected models, and active model.
    pub(crate) fn new(
        tabs: Vec<ModelPanelTab>,
        connected_models: Vec<ModelSummary>,
        active_model: ActiveModel,
    ) -> Self {
        let mut panel = Self {
            tabs,
            selected_tab_index: 0,
            query: String::new(),
            items_cache: Vec::new(),
            connected_models,
            active_model,
        };
        panel.rebuild_items_from_cache();
        panel.reset_selection_for_current_tab();
        panel
    }

    // ── Tab accessors ──

    fn current_tab(&self) -> Option<&ModelPanelTab> {
        self.tabs.get(self.selected_tab_index)
    }

    fn current_tab_mut(&mut self) -> Option<&mut ModelPanelTab> {
        let idx = self.selected_tab_index;
        self.tabs.get_mut(idx)
    }

    fn is_general_tab(&self) -> bool {
        self.tabs
            .get(self.selected_tab_index)
            .is_some_and(|t| t.agent_type_str == "general")
    }

    // ── Items ──

    /// Rebuild `items_cache` from `connected_models`, filtered by `query`.
    fn rebuild_items_from_cache(&mut self) {
        let query = self.query.trim().to_ascii_lowercase();
        let mut items = Vec::new();
        let mut current_provider_id: Option<String> = None;

        for summary in &self.connected_models {
            if !query.is_empty() {
                let provider_id = summary.provider_id.to_ascii_lowercase();
                let provider_display_name = summary.provider_display_name.to_ascii_lowercase();
                let model_id = summary.model_id.to_ascii_lowercase();
                let model_display_name = summary.model_display_name.to_ascii_lowercase();
                let matches = provider_id.contains(&query)
                    || provider_display_name.contains(&query)
                    || model_id.contains(&query)
                    || model_display_name.contains(&query);
                if !matches {
                    continue;
                }
            }
            if current_provider_id.as_deref() != Some(summary.provider_id.as_str()) {
                current_provider_id = Some(summary.provider_id.clone());
                items.push(ModelPanelItem::ProviderHeader {
                    provider_id: summary.provider_id.clone(),
                    display_name: summary.provider_display_name.clone(),
                });
            }
            items.push(ModelPanelItem::Model {
                summary: summary.clone(),
            });
        }

        self.items_cache = items;
    }

    /// Reset the current tab's selection.
    fn reset_selection_for_current_tab(&mut self) {
        let (agent_type_str, current_label) = match self.tabs.get(self.selected_tab_index) {
            Some(tab) => (tab.agent_type_str.clone(), tab.current_label.clone()),
            None => return,
        };
        let is_general = agent_type_str == "general";

        let tab_index = if is_general {
            self.items_cache.iter().position(|item| {
                matches!(item, ModelPanelItem::Model { summary, .. }
                    if summary.provider_id == self.active_model.provider_id
                    && summary.model_id == self.active_model.model_id)
            })
        } else {
            let label = current_label.to_ascii_lowercase();
            if label != "<inherit>" && !label.is_empty() {
                self.items_cache.iter().position(|item| {
                    matches!(item, ModelPanelItem::Model { summary, .. }
                        if summary.label().to_ascii_lowercase() == label)
                })
            } else {
                // "<inherit>": fall back to the general tab's active model
                self.items_cache.iter().position(|item| {
                    matches!(item, ModelPanelItem::Model { summary, .. }
                        if summary.provider_id == self.active_model.provider_id
                            && summary.model_id == self.active_model.model_id)
                })
            }
        };

        if let Some(tab) = self.tabs.get_mut(self.selected_tab_index) {
            tab.selected_index =
                tab_index.unwrap_or_else(|| first_selectable_index(&self.items_cache).unwrap_or(0));
        }
    }

    // ── Navigation ──

    fn move_selection(&mut self, delta: isize) {
        let (selected_index, thinking_expanded, tl_idx) =
            match self.tabs.get(self.selected_tab_index) {
                Some(tab) => (
                    tab.selected_index,
                    tab.thinking_level_expanded,
                    tab.thinking_level_index,
                ),
                None => return,
            };

        // If thinking level is expanded, cycle through thinking options
        if thinking_expanded {
            let options = thinking_options_for_model(&self.items_cache, selected_index);
            if !options.is_empty() {
                if let Some(tab) = self.tabs.get_mut(self.selected_tab_index) {
                    let len = options.len() as isize;
                    tab.thinking_level_index = ((tl_idx as isize + delta).rem_euclid(len)) as usize;
                }
                return;
            }
        }

        let selectable = selectable_indices(&self.items_cache);
        if selectable.is_empty() {
            if let Some(tab) = self.tabs.get_mut(self.selected_tab_index) {
                tab.selected_index = 0;
            }
            return;
        }

        let current_position = selectable
            .iter()
            .position(|i| *i == selected_index)
            .unwrap_or(0) as isize;
        let len = selectable.len() as isize;
        let next_position = (current_position + delta).rem_euclid(len) as usize;
        if let Some(tab) = self.tabs.get_mut(self.selected_tab_index) {
            tab.selected_index = selectable[next_position];
        }
    }

    fn selected_model(&self) -> Option<&ModelSummary> {
        let selected_index = self.tabs.get(self.selected_tab_index)?.selected_index;
        self.items_cache
            .get(selected_index)
            .and_then(ModelPanelItem::as_model)
    }

    fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.selected_tab_index = (self.selected_tab_index + 1) % self.tabs.len();
        }
    }

    fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.selected_tab_index = if self.selected_tab_index == 0 {
                self.tabs.len() - 1
            } else {
                self.selected_tab_index - 1
            };
        }
    }
}

impl Component for ModelPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(-1);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(1);
                None
            }
            KeyCode::Enter => {
                let is_expanded = self
                    .tabs
                    .get(self.selected_tab_index)
                    .is_some_and(|t| t.thinking_level_expanded);

                if is_expanded {
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::ModelPanel,
                    )))
                } else {
                    let summary = self.selected_model().cloned();
                    if let Some(selected_summary) = summary {
                        let model_pos = self
                            .items_cache
                            .iter()
                            .position(|item| {
                                matches!(item, ModelPanelItem::Model { summary: s, .. }
                                    if s.provider_id == selected_summary.provider_id
                                    && s.model_id == selected_summary.model_id)
                            })
                            .unwrap_or(
                                self.tabs
                                    .get(self.selected_tab_index)
                                    .map(|t| t.selected_index)
                                    .unwrap_or(0),
                            );
                        let tl_options = thinking_options_for_model(&self.items_cache, model_pos);

                        if tl_options.is_empty() {
                            Some(Action::Overlay(OverlayAction::Close(
                                OverlayKind::ModelPanel,
                            )))
                        } else {
                            if let Some(t) = self.tabs.get_mut(self.selected_tab_index) {
                                t.thinking_level_expanded = true;
                                // Initialize index to match the current thinking level
                                let current_tl = self
                                    .active_model
                                    .thinking_level
                                    .to_string()
                                    .to_ascii_lowercase();
                                t.thinking_level_index = tl_options
                                    .iter()
                                    .position(|opt| opt.to_ascii_lowercase() == current_tl)
                                    .unwrap_or(0);
                            }
                            None
                        }
                    } else {
                        None
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                // If thinking level is expanded, collapse first
                if self
                    .tabs
                    .get(self.selected_tab_index)
                    .is_some_and(|t| t.thinking_level_expanded)
                {
                    if let Some(t) = self.tabs.get_mut(self.selected_tab_index) {
                        t.thinking_level_expanded = false;
                    }
                    None
                } else {
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::ModelPanel,
                    )))
                }
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.next_tab();
                self.rebuild_items_from_cache();
                self.reset_selection_for_current_tab();
                None
            }
            KeyCode::BackTab | KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.prev_tab();
                self.rebuild_items_from_cache();
                self.reset_selection_for_current_tab();
                None
            }
            KeyCode::Backspace => {
                if !self.query.is_empty() {
                    self.query.pop();
                    self.rebuild_items_from_cache();
                    self.reset_selection_for_current_tab();
                }
                None
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.query.push(ch);
                self.rebuild_items_from_cache();
                self.reset_selection_for_current_tab();
                None
            }
            _ => None,
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay = centered_rect(area.width.min(104), area.height.min(34), area);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                self.move_selection(-1);
                None
            }
            MouseEventKind::ScrollDown => {
                self.move_selection(1);
                None
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // If thinking expanded, clicking on a thinking option selects it
                if self
                    .current_tab()
                    .is_some_and(|t| t.thinking_level_expanded)
                {
                    // Simple: just confirm the current selection
                    return Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::ModelPanel,
                    )));
                }

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                // header rows = title(1) + tab_bar(1) + instruction(2) + search(3) = 7
                let header_rows = 7u16;
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < header_rows {
                    return Some(Action::Noop);
                }
                let first_row = (local_y - header_rows) as usize;

                // Walk the rendered rows to find which selectable item was clicked.
                // Each selectable item renders as 1 row (+ N thinking sub-rows if expanded).
                let selectable = selectable_indices(&self.items_cache);
                let mut rendered_row = 0usize;
                for &item_idx in selectable.iter() {
                    if rendered_row == first_row {
                        if let Some(t) = self.current_tab_mut() {
                            t.selected_index = item_idx;
                        }
                        // Same as Enter
                        return Some(Action::Overlay(OverlayAction::Close(
                            OverlayKind::ModelPanel,
                        )));
                    }
                    rendered_row += 1;

                    // If this item has thinking expanded, skip its sub-rows
                    let is_expanded = self
                        .current_tab()
                        .is_some_and(|t| t.thinking_level_expanded && t.selected_index == item_idx);
                    if is_expanded {
                        let tl_options = thinking_options_for_model(&self.items_cache, item_idx);
                        rendered_row += tl_options.len();
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::ModelPanel)) => {
                // Save the selected model for the current tab
                let tab_info = self.tabs.get(self.selected_tab_index).map(|tab| {
                    (
                        tab.agent_type_str.clone(),
                        tab.selected_index,
                        tab.thinking_level_expanded,
                        tab.thinking_level_index,
                    )
                });
                let Some((agent_type_str, selected_index, thinking_expanded, tl_idx)) = tab_info
                else {
                    return vec![];
                };
                let Some(summary) = self
                    .items_cache
                    .get(selected_index)
                    .and_then(ModelPanelItem::as_model)
                    .cloned()
                else {
                    return vec![];
                };

                let tl = if thinking_expanded {
                    let options = thinking_options_for_model(&self.items_cache, selected_index);
                    if !options.is_empty() {
                        Some(options[tl_idx % options.len()].to_string())
                    } else {
                        None
                    }
                } else {
                    None
                };

                if agent_type_str == "general" {
                    // Persist the selection to config.
                    ctx.runtime.update_config(|cfg| {
                        cfg.default_provider = summary.provider_id.clone();
                        cfg.default_model = summary.model_id.clone();
                    });
                    // Resolve the full ActiveModel and update runtime so that
                    // runtime.active_model() returns the correct model immediately.
                    let config = ctx.runtime.config();
                    let auth = ctx.runtime.auth();
                    if let Ok(model) =
                        config.resolve_model_by_ids(&auth, &summary.provider_id, &summary.model_id)
                    {
                        ctx.runtime.set_active_model(model);
                    }
                } else {
                    // Set agent-specific model
                    let at = agent_type_str.clone();
                    let label = summary.label();
                    ctx.runtime.update_config(|cfg| {
                        cfg.agent.models.insert(at.clone(), label.clone());
                        if tl.is_none() {
                            cfg.agent.thinking_levels.remove(&at);
                        }
                    });
                }

                // Save thinking level
                if let Some(tl_str) = &tl {
                    if agent_type_str == "general" {
                        let _ = ctx.runtime.set_model_thinking_level(
                            &summary.provider_id,
                            &summary.model_id,
                            tl_str,
                        );
                    } else {
                        ctx.runtime.update_config(|cfg| {
                            cfg.agent
                                .thinking_levels
                                .insert(agent_type_str.clone(), tl_str.clone());
                        });
                    }
                }

                let _ = ctx.runtime.save_config();
                vec![]
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(rect.width.min(104), rect.height.min(34), rect);
        frame.render_widget(Clear, overlay);
        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // tab bar
            Constraint::Length(2), // instruction
            Constraint::Length(3), // search box
            Constraint::Min(8),    // model list
            Constraint::Length(1), // footer
        ])
        .split(inner);

        // ── Title ──
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Select model ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // ── Tab bar ──
        let tab_spans: Vec<Span> = self
            .tabs
            .iter()
            .enumerate()
            .flat_map(|(idx, tab)| {
                let is_active = idx == self.selected_tab_index;
                let tab_style = if is_active {
                    Style::default()
                        .fg(palette.selection_fg)
                        .bg(palette.selection_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.muted)
                };
                let mut spans = vec![Span::styled(format!(" {} ", tab.display_name), tab_style)];
                if idx + 1 < self.tabs.len() {
                    spans.push(Span::styled(" │ ", Style::default().fg(palette.border)));
                }
                spans
            })
            .collect();

        frame.render_widget(
            Paragraph::new(Line::from(tab_spans))
                .style(Style::default().bg(palette.panel_alt))
                .alignment(ratatui::layout::Alignment::Left),
            sections[1],
        );

        // ── Instruction ──
        let instruction = if self
            .current_tab()
            .is_some_and(|t| t.thinking_level_expanded)
        {
            "Select a thinking level. Enter to confirm, Esc to collapse."
        } else {
            "Select a model for this agent. Enter to save, Esc to close."
        };
        frame.render_widget(
            Paragraph::new(instruction)
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        // ── Search box ──
        let search_style = Style::default().bg(palette.panel_alt);
        let prefix = " Search models: ";
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.muted)),
                Span::styled(&self.query, Style::default().fg(palette.text)),
            ]))
            .style(search_style),
            sections[3],
        );
        frame.set_cursor_position((
            sections[3].x + prefix.width() as u16 + self.query.as_str().width() as u16,
            sections[3].y,
        ));

        // ── Model list ──
        let items = &self.items_cache;
        if items.is_empty() {
            frame.render_widget(
                Paragraph::new("No connected models match this search.")
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                sections[4],
            );
        } else {
            let mut rows: Vec<ListItem> = Vec::new();

            for (index, item) in items.iter().enumerate() {
                match item {
                    ModelPanelItem::ProviderHeader {
                        display_name,
                        provider_id,
                    } => {
                        rows.push(ListItem::new(Line::from(vec![
                            Span::styled(
                                display_name.to_string(),
                                Style::default()
                                    .fg(palette.accent)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("({provider_id})"),
                                Style::default().fg(palette.muted),
                            ),
                        ])));
                    }
                    ModelPanelItem::Model { summary } => {
                        let is_selected = self
                            .current_tab()
                            .is_some_and(|t| t.selected_index == index);

                        // Active checkmark: show if this model is the currently
                        // configured model for the active tab.
                        let is_active = self.current_tab().is_some_and(|tab| {
                            if tab.agent_type_str == "general" {
                                summary.provider_id == self.active_model.provider_id
                                    && summary.model_id == self.active_model.model_id
                            } else {
                                let label = tab.current_label.to_ascii_lowercase();
                                if label == "<inherit>" || label.is_empty() {
                                    summary.provider_id == self.active_model.provider_id
                                        && summary.model_id == self.active_model.model_id
                                } else {
                                    summary.label().to_ascii_lowercase() == label
                                }
                            }
                        });

                        let active_marker = if is_active {
                            Span::styled("✓ ", Style::default().fg(palette.accent))
                        } else {
                            Span::raw("  ")
                        };

                        let mut spans = vec![
                            active_marker,
                            Span::styled(
                                summary.model_display_name.clone(),
                                Style::default()
                                    .fg(palette.text)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(
                                format!("({})", summary.model_id),
                                Style::default().fg(palette.muted),
                            ),
                        ];

                        // Thinking level tag (expanded preview + persistent display)
                        let tl_tag: Option<String> = {
                            // 1. Expanded: show the selected thinking option preview
                            if is_selected
                                && self
                                    .current_tab()
                                    .is_some_and(|t| t.thinking_level_expanded)
                            {
                                let tl_options = thinking_options_for_model(items, index);
                                if !tl_options.is_empty() {
                                    let tl_idx = self
                                        .current_tab()
                                        .map(|t| t.thinking_level_index)
                                        .unwrap_or(0);
                                    let tl = &tl_options[tl_idx % tl_options.len()];
                                    Some(ThinkingLevelType::from_string(tl).display_name().to_string())
                                } else {
                                    None
                                }
                            // 2. Active model on General tab: show current thinking level
                            } else if is_active
                                && self.is_general_tab()
                                && self.active_model.thinking_level.is_supported()
                            {
                                let name = self.active_model.thinking_level.display_name();
                                if name.is_empty() {
                                    None
                                } else {
                                    Some(name.to_string())
                                }
                            } else {
                                None
                            }
                        };
                        if let Some(ref tag) = tl_tag {
                            spans.push(Span::raw("  "));
                            spans.push(Span::styled(
                                format!("[{}]", tag),
                                Style::default().fg(palette.accent_soft),
                            ));
                        }

                        rows.push(ListItem::new(Line::from(spans)));

                        // If thinking level is expanded, render sub-options
                        if is_selected
                            && self
                                .current_tab()
                                .is_some_and(|t| t.thinking_level_expanded)
                        {
                            let tl_options = thinking_options_for_model(items, index);
                            if !tl_options.is_empty() {
                                let tl_idx = self
                                    .current_tab()
                                    .map(|t| t.thinking_level_index)
                                    .unwrap_or(0);
                                for (oi, opt) in tl_options.iter().enumerate() {
                                    let is_tl_selected = oi == tl_idx % tl_options.len();
                                    let level_name = ThinkingLevelType::from_string(opt).display_name().to_string();
                                    let bullet = if is_tl_selected { " ● " } else { " ○ " };
                                    let tl_style = if is_tl_selected {
                                        Style::default()
                                            .fg(palette.accent)
                                            .add_modifier(Modifier::BOLD)
                                    } else {
                                        Style::default().fg(palette.muted)
                                    };
                                    rows.push(ListItem::new(Line::from(vec![
                                        Span::raw("    "),
                                        Span::styled(bullet, tl_style),
                                        Span::styled(level_name, tl_style),
                                    ])));
                                }
                            }
                        }
                    }
                }
            }

            let sel = self
                .current_tab()
                .map(|t| t.selected_index)
                .unwrap_or(0)
                .min(items.len().saturating_sub(1));
            let mut state = ratatui::widgets::ListState::default();
            state.select(Some(sel.min(rows.len().saturating_sub(1))));

            // When thinking level is expanded, adjust the scroll offset so
            // that the thinking sub-rows are visible even when the selected
            // model is near the bottom of the visible area.
            let is_tl_expanded = self
                .current_tab()
                .is_some_and(|t| t.thinking_level_expanded);
            if is_tl_expanded {
                let tl_options = thinking_options_for_model(items, sel);
                if !tl_options.is_empty() {
                    let num_tl = tl_options.len();
                    let visible_height = sections[4].height as usize;
                    // The last thinking sub-row in `rows` is at sel + num_tl.
                    // Ensure offset + visible_height > sel + num_tl.
                    let min_offset = sel
                        .saturating_add(num_tl)
                        .saturating_add(1)
                        .saturating_sub(visible_height);
                    // Keep offset <= sel so the model row itself stays visible.
                    let offset = min_offset.min(sel);
                    *state.offset_mut() = offset;
                }
            }

            let list = List::new(rows)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD),
                );

            frame.render_stateful_widget(list, sections[4], &mut state);
        }

        // ── Footer ──
        let is_expanded = self
            .current_tab()
            .is_some_and(|t| t.thinking_level_expanded);
        let footer = if is_expanded {
            "Enter confirm thinking · ↑ ↓ select level · Esc collapse"
        } else {
            "Enter apply / expand thinking · Tab switch tab · Esc close"
        };
        frame.render_widget(
            Paragraph::new(footer)
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[5],
        );
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        10
    }

    fn blocks_input(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_config::auth::ModelSummary;

    fn make_model_item(
        model_id: &str,
        request_model_id: &str,
        model_display_name: &str,
    ) -> ModelPanelItem {
        ModelPanelItem::Model {
            summary: ModelSummary {
                provider_id: "test".into(),
                provider_display_name: "Test".into(),
                model_id: model_id.into(),
                request_model_id: request_model_id.into(),
                model_display_name: model_display_name.into(),
                base_url: "https://test.com".into(),
                context_window: 128000,
                max_output_tokens: 4096,
            },
        }
    }

    fn assert_options(items: &[ModelPanelItem], index: usize, expected: &[&str]) {
        let opts = thinking_options_for_model(items, index);
        let opts: Vec<&str> = opts.iter().map(|s| s.as_str()).collect();
        assert_eq!(opts, expected);
    }

    // -----------------------------------------------------------------------
    // GPT-5.6
    // -----------------------------------------------------------------------

    #[test]
    fn gpt_5_6_via_request_model_id() {
        // request_model_id contains "5.6" → should include Max
        let item = make_model_item("gpt-5-6-luna", "gpt-5.6-luna", "GPT-5.6 Luna");
        let items = vec![item];
        assert_options(
            &items,
            0,
            &[
                "gpt5:off",
                "gpt5:low",
                "gpt5:medium",
                "gpt5:high",
                "gpt5:xhigh",
                "gpt5:max",
            ],
        );
    }

    #[test]
    fn gpt_5_6_via_display_name_fallback() {
        // Empty request_model_id → falls back to display_name "GPT-5.6 Luna"
        let item = make_model_item("gpt-5-6-luna", "", "GPT-5.6 Luna");
        let items = vec![item];
        assert_options(
            &items,
            0,
            &[
                "gpt5:off",
                "gpt5:low",
                "gpt5:medium",
                "gpt5:high",
                "gpt5:xhigh",
                "gpt5:max",
            ],
        );
    }

    // -----------------------------------------------------------------------
    // Older GPT-5 (5.4, 5.5)
    // -----------------------------------------------------------------------

    #[test]
    fn gpt_5_4_no_max() {
        let item = make_model_item("gpt-5-4", "gpt-5.4", "GPT-5.4");
        let items = vec![item];
        assert_options(
            &items,
            0,
            &[
                "gpt5:off",
                "gpt5:low",
                "gpt5:medium",
                "gpt5:high",
                "gpt5:xhigh",
            ],
        );
    }

    #[test]
    fn gpt_5_5_no_max() {
        let item = make_model_item("gpt-5-5", "gpt-5.5", "GPT-5.5");
        let items = vec![item];
        assert_options(
            &items,
            0,
            &[
                "gpt5:off",
                "gpt5:low",
                "gpt5:medium",
                "gpt5:high",
                "gpt5:xhigh",
            ],
        );
    }

    // -----------------------------------------------------------------------
    // Other providers
    // -----------------------------------------------------------------------

    #[test]
    fn deepseek_v4() {
        let item = make_model_item(
            "deepseek-v4-flash",
            "deepseek-v4-flash",
            "DeepSeek V4 Flash",
        );
        let items = vec![item];
        assert_options(
            &items,
            0,
            &["deepseek:off", "deepseek:high", "deepseek:max"],
        );
    }

    #[test]
    fn unsupported_model_returns_empty() {
        let item = make_model_item("claude-opus-4-8", "claude-opus-4-8", "Claude Opus 4.8");
        let items = vec![item];
        let opts = thinking_options_for_model(&items, 0);
        assert!(opts.is_empty());
    }

    #[test]
    fn out_of_range_index_returns_empty() {
        let item = make_model_item("gpt-5.6-sol", "gpt-5.6-sol", "GPT-5.6 Sol");
        let items = vec![item];
        let opts = thinking_options_for_model(&items, 1);
        assert!(opts.is_empty());
    }

    #[test]
    fn provider_header_returns_empty() {
        let items = vec![ModelPanelItem::ProviderHeader {
            provider_id: "test".into(),
            display_name: "Test".into(),
        }];
        let opts = thinking_options_for_model(&items, 0);
        assert!(opts.is_empty());
    }
}
