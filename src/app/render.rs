use crate::{
    app::model_panel::{ModelPanelItem, ModelPanelState},
    app::permission::PermissionDialogState,
    app::theme_panel::ThemePanelState,
    config::ProviderSource,
    markdown::append_markdown,
    prompts::SessionMode,
    provider_setup::{ConnectDialog, EditProviderStep, NewProviderStep},
    session::{Message, MessageRole, ToolCall},
    theme::ThemePalette,
    tooling::canonical_tool_name,
    wrapping::{RtOptions, adaptive_wrap_lines},
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;
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

    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        match self.screen {
            Screen::Welcome => self.render_welcome(frame),
            Screen::Chat => self.render_chat(frame),
        }
        let area = frame.area();
        self.render_connect_dialog(frame, area);
        if let Some(panel) = &self.theme_panel {
            self.render_theme_panel(frame, area, panel);
        }
        if let Some(panel) = &self.model_panel {
            self.render_model_panel(frame, area, panel);
        }
        if let Some(dialog) = &self.permission_dialog {
            self.render_permission_dialog(frame, area, dialog);
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
            "Enter to send · Shift+Enter/Ctrl+J newline · PageUp/PageDown scroll · Ctrl+P/N history · Ctrl+C quit",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(palette.accent_soft));
        frame.render_widget(hint, sections[4]);

        self.render_command_palette(frame, sections[3]);
    }

    fn render_chat(&mut self, frame: &mut Frame<'_>) {
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

    fn render_messages(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(format!(
                "Conversation · {}{}",
                shorten(&self.conversation.title, 32),
                if !self.message_follow_tail { " · history" } else { "" }
            ));
        frame.render_widget(block, area);

        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let scrollbar_area = if inner.width > 1 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            (chunks[0], Some(chunks[1]))
        } else {
            (inner, None)
        };

        let content_area = scrollbar_area.0;
        let content_width = content_area.width.max(1) as usize;
        let (text, total_lines) = self.messages_text(Some(content_width));

        self.message_viewport_lines = content_area.height as usize;
        self.message_total_lines = total_lines;

        let max_scroll = total_lines.saturating_sub(self.message_viewport_lines);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = scroll;
        self.message_follow_tail = scroll >= max_scroll;

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(palette.background).fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((scroll as u16, 0));

        frame.render_widget(paragraph, content_area);

        if let Some(scrollbar_area) = scrollbar_area.1 {
            self.render_scrollbar(frame, scrollbar_area, scroll, max_scroll);
        }
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
        for tool in self.tools.available_definitions(self.mode) {
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
        lines.push(Line::from("/model - open the model panel"));
        lines.push(Line::from("/model <query> - prefilter the model panel"));
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
        let total_lines = self.composer.text().split('\n').count().max(1);
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

    fn messages_text(&self, content_width: Option<usize>) -> (Text<'static>, usize) {
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);

        if self.conversation.messages.is_empty() {
            let lines = decorate_card_lines(
                vec![
                    line_with_style("No messages yet.", palette.muted),
                    line_with_style(
                        "Start with a prompt in the input box below.",
                        palette.muted,
                    ),
                ],
                width,
                palette.panel,
            );
            let total_lines = lines.len().max(1);
            return (Text::from(lines), total_lines);
        }

        let mut lines = Vec::new();

        for message in &self.conversation.messages {
            for (card_bg, card_lines) in self.render_message_cards(message, body_width) {
                if card_lines.is_empty() {
                    continue;
                }

                lines.extend(decorate_card_lines(card_lines, width, card_bg));
                lines.push(Line::from(""));
            }
        }

        if lines.is_empty() {
            let fallback = decorate_card_lines(
                vec![line_with_style("(empty)", palette.muted)],
                width,
                palette.panel,
            );
            let total_lines = fallback.len().max(1);
            return (Text::from(fallback), total_lines);
        }

        let total_lines = lines.len().max(1);
        (Text::from(lines), total_lines)
    }

    fn render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        let palette = self.palette();

        match message.role {
            MessageRole::User => vec![(
                palette.panel_alt,
                self.render_text_body_lines(&message.content, body_width, Some(self.workspace_root.as_path())),
            )],
            MessageRole::Assistant => {
                let mut cards = Vec::new();
                let body_lines = self.render_assistant_body_lines(message, body_width);
                if !body_lines.is_empty() {
                    cards.push((palette.panel, body_lines));
                }

                for tool_call in &message.tool_calls {
                    cards.push((palette.panel_alt, self.render_tool_call_lines(tool_call, body_width)));
                }

                cards
            }
            MessageRole::Tool => {
                let lines = self.render_tool_result_lines(message, body_width);
                if lines.is_empty() {
                    Vec::new()
                } else {
                    vec![(palette.panel, lines)]
                }
            }
            MessageRole::System => vec![(
                palette.background,
                self.render_text_body_lines(&message.content, body_width, Some(self.workspace_root.as_path())),
            )],
            MessageRole::Error => vec![(
                palette.panel_alt,
                self.render_error_body_lines(message, body_width),
            )],
        }
    }

    fn render_assistant_body_lines(&self, message: &Message, body_width: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if !message.reasoning.trim().is_empty() {
            lines.extend(self.render_reasoning_lines(&message.reasoning, body_width));
            if !message.content.trim().is_empty() {
                lines.push(Line::from(""));
            }
        }

        if message.streaming && matches!(message.role, MessageRole::Assistant) {
            if !self.streaming_preview_lines.is_empty() {
                if let Some(width) = Some(body_width) {
                    let wrapped_preview = adaptive_wrap_lines(
                        self.streaming_preview_lines.iter(),
                        RtOptions::new(width),
                    );
                    lines.extend(wrapped_preview);
                } else {
                    lines.extend(self.streaming_preview_lines.clone());
                }
            }

            let tail = message
                .content
                .rsplit_once('\n')
                .map(|(_, tail)| tail)
                .unwrap_or(message.content.as_str());
            if !tail.is_empty() {
                lines.push(line_with_prefix(
                    "▌",
                    tail,
                    Style::default().fg(self.palette().accent),
                    Style::default().fg(self.palette().text),
                ));
            } else if lines.is_empty() {
                lines.push(line_with_style("▌", self.palette().muted));
            }
        } else if !message.content.is_empty() {
            append_markdown(
                &message.content,
                Some(body_width),
                Some(self.workspace_root.as_path()),
                &mut lines,
            );
        }

        if lines.is_empty() && message.reasoning.trim().is_empty() && message.tool_calls.is_empty() {
            lines.push(line_with_style("(empty)", self.palette().muted));
        }

        lines
    }

    fn render_text_body_lines(
        &self,
        text: &str,
        body_width: usize,
        cwd: Option<&std::path::Path>,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if text.trim().is_empty() {
            lines.push(line_with_style("(empty)", self.palette().muted));
        } else {
            append_markdown(text, Some(body_width), cwd, &mut lines);
        }
        lines
    }

    fn render_reasoning_lines(&self, reasoning: &str, body_width: usize) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        for line in reasoning.lines() {
            let content = shorten_single_line(line, body_width.saturating_sub(2));
            lines.push(line_with_prefix(
                "│",
                &content,
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("│", palette.muted));
        }

        lines
    }

    fn render_error_body_lines(&self, message: &Message, body_width: usize) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();
        let error_text = if message.content.trim().is_empty() {
            "Request failed.".to_string()
        } else {
            message.content.clone()
        };

        for line in error_text.lines() {
            lines.push(line_with_prefix(
                "!",
                &shorten_single_line(line, body_width.saturating_sub(2)),
                Style::default().fg(palette.error),
                Style::default().fg(palette.error),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("! Request failed.", palette.error));
        }

        lines
    }

    fn render_tool_call_lines(&self, tool_call: &ToolCall, body_width: usize) -> Vec<Line<'static>> {
        let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);
        let palette = self.palette();

        vec![line_with_prefix(
            "│",
            &summary,
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        )]
    }

    fn render_tool_result_lines(&self, message: &Message, body_width: usize) -> Vec<Line<'static>> {
        let tool_name = message.tool_name.as_deref().unwrap_or(message.role.label());
        let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
        let output = message.content.trim_end();

        if output.is_empty() {
            return Vec::new();
        }

        if matches!(canonical_name, "read" | "write" | "edit" | "list" | "todowrite") {
            if tool_output_is_error(output) {
                return self.render_output_preview_lines(output, body_width, true);
            }

            return Vec::new();
        }

        self.render_output_preview_lines(output, body_width, tool_output_is_error(output))
    }

    fn render_output_preview_lines(
        &self,
        output: &str,
        body_width: usize,
        is_error: bool,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();
        let max_lines = if is_error { 4 } else { 5 };
        let prefix = if is_error { "!" } else { "↳" };
        let fg = if is_error { palette.error } else { palette.text };

        for line in output.lines().take(max_lines) {
            lines.push(line_with_prefix(
                prefix,
                &shorten_single_line(line, body_width.saturating_sub(2)),
                Style::default().fg(if is_error { palette.error } else { palette.accent_soft }),
                Style::default().fg(fg),
            ));
        }

        if output.lines().count() > max_lines {
            lines.push(line_with_prefix(
                prefix,
                &format!("... {} more line(s)", output.lines().count() - max_lines),
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("(no output)", palette.muted));
        }

        lines
    }

    fn render_scrollbar(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        max_scroll: usize,
    ) {
        let palette = self.palette();
        if area.width == 0 || area.height == 0 {
            return;
        }

        let track_style = Style::default().bg(palette.background).fg(palette.border);
        let thumb_style = Style::default().bg(palette.background).fg(palette.accent);
        let height = area.height as usize;
        let mut lines = Vec::with_capacity(height);

        if max_scroll == 0 || height == 0 {
            for _ in 0..height {
                lines.push(Line::from(vec![Span::styled(" ", track_style)]));
            }
        } else {
            let thumb_height = ((height * height) / self.message_total_lines.max(1))
                .clamp(1, height)
                .max(1);
            let track_span = height.saturating_sub(thumb_height);
            let thumb_top = if track_span == 0 {
                0
            } else {
                ((scroll as f32 / max_scroll as f32) * track_span as f32).round() as usize
            };

            for row in 0..height {
                let is_thumb = row >= thumb_top && row < thumb_top + thumb_height;
                let style = if is_thumb { thumb_style } else { track_style };
                let glyph = if is_thumb { "█" } else { "░" };
                lines.push(Line::from(vec![Span::styled(glyph, style)]));
            }
        }

        let paragraph = Paragraph::new(Text::from(lines)).style(Style::default().bg(palette.background));
        frame.render_widget(paragraph, area);
    }

    pub(crate) fn help_message(&self) -> String {
        let mut lines = vec![
            "Commands:",
            "/help - show this message",
            "/connect - open the provider picker",
            "/model - open the model panel",
            "/model <query> - prefilter the model panel",
            "/theme [light|dark] - switch theme",
            "/clear - start a fresh session",
            "/exit - exit TiDev",
            "",
            "Keys:",
            "Enter - send prompt or execute the highlighted slash command",
            "Shift+Enter / Ctrl+J - insert newline",
            "PageUp / PageDown / mouse wheel - scroll conversation",
            "Tab - switch mode (when no command is being entered)",
            "Up/Down - move through command suggestions",
            "Ctrl+P / Ctrl+N - navigate input history",
            "Ctrl+C - exit",
            "Permission prompt - Y allow · N deny · R allow and remember · X deny and remember",
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

    fn render_model_panel(&self, frame: &mut Frame<'_>, area: Rect, panel: &ModelPanelState) {
        let palette = self.palette();
        let overlay = centered_rect(area.width.min(104), area.height.min(34), area);
        frame.render_widget(Clear, overlay);

        let title = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(" Select model ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(
                "Type to filter by provider or model. Enter switches to the highlighted model.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[0],
        );

        self.render_input_block(
            frame,
            sections[1],
            "Search models",
            self.composer.placeholder(),
            false,
        );

        let items = self.model_panel_items();
        let mut rows = Vec::new();
        for item in &items {
            match item {
                ModelPanelItem::ProviderHeader {
                    provider_id,
                    display_name,
                } => {
                    rows.push(ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("{display_name}"),
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
                    rows.push(ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("  {}", summary.model_display_name),
                            Style::default()
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            format!("({})", summary.model_id),
                            Style::default().fg(palette.muted),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            format!(
                                "{} · max {}",
                                summary.provider_display_name, summary.max_output_tokens
                            ),
                            Style::default().fg(palette.accent_soft),
                        ),
                    ])));
                }
            }
        }

        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No connected models match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[2],
            );
        } else {
            let mut state = ListState::default();
            state.select(Some(
                panel.selected_index.min(items.len().saturating_sub(1)),
            ));

            let list = List::new(rows)
                .style(Style::default().bg(palette.panel).fg(palette.text))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD),
                );

            frame.render_stateful_widget(list, sections[2], &mut state);
        }

        frame.render_widget(
            Paragraph::new("Enter switch · Ctrl+E edit selected provider · Esc close")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[3],
        );
    }

    fn render_permission_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        dialog: &PermissionDialogState,
    ) {
        let palette = self.palette();
        let preview = pretty_tool_arguments(&dialog.tool_call.arguments);
        let preview_height = preview.lines().count().min(8) as u16;
        let overlay = centered_rect(area.width.min(96), preview_height.saturating_add(10), area);
        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(" Tool approval ");
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(dialog.title())
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(
                "This tool can change state. Review the arguments and choose whether to allow it.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new(preview)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[2],
        );

        frame.render_widget(
            Paragraph::new(
                "Y allow · N deny · R allow and remember · X deny and remember · Esc deny",
            )
            .alignment(Alignment::Center)
            .style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.accent_soft),
            ),
            sections[3],
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

