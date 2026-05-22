use crate::tui::App;
use crate::tui::render::render::centered_rect;
use crate::tui::ui::connect::ProviderPickerItem;
use crate::tui::ui::sensitive::SensitiveFileDialogState;
use crate::{
    config::ProviderSource,
    provider_setup::{ConnectDialog, EditProviderStep, NewProviderStep},
    tui::{
        permission::PermissionDialogState,
        question::QuestionDialogState,
        session_panel::{SessionPanelDialog, SessionPanelState},
        ui::{rename::RenameSessionDialogState, workspace_boundary::WorkspaceBoundaryDialogState},
    },
};
use chrono;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

impl App {
    pub(crate) fn render_connect_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
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

        let panel = Block::default().style(Style::default().bg(palette.panel_alt));

        frame.render_widget(panel, overlay);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let dialog_title = match dialog {
            ConnectDialog::ProviderPicker { .. } => " Connect to provider ",
            ConnectDialog::ApiKey { .. } => " API Key ",
            ConnectDialog::NewProvider { .. } => " New Provider ",
            ConnectDialog::EditProvider { .. } => " Edit Provider ",
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog_title,
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

        match dialog {
            ConnectDialog::ProviderPicker { selected } => {
                let sections = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(3),
                    Constraint::Min(6),
                    Constraint::Length(1),
                ])
                .split(body);

                frame.render_widget(
                    Paragraph::new("Type to filter by provider id or display name. Press Ctrl+E to edit custom providers.")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text))
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
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                .split(body);

                frame.render_widget(
                    Paragraph::new(format!("Enter API key for {label}"))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
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
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                .split(body);

                frame.render_widget(
                    Paragraph::new(format!("Create provider · {}", step.title()))
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    lines[0],
                );

                frame.render_widget(
                    Paragraph::new(step.help())
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                    .split(body);

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
                                .bg(palette.panel_alt)
                                .fg(palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new(model_step.help())
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.accent_soft),
                            ),
                        lines[3],
                    );

