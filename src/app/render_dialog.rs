use crate::{
    app::mcp_panel::McpPanelState,
    app::mcp_panel::McpServerEditorState,
    app::model_panel::{ModelPanelItem, ModelPanelState},
    app::permission::PermissionDialogState,
    app::question::QuestionDialogState,
    app::session_panel::{SessionPanelDialog, SessionPanelState, SessionViewMode},
    app::theme_panel::ThemePanelState,
    config::ProviderSource,
    provider_setup::{ConnectDialog, EditProviderStep, NewProviderStep},
};
use chrono;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use super::{App, connect::ProviderPickerItem, render::*};

impl App {
    pub(super) fn render_command_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.command_palette.visible || self.command_palette.suggestions.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.command_palette.suggestions.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

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

    pub(super) fn render_at_mention_palette(&self, frame: &mut Frame<'_>, area: Rect) {
        if !self.at_mention.visible || self.at_mention.suggestions.is_empty() {
            return;
        }

        let palette = self.palette();
        let width = area.width.min(72);
        let height = (self.at_mention.suggestions.len() as u16)
            .min(6)
            .saturating_add(2);
        let rect = Rect::new(area.x, area.y.saturating_sub(height), width, height);
        let inner = rect.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let items = self
            .at_mention
            .suggestions
            .iter()
            .map(|suggestion| {
                let path_style = Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD);
                let highlight_style = Style::default()
                    .fg(palette.accent_soft)
                    .add_modifier(Modifier::BOLD);
                let mut path_spans = vec![Span::styled("@", path_style)];
                path_spans.extend(spans_with_highlights(
                    &suggestion.path,
                    &suggestion.matched_indices,
                    path_style,
                    highlight_style,
                ));
                let mut spans = path_spans;
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    suggestion.display.clone(),
                    Style::default().fg(palette.muted),
                ));

