//! SearchPanel component — web search provider selection panel.
//!
//! Mirrors the old `tidev_tui::ui::search_panel` module with a self-contained
//! Component implementation.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, Paragraph};

use crate::action::{Action, OverlayAction, OverlayKind, SearchAction};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::centered_rect;

// ---------------------------------------------------------------------------
// Built-in provider metadata
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct ProviderInfo {
    id: &'static str,
    display_name: &'static str,
    needs_api_key: bool,
    needs_cx: bool,
    #[allow(dead_code)]
    description: &'static str,
}

const BUILTIN_PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo {
        id: "exa",
        display_name: "Exa",
        needs_api_key: false,
        needs_cx: false,
        description: "Public endpoint, no key required",
    },
    ProviderInfo {
        id: "brave",
        display_name: "Brave Search",
        needs_api_key: true,
        needs_cx: false,
        description: "Free tier: 2,000 queries/month",
    },
    ProviderInfo {
        id: "google",
        display_name: "Google Custom Search",
        needs_api_key: true,
        needs_cx: true,
        description: "Free tier: 100 queries/day",
    },
    ProviderInfo {
        id: "tavily",
        display_name: "Tavily",
        needs_api_key: true,
        needs_cx: false,
        description: "Free tier: 1,000 requests/month",
    },
];

// ---------------------------------------------------------------------------
// SearchPanel component
// ---------------------------------------------------------------------------

pub(crate) struct SearchPanel {
    selected_index: usize,
    active_provider: String,
    /// Snapshot: which providers (by index) have their API key set.
    provider_keys_set: Vec<bool>,
    /// Snapshot: whether Google CX is set.
    provider_cx_set: bool,

    // ── Editing state ──
    /// Some(provider_id) when in API key / CX editing mode.
    editing_api_key: Option<String>,
    /// True when editing Google CX (as opposed to API key).
    editing_cx: bool,
    /// Input buffer for the editing mode.
    input_buffer: String,
}

impl SearchPanel {
    pub(crate) fn new(active_provider: &str, auth: &tidev_config::AuthStore) -> Self {
        let provider_keys_set: Vec<bool> = BUILTIN_PROVIDERS
            .iter()
            .map(|info| auth.web.search_api_keys.contains_key(info.id))
            .collect();
        let provider_cx_set = auth.web.google_cx.is_some();

        Self {
            selected_index: 0,
            active_provider: active_provider.to_string(),
            provider_keys_set,
            provider_cx_set,
            editing_api_key: None,
            editing_cx: false,
            input_buffer: String::new(),
        }
    }

    // ── Navigation helpers ──

    fn provider_count(&self) -> usize {
        BUILTIN_PROVIDERS.len()
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.provider_count();
        if count == 0 {
            return;
        }
        let new = (self.selected_index as isize + delta).rem_euclid(count as isize) as usize;
        self.selected_index = new;
    }

    fn selected_provider_missing_key(&self) -> bool {
        self.selected_index < BUILTIN_PROVIDERS.len()
            && BUILTIN_PROVIDERS[self.selected_index].needs_api_key
            && !self.provider_keys_set[self.selected_index]
    }

    fn selected_provider_missing_cx(&self) -> bool {
        self.selected_index < BUILTIN_PROVIDERS.len()
            && BUILTIN_PROVIDERS[self.selected_index].needs_cx
            && !self.provider_cx_set
    }

    fn start_editing_api_key(&mut self) {
        if self.selected_index < BUILTIN_PROVIDERS.len() {
            let info = &BUILTIN_PROVIDERS[self.selected_index];
            self.editing_api_key = Some(info.id.to_string());
            self.editing_cx = false;
            self.input_buffer.clear();
        }
    }

    fn start_editing_cx(&mut self) {
        self.editing_api_key = Some("google".to_string());
        self.editing_cx = true;
        self.input_buffer.clear();
    }

