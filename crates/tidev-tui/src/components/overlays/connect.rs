//! ConnectDialog — provider picker and API key entry dialog.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use tidev_config::provider::ProviderSource;

use crate::action::{Action, ConnectAction, OverlayAction, OverlayKind};
use crate::component::Component;
use crate::context::{DrawContext, InitContext, UpdateContext};
use crate::utils::{centered_rect, paste_from_clipboard};
use unicode_width::UnicodeWidthStr;

/// Phase of the connect dialog.
enum ConnectPhase {
    /// Browsing/filtering available providers.
    ProviderPicker,
    /// Entering an API key for a specific provider.
    ApiKey {
        provider_id: String,
        display_name: String,
        /// The API key being typed.
        buffer: String,
    },
    /// Confirm before disconnecting a provider.
    DisconnectConfirm {
        provider_id: String,
        display_name: String,
    },
}

pub(crate) struct ConnectDialog {
    phase: ConnectPhase,
    /// Full provider list, built once from runtime in `update()`.
    all_providers: Vec<ProviderItem>,
    /// Current search query.
    query: String,
    /// Currently selected index (into filtered results).
    selected: usize,
    /// Whether Enter was pressed to confirm the API key.
    confirmed: bool,
}

/// Display info for a single provider in the picker list.
#[derive(Clone, Debug)]
pub(crate) struct ProviderItem {
    pub provider_id: String,
    pub display_name: String,
    pub source: ProviderSource,
    pub connected: bool,
}

impl ConnectDialog {
    pub(crate) fn new() -> Self {
        Self {
            phase: ConnectPhase::ProviderPicker,
            all_providers: Vec::new(),
            query: String::new(),
            selected: 0,
            confirmed: false,
        }
    }

    /// Rebuild the full (unfiltered) provider list from config + auth.
    fn rebuild_all_providers(
        &mut self,
        config: &tidev_config::AppConfig,
        auth: &tidev_config::AuthStore,
    ) {
        self.all_providers = config
            .provider_ids()
            .into_iter()
            .map(|provider_id| {
                let display_name = config
                    .provider_display_name(&provider_id)
                    .unwrap_or(&provider_id)
                    .to_string();
                let source = config
                    .provider_source(&provider_id)
                    .unwrap_or(ProviderSource::User);
                let connected = auth.api_key(&provider_id).is_some();
                ProviderItem {
                    provider_id,
                    display_name,
                    source,
                    connected,
                }
            })
            .collect();
    }

    /// Number of providers matching the current query.
    fn visible_count(&self) -> usize {
        if self.query.is_empty() {
            self.all_providers.len()
        } else {
            let q = self.query.trim().to_ascii_lowercase();
            self.all_providers
                .iter()
                .filter(|p| provider_picker_matches(&q, &p.provider_id, &p.display_name))
                .count()
        }
    }

    /// The provider at visual (filtered) index `idx`.
    fn visible_provider(&self, idx: usize) -> Option<&ProviderItem> {
        if self.query.is_empty() {
            return self.all_providers.get(idx);
        }
        let q = self.query.trim().to_ascii_lowercase();
        self.all_providers
            .iter()
            .filter(|p| provider_picker_matches(&q, &p.provider_id, &p.display_name))
            .nth(idx)
    }

    fn title(&self) -> &str {
        match self.phase {
            ConnectPhase::ProviderPicker => " Connect to provider ",
            ConnectPhase::ApiKey { .. } => " API Key ",
            ConnectPhase::DisconnectConfirm { .. } => " Disconnect Provider ",
        }
    }

    fn switch_to_api_key(&mut self, provider_id: String, display_name: String) {
        self.phase = ConnectPhase::ApiKey {
            provider_id,
            display_name,
            buffer: String::new(),
        };
    }

    fn provider_source_label(source: &ProviderSource) -> &'static str {
        match source {
            ProviderSource::User => "custom",
            ProviderSource::Bundled => "preset",
        }
    }
}

impl Component for ConnectDialog {
    fn init(&mut self, ctx: &InitContext) -> Result<()> {
        self.rebuild_all_providers(ctx.config, ctx.auth);
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return None;
        }