                ListItem::new(Line::from(spans))
            })
            .collect::<Vec<_>>();

        let mut state = ListState::default();
        state.select(Some(self.at_mention.selected_index));

        let panel = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(format!("Files · @{}", self.at_mention.query));

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

    pub(super) fn render_connect_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
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
        self.register_selection_region(inner);

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
                                    display_name.to_string(),
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
                                    model.display_name.to_string(),
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

    pub(super) fn render_theme_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ThemePanelState,
    ) {
        let current_palette = self.palette();
        let overlay = centered_rect(40, 18, area);
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
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);
        frame.render_stateful_widget(list, inner, &mut state);
    }

    pub(super) fn render_session_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SessionPanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(area.width.min(112), area.height.min(36), area);
        frame.render_widget(Clear, overlay);

        let view_mode_text = match panel.view_mode {
            SessionViewMode::CurrentWorkspace => "Current Workspace",
            SessionViewMode::AllSessions => "All Sessions",
        };
        let title_text =
            if panel.operation_mode == crate::app::session_panel::OperationMode::MultiSelect {
                format!(
                    " Sessions: {} ({} selected) ",
                    view_mode_text,
                    panel.selected_count()
                )
            } else {
                format!(" Sessions: {} ", view_mode_text)
            };

        let title = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(title_text)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new("Type to filter by title, model, provider, or session id.")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[0],
        );

        self.render_input_block(
            frame,
            sections[1],
            "Search sessions",
            self.composer.placeholder(),
            false,
        );

        let query = self.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        let is_multi_select =
            panel.operation_mode == crate::app::session_panel::OperationMode::MultiSelect;

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No sessions match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[2],
            );
        } else {
            let mut items: Vec<ListItem> = Vec::new();

            let mut current_workspace = String::new();

            for index in matches.iter() {
                let session = &panel.sessions[*index];

                if panel.view_mode == SessionViewMode::AllSessions
                    && session.workspace_root != current_workspace
                {
                    if !current_workspace.is_empty() {
                        items.push(ListItem::new(Line::from("")));
                    }
                    current_workspace = session.workspace_root.clone();
                    items.push(ListItem::new(Line::from(vec![Span::styled(
                        format!("[ {} ]", session.workspace_root),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )])));
                }

                let is_current = session.session_id == self.conversation.session_id;
                let updated_at = session.updated_at.format("%Y-%m-%d %H:%M").to_string();
                let is_selected = panel.is_selected(*index);

                let checkbox = if is_multi_select {
                    if is_selected { "[✓] " } else { "[ ] " }
                } else {
                    ""
                };

                let mut spans = vec![
                    Span::raw(checkbox),
                    Span::styled(
                        shorten(&session.title, 24),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("({})", session.session_id.simple()),
                        Style::default().fg(palette.muted),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!(
                            "{} / {}",
                            shorten(&session.provider_display_name, 12),
                            shorten(&session.model_display_name, 14)
                        ),
                        Style::default().fg(palette.accent_soft),
                    ),
                    Span::raw("  "),
                    Span::styled(updated_at, Style::default().fg(palette.muted)),
                ];

                if is_current {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        "current",
                        Style::default().fg(palette.success),
                    ));
                }
                if session.parent_session_id.is_some() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        "child",
                        Style::default().fg(palette.accent_soft),
                    ));
                }
                items.push(ListItem::new(Line::from(spans)));
            }

            let mut state = ListState::default();
            state.select(Some(
                panel.selected_index.min(matches.len().saturating_sub(1)),
            ));

            let list = List::new(items)
                .style(Style::default().bg(palette.panel).fg(palette.text))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD),
                );

            frame.render_stateful_widget(list, sections[2], &mut state);
        }

        let help_text = if panel.operation_mode
            == crate::app::session_panel::OperationMode::MultiSelect
        {
            "Enter/D: switch/delete · Space: select · Ctrl+A: exit multi-select · Tab: switch view · C: cleanup"
        } else {
            "Enter: switch · D: delete · C: cleanup · Ctrl+A: multi-select · Tab: switch view · W: all sessions"
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[3],
        );
    }

    pub(super) fn render_model_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ModelPanelState,
    ) {
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
        self.register_selection_region(inner);

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

    pub(super) fn render_mcp_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &McpPanelState,
    ) {
        let palette = self.palette();
        let has_editor = panel.editor.is_some();
        let overlay = centered_rect(
            area.width.min(112),
            if has_editor {
                area.height.min(42)
            } else {
                area.height.min(34)
            },
            area,
        );
        frame.render_widget(Clear, overlay);

        let title_text = panel
            .editor
            .as_ref()
            .map(McpServerEditorState::title)
            .unwrap_or_else(|| " MCP servers ".to_string());
        let title = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(title_text)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        if let Some(editor) = &panel.editor {
            let sections = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(inner);

            frame.render_widget(
                Paragraph::new(editor.help())
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[0],
            );

            self.render_input_block(
                frame,
                sections[1],
                editor.step_label(),
                self.composer.placeholder(),
                false,
            );

            frame.render_widget(
                Paragraph::new(editor.draft.summary_text())
                    .style(Style::default().bg(palette.panel).fg(palette.text))
                    .wrap(Wrap { trim: false }),
                sections[2],
            );

            frame.render_widget(
                Paragraph::new("Enter advance/save · Tab advance/save · Esc cancel")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                sections[3],
            );
        } else {
            let sections = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(inner);

            frame.render_widget(
                Paragraph::new("Type to filter by server name, transport, or status. Enter toggles connect/disconnect.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[0],
            );

            self.render_input_block(
                frame,
                sections[1],
                "Search MCP servers",
                self.composer.placeholder(),
                false,
            );

            let items = self.mcp_panel_items();
            let mut rows = Vec::new();
            for item in &items {
                let summary = &item.summary;
                rows.push(ListItem::new(Line::from(vec![
                    Span::styled(
                        summary.name.clone(),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("({})", summary.kind),
                        Style::default().fg(palette.muted),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        summary.status_text(),
                        Style::default().fg(match summary.status.label() {
                            "connected" => palette.success,
                            "connecting" => palette.warning,
                            "failed" => palette.error,
                            _ => palette.muted,
                        }),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{} tools", summary.tool_count),
                        Style::default().fg(palette.accent_soft),
                    ),
                ])));
            }

            if rows.is_empty() {
                frame.render_widget(
                    Paragraph::new("No MCP servers match this search.")
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
                Paragraph::new(
                    "Enter connect/disconnect · a add · e edit · d remove · R refresh · Esc close",
                )
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                sections[3],
            );
        }
    }

    pub(super) fn render_permission_dialog(
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
        self.register_selection_region(inner);

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

    pub(super) fn render_question_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        dialog: &QuestionDialogState,
    ) {
        let palette = self.palette();
        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let options_lines = dialog.options_lines(inner.width);
        let options_text = options_lines.join("\n");

        frame.render_widget(Clear, area);

        let block = Block::default()
            .style(Style::default().bg(palette.panel_alt))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .title(" Question prompt ");
        frame.render_widget(block, area);

        let options_height = options_lines.len().max(2) as u16;
        let available_input_height = inner
            .height
            .saturating_sub(options_height.saturating_add(6));
        let input_height = self
            .composer
            .preferred_height(
                inner.width.saturating_sub(4),
                self.config.ui.max_input_lines,
            )
            .min(available_input_height.max(3));

        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(options_height),
            Constraint::Length(input_height),
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
            Paragraph::new(dialog.body_title())
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new(options_text)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[2],
        );

        self.render_input_block(
            frame,
            sections[3],
            "Answer",
            &dialog.answer_placeholder(),
            false,
        );

        frame.render_widget(
            Paragraph::new("Enter submit · Ctrl+P/Ctrl+N/←/→ previous/next · Esc dismiss")
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.accent_soft),
                ),
            sections[4],
        );
    }

    pub(super) fn render_session_panel_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SessionPanelState,
    ) {
        let palette = self.palette();

        match &panel.dialog {
            SessionPanelDialog::None => {}
            SessionPanelDialog::DeleteConfirm {
                session_ids,
                session_titles,
            } => {
                let overlay = centered_rect(60, 20, area);
                frame.render_widget(Clear, overlay);

                let block = Block::default()
                    .style(Style::default().bg(palette.panel))
                    .title(" Confirm Delete ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_active()));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                self.register_selection_region(inner);
                let sections = Layout::vertical([
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(format!("Delete {} session(s)?", session_ids.len()))
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.text)),
                    sections[0],
                );

                let mut content = String::new();
                for title in session_titles.iter().take(5) {
                    content.push_str(&format!("  • {}\n", title));
                }
                if session_titles.len() > 5 {
                    content.push_str(&format!("  ... and {} more\n", session_titles.len() - 5));
                }

                frame.render_widget(
                    Paragraph::new(content)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[1],
                );

                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                    sections[2],
                );
            }
            SessionPanelDialog::Cleanup {
                preview,
                selected_duration,
                cleanup_workspace,
            } => {
                let overlay = centered_rect(70, 25, area);
                frame.render_widget(Clear, overlay);

                let block = Block::default()
                    .style(Style::default().bg(palette.panel))
                    .title(" Cleanup Old Sessions ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_active()));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                self.register_selection_region(inner);
                let sections = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Min(6),
                    Constraint::Length(3),
                ])
                .split(inner);

                let (title_text, hint_text) = if *cleanup_workspace {
                    (
                        "Delete all sessions in current workspace".to_string(),
                        "5: current workspace (selected)".to_string(),
                    )
                } else {
                    let duration_text = match selected_duration {
                        Some(d) if *d <= chrono::Duration::weeks(1) => "1 week",
                        Some(d) if *d <= chrono::Duration::days(30) => "1 month",
                        Some(d) if *d <= chrono::Duration::days(90) => "3 months",
                        Some(d) if *d <= chrono::Duration::days(365) => "1 year",
                        None => "Select duration",
                        _ => "Custom",
                    };
                    (
                        format!("Delete sessions older than: {}", duration_text),
                        "1: 1 week · 2: 1 month · 3: 3 months · 4: 1 year · 5: current workspace"
                            .to_string(),
                    )
                };

                frame.render_widget(
                    Paragraph::new(title_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.text)),
                    sections[0],
                );

                frame.render_widget(
                    Paragraph::new(hint_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[1],
                );

                frame.render_widget(
                    Paragraph::new(format!(
                        "Preview: {} session(s) will be deleted",
                        preview.total_count
                    ))
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                    sections[2],
                );

                let mut content = String::new();
                for (workspace, count) in preview.workspace_counts.iter().take(5) {
                    content.push_str(&format!("  {} ({} sessions)\n", workspace, count));
                }

                frame.render_widget(
                    Paragraph::new(content)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[3],
                );

                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                    sections[4],
                );
            }
        }
    }
}

fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}