fn display_tool_name(tool_name: &str) -> String {
    canonical_tool_name(tool_name)
        .unwrap_or(tool_name)
        .to_string()
}

fn summarize_tool_call(tool_name: &str, arguments: &str, body_width: usize) -> String {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let fields = summarize_tool_arguments(tool_name, arguments);

    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let summary = match canonical_name {
        "read" => field("path")
            .map(|path| format!("read file {path}"))
            .unwrap_or_else(|| "read file".to_string()),
        "write" => field("path")
            .map(|path| format!("write file {path}"))
            .unwrap_or_else(|| "write file".to_string()),
        "edit" => field("path")
            .map(|path| format!("edit file {path}"))
            .unwrap_or_else(|| "edit file".to_string()),
        "list" => field("path")
            .map(|path| format!("list items under path {path}"))
            .unwrap_or_else(|| "list items under path .".to_string()),
        "glob" => {
            let pattern = field("pattern").unwrap_or("*");
            let path = field("path").unwrap_or(".");
            format!("find {pattern} under path {path}")
        }
        "grep" => {
            let pattern = field("pattern").unwrap_or("");
            let path = field("path").unwrap_or(".");
            if pattern.is_empty() {
                format!("search under path {path}")
            } else {
                format!("grep {pattern} under path {path}")
            }
        }
        "bash" => field("command")
            .map(|command| format!("run shell command {command}"))
            .unwrap_or_else(|| "run shell command".to_string()),
        "todowrite" => fields
            .iter()
            .find(|(key, _)| key == "todos")
            .map(|(_, value)| format!("update todo list with {value}"))
            .unwrap_or_else(|| "update todo list".to_string()),
        _ => {
            let mut summary = display_tool_name(tool_name);
            for (label, value) in fields.iter().take(2) {
                summary.push(' ');
                summary.push_str(label);
                summary.push(' ');
                summary.push_str(value);
            }
            summary
        }
    };

    shorten_single_line(&summary, body_width.saturating_sub(2))
}

