use crate::tui::App;
use crate::tui::render::render::{centered_rect, render_scrollbar, shorten};
use crate::{
    tui::mcp_panel::McpPanelState,
    tui::mcp_panel::McpServerEditorState,
    tui::memory_panel::{MemoryPanelMode, MemoryPanelState},
    tui::message_panel::MessagePanelState,
    tui::model_panel::{ModelPanelItem, ModelPanelState},
    tui::session_panel::{SessionPanelState, SessionViewMode},
    tui::settings_panel::SettingsPanelState,
    tui::theme_panel::ThemePanelState,
    tui::ui::agents_panel::AgentsPanelState,
    tui::ui::skills_panel::SkillsPanelState,
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
        Wrap,
    },
};

impl App {
    pub(crate) fn render_theme_panel(
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

    pub(crate) fn render_agents_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &AgentsPanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(70, 24, area);

        frame.render_widget(Clear, overlay);

        let panel_block = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(" Agents ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        // Header line
        let header = Line::from(vec![
            Span::styled(
                "  Agent",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                "Description",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(header).style(Style::default().bg(palette.panel)),
            inner,
        );

        let divider = Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(palette.muted),
        ));
        let sections = Layout::vertical([
            Constraint::Length(1), // header
            Constraint::Length(1), // divider
            Constraint::Min(0),    // content
        ])
        .split(inner);
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().bg(palette.panel)),
            sections[1],
        );