                    frame.render_widget(
                        Paragraph::new("Model ids stay fixed while editing existing models")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                        lines[4],
                    );
                } else if *step == EditProviderStep::ModelList {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Min(8),
                        Constraint::Length(1),
                    ])
                    .split(body);

                    frame.render_widget(
                        Paragraph::new(format!("Manage models for {}", provider_id))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text))
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
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.accent_soft),
                            ),
                        lines[2],
                    );
                } else if *step == EditProviderStep::ConfirmDeleteModel {
                    let lines = Layout::vertical([
                        Constraint::Length(2),
                        Constraint::Length(2),
                        Constraint::Length(4),
                        Constraint::Length(1),
                    ])
                    .split(body);

                    let pending = draft
                        .pending_delete_model_id
                        .as_deref()
                        .unwrap_or("unknown model");

                    frame.render_widget(
                        Paragraph::new(format!("Delete model {pending}?"))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.error)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new("This only removes the model from config.toml. Historical sessions keep their stored snapshot.")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                        lines[1],
                    );

                    self.render_input_block(frame, lines[2], "Confirm", "y or n", false);

                    frame.render_widget(
                        Paragraph::new("Y to delete · N / Esc to keep")
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.accent_soft),
                            ),
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
                    .split(body);

                    frame.render_widget(
                        Paragraph::new(format!("Edit provider · {}", provider_id))
                            .alignment(Alignment::Center)
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.text)
                                    .add_modifier(Modifier::BOLD),
                            ),
                        lines[0],
                    );

                    frame.render_widget(
                        Paragraph::new(step.help())
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                            .style(
                                Style::default()
                                    .bg(palette.panel_alt)
                                    .fg(palette.accent_soft),
                            ),
                        lines[3],
                    );

                    frame.render_widget(
                        Paragraph::new("After the fields, manage models from the list")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                        lines[4],
                    );
                }
            }
        }
    }

    pub(crate) fn render_permission_dialog(
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

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Tool approval ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(dialog.title())
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new(
                "This tool can change state. Review the arguments and choose whether to allow it.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        frame.render_widget(
            Paragraph::new(preview)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[3],
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
            sections[4],
        );
    }

    pub(crate) fn render_workspace_boundary_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        dialog: &WorkspaceBoundaryDialogState,
    ) {
        let palette = self.palette();
        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        frame.render_widget(Clear, area);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, area);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new("A tool is trying to access a path outside the workspace:")
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        let path_text = format!(
            "Requested: {}\nWorkspace: {}",
            dialog.path_display(),
            dialog.workspace_display()
        );
        frame.render_widget(
            Paragraph::new(path_text).style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.accent_soft),
            ),
            sections[2],
        );

        frame.render_widget(
            Paragraph::new("Y allow once · A allow until exit · N deny once · D deny until exit · Esc deny once")
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[3],
        );
    }

    pub(crate) fn render_sensitive_file_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        dialog: &SensitiveFileDialogState,
    ) {
        let palette = self.palette();
        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        frame.render_widget(Clear, area);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, area);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new("A tool is trying to read a sensitive file:")
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        let path_text = format!(
            "Requested: {}\nWorkspace: {}",
            dialog.path_display(),
            dialog.workspace_display()
        );
        frame.render_widget(
            Paragraph::new(path_text).style(
                Style::default()
                    .bg(palette.panel_alt)
                    .fg(palette.accent_soft),
            ),
            sections[2],
        );

        frame.render_widget(
            Paragraph::new("Y allow once · A allow until exit · N deny once · D deny until exit · Esc deny once")
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            sections[3],
        );
    }

    pub(crate) fn render_fork_confirm_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(dialog) = &self.fork_confirm_dialog else {
            return;
        };

        let palette = self.palette();
        // 使用居中矩形，在屏幕中间显示
        let overlay = centered_rect(60, 10, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // 描述文本
        frame.render_widget(
            Paragraph::new(dialog.description())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[2],
        );

        // 底部提示
        frame.render_widget(
            Paragraph::new("Enter to confirm · Esc or N to cancel")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[3],
        );
    }

    pub(crate) fn render_undo_confirm_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(dialog) = &self.undo_confirm_dialog else {
            return;
        };

        let palette = self.palette();
        // 使用居中矩形，在屏幕中间显示
        let overlay = centered_rect(60, 12, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        // 描述文本
        frame.render_widget(
            Paragraph::new(dialog.description())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[2],
        );

        // 底部提示
        frame.render_widget(
            Paragraph::new("Enter to confirm · Esc or N to cancel")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[3],
        );
    }

    pub(crate) fn render_question_dialog(
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

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, area);

        let options_height = options_lines.len().max(2) as u16;
        let body_height = dialog.body_height(inner.width);
        let sections = if dialog.editing_custom {
            let available_input_height = inner
                .height
                .saturating_sub(options_height.saturating_add(6));
            let input_height = self
                .composer
                .preferred_height(
                    inner.width.saturating_sub(3),
                    self.config.ui.max_input_lines,
                )
                .min(available_input_height.max(3));

            Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(body_height),
                Constraint::Min(options_height),
                Constraint::Min(input_height),
                Constraint::Length(1),
            ])
            .split(inner)
        } else {
            Layout::vertical([
                Constraint::Length(1),
                Constraint::Min(body_height),
                Constraint::Min(options_height),
                Constraint::Length(1),
            ])
            .split(inner)
        };

        let footer_text = if dialog.editing_custom {
            "Enter save custom answer · Esc cancel · Ctrl+P/Ctrl+N/←/→ previous/next"
        } else {
            "Enter select · Space toggle · Ctrl+P/Ctrl+N/←/→ previous/next · Esc dismiss"
        };

        frame.render_widget(
            Paragraph::new(dialog.title())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false })
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
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new(options_text)
                .wrap(Wrap { trim: false })
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[2],
        );

        if dialog.editing_custom {
            self.render_input_block(
                frame,
                sections[3],
                "Answer",
                &dialog.answer_placeholder(),
                false,
            );
        }

        frame.render_widget(
            Paragraph::new(footer_text)
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.accent_soft),
                ),
            if dialog.editing_custom {
                sections[4]
            } else {
                sections[3]
            },
        );
    }

    pub(crate) fn render_session_panel_dialog(
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

                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                self.register_selection_region(inner);
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Delete session(s) ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                frame.render_widget(
                    Paragraph::new(format!("Delete {} session(s)?", session_ids.len()))
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[1],
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[2],
                );

                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[3],
                );
            }
            SessionPanelDialog::ExportConfirm {
                session_ids,
                session_titles,
            } => {
                let overlay = centered_rect(60, 20, area);
                frame.render_widget(Clear, overlay);

                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                self.register_selection_region(inner);
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Export session(s) ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                frame.render_widget(
                    Paragraph::new(format!("Export {} session(s) to JSONL?", session_ids.len()))
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[1],
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[2],
                );

                frame.render_widget(
                    Paragraph::new("Enter: export · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[3],
                );
            }
            SessionPanelDialog::Cleanup {
                preview,
                selected_duration,
                cleanup_workspace,
            } => {
                let overlay = centered_rect(70, 25, area);
                frame.render_widget(Clear, overlay);

                let block = Block::default().style(Style::default().bg(palette.panel_alt));
                frame.render_widget(block, overlay);

                let inner = overlay.inner(Margin {
                    horizontal: 1,
                    vertical: 1,
                });
                self.register_selection_region(inner);
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Min(6),
                    Constraint::Length(3),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Cleanup Sessions ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                    sections[1],
                );

                frame.render_widget(
                    Paragraph::new(hint_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[2],
                );

                frame.render_widget(
                    Paragraph::new(format!(
                        "Preview: {} session(s) will be deleted",
                        preview.total_count
                    ))
                    .alignment(Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                    sections[3],
                );

                let mut content = String::new();
                for (workspace, count) in preview.workspace_counts.iter().take(5) {
                    content.push_str(&format!("  {} ({} sessions)\n", workspace, count));
                }

                frame.render_widget(
                    Paragraph::new(content)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[4],
                );

                frame.render_widget(
                    Paragraph::new("Enter: confirm · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(
                            Style::default()
                                .bg(palette.panel_alt)
                                .fg(palette.accent_soft),
                        ),
                    sections[5],
                );
            }
        }
    }

    pub(crate) fn render_rename_session_dialog(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        dialog: &RenameSessionDialogState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(60, 12, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                dialog.title(),
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(dialog.description())
                .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
            sections[1],
        );

        frame.render_widget(
            Paragraph::new("Press Enter to save, Esc to cancel")
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        frame.render_widget(
            Paragraph::new(self.composer.text())
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .wrap(Wrap { trim: false }),
            sections[4],
        );
    }
    pub(crate) fn render_sync_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &crate::tui::ui::sync_panel::SyncPanelState,
    ) {
        use crate::tui::ui::sync_panel::{AddRemoteStep, SyncView};

        let palette = self.palette();
        let (overlay_width, overlay_height) = match &panel.view {
            SyncView::RemoteList => (72, 18),
            SyncView::RemoteActions { .. } => (60, 16),
            SyncView::SessionPicker { .. } => (80, 24),
            SyncView::AddRemote { .. } => (66, 10),
            SyncView::Confirm { .. } => (60, 10),
            SyncView::Result { .. } => (60, 10),
        };
        let overlay = centered_rect(overlay_width, overlay_height, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match &panel.view {
            SyncView::RemoteList => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Sync - Remote Machines ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                let remotes = &self.config.sync.remotes;
                if remotes.is_empty() {
                    frame.render_widget(
                        Paragraph::new("No remotes configured. Press 'a' to add one.")
                            .style(Style::default().fg(palette.muted))
                            .alignment(Alignment::Center),
                        sections[1],
                    );
                } else {
                    let items: Vec<ListItem> = remotes
                        .iter()
                        .enumerate()
                        .map(|(i, r)| {
                            let prefix = if i == panel.selected_index {
                                "> "
                            } else {
                                "  "
                            };
                            let last = r.last_sync_at.as_deref().unwrap_or("never");
                            let text =
                                format!("{}{}  ({})  last sync: {}", prefix, r.name, r.host, last);
                            let style = if i == panel.selected_index {
                                Style::default().fg(palette.accent)
                            } else {
                                Style::default().fg(palette.text)
                            };
                            ListItem::new(text).style(style)
                        })
                        .collect();
                    frame.render_widget(
                        List::new(items).style(Style::default().bg(palette.panel_alt)),
                        sections[1],
                    );
                }

                frame.render_widget(
                    Paragraph::new("a: add remote  up/down: navigate  Enter: select  Esc: close")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(palette.muted)),
                    sections[2],
                );
            }

            SyncView::RemoteActions { remote_index } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

                if let Some(remote) = self.config.sync.remotes.get(*remote_index) {
                    frame.render_widget(
                        Paragraph::new(Line::from(vec![Span::styled(
                            format!(" {} - Actions ", remote.name),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        )]))
                        .style(Style::default().bg(palette.panel_alt)),
                        sections[0],
                    );

                    frame.render_widget(
                        Paragraph::new(format!("Host: {}", remote.host))
                            .style(Style::default().fg(palette.muted)),
                        sections[1],
                    );

                    frame.render_widget(
                        Paragraph::new(
                            "p  Push sessions to remote\nu  Pull sessions from remote\nt  Test SSH connection\nd  Remove this remote",
                        )
                        .style(Style::default().fg(palette.text)),
                        sections[2],
                    );

                    frame.render_widget(
                        Paragraph::new("Esc: back")
                            .alignment(Alignment::Center)
                            .style(Style::default().fg(palette.muted)),
                        sections[3],
                    );
                }
            }

            SyncView::SessionPicker {
                action,
                selected_indices,
                cursor,
                ..
            } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(1),
                ])
                .split(inner);

                let action_label = action.label();
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(" {} - Select Sessions ", action_label),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                let count = selected_indices.len();
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} session(s) selected.  Space: toggle  Ctrl+A: all  Enter: confirm",
                        count
                    ))
                    .style(Style::default().fg(palette.muted)),
                    sections[1],
                );

                let items: Vec<ListItem> = panel
                    .sessions
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let checked = if selected_indices.contains(&i) {
                            "[*]"
                        } else {
                            "[ ]"
                        };
                        let prefix = if i == *cursor { "> " } else { "  " };
                        let title = if s.title.is_empty() {
                            &s.session_id.to_string()[..8]
                        } else {
                            &s.title
                        };
                        let style = if i == *cursor {
                            Style::default().fg(palette.accent)
                        } else {
                            Style::default().fg(palette.text)
                        };
                        ListItem::new(format!("{}{}{}  {}", prefix, checked, title, s.model_id))
                            .style(style)
                    })
                    .collect();

                frame.render_widget(
                    List::new(items).style(Style::default().bg(palette.panel_alt)),
                    sections[2],
                );

                frame.render_widget(
                    Paragraph::new("up/down: navigate  Space: toggle  Esc: back")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(palette.muted)),
                    sections[3],
                );
            }

            SyncView::AddRemote { step, .. } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ])
                .split(inner);

                let step_title = match step {
                    AddRemoteStep::Host => "Step 1/2 - SSH Host",
                    AddRemoteStep::Name => "Step 2/2 - Display Name",
                };

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        format!(" {} ", step_title),
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                let prompt = match step {
                    AddRemoteStep::Host => {
                        "SSH host alias or user@host (e.g. devbox or eslzzyl@192.168.1.100):"
                    }
                    AddRemoteStep::Name => "Friendly name (or empty to use host):",
                };

                frame.render_widget(
                    Paragraph::new(prompt).style(Style::default().fg(palette.text)),
                    sections[1],
                );

                let hint = match step {
                    AddRemoteStep::Host => "Enter: continue  Esc: cancel",
                    AddRemoteStep::Name => "Enter: confirm  Backspace: previous step  Esc: cancel",
                };
                frame.render_widget(
                    Paragraph::new(hint)
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(palette.muted)),
                    sections[3],
                );
            }

            SyncView::Confirm { message, .. } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        " Confirm ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                frame.render_widget(
                    Paragraph::new(message.as_str())
                        .style(Style::default().fg(palette.text))
                        .alignment(Alignment::Center),
                    sections[1],
                );

                frame.render_widget(
                    Paragraph::new("y/Enter: confirm  n/Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(palette.muted)),
                    sections[2],
                );
            }

            SyncView::Result { message, success } => {
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(1),
                    Constraint::Length(3),
                ])
                .split(inner);

                let title = if *success { "Success" } else { "Error" };
                let title_color = if *success {
                    palette.success
                } else {
                    palette.error
                };

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        title,
                        Style::default()
                            .fg(title_color)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                frame.render_widget(
                    Paragraph::new(message.as_str()).style(Style::default().fg(palette.text)),
                    sections[1],
                );

                frame.render_widget(
                    Paragraph::new("Enter/Esc: close")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(palette.muted)),
                    sections[2],
                );
            }
        }
    }
}

pub(super) fn pretty_tool_arguments(arguments: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| arguments.to_string()),
        Err(_) => arguments.to_string(),
    }
}