    fn provider_status(&self, index: usize) -> String {
        if index >= BUILTIN_PROVIDERS.len() {
            return String::new();
        }
        let info = &BUILTIN_PROVIDERS[index];

        let status = if info.needs_cx {
            if self.provider_keys_set[index] && self.provider_cx_set {
                "Ready"
            } else if !self.provider_keys_set[index] {
                "Set API key"
            } else {
                "Set Search Engine ID"
            }
        } else if info.needs_api_key {
            if self.provider_keys_set[index] {
                "Ready"
            } else {
                "Set API key"
            }
        } else {
            "Ready"
        };

        format!("{}  —  {}", info.display_name, status)
    }
}

impl Component for SearchPanel {
    fn init(&mut self, _ctx: &InitContext) -> Result<()> {
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        // ── Editing mode ──
        if self.editing_api_key.is_some() {
            match key.code {
                KeyCode::Enter => {
                    let input = self.input_buffer.trim().to_string();
                    if !input.is_empty() {
                        let provider = self.editing_api_key.clone().unwrap_or_default();
                        let is_cx = self.editing_cx;

                        // Update local snapshot
                        if is_cx {
                            self.provider_cx_set = true;
                        } else if let Some(idx) = BUILTIN_PROVIDERS
                            .iter()
                            .position(|info| info.id == provider)
                        {
                            self.provider_keys_set[idx] = true;
                        }

                        // Clear editing state before returning action
                        self.editing_api_key = None;
                        self.editing_cx = false;
                        self.input_buffer.clear();

                        return Some(Action::Search(SearchAction::SaveApiKey {
                            provider,
                            key: input,
                            is_cx,
                        }));
                    }
                    // Empty input: cancel
                    self.editing_api_key = None;
                    self.editing_cx = false;
                    self.input_buffer.clear();
                    None
                }
                KeyCode::Esc => {
                    self.editing_api_key = None;
                    self.editing_cx = false;
                    self.input_buffer.clear();
                    None
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                    None
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                    None
                }
                _ => None,
            }
        } else {
            // ── Normal mode ──
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
                    // Case 1: provider needs API key but none set → edit mode
                    if self.selected_provider_missing_key() {
                        self.start_editing_api_key();
                        return None;
                    }
                    // Case 2: provider needs Google CX but none set → edit mode
                    if self.selected_provider_missing_cx() {
                        self.start_editing_cx();
                        return None;
                    }
                    // Case 3: switch to this provider
                    if self.selected_index < BUILTIN_PROVIDERS.len() {
                        let info = &BUILTIN_PROVIDERS[self.selected_index];
                        self.active_provider = info.id.to_string();
                        Some(Action::Search(SearchAction::SwitchProvider(
                            info.id.to_string(),
                        )))
                    } else {
                        None
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    Some(Action::Overlay(OverlayAction::Close(OverlayKind::SearchPanel)))
                }
                _ => None,
            }
        }
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> {
        let position = Position::new(mouse.column, mouse.row);
        if !area.contains(position) {
            return None;
        }

        let overlay = centered_rect(
            area.width.min(60),
            area.height.min(20),
            area,
        );

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
                if self.editing_api_key.is_some() {
                    return Some(Action::Noop);
                }
                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                // Title (1) + instruction (1) offset
                let header_rows = 2u16;
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < header_rows {
                    return Some(Action::Noop);
                }
                let row = (local_y - header_rows) as usize;
                if row < BUILTIN_PROVIDERS.len() {
                    self.selected_index = row;
                    // Same logic as Enter on selected provider
                    if self.selected_provider_missing_key() {
                        self.start_editing_api_key();
                        return None;
                    }
                    if self.selected_provider_missing_cx() {
                        self.start_editing_cx();
                        return None;
                    }
                    if let Some(info) = BUILTIN_PROVIDERS.get(self.selected_index) {
                        self.active_provider = info.id.to_string();
                        return Some(Action::Search(SearchAction::SwitchProvider(
                            info.id.to_string(),
                        )));
                    }
                }
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    fn update(&mut self, action: &Action, _ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Search(SearchAction::SaveApiKey {
                provider,
                key: _,
                is_cx,
            }) => {
                // Update local snapshot
                if *is_cx {
                    self.provider_cx_set = true;
                } else if let Some(idx) =
                    BUILTIN_PROVIDERS.iter().position(|info| info.id == provider)
                {
                    self.provider_keys_set[idx] = true;
                }
                // Runtime mutation is handled by App::process_action
                vec![]
            }
            _ => vec![],
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let overlay = centered_rect(rect.width.min(60), rect.height.min(20), rect);
        frame.render_widget(Clear, overlay);

        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Title
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Search Provider ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        let body = Rect::new(
            inner.x,
            inner.y + 1,
            inner.width,
            inner.height.saturating_sub(1),
        );

        // ── Editing mode: show input view ──
        if self.editing_api_key.is_some() {
            let sections = Layout::vertical([
                Constraint::Length(3), // prompt + input
                Constraint::Length(1), // footer help
            ])
            .split(body);

            // Placeholder label
            let placeholder = if self.editing_cx {
                "Enter Google Search Engine ID (cx): "
            } else {
                let provider_id = self.editing_api_key.as_deref().unwrap_or("");
                let display_name = BUILTIN_PROVIDERS
                    .iter()
                    .find(|info| info.id == provider_id)
                    .map(|info| info.display_name)
                    .unwrap_or(provider_id);
                &format!("Enter API key for {}: ", display_name)
            };

            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    placeholder,
                    Style::default().fg(palette.muted),
                )]))
                .style(Style::default().bg(palette.panel_alt)),
                sections[0],
            );

            // Render the actual input text (right-aligned to show end)
            let input_width = sections[0].width.saturating_sub(2);
            let text = &self.input_buffer;
            let display = if text.len() > input_width as usize {
                &text[text.len().saturating_sub(input_width as usize)..]
            } else {
                text.as_str()
            };
            frame.render_widget(
                Paragraph::new(display.to_string())
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                sections[0],
            );
            frame.set_cursor_position((
                sections[0].right().saturating_sub(1),
                sections[0].y,
            ));

            // Footer
            let footer = Line::from(vec![
                Span::styled(
                    "Enter to save",
                    Style::default()
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  ·  "),
                Span::styled("Esc to cancel", Style::default().fg(palette.muted)),
            ]);
            frame.render_widget(
                Paragraph::new(footer)
                    .alignment(ratatui::layout::Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt)),
                sections[1],
            );
            return;
        }

        // ── Normal mode: provider list ──
        let sections = Layout::vertical([
            Constraint::Length(1), // instruction
            Constraint::Min(4),    // provider list
            Constraint::Length(1), // footer help
        ])
        .split(body);

        // Instruction
        frame.render_widget(
            Paragraph::new("Select a web search provider. ↑↓ navigate, Enter select.")
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[0],
        );

        // Provider list
        let mut rows: Vec<ListItem> = Vec::new();
        for (i, info) in BUILTIN_PROVIDERS.iter().enumerate() {
            let status_text = self.provider_status(i);

            let is_selected = i == self.selected_index;
            let row_style = if is_selected {
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
            } else {
                Style::default().bg(palette.panel_alt)
            };

            // Active checkmark
            let active_marker = if info.id == self.active_provider {
                Span::styled(" ✓", Style::default().fg(palette.accent))
            } else {
                Span::raw("  ")
            };

            let parts = vec![
                active_marker,
                Span::raw("  "),
                Span::styled(status_text, row_style),
            ];

            rows.push(ListItem::new(Line::from(parts)).style(row_style));
        }

        frame.render_widget(
            List::new(rows).style(Style::default().bg(palette.panel_alt)),
            sections[1],
        );

        // Footer help
        let footer = Line::from(vec![
            Span::styled(
                "Enter",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" select provider · "),
            Span::styled(
                "Esc",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" close"),
        ]);
        frame.render_widget(
            Paragraph::new(footer)
                .alignment(ratatui::layout::Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
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