fn summarize_tool_arguments(tool_name: &str, arguments: &str) -> Vec<(String, String)> {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let parsed = serde_json::from_str::<Value>(arguments).ok();
    let mut fields = Vec::new();

    let string_field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(|value| shorten_single_line(value, 96))
    };

    match canonical_name {
        "read" | "write" | "edit" => {
            if let Some(path) = string_field("path") {
                fields.push(("path".to_string(), path));
            }
        }
        "list" => {
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
        }
        "glob" => {
            if let Some(pattern) = string_field("pattern") {
                fields.push(("pattern".to_string(), pattern));
            }
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
        }
        "grep" => {
            if let Some(pattern) = string_field("pattern") {
                fields.push(("pattern".to_string(), pattern));
            }
            fields.push((
                "path".to_string(),
                string_field("path").unwrap_or_else(|| ".".to_string()),
            ));
            if let Some(include) = string_field("include") {
                fields.push(("include".to_string(), include));
            }
        }
        "bash" => {
            if let Some(command) = string_field("command") {
                fields.push(("command".to_string(), command));
            }
        }
        "todowrite" => {
            let todo_count = parsed
                .as_ref()
                .and_then(|value| value.get("todos"))
                .and_then(Value::as_array)
                .map(|todos| format!("{} item(s)", todos.len()));

            if let Some(todo_count) = todo_count {
                fields.push(("todos".to_string(), todo_count));
            }
        }
        _ => {}
    }

    if fields.is_empty() {
        fields.push((
            "arguments".to_string(),
            shorten_single_line(&pretty_tool_arguments(arguments), 120),
        ));
    }

    fields
}

fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();

    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
}

fn shorten_single_line(value: &str, max_chars: usize) -> String {
    let single_line = value.replace('\n', " ").replace('\r', "");
    shorten(&single_line, max_chars)
}

fn line_with_style(text: &str, fg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        text.to_string(),
        Style::default().fg(fg),
    )])
}

fn line_with_prefix(
    prefix: &str,
    text: &str,
    prefix_style: Style,
    text_style: Style,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{prefix} "), prefix_style),
        Span::styled(text.to_string(), text_style),
    ])
}

fn decorate_card_lines(lines: Vec<Line<'static>>, width: usize, background: Color) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| decorate_card_line(line, width, background))
        .collect()
}

fn decorate_card_line(line: Line<'static>, width: usize, background: Color) -> Line<'static> {
    let bg_style = Style::default().bg(background);
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(2));
    spans.push(Span::styled(" ", bg_style));

    for mut span in line.spans {
        span.style = span.style.patch(bg_style);
        spans.push(span);
    }

    let used_width = line_display_width(&Line::from(spans.clone()));
    if used_width < width {
        spans.push(Span::styled(" ".repeat(width - used_width), bg_style));
    }

    Line::from(spans)
}

fn line_display_width(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}