        match &mut self.phase {
            ConnectPhase::ProviderPicker => match key.code {
                KeyCode::Esc => Some(Action::Overlay(OverlayAction::Close(
                    OverlayKind::ConnectDialog,
                ))),
                KeyCode::Tab => None,
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(item) = self.visible_provider(self.selected) {
                        self.switch_to_api_key(item.provider_id.clone(), item.display_name.clone());
                    }
                    None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let count = self.visible_count();
                    if count > 0 {
                        let current = self.selected.min(count.saturating_sub(1));
                        self.selected = if current == 0 {
                            count.saturating_sub(1)
                        } else {
                            current - 1
                        };
                    }
                    None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let count = self.visible_count();
                    if count > 0 {
                        let current = self.selected.min(count.saturating_sub(1));
                        self.selected = (current + 1) % count;
                    }
                    None
                }
                KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'p' => {
                    Some(Action::Connect(ConnectAction::PruneOrphans))
                }
                KeyCode::Char('d') | KeyCode::Char('D')
                    if !key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    if let Some(item) = self.visible_provider(self.selected) {
                        if item.connected {
                            self.phase = ConnectPhase::DisconnectConfirm {
                                provider_id: item.provider_id.clone(),
                                display_name: item.display_name.clone(),
                            };
                        }
                    }
                    None
                }
                KeyCode::Char(c) if !c.is_control() => {
                    self.query.push(c);
                    self.selected = 0;
                    None
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    self.selected = 0;
                    None
                }
                _ => None,
            },
            ConnectPhase::ApiKey {
                provider_id: _,
                display_name: _,
                buffer,
            } => match key.code {
                KeyCode::Esc => {
                    self.phase = ConnectPhase::ProviderPicker;
                    self.query.clear();
                    self.selected = 0;
                    None
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.confirmed = true;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::ConnectDialog,
                    )))
                }
                KeyCode::Char('v')
                    if (key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::SUPER))
                        && !key.modifiers.contains(KeyModifiers::ALT)
                        && !key.modifiers.contains(KeyModifiers::SHIFT) =>
                {
                    if let Some(text) = paste_from_clipboard() {
                        buffer.push_str(&text);
                    }
                    None
                }
                KeyCode::Char(c) if !c.is_control() => {
                    buffer.push(c);
                    None
                }
                KeyCode::Backspace => {
                    buffer.pop();
                    None
                }
                _ => None,
            },
            ConnectPhase::DisconnectConfirm {
                provider_id: _,
                display_name: _,
            } => match key.code {
                KeyCode::Enter => {
                    self.confirmed = true;
                    Some(Action::Overlay(OverlayAction::Close(
                        OverlayKind::ConnectDialog,
                    )))
                }
                KeyCode::Esc => {
                    self.phase = ConnectPhase::ProviderPicker;
                    None
                }
                _ => None,
            },
        }
    }

    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
        match action {
            Action::Overlay(OverlayAction::Close(OverlayKind::ConnectDialog)) => {
                if self.confirmed {
                    match &self.phase {
                        ConnectPhase::ApiKey {
                            provider_id,
                            buffer,
                            ..
                        } if !buffer.is_empty() => {
                            return vec![Action::Connect(ConnectAction::SaveApiKey {
                                provider_id: provider_id.clone(),
                                key: buffer.clone(),
                            })];
                        }
                        ConnectPhase::DisconnectConfirm { provider_id, display_name } => {
                            return vec![Action::Connect(ConnectAction::Disconnect {
                                provider_id: provider_id.clone(),
                                display_name: display_name.clone(),
                            })];
                        }
                        _ => {}
                    }
                }
                vec![]
            }
            Action::Connect(ConnectAction::PruneOrphans) => {
                // Provider list may have changed; rebuild.
                let config = ctx.runtime.config();
                let auth = ctx.runtime.auth();
                self.rebuild_all_providers(&config, &auth);
                self.selected = 0;
                vec![]
            }
            Action::Connect(ConnectAction::Disconnect { .. }) => {
                // Provider list may have changed; rebuild.
                let config = ctx.runtime.config();
                let auth = ctx.runtime.auth();
                self.rebuild_all_providers(&config, &auth);
                self.selected = 0;
                vec![]
            }
            _ => {
                // Initial build or refresh after any action.
                if self.all_providers.is_empty() {
                    let config = ctx.runtime.config();
                    let auth = ctx.runtime.auth();
                    self.rebuild_all_providers(&config, &auth);
                }
                vec![]
            }
        }
    }

    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext) {
        let palette = ctx.palette;
        let (overlay_width, overlay_height) = match &self.phase {
            ConnectPhase::ProviderPicker => (rect.width.min(92), rect.height.min(28)),
            ConnectPhase::ApiKey { .. } => (rect.width.min(80), rect.height.min(24)),
            ConnectPhase::DisconnectConfirm { .. } => (rect.width.min(60), rect.height.min(12)),
        };
        let overlay = centered_rect(overlay_width, overlay_height, rect);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Title
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                self.title(),
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

        match &self.phase {
            ConnectPhase::ProviderPicker => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(3),
                    Constraint::Length(2),
                ])
                .split(body);

                // Search input
                let search_display = if self.query.is_empty() {
                    "Search providers by id or display name... (type to filter)"
                } else {
                    self.query.as_str()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("> ", Style::default().fg(palette.accent)),
                        Span::styled(search_display, Style::default().fg(palette.text)),
                    ]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );
                frame.set_cursor_position((
                    sections[0].x + 2 + self.query.as_str().width() as u16,
                    sections[0].y,
                ));

                // Hint
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} provider(s) available · Ctrl+P to prune orphan auth entries",
                        self.visible_count(),
                    ))
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[1],
                );

                // Provider list (scrollable)
                let list_area = sections[2];
                let count = self.visible_count();
                let sel = self.selected.min(count.saturating_sub(1));
                let mut vis_idx = 0usize;

                for item in self.all_providers.iter() {
                    if !self.query.is_empty() {
                        let q = self.query.trim().to_ascii_lowercase();
                        if !provider_picker_matches(&q, &item.provider_id, &item.display_name) {
                            continue;
                        }
                    }

                    let y = list_area.y + vis_idx as u16;
                    if y >= list_area.bottom() {
                        break;
                    }

                    let is_selected = vis_idx == sel;
                    let bg = if is_selected {
                        palette.selection_bg
                    } else {
                        palette.panel_alt
                    };
                    let fg = if is_selected {
                        palette.selection_fg
                    } else {
                        palette.text
                    };

                    let source_label = Self::provider_source_label(&item.source);
                    let prefix = if is_selected { "▶ " } else { "  " };
                    let status_style = if item.connected {
                        Style::default().fg(palette.success).bg(bg)
                    } else {
                        Style::default().fg(palette.muted).bg(bg)
                    };
                    let line = Line::from(vec![
                        Span::styled(
                            prefix,
                            Style::default().fg(if is_selected {
                                palette.accent
                            } else {
                                palette.panel_alt
                            }),
                        ),
                        Span::styled(
                            &item.display_name,
                            Style::default().fg(fg).add_modifier(Modifier::BOLD).bg(bg),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            format!("({})", item.provider_id),
                            Style::default().fg(palette.muted).bg(bg),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            format!("[{}]", source_label),
                            Style::default().fg(palette.accent_soft).bg(bg),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            if item.connected {
                                "connected"
                            } else {
                                "not connected"
                            },
                            status_style,
                        ),
                    ]);
                    frame.render_widget(
                        Paragraph::new(line).style(Style::default().bg(bg)),
                        Rect::new(list_area.x, y, list_area.width, 1),
                    );

                    vis_idx += 1;
                }

                // Help footer
                frame.render_widget(
                    Paragraph::new("↑↓ navigate · Enter select · D disconnect · Esc cancel · type to filter")
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[3],
                );
            }
            ConnectPhase::ApiKey {
                provider_id: _,
                display_name,
                buffer,
            } => {
                let sections = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(1),
                ])
                .split(body);

                // Provider label
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!("Enter API key for {}", display_name),
                        Style::default().fg(palette.text),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // Security notice
                frame.render_widget(
                    Paragraph::new(
                        "The key will be stored in auth.json and used for future requests.",
                    )
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[1],
                );

                // Key input
                let display = if buffer.is_empty() {
                    "Paste or type your API key..."
                } else {
                    buffer.as_str()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Key: ", Style::default().fg(palette.accent)),
                        Span::styled(display, Style::default().fg(palette.text)),
                    ]))
                    .style(Style::default().bg(palette.panel_alt))
                    .wrap(Wrap { trim: false }),
                    sections[2],
                );
                frame.set_cursor_position((
                    sections[2].x + 5 + buffer.as_str().width() as u16,
                    sections[2].y,
                ));

                // Help
                frame.render_widget(
                    Paragraph::new("Enter save · Esc back · type to enter key")
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[3],
                );
            }
            ConnectPhase::DisconnectConfirm {
                provider_id: _,
                display_name,
            } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(body);

                // Confirmation message
                frame.render_widget(
                    Paragraph::new(format!(
                        "Disconnect from {}?",
                        display_name,
                    ))
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[0],
                );

                // Hint
                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[1],
                );
            }
        }
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

    fn handle_paste(&mut self, text: &str) -> Option<Action> {
        if !text.is_empty() {
            if let ConnectPhase::ApiKey { buffer, .. } = &mut self.phase {
                buffer.push_str(text);
            }
        }
        None
    }
}

fn provider_picker_matches(query: &str, provider_id: &str, display_name: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let provider_id = provider_id.to_ascii_lowercase();
    let display_name = display_name.to_ascii_lowercase();
    provider_id.contains(query) || display_name.contains(query)
}
