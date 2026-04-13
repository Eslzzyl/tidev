use crate::{
    app::theme_panel::ThemePanelState,
    config::ProviderSource,
    prompts::SessionMode,
    provider_setup::{ConnectDialog, EditProviderStep, NewProviderStep},
    session::MessageRole,
    theme::ThemePalette,
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    prelude::{Frame, Modifier, Style, Text},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use uuid::Uuid;

use super::{App, Screen, connect::ProviderPickerItem};

impl App {
    fn palette(&self) -> ThemePalette {
        self.theme.palette()
    }

    fn render_command_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.command_palette.visible || self.command_palette.suggestions.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72).max(28).min(area.width);
        let height = (self.command_palette.suggestions.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let items = self
            .command_palette
            .suggestions
            .iter()
            .map(|suggestion| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        suggestion.spec.label(),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        suggestion.spec.description,
                        Style::default().fg(palette.muted),
                    ),
                ]))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(Some(self.command_palette.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!("Commands · /{}", self.command_palette.query));

        let list = List::new(items)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, rect);
        frame.render_widget(panel, rect);
        frame.render_stateful_widget(list, inner, &mut state);
    }

    fn render_connect_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(dialog) = &self.connect_dialog else {
            return;
        };

        let palette = self.palette();
        let (overlay_width, overlay_height) = match dialog {
            ConnectDialog::ProviderPicker { .. } => (area.width.min(92), area.height.min(28)),
            ConnectDialog::EditProvider {
                step, model_step, ..
            } => {
                if model_step.is_some() {
                    (area.width.min(90), area.height.min(26))
                } else {
                    match step {
                        EditProviderStep::ModelList | EditProviderStep::ConfirmDeleteModel => {
                            (area.width.min(96), area.height.min(34))
                        }
                        _ => (area.width.min(84), area.height.min(24)),
                    }
                }
            }
            _ => (area.width.min(80), area.height.min(24)),
        };
        let overlay = centered_rect(overlay_width, overlay_height, area);
        frame.render_widget(Clear, overlay);

        let dialog_title = match dialog {
            ConnectDialog::ProviderPicker { .. } => "Connect provider".to_string(),
            ConnectDialog::ApiKey { provider_id } => {
                let label = self
                    .config
                    .provider_display_name(provider_id)
                    .unwrap_or(provider_id)
                    .to_string();
                format!("API key · {label}")
            }
            ConnectDialog::NewProvider { step, .. } => {
                format!("Create provider · {}", step.title())
            }
            ConnectDialog::EditProvider {
                provider_id,
                step,
                model_step,
                ..
            } => {
                if let Some(model_step) = model_step {
                    format!("Edit model · {provider_id} · {}", model_step.title())
                } else {
                    format!("Edit provider · {provider_id} · {}", step.title())
                }
            }
        };

        let panel = Block::default()
            .style(Style::default().bg(palette.panel))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(dialog_title);

        frame.render_widget(panel, overlay);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match dialog {
            ConnectDialog::ProviderPicker { selected } => {
                let sections = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(3),
                    Constraint::Min(6),
                    Constraint::Length(1),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new("Type to filter by provider id or display name. Press Ctrl+E to edit custom providers.")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[0],
                );

                self.render_input_block(
                    frame,
                    sections[1],
                    "Search",
                    self.composer.placeholder(),
                    false,
                );

                let items = self.provider_picker_items();
                let list_items = items
                    .iter()
                    .map(|item| match item {
                        ProviderPickerItem::Provider {
                            provider_id,
                            display_name,
                            source,
                            connected,
                        } => {
                            let status_style = if *connected {
                                Style::default().fg(palette.success)
                            } else {
                                Style::default().fg(palette.muted)
                            };

                            let source_label = match source {
                                ProviderSource::Bundled => "preset",
                                ProviderSource::User => "custom",
                            };

                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    format!("{}", display_name),
                                    Style::default()
                                        .fg(palette.text)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    format!("({provider_id})"),
                                    Style::default().fg(palette.muted),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    format!("[{source_label}]"),
                                    Style::default().fg(palette.accent_soft),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    if *connected {
                                        "connected"
                                    } else {
                                        "not connected"
                                    },
                                    status_style,
                                ),
                            ]))
                        }
                        ProviderPickerItem::AddNew { query } => {
                            let label = if query.is_empty() {
                                "Add new provider".to_string()
                            } else {
                                format!("Add new provider: {query}")
                            };

                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    label,
                                    Style::default()
                                        .fg(palette.accent)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    "Create a new OpenAI-compatible provider",
                                    Style::default().fg(palette.warning),
                                ),
                            ]))
                        }
                    })
                    .collect::<Vec<_>>();

                let mut state = ListState::default();
                state.select(Some((*selected).min(items.len().saturating_sub(1))));

                let list = List::new(list_items)
                    .style(Style::default().bg(palette.panel).fg(palette.text))
                    .highlight_style(
                        Style::default()
                            .bg(palette.selection_bg)
                            .fg(palette.selection_fg)
                            .add_modifier(Modifier::BOLD),
                    );

                frame.render_stateful_widget(list, sections[2], &mut state);

                frame.render_widget(
                    Paragraph::new(
                        "Enter to connect · Ctrl+E to edit custom providers · Esc to cancel",
                    )
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[3],
                );
            }
            ConnectDialog::ApiKey { provider_id } => {
                let label = self
                    .config
                    .provider_display_name(provider_id)
                    .unwrap_or(provider_id)
                    .to_string();

                let lines = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(4),
                    Constraint::Length(1),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("Enter API key for {label}"))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    lines[0],
                );

                frame.render_widget(
                    Paragraph::new(
                        "The key will be stored in auth.json and used for future requests.",
                    )
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    lines[1],
                );

                self.render_input_block(
                    frame,
                    lines[2],
                    "API Key",
                    self.composer.placeholder(),
                    true,
                );

                frame.render_widget(
                    Paragraph::new("Enter to save · Esc to cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    lines[3],
                );
            }
            ConnectDialog::NewProvider { step, draft: _ } => {
                let lines = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(4),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("Create provider · {}", step.title()))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    lines[0],
                );

                frame.render_widget(
                    Paragraph::new(step.help())
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    lines[1],
                );

                self.render_input_block(
                    frame,
                    lines[2],
                    step.label(),
                    self.composer.placeholder(),
                    step.is_secret(),
                );

                let prompt_line = if matches!(step, NewProviderStep::AddAnotherModel) {
                    "y to add another model · Enter to save provider".to_string()
                } else {
                    format!(
                        "Next: {}",
                        step.next()
                            .map(|next| next.label())
                            .unwrap_or("Save provider")
                    )
                };
                frame.render_widget(
                    Paragraph::new(prompt_line)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                    lines[3],
                );

                let footer = if matches!(step, NewProviderStep::AddAnotherModel) {
                    "Enter to save provider · y to add another model · Esc to cancel"
                } else {
                    "Enter to continue · Esc to cancel"
                };
                frame.render_widget(
                    Paragraph::new(footer)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    lines[4],
                );
            }
            ConnectDialog::EditProvider {
                provider_id,
                step,
                model_step,
                draft,
            } => {
                if let Some(model_step) = model_step {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Length(2),
                        Constraint::Length(4),
                        Constraint::Length(2),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                    let provider_label = self
                        .config
                        .provider_display_name(provider_id)
                        .unwrap_or(provider_id)
                        .to_string();

                    frame.render_widget(
                        Paragraph::new(format!(
                            "Edit model for {provider_label} · {}",
                            model_step.title()
                        ))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new(model_step.help())
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        lines[1],
                    );

                    self.render_input_block(
                        frame,
                        lines[2],
                        model_step.label(),
                        model_step.placeholder(),
                        model_step.is_secret(),
                    );

                    frame.render_widget(
                        Paragraph::new("Enter to continue · Esc to cancel")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                        lines[3],
                    );

                    frame.render_widget(
                        Paragraph::new("Model ids stay fixed while editing existing models")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        lines[4],
                    );
                } else if *step == EditProviderStep::ModelList {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Min(8),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                    frame.render_widget(
                        Paragraph::new(format!("Manage models for {}", provider_id))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel)
                                    .fg(palette.text)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        lines[0],
                    );

                    let items = draft
                        .models
                        .iter()
                        .enumerate()
                        .map(|(index, (model_id, model))| {
                            let is_selected = index == draft.selected_model_index;
                            let status_style = if is_selected {
                                Style::default().fg(palette.selection_fg)
                            } else {
                                Style::default().fg(palette.muted)
                            };

                            ListItem::new(Line::from(vec![
                                Span::styled(
                                    format!("{}", model.display_name),
                                    Style::default()
                                        .fg(palette.text)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::raw("  "),
                                Span::styled(
                                    format!("({model_id})"),
                                    Style::default().fg(palette.muted),
                                ),
                                Span::raw("  "),
                                Span::styled(format!("ctx {}", model.context_window), status_style),
                                Span::raw("  "),
                                Span::styled(
                                    format!("max {}", model.max_output_tokens),
                                    status_style,
                                ),
                            ]))
                        })
                        .collect::<Vec<_>>();

                    let mut state = ListState::default();
                    state.select(Some(
                        draft
                            .selected_model_index
                            .min(items.len().saturating_sub(1)),
                    ));

                    let list = List::new(items)
                        .style(Style::default().bg(palette.panel).fg(palette.text))
                        .highlight_style(
                            Style::default()
                                .bg(palette.selection_bg)
                                .fg(palette.selection_fg)
                                .add_modifier(Modifier::BOLD),
                        );

                    frame.render_stateful_widget(list, lines[1], &mut state);

                    frame.render_widget(
                        Paragraph::new("Enter edit · N new · D delete · S save · Esc cancel")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                        lines[2],
                    );
                } else if *step == EditProviderStep::ConfirmDeleteModel {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Length(2),
                        Constraint::Length(4),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                    let pending = draft
                        .pending_delete_model_id
                        .as_deref()
                        .unwrap_or("unknown model");

                    frame.render_widget(
                        Paragraph::new(format!("Delete model {pending}?"))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel)
                                    .fg(palette.error)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new("This only removes the model from config.toml. Historical sessions keep their stored snapshot.")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        lines[1],
                    );

                    self.render_input_block(frame, lines[2], "Confirm", "y or n", false);

                    frame.render_widget(
                        Paragraph::new("Y to delete · N / Esc to keep")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                        lines[3],
                    );
                } else {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Length(2),
                        Constraint::Length(4),
                        Constraint::Length(2),
                        Constraint::Length(1),
                    ])
                    .split(inner);

                    frame.render_widget(
                        Paragraph::new(format!("Edit provider · {}", provider_id))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel)
                                    .fg(palette.text)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new(step.help())
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        lines[1],
                    );

                    self.render_input_block(
                        frame,
                        lines[2],
                        step.label(),
                        step.placeholder(),
                        step.is_secret(),
                    );

                    frame.render_widget(
                        Paragraph::new("Enter to continue · Esc to cancel")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                        lines[3],
                    );

                    frame.render_widget(
                        Paragraph::new("After the fields, manage models from the list")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        lines[4],
                    );
                }
            }
        }
    }

    pub(crate) fn render(&self, frame: &mut Frame<'_>) {
        match self.screen {
            Screen::Welcome => self.render_welcome(frame),
            Screen::Chat => self.render_chat(frame),
        }
        let area = frame.area();
        self.render_connect_dialog(frame, area);
        if let Some(panel) = &self.theme_panel {
            self.render_theme_panel(frame, area, panel);
        }
    }

    fn render_welcome(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let card_width = self
            .config
            .ui
            .welcome_width
            .min(area.width.saturating_sub(4).max(32));
        let card_height = 12u16.min(area.height.saturating_sub(2).max(10));
        let card = centered_rect(card_width, card_height, area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title("TiDev");
        frame.render_widget(block, card);

        let inner = card.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(
                self.composer
                    .preferred_height(self.config.ui.max_input_lines),
            ),
            Constraint::Length(1),
        ])
        .split(inner);

        let title = Paragraph::new("TiDev").alignment(Alignment::Center).style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(title, sections[0]);

        let subtitle = Paragraph::new("Terminal AI assistant for focused coding work")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.muted));
        frame.render_widget(subtitle, sections[1]);

        let session_matches_active = self.conversation.provider_id == self.active_model.provider_id
            && self.conversation.model_id == self.active_model.model_id
            && self.conversation.provider_display_name == self.active_model.provider_display_name
            && self.conversation.model_display_name == self.active_model.display_name;

        let mut model_line = if self.active_model.api_key_present() {
            format!(
                "{} · {} · {} mode · API key ready",
                self.active_model.provider_display_name,
                self.active_model.label(),
                self.mode.as_str()
            )
        } else {
            format!(
                "{} · {} · {} mode · API key missing",
                self.active_model.provider_display_name,
                self.active_model.label(),
                self.mode.as_str()
            )
        };

        if !session_matches_active {
            model_line.push_str(&format!(
                " · session {}",
                shorten(&self.conversation.model_label(), 28)
            ));
        }

        let status_style = if self.active_model.api_key_present() {
            Style::default().fg(palette.success)
        } else {
            Style::default().fg(palette.error)
        };
        frame.render_widget(
            Paragraph::new(model_line)
                .alignment(Alignment::Center)
                .style(status_style),
            sections[2],
        );

        let prompt_title = if self.pending_request {
            format!("{} prompt (streaming)", self.mode.title())
        } else {
            format!("{} prompt", self.mode.title())
        };
        self.render_input_block(
            frame,
            sections[3],
            &prompt_title,
            self.composer.placeholder(),
            false,
        );

        let hint = Paragraph::new(
            "Enter to send · Shift+Enter for newline · Ctrl+P/N history · Ctrl+C quit",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(palette.accent_soft));
        frame.render_widget(hint, sections[4]);

        self.render_command_palette(frame, sections[3]);
    }

    fn render_chat(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let sidebar_visible = area.width >= self.config.ui.sidebar_width.saturating_add(55);
        let main_area = if sidebar_visible {
            let split = Layout::horizontal([
                Constraint::Min(20),
                Constraint::Length(self.config.ui.sidebar_width),
            ])
            .split(area);
            self.render_sidebar(frame, split[1]);
            split[0]
        } else {
            area
        };

        let composer_height = self
            .composer
            .preferred_height(self.config.ui.max_input_lines)
            .min(main_area.height.saturating_sub(3).max(3));

        let layout = Layout::vertical([
            Constraint::Min(6),
            Constraint::Length(1),
            Constraint::Length(composer_height),
        ])
        .split(main_area);

        self.render_messages(frame, layout[0]);
        self.render_status_line(frame, layout[1]);
        let prompt_title = if self.pending_request {
            format!("{} prompt (streaming)", self.mode.title())
        } else {
            format!("{} prompt", self.mode.title())
        };
        self.render_input_block(
            frame,
            layout[2],
            &prompt_title,
            self.composer.placeholder(),
            false,
        );
        self.render_command_palette(frame, layout[2]);
    }

    fn render_messages(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(format!(
                "Conversation · {}",
                shorten(&self.conversation.title, 32)
            ));

        let inner_height = area.height.saturating_sub(2) as usize;
        let (text, total_lines) = self.messages_text();
        let scroll = total_lines.saturating_sub(inner_height) as u16;

        let paragraph = Paragraph::new(text)
            .block(block)
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        frame.render_widget(paragraph, area);
    }

    fn render_status_line(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let notice = self
            .last_notice
            .as_deref()
            .unwrap_or(if self.pending_request {
                "Thinking..."
            } else {
                "Idle"
            });

        let content = Line::from(vec![
            Span::styled("cwd ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                shorten(&self.workspace_root.display().to_string(), 28),
                Style::default().fg(palette.text),
            ),
            Span::raw("  "),
            Span::styled("model ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                self.active_model.label(),
                Style::default().fg(palette.accent),
            ),
            Span::raw("  "),
            Span::styled("mode ", Style::default().fg(palette.accent_soft)),
            Span::styled(self.mode.as_str(), Style::default().fg(palette.accent)),
            Span::raw("  "),
            Span::styled("session ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                short_uuid(self.conversation.session_id),
                Style::default().fg(palette.text),
            ),
            Span::raw("  "),
            Span::styled("state ", Style::default().fg(palette.accent_soft)),
            Span::styled(shorten(notice, 48), Style::default().fg(palette.warning)),
        ]);

        let paragraph = Paragraph::new(content).style(Style::default().fg(palette.text));
        frame.render_widget(paragraph, area);
    }

    fn render_sidebar(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut lines = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "State",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{} / {}",
                self.active_model.provider_id, self.active_model.model_id
            ),
            Style::default().fg(palette.accent),
        )]));
        lines.push(Line::from(vec![Span::styled(
            if self.active_model.api_key_present() {
                "API key present"
            } else {
                "API key missing"
            },
            if self.active_model.api_key_present() {
                Style::default().fg(palette.success)
            } else {
                Style::default().fg(palette.error)
            },
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Mode: {}", self.mode.title()),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Theme: {}", self.theme.name()),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Workspace",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(shorten(
            &self.workspace_root.display().to_string(),
            32,
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Tools",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        for tool in self.tools.definitions() {
            lines.push(Line::from(format!("- {}", tool.name)));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Commands",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from("/connect"));
        lines.push(Line::from("/theme"));
        lines.push(Line::from("/help"));
        lines.push(Line::from("/model <id>"));
        lines.push(Line::from("/clear"));
        lines.push(Line::from("/exit"));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from("Tab - switch mode"));
        lines.push(Line::from("/quit"));
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Config",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(shorten(
            &self.paths.default_config_path().display().to_string(),
            32,
        )));

        if let Some(notice) = &self.last_notice {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "Notice",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(shorten(notice, 32)));
        }

        let paragraph = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title("Sidebar"),
            )
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    fn render_input_block(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        placeholder: &str,
        mask_input: bool,
    ) {
        let palette = self.palette();
        let border_style = if self.pending_request {
            Style::default().fg(palette.warning)
        } else {
            Style::default().fg(palette.border_active())
        };

        let content = if self.composer.is_empty() {
            Text::from(Line::from(Span::styled(
                placeholder.to_string(),
                Style::default().fg(palette.muted),
            )))
        } else if mask_input {
            Text::from(Line::from(Span::styled(
                "•".repeat(self.composer.text().chars().count().max(1)),
                Style::default().fg(palette.text),
            )))
        } else {
            Text::from(self.composer.text().to_string())
        };

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let visible_lines = inner.height.max(1) as usize;
        let total_lines = self.composer.text().lines().count().max(1);
        let scroll = total_lines.saturating_sub(visible_lines) as u16;

        let paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));

        frame.render_widget(paragraph, area);

        if inner.width > 0 && inner.height > 0 {
            let (cursor_line, cursor_col) = self.composer.cursor_position();
            let cursor_line = cursor_line.saturating_sub(scroll);
            let cursor_x = inner
                .x
                .saturating_add(cursor_col.min(inner.width.saturating_sub(1)));
            let cursor_y = inner
                .y
                .saturating_add(cursor_line.min(inner.height.saturating_sub(1)));

            frame.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
    }

    fn messages_text(&self) -> (Text<'static>, usize) {
        let palette = self.palette();
        if self.conversation.messages.is_empty() {
            let lines = vec![
                Line::from(vec![Span::styled(
                    "No messages yet.",
                    Style::default().fg(palette.muted),
                )]),
                Line::from(vec![Span::styled(
                    "Start with a prompt in the input box below.",
                    Style::default().fg(palette.muted),
                )]),
            ];
            return (Text::from(lines.clone()), lines.len());
        }

        let mut lines = Vec::new();

        for message in &self.conversation.messages {
            let role_color = match message.role {
                MessageRole::System => palette.warning,
                MessageRole::User => palette.success,
                MessageRole::Assistant => palette.accent,
                MessageRole::Tool => palette.accent_soft,
                MessageRole::Error => palette.error,
            };
            let mut role_label = match message.role {
                MessageRole::Tool => message
                    .tool_name
                    .as_deref()
                    .map(|tool_name| format!("tool · {tool_name}"))
                    .unwrap_or_else(|| message.role.label().to_string()),
                _ if message.streaming => format!("{} · streaming", message.role.label()),
                _ => message.role.label().to_string(),
            };

            if matches!(message.role, MessageRole::Assistant) && !message.tool_calls.is_empty() {
                role_label.push_str(" · tool calls");
            }

            lines.push(Line::from(vec![
                Span::styled(
                    role_label.to_uppercase(),
                    Style::default().fg(role_color).add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    message.created_at.format("%H:%M:%S").to_string(),
                    Style::default().fg(palette.muted),
                ),
            ]));

            if !message.reasoning.trim().is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "Thinking",
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::BOLD | Modifier::ITALIC),
                )]));

                for line in message.reasoning.lines() {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {line}"),
                        Style::default()
                            .fg(palette.muted)
                            .add_modifier(Modifier::ITALIC),
                    )]));
                }
            }

            if !message.tool_calls.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    "Tool calls",
                    Style::default()
                        .fg(palette.warning)
                        .add_modifier(Modifier::BOLD),
                )]));

                for tool_call in &message.tool_calls {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {}", tool_call.name),
                            Style::default().fg(palette.accent_soft),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            shorten(&tool_call.id, 24),
                            Style::default().fg(palette.muted),
                        ),
                    ]));

                    let arguments = pretty_tool_arguments(&tool_call.arguments);
                    for line in arguments.lines() {
                        lines.push(Line::from(vec![Span::styled(
                            format!("    {line}"),
                            Style::default().fg(palette.text),
                        )]));
                    }
                }
            }

            if message.content.is_empty() {
                if message.streaming {
                    lines.push(Line::from(vec![Span::styled(
                        "▌",
                        Style::default().fg(palette.muted),
                    )]));
                } else if message.reasoning.trim().is_empty() && message.tool_calls.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        "(empty)",
                        Style::default().fg(palette.muted),
                    )]));
                }
            } else {
                for line in message.content.lines() {
                    lines.push(Line::from(line.to_string()));
                }

                if message.streaming {
                    lines.push(Line::from(vec![Span::styled(
                        "▌",
                        Style::default().fg(palette.muted),
                    )]));
                }
            }

            lines.push(Line::from(""));
        }

        let total_lines = lines.len().max(1);
        (Text::from(lines), total_lines)
    }

    pub(crate) fn help_message(&self) -> String {
        let mut lines = vec![
            "Commands:",
            "/help - show this message",
            "/connect - open the provider picker",
            "/model - list available models",
            "/model <provider:model> - switch active model",
            "/theme [light|dark] - switch theme",
            "/clear - start a fresh session",
            "/exit - exit TiDev",
            "",
            "Keys:",
            "Enter - send prompt or execute the highlighted slash command",
            "Shift+Enter - insert newline",
            "Tab - switch mode (when no command is being entered)",
            "Up/Down - move through command suggestions",
            "Ctrl+P / Ctrl+N - navigate input history",
            "Ctrl+C - exit",
            "Connect picker - type to filter providers, Enter to select, Esc to cancel",
            "",
            "Modes:",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();

        for mode in SessionMode::all() {
            lines.push(format!("- {} - {}", mode.as_str(), mode.description()));
        }

        lines.join("\n")
    }

    pub(crate) fn model_catalog_message(&self) -> String {
        let mut lines = vec!["Available models:".to_string()];

        for summary in self.config.available_models() {
            lines.push(format!(
                "- {} ({}) · context {} · max output {}",
                summary.label(),
                summary.model_display_name,
                summary.context_window,
                summary.max_output_tokens,
            ));
        }

        lines.join("\n")
    }

    fn render_theme_panel(&self, frame: &mut Frame<'_>, area: Rect, panel: &ThemePanelState) {
        let current_palette = self.palette();
        let overlay = centered_rect(40, 12, area);
        let themes = ThemePanelState::themes();

        let items: Vec<ListItem> = themes
            .iter()
            .map(|theme| {
                ListItem::new(Line::from(vec![Span::styled(
                    format!("  {}  ", theme.as_str()),
                    Style::default()
                        .fg(current_palette.text)
                        .add_modifier(Modifier::BOLD),
                )]))
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(panel.selected_index));

        let panel_block = Block::default()
            .style(Style::default().bg(current_palette.panel_alt))
            .title(" Theme ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(current_palette.border_active()));

        let list = List::new(items)
            .style(
                Style::default()
                    .bg(current_palette.panel_alt)
                    .fg(current_palette.text),
            )
            .highlight_style(
                Style::default()
                    .bg(current_palette.selection_bg)
                    .fg(current_palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

        frame.render_widget(Clear, overlay);
        frame.render_widget(panel_block, overlay);
        frame.render_stateful_widget(
            list,
            overlay.inner(Margin {
                horizontal: 1,
                vertical: 1,
            }),
            &mut state,
        );
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect::new(x, y, width, height)
}

fn shorten(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }

    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

fn short_uuid(id: Uuid) -> String {
    let value = id.simple().to_string();
    value.chars().take(8).collect()
}

fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}