        // Content area with scrollbar
        let content_area = sections[2];
        let (content_area, scrollbar_area) = if content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(content_area);
            (chunks[0], Some(chunks[2]))
        } else if content_area.width > 1 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (content_area, None)
        };

        // Agent rows with scroll offset
        let mut lines: Vec<Line<'_>> = Vec::new();
        let scroll = panel.scroll_offset;
        let visible_height = content_area.height as usize;
        for agent in panel.agents.iter().skip(scroll).take(visible_height) {
            let tag = if agent.read_only { " [read-only]" } else { "" };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  @{}", agent.display_name),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("  {}{}", agent.description, tag),
                    Style::default().fg(palette.muted),
                ),
            ]));
        }

        // If there's room, show a footer hint after the content
        let remaining = visible_height.saturating_sub(lines.len());
        if remaining >= 2 {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  ↑/↓ scroll · Esc/q close",
                Style::default().fg(palette.muted),
            )));
        }

        // Fill remaining space with empty lines
        while lines.len() < visible_height {
            lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(lines).style(Style::default().bg(palette.panel)),
            content_area,
        );

        // Render scrollbar
        if let Some(sb_area) = scrollbar_area {
            render_scrollbar(
                frame,
                sb_area,
                panel.scroll_offset,
                panel.agents.len() + 2, // agents + footer
                palette,
            );
        }
    }

    pub(crate) fn render_settings_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SettingsPanelState,
    ) {
        use crate::tui::ui::settings_panel::SettingType;
        let current_palette = self.palette();
        let overlay = centered_rect(60, 12, area);

        let items: Vec<ListItem> = panel
            .items
            .iter()
            .map(|item| {
                let status = match item.setting_type {
                    SettingType::Toggle(true) => "[x]",
                    SettingType::Toggle(false) => "[ ]",
                    SettingType::Number { .. } => "[~]",
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", status),
                            Style::default()
                                .fg(match item.setting_type {
                                    SettingType::Toggle(true) => current_palette.accent,
                                    _ => current_palette.muted,
                                })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            &item.name,
                            Style::default()
                                .fg(current_palette.text)
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                    Line::from(vec![
                        Span::raw("    "),
                        Span::styled(
                            &item.description,
                            Style::default().fg(current_palette.muted),
                        ),
                    ]),
                ])
            })
            .collect();

        let mut state = ListState::default();
        state.select(Some(panel.selected_index));

        let panel_block = Block::default()
            .style(Style::default().bg(current_palette.panel_alt))
            .title(" Settings ")
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
                    .fg(current_palette.selection_fg),
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

    pub(crate) fn render_session_panel(
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
            if panel.operation_mode == crate::tui::session_panel::OperationMode::MultiSelect {
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
            panel.operation_mode == crate::tui::session_panel::OperationMode::MultiSelect;

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No sessions match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[2],
            );
        } else {
            // Compute minimum width needed for the right column
            let max_right_width = matches
                .iter()
                .map(|&idx| {
                    let session = &panel.sessions[idx];
                    let pm = format!(
                        "{} / {}",
                        shorten(&session.provider_display_name, 12),
                        shorten(&session.model_display_name, 14)
                    );
                    let time = session.updated_at.format("%Y-%m-%d %H:%M").to_string();
                    let mut w = pm.chars().count() + 2 + time.chars().count();
                    if session.session_id == self.conversation.session_id {
                        w += 9; // "  current"
                    }
                    if session.parent_session_id.is_some() {
                        w += 7; // "  child"
                    }
                    w
                })
                .max()
                .unwrap_or(35)
                .max(30) as u16;

            let mut rows: Vec<Row> = Vec::new();
            let mut current_workspace = String::new();

            for index in matches.iter() {
                let session = &panel.sessions[*index];

                if panel.view_mode == SessionViewMode::AllSessions
                    && session.workspace_root != current_workspace
                {
                    if !current_workspace.is_empty() {
                        rows.push(Row::new(vec![Cell::from(""), Cell::from("")]));
                    }
                    current_workspace = session.workspace_root.clone();
                    rows.push(Row::new(vec![
                        Cell::from(Line::from(vec![Span::styled(
                            format!("[ {} ]", session.workspace_root),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        )])),
                        Cell::from(""),
                    ]));
                }

                let is_current = session.session_id == self.conversation.session_id;
                let updated_at = session.updated_at.format("%Y-%m-%d %H:%M").to_string();
                let is_selected = panel.is_selected(*index);

                let checkbox = if is_multi_select {
                    if is_selected { "[✓] " } else { "[ ] " }
                } else {
                    ""
                };

                // Left cell: checkbox + title
                let left_line = Line::from(vec![
                    Span::raw(checkbox),
                    Span::styled(
                        shorten(
                            &session.title,
                            sections[2].width.saturating_sub(max_right_width + 4) as usize,
                        ),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]);

                // Right cell: provider/model + time + optional badges (right-aligned)
                let provider_model = format!(
                    "{} / {}",
                    shorten(&session.provider_display_name, 12),
                    shorten(&session.model_display_name, 14)
                );
                let mut right_spans: Vec<Span> = vec![
                    Span::styled(
                        provider_model.clone(),
                        Style::default().fg(palette.accent_soft),
                    ),
                    Span::raw("  "),
                    Span::styled(updated_at.clone(), Style::default().fg(palette.muted)),
                ];

                if is_current {
                    right_spans.push(Span::raw("  "));
                    right_spans.push(Span::styled(
                        "current",
                        Style::default().fg(palette.success),
                    ));
                }
                if session.parent_session_id.is_some() {
                    right_spans.push(Span::raw("  "));
                    right_spans.push(Span::styled(
                        "child",
                        Style::default().fg(palette.accent_soft),
                    ));
                }

                let right_line = Line::from(right_spans).alignment(Alignment::Right);

                rows.push(Row::new(vec![
                    Cell::from(left_line),
                    Cell::from(right_line),
                ]));
            }

            let mut state = TableState::default();
            state.select(Some(
                panel.selected_index.min(matches.len().saturating_sub(1)),
            ));

            let table = Table::new(
                rows,
                [Constraint::Fill(1), Constraint::Min(max_right_width)],
            )
            .style(Style::default().bg(palette.panel).fg(palette.text))
            .row_highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

            frame.render_stateful_widget(table, sections[2], &mut state);
        }

        let help_text = if panel.operation_mode
            == crate::tui::session_panel::OperationMode::MultiSelect
        {
            "Enter/D: switch/delete · Space: select · Ctrl+A: exit multi-select · Tab: switch view · C: cleanup · E: export"
        } else {
            "Enter: switch · D: delete · C: cleanup · Ctrl+A: multi-select · Tab: switch view · W: all sessions · E: export"
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[3],
        );
    }

    pub(crate) fn render_message_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &MessagePanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(area.width.min(112), area.height.min(36), area);
        frame.render_widget(Clear, overlay);

        let title = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(" User messages ")
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
                "Type to filter current session user messages. Enter jumps to the selected message.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[0],
        );

        self.render_input_block(
            frame,
            sections[1],
            "Search user messages",
            self.composer.placeholder(),
            false,
        );

        let query = self.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No user messages match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                sections[2],
            );
        } else {
            let mut items: Vec<ListItem> = Vec::new();
            for index in matches.iter() {
                let message = &panel.messages[*index];
                let timestamp = message.created_at.format("%Y-%m-%d %H:%M").to_string();
                let spans = vec![
                    Span::styled(
                        timestamp.to_string(),
                        Style::default().fg(palette.accent_soft),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        shorten(&message.content, 64),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("({})", message.message_id.as_simple()),
                        Style::default().fg(palette.muted),
                    ),
                ];
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

        frame.render_widget(
            Paragraph::new("Enter: jump · Esc: close · Ctrl+P/N: nav")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[3],
        );
    }

    pub(crate) fn render_model_panel(
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
            Constraint::Length(1), // tab bar
            Constraint::Length(2), // instruction
            Constraint::Length(3), // search box
            Constraint::Min(8),    // model list
            Constraint::Length(1), // footer help
        ])
        .split(inner);

        // --- Tab bar ---
        let tab_spans: Vec<Span> = panel
            .tabs
            .iter()
            .enumerate()
            .flat_map(|(idx, tab)| {
                let is_active = idx == panel.selected_tab_index;
                let tab_style = if is_active {
                    Style::default()
                        .fg(palette.selection_fg)
                        .bg(palette.selection_bg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.muted)
                };
                let label = format!(" {} ", tab.display_name);
                let mut spans = vec![Span::styled(label, tab_style)];
                // Separator between tabs
                if idx + 1 < panel.tabs.len() {
                    spans.push(Span::styled(" │ ", Style::default().fg(palette.border)));
                }
                spans
            })
            .collect();

        frame.render_widget(
            Paragraph::new(Line::from(tab_spans))
                .style(Style::default().bg(palette.panel))
                .alignment(Alignment::Left),
            sections[0],
        );

        // --- Instruction ---
        let instruction = if panel.is_general_tab() {
            "Select a model for the main session. Enter to switch, Esc to close."
        } else {
            "Select a model for this agent. Enter to save, Esc to close."
        };
        frame.render_widget(
            Paragraph::new(instruction)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[1],
        );

        // --- Search box ---
        self.render_input_block_with_composer(
            frame,
            sections[2],
            "Search models",
            &panel.query,
            panel.query.placeholder(),
            false,
            false,
        );

        // --- Model list ---
        let items = self.model_panel_items(panel);

        // Determine the "active" model index (the model currently in use / saved)
        let active_index = panel.current_tab().and_then(|tab| {
            let label = &tab.current_label;
            if label == "<inherit>" || label.is_empty() {
                // For inherit, use the main session's active model
                items.iter().position(|item| {
                    matches!(item, ModelPanelItem::Model { summary }
                        if summary.provider_id == self.active_model.provider_id
                        && summary.model_id == self.active_model.model_id)
                })
            } else if let Some(slash_pos) = label.find('/') {
                let p = &label[..slash_pos];
                let m = &label[slash_pos + 1..];
                items.iter().position(|item| {
                    matches!(item, ModelPanelItem::Model { summary }
                        if summary.provider_id == p && summary.model_id == m)
                })
            } else {
                None
            }
        });

        let mut rows = Vec::new();
        for (index, item) in items.iter().enumerate() {
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
                    // Show checkmark for active model, space otherwise
                    let active_marker = if active_index == Some(index) {
                        Span::styled("✓ ", Style::default().fg(palette.accent))
                    } else {
                        Span::raw("  ")
                    };
                    rows.push(ListItem::new(Line::from(vec![
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
                sections[3],
            );
        } else {
            let sel = panel
                .current_tab()
                .map(|t| t.selected_index)
                .unwrap_or(0)
                .min(items.len().saturating_sub(1));
            let mut state = ListState::default();
            state.select(Some(sel));

            let list = List::new(rows)
                .style(Style::default().bg(palette.panel).fg(palette.text))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD),
                );

            frame.render_stateful_widget(list, sections[3], &mut state);
        }

        // --- Footer ---
        let footer = "Enter apply · Ctrl+E edit provider · Tab switch tab · Esc close";
        frame.render_widget(
            Paragraph::new(footer)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel).fg(palette.muted)),
            sections[4],
        );
    }

    pub(crate) fn render_mcp_panel(
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

    pub(crate) fn render_memory_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &MemoryPanelState,
    ) {
        let palette = self.palette();
        let filtered = panel.filtered_indices();

        let overlay = centered_rect(area.width.min(96), area.height.min(36), area);
        frame.render_widget(Clear, overlay);

        let title_block = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(" Memories ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(title_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match panel.mode {
            MemoryPanelMode::Browse => {
                let sections = Layout::vertical([
                    Constraint::Length(1), // filter indicator
                    Constraint::Min(6),    // list
                    Constraint::Length(1), // count
                    Constraint::Length(1), // help
                ])
                .split(inner);

                // Filter indicator
                let filter_text = match panel.filter_type {
                    None => "All types".to_string(),
                    Some(t) => format!("Type: {}", t.as_str()),
                };
                frame.render_widget(
                    Paragraph::new(filter_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[0],
                );

                // Memory list
                if filtered.is_empty() {
                    frame.render_widget(
                        Paragraph::new("No memories yet. Press 'a' to add one.")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.muted)),
                        sections[1],
                    );
                } else {
                    let items: Vec<ListItem> = filtered
                        .iter()
                        .enumerate()
                        .map(|(list_idx, &mem_idx)| {
                            let entry = &panel.memories[mem_idx];
                            let is_selected = list_idx == panel.selected_index;
                            let prefix = if is_selected { "▸ " } else { "  " };
                            let type_label = entry.memory_type.short_label();
                            let preview: String = entry.content.chars().take(80).collect();
                            let suffix = if entry.content.len() > 80 { "…" } else { "" };
                            let text = format!(
                                "{}[{}] {} – {}{}",
                                prefix, type_label, entry.title, preview, suffix
                            );
                            let style = if is_selected {
                                Style::default()
                                    .fg(palette.accent)
                                    .bg(palette.selection_bg)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().bg(palette.panel).fg(palette.text)
                            };
                            ListItem::new(text).style(style)
                        })
                        .collect();

                    let list = List::new(items);
                    frame.render_widget(list, sections[1]);
                }

                // Count
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} / {} memories",
                        filtered.len(),
                        panel.memories.len()
                    ))
                    .alignment(Alignment::Right)
                    .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[2],
                );

                // Help
                frame.render_widget(
                    Paragraph::new(
                        "↑↓ navigate · a add · e edit · d delete · r filter type · Esc close",
                    )
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel).fg(palette.accent_soft)),
                    sections[3],
                );
            }

            MemoryPanelMode::Add | MemoryPanelMode::Edit => {
                let label = match panel.mode {
                    MemoryPanelMode::Add => "Add Memory",
                    MemoryPanelMode::Edit => "Edit Memory",
                    _ => unreachable!(),
                };

                let sections = Layout::vertical([
                    Constraint::Length(1), // label
                    Constraint::Length(1), // type
                    Constraint::Length(3), // title
                    Constraint::Min(8),    // content
                    Constraint::Length(1), // tags
                    Constraint::Length(1), // hints
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(label)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.accent)),
                    sections[0],
                );

                // Type
                frame.render_widget(
                    Paragraph::new(format!("Type: {}", panel.edit_type.as_str()))
                        .style(Style::default().bg(palette.panel).fg(palette.text)),
                    sections[1],
                );

                // Title edit area
                frame.render_widget(
                    Paragraph::new(format!("Title: {}", panel.edit_title))
                        .style(Style::default().bg(palette.panel).fg(palette.text))
                        .wrap(Wrap { trim: false }),
                    sections[2],
                );

                // Content edit area
                frame.render_widget(
                    Paragraph::new(if panel.edit_content.is_empty() {
                        "Content: (type in input box below)"
                    } else {
                        &panel.edit_content
                    })
                    .style(Style::default().bg(palette.panel).fg(palette.text))
                    .wrap(Wrap { trim: false }),
                    sections[3],
                );

                // Tags
                frame.render_widget(
                    Paragraph::new(format!("Tags: {}", panel.edit_tags))
                        .style(Style::default().bg(palette.panel).fg(palette.text)),
                    sections[4],
                );

                // Hints
                frame.render_widget(
                    Paragraph::new("Tab: cycle type · Enter: save · Esc: cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[5],
                );
            }

            MemoryPanelMode::DeleteConfirm => {
                let sections = Layout::vertical([
                    Constraint::Length(2),
                    Constraint::Length(2),
                    Constraint::Length(1),
                ])
                .split(inner);

                if let Some(entry) = panel.selected_entry() {
                    frame.render_widget(
                        Paragraph::new(format!("Delete memory: {}?", entry.title))
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel).fg(palette.warning)),
                        sections[0],
                    );
                }

                frame.render_widget(
                    Paragraph::new("Press Y to confirm, N or Esc to cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel).fg(palette.muted)),
                    sections[1],
                );
            }
        }
    }
    /// Render the skills panel with a two-pane layout:
    /// - Left: searchable list of skills
    /// - Right: markdown preview of selected skill
    pub(crate) fn render_skills_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SkillsPanelState,
    ) {
        use crate::markdown_render::render_markdown_text_with_width_and_cwd;

        let palette = self.palette();

        // Main overlay - 85% width, 80% height
        let overlay = centered_rect(85, 80, area);
        frame.render_widget(Clear, overlay);

        // Main block with title
        let title = if panel.is_empty() {
            " Skills ".to_string()
        } else {
            format!(
                " Skills · {}/{} ",
                panel.selected_index + 1,
                panel.filtered_count()
            )
        };

        let panel_block = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        // Check if empty
        if panel.is_empty() {
            let empty_text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No skills discovered",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Create .opencode/skills/SKILL.md to add skills",
                    Style::default().fg(palette.muted),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "  Press Esc or q to close",
                    Style::default().fg(palette.muted),
                )),
            ];
            frame.render_widget(
                Paragraph::new(empty_text).style(Style::default().bg(palette.panel)),
                inner,
            );
            return;
        }

        // Split into left (list) and right (preview) panes
        // Left: 35%, Right: 65%
        let panes = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(inner);

        let list_area = panes[0];
        let preview_area = panes[1];

        // --- Left Pane: Skill List ---
        // Header with search status
        let search_status = if panel.query_active {
            format!("Search: {}_", panel.query)
        } else if !panel.query.is_empty() {
            format!("Filter: {} (press / to edit)", panel.query)
        } else {
            "Press / to search".to_string()
        };

        let header_lines = vec![
            Line::from(vec![Span::styled(
                "  Name",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(Span::styled(
                format!("  {}", search_status),
                Style::default().fg(palette.muted),
            )),
        ];

        let header_height = header_lines.len() as u16;
        frame.render_widget(
            Paragraph::new(header_lines).style(Style::default().bg(palette.panel)),
            Rect::new(list_area.x, list_area.y, list_area.width, header_height),
        );

        // Divider
        let divider_y = list_area.y + header_height;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(list_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel)),
            Rect::new(list_area.x, divider_y, list_area.width, 1),
        );

        // Skill list with scrollbar
        let list_start_y = divider_y + 1;
        let list_content_height = list_area.height.saturating_sub(header_height + 1);
        let list_content_area = Rect::new(
            list_area.x,
            list_start_y,
            list_area.width,
            list_content_height,
        );

        // Split list area into content + scrollbar
        let (list_content_area, list_scrollbar_area) = if list_content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(list_content_area);
            (chunks[0], Some(chunks[2]))
        } else if list_content_area.width > 1 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(list_content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (list_content_area, None)
        };

        let mut list_lines: Vec<Line<'_>> = Vec::new();
        let visible_start = panel.list_scroll;
        let visible_end =
            (panel.list_scroll + list_content_height as usize).min(panel.filtered_indices.len());

        for (i, skill_idx) in panel
            .filtered_indices
            .iter()
            .enumerate()
            .skip(visible_start)
            .take(visible_end - visible_start)
        {
            let skill = &panel.all_skills[*skill_idx];
            let is_selected = i == panel.selected_index;

            let icon = "📁 ";
            let name_style = if is_selected {
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };

            let line = Line::from(vec![
                Span::styled(icon, name_style),
                Span::styled(&skill.name, name_style),
            ]);
            list_lines.push(line);
        }

        // Fill remaining space with background
        while list_lines.len() < list_content_height as usize {
            list_lines.push(Line::from(""));
        }

        frame.render_widget(
            Paragraph::new(list_lines).style(Style::default().bg(palette.panel)),
            list_content_area,
        );

        // Render list scrollbar
        if let Some(sb_area) = list_scrollbar_area {
            render_scrollbar(
                frame,
                sb_area,
                panel.list_scroll,
                panel.filtered_indices.len(),
                palette,
            );
        }

        // --- Right Pane: Preview ---
        // Header
        let preview_header = vec![Line::from(vec![Span::styled(
            "  Preview",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )])];
        frame.render_widget(
            Paragraph::new(preview_header).style(Style::default().bg(palette.panel)),
            Rect::new(preview_area.x, preview_area.y, preview_area.width, 1),
        );

        // Divider
        let preview_divider_y = preview_area.y + 1;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(preview_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel)),
            Rect::new(preview_area.x, preview_divider_y, preview_area.width, 1),
        );

        // Preview content with scrollbar
        let preview_content_y = preview_divider_y + 1;
        let preview_content_height = preview_area.height.saturating_sub(2);
        let preview_content_area = Rect::new(
            preview_area.x,
            preview_content_y,
            preview_area.width,
            preview_content_height,
        );

        // Split preview area into content + scrollbar
        let (preview_content_area, preview_scrollbar_area) = if preview_content_area.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(preview_content_area);
            (chunks[0], Some(chunks[2]))
        } else if preview_content_area.width > 1 {
            let chunks = Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                .split(preview_content_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (preview_content_area, None)
        };

        if let Some(skill) = panel.selected_skill() {
            // Get rendered skill content from catalog
            let content = self
                .tools
                .skills()
                .render_skill(&skill.name)
                .unwrap_or_default();

            // Render markdown with syntax highlighting
            let content_width = preview_content_area.width.saturating_sub(2) as usize;
            let rendered =
                render_markdown_text_with_width_and_cwd(&content, Some(content_width), None);
            let total_preview_lines = rendered.lines.len();

            // Apply scroll offset
            let scroll = panel.preview_scroll;
            let visible_lines: Vec<Line<'_>> = rendered
                .into_iter()
                .skip(scroll)
                .take(preview_content_height as usize)
                .collect();

            frame.render_widget(
                Paragraph::new(visible_lines).style(Style::default().bg(palette.panel)),
                preview_content_area,
            );

            // Render preview scrollbar
            if let Some(sb_area) = preview_scrollbar_area {
                render_scrollbar(
                    frame,
                    sb_area,
                    panel.preview_scroll,
                    total_preview_lines,
                    palette,
                );
            }
        } else {
            // No skill selected, just render scrollbar track if area exists
            if let Some(sb_area) = preview_scrollbar_area {
                render_scrollbar(frame, sb_area, 0, 0, palette);
            }
        }

        // --- Footer hints ---
        let footer_y = inner.y + inner.height - 1;
        let hints = if panel.query_active {
            "Enter: confirm search  •  Esc: cancel"
        } else {
            "↑/↓: navigate  •  ←/→: scroll preview  •  /: search  •  c: copy  •  Esc: close"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {}", hints),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel)),
            Rect::new(inner.x, footer_y, inner.width, 1),
        );
    }
}
