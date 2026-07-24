//! PanelLauncher — quick panel switcher opened via Ctrl+P.
//!
//! Shows a filterable list of available panels. Selecting one closes the
//! launcher and opens the chosen panel.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::action::{Action, OverlayAction, OverlayKind, PanelAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Panel entries
// ---------------------------------------------------------------------------

struct PanelEntry {
    description: &'static str,
    action: PanelAction,
}

static PANEL_ENTRIES: &[PanelEntry] = &[
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
        description: "List available sub-agent types",
        action: PanelAction::Agents,
    },
    PanelEntry {
        description: "Browse and preview available skills",
        action: PanelAction::Skills,
    },
    PanelEntry {
        description: "View message details in the current session",
        action: PanelAction::Message,
    },
    PanelEntry {
        description: "Search web / providers",
        action: PanelAction::Search,
    },
];

// ---------------------------------------------------------------------------
// Fuzzy scoring helpers
// ---------------------------------------------------------------------------

/// Simple fuzzy score: exact match > starts_with > contains.
fn fuzzy_score(query: &str, text: &str) -> i32 {
    let query = query.to_ascii_lowercase();
    let text = text.to_ascii_lowercase();
    if text == query {
        100
    } else if text.starts_with(&query) {
        50
    } else if text.contains(&query) {
        10
    } else {
        0
    }
}

/// Return the description text that will be searched.
fn entry_search_text(entry: &PanelEntry) -> String {
    format!("{:?} {}", entry.action, entry.description)
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

pub(crate) struct PanelLauncher {
    visible: bool,
    query: String,
    selected_index: usize,
    filtered: Vec<&'static PanelEntry>,
}

impl PanelLauncher {
    pub(crate) fn new() -> Self {
        Self {
            visible: true,
            query: String::new(),
            selected_index: 0,
            filtered: Vec::new(),
        }
    }

    fn sync(&mut self) {
        if self.query.is_empty() {
            self.filtered = PANEL_ENTRIES.iter().collect();
        } else {
            let mut scored: Vec<(&'static PanelEntry, i32)> = PANEL_ENTRIES
                .iter()
                .map(|e| {
                    let score = fuzzy_score(&self.query, &entry_search_text(e));
                    (e, score)
                })
                .filter(|(_, s)| *s > 0)
                .collect();
            scored.sort_by_key(|b| std::cmp::Reverse(b.1));
            self.filtered = scored.into_iter().map(|(e, _)| e).collect();
        }
        self.selected_index = self
            .selected_index
            .min(self.filtered.len().saturating_sub(1));
    }
}

impl Component for PanelLauncher {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        self.sync();
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match key.code {
            KeyCode::Esc => Some(Action::Overlay(OverlayAction::Close(
                OverlayKind::PanelLauncher,
            ))),
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.filtered.is_empty() {
                    self.selected_index = self
                        .selected_index
                        .saturating_sub(1)
                        .min(self.filtered.len().saturating_sub(1));
                }
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.filtered.is_empty() {
                    self.selected_index = self
                        .selected_index
                        .saturating_add(1)
                        .min(self.filtered.len().saturating_sub(1));
                }
                None
            }
            KeyCode::Enter => {
                if self.filtered.get(self.selected_index).is_some() {
                    // Close self, then open the chosen panel
                    // Return two actions: first close, then the panel action
                    // Close will come first because overlay stack pops,
                    // and the panel open will be in update()
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::PanelLauncher,
                    )))
                } else {
                    None
                }
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(c);
                self.sync();
                None
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.sync();
                None
            }
            _ => None,
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::PanelLauncher)) => {
                // Map the selected action to an OverlayAction::Open
                let panel_action = self.filtered.get(self.selected_index).map(|e| e.action);
                if let Some(action) = panel_action {
                    let kind = match action {
                        PanelAction::Model => OverlayKind::ModelPanel,
                        PanelAction::Session => OverlayKind::SessionPanel,
                        PanelAction::Theme => OverlayKind::ThemePanel,
                        PanelAction::Settings => OverlayKind::SettingsPanel,
                        PanelAction::Agents => OverlayKind::AgentsPanel,
                        PanelAction::Skills => OverlayKind::SkillsPanel,
                        PanelAction::Message => OverlayKind::MessagePanel,
                        PanelAction::Search => OverlayKind::SearchPanel,
                    };
                    vec![Action::Overlay(OverlayAction::Open(kind))]
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        if !self.visible {
            return;
        }

        let palette = ctx.palette;
        let width = rect.width.min(56);
        let height = (self.filtered.len() as u16 + 3).min(18).saturating_add(2);
        let overlay = centered_rect(width, height, rect);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Search bar
        let search_text = if self.query.is_empty() {
            "  Type to filter panels...".to_string()
        } else {
            format!("  {}", self.query)
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                search_text,
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        if !self.query.is_empty() {
            frame.set_cursor_position((inner.x + 2 + self.query.as_str().width() as u16, inner.y));
        }

        // Divider
        let divider_y = inner.y + 1;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, divider_y, inner.width, 1),
        );

        // List
        let list_area = Rect::new(
            inner.x,
            inner.y + 2,
            inner.width,
            inner.height.saturating_sub(2),
        );
        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .map(|entry| ListItem::new(Line::from(Span::raw(entry.description))))
            .collect();

        let mut state = ListState::default();
        state.select(Some(self.selected_index));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    fn is_overlay(&self) -> bool {
        true
    }

    fn z_order(&self) -> u8 {
        30
    }

    fn blocks_input(&self) -> bool {
        true
    }
}
