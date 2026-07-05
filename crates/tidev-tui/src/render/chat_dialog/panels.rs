use crate::App;
use crate::render::render::{centered_rect, render_scrollbar, shorten};
use crate::{
    message_panel::MessagePanelState,
    model_panel::{ModelPanelItem, ModelPanelState, thinking_options_for_model},
    session_panel::{SessionPanelState, SessionViewMode},
    settings_panel::SettingsPanelState,
    theme_panel::{DisplayItem, ThemePanelState},
    ui::agents_panel::AgentsPanelState,
    ui::search_panel::{BUILTIN_PROVIDERS, SearchPanelState},
    ui::skills_panel::SkillsPanelState,
};
use chrono::Local;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
    },
};
use tidev_types::prompts::SessionMode;

impl App {
    pub(crate) fn render_theme_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ThemePanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(36, 22, area);
        self.ui.theme_panel_overlay.set(Some(overlay));

        let block = Block::default().style(Style::default().bg(palette.panel_alt));

        frame.render_widget(Clear, overlay);
        frame.render_widget(block, overlay);
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        // --- Title ---
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Theme ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );

        // --- Search / filter bar ---
        let search_text = if panel.query.is_empty() {
            "  Type to search...".to_string()
        } else {
            format!("  {}", panel.query)
        };
        let search_style = Style::default().fg(palette.muted);
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(search_text, search_style)]))
                .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
        );

        // Divider
        let divider_y = inner.y + 2;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(inner.width as usize),
                Style::default().fg(palette.border),
            )))
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, divider_y, inner.width, 1),
        );

        // --- List area ---
        let list_y = inner.y + 3;
        let list_height = inner.height.saturating_sub(3);
        if list_height == 0 {
            return;
        }
        let list_area = Rect::new(inner.x, list_y, inner.width, list_height);

        // Split into content + scrollbar
        let (content_area, scrollbar_area) = if list_area.width > 2 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(list_area);
            (chunks[0], Some(chunks[1]))
        } else {
            (list_area, None)
        };

        // Compute scroll offset so selected_index is visible
        let display_len = panel.display_items.len();
        let scroll = if panel.selected_index < list_height as usize {
            0
        } else {
            // Try to keep selection centered-ish
            let target = panel
                .selected_index
                .saturating_sub(list_height as usize / 2);
            target.min(display_len.saturating_sub(list_height as usize))
        };

        // Render visible items
        for i in 0..list_height {
            let idx = scroll + i as usize;
            if idx >= display_len {
                break;
            }
            let item = &panel.display_items[idx];
            let y = content_area.y + i;

            match item {
                DisplayItem::Header(label) => {
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!(" {} ", label),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        )))
                        .style(Style::default().bg(palette.panel_alt)),
                        Rect::new(content_area.x, y, content_area.width, 1),
                    );
                }
                DisplayItem::Theme(t) => {
                    let is_selected = idx == panel.selected_index;
                    let (text_style, bg_block) = if is_selected {
                        (
                            Style::default()
                                .fg(palette.selection_fg)
                                .bg(palette.selection_bg)
                                .add_modifier(Modifier::BOLD),
                            Style::default().bg(palette.selection_bg),
                        )
                    } else {
                        (
                            Style::default().fg(palette.text),
                            Style::default().bg(palette.panel_alt),
                        )
                    };
                    frame.render_widget(
                        Paragraph::new(Line::from(Span::styled(
                            format!("  {}", t.as_str()),
                            text_style,
                        )))
                        .style(bg_block),
                        Rect::new(content_area.x, y, content_area.width, 1),
                    );
                }
            }
        }

        // Scrollbar
        if let Some(sb_area) = scrollbar_area {
            render_scrollbar(frame, sb_area, scroll, display_len, palette, false);
        }
    }

    pub(crate) fn render_agents_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &AgentsPanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(70, 24, area);
        self.ui.agents_panel_overlay.set(Some(overlay));

        frame.render_widget(Clear, overlay);
        let panel_block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let sections = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // header
            Constraint::Length(1), // divider
            Constraint::Min(0),    // content
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " Agents ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

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
            Paragraph::new(header).style(Style::default().bg(palette.panel_alt)),
            sections[1],
        );

        let divider = Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            Style::default().fg(palette.muted),
        ));
        frame.render_widget(
            Paragraph::new(divider).style(Style::default().bg(palette.panel_alt)),
            sections[2],
        );

        // Content area with scrollbar
        let content_area = sections[3];
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
            Paragraph::new(lines).style(Style::default().bg(palette.panel_alt)),
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
                false,
            );
        }
    }

    pub(crate) fn render_settings_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SettingsPanelState,
    ) {
        use crate::ui::settings_panel::SettingType;
        let current_palette = self.palette();
        // 10 items × ~2 lines each = 22 rows
        let overlay = centered_rect(64, 22, area);
        self.ui.settings_panel_overlay.set(Some(overlay));

        let items: Vec<ListItem> = panel
            .items
            .iter()
            .map(|item| {
                let fg = if item.disabled {
                    current_palette.muted
                } else {
                    current_palette.text
                };
                let status: String = match &item.setting_type {
                    SettingType::Toggle(true) => "[x]".to_string(),
                    SettingType::Toggle(false) => "[ ]".to_string(),
                    SettingType::Number { .. } => "[~]".to_string(),
                    SettingType::Cycle { options, selected } => {
                        let current = options.get(*selected).map(|s| s.as_str()).unwrap_or("?");
                        format!("[{current}]")
                    }
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            format!(" {} ", status),
                            Style::default()
                                .fg(match &item.setting_type {
                                    SettingType::Toggle(true) => {
                                        if item.disabled {
                                            current_palette.muted
                                        } else {
                                            current_palette.accent
                                        }
                                    }
                                    _ => current_palette.muted,
                                })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            &item.name,
                            Style::default().fg(fg).add_modifier(Modifier::BOLD),
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

        let panel_block = Block::default().style(Style::default().bg(current_palette.panel_alt));

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

    pub(crate) fn render_session_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SessionPanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(area.width.min(112), area.height.min(36), area);
        self.ui.session_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);

        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let view_mode_text = match panel.view_mode {
            SessionViewMode::CurrentWorkspace => "Current Workspace",
            SessionViewMode::AllSessions => "All Sessions",
        };
        let title_text = format!(" Sessions: {} ", view_mode_text);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &title_text,
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new("Type to filter by title, model, provider, or session id.")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        self.render_input_block(
            frame,
            sections[2],
            "Search sessions",
            self.ui.composer.placeholder(),
            false,
        );

        let query = self.ui.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        let is_multi_select =
            panel.operation_mode == crate::session_panel::OperationMode::MultiSelect;

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No sessions match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                sections[3],
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
                    let time = session
                        .updated_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string();
                    let mut w = pm.chars().count() + 2 + time.chars().count();
                    if session.session_id == self.ui.chat_context.session_id {
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

                let is_current = session.session_id == self.ui.chat_context.session_id;
                let updated_at = session
                    .updated_at
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
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
                            sections[3].width.saturating_sub(max_right_width + 4) as usize,
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
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .row_highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

            frame.render_stateful_widget(table, sections[3], &mut state);
        }

        let help_text = if panel.operation_mode == crate::session_panel::OperationMode::MultiSelect
        {
            "Enter/D: switch/delete · Space: select · Ctrl+A: exit multi-select · Tab: switch view · C: cleanup · E: export"
        } else {
            "Enter: switch · D: delete · C: cleanup · Ctrl+A: multi-select · Tab: switch view · W: all sessions · E: export"
        };

        frame.render_widget(
            Paragraph::new(help_text)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[4],
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
        self.ui.message_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);

        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let sections = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(inner);

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " User messages ",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]))
            .style(Style::default().bg(palette.panel_alt)),
            sections[0],
        );

        frame.render_widget(
            Paragraph::new(
                "Type to filter current session user messages. Enter jumps to the selected message.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[1],
        );

        self.render_input_block(
            frame,
            sections[2],
            "Search user messages",
            self.ui.composer.placeholder(),
            false,
        );

        let query = self.ui.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No user messages match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                sections[3],
            );
        } else {
            let mut rows: Vec<Row> = Vec::new();
            for index in matches.iter() {
                let message = &panel.messages[*index];

                let ts_str = format!(
                    "{:<16}",
                    message
                        .created_at
                        .with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                );
                let ts_cell = Cell::from(Line::from(vec![Span::styled(
                    ts_str,
                    Style::default().fg(palette.accent_soft),
                )]));

                let mode_str = match message.mode {
                    Some(SessionMode::Build) => " Build",
                    Some(SessionMode::Plan) => "  Plan",
                    None => "      ",
                };
                let mode_color = message.mode.map_or(palette.muted, |m| match m {
                    SessionMode::Build => palette.mode_build,
                    SessionMode::Plan => palette.mode_plan,
                });
                let mode_cell = Cell::from(Line::from(vec![Span::styled(
                    mode_str,
                    Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
                )]));

                let content_cell = Cell::from(Line::from(vec![Span::styled(
                    shorten(&message.content, 80),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                )]));

                rows.push(Row::new(vec![ts_cell, mode_cell, content_cell]));
            }

            let mut state = TableState::default();
            state.select(Some(
                panel.selected_index.min(matches.len().saturating_sub(1)),
            ));

            let table = Table::new(
                rows,
                [
                    Constraint::Length(17),
                    Constraint::Length(6),
                    Constraint::Fill(1),
                ],
            )
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .row_highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );

            frame.render_stateful_widget(table, sections[3], &mut state);
        }

        frame.render_widget(
            Paragraph::new("Enter: jump · Esc: close · Ctrl+P/N: nav")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[4],
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
        self.ui.model_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);
        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        self.render_model_panel_standard(frame, inner, panel);
    }

    /// Standard single-column layout for non-Memory tabs.
    fn render_model_panel_standard(
        &self,
        frame: &mut Frame<'_>,
        inner: Rect,
        panel: &ModelPanelState,
    ) {
        let palette = self.palette();

        let sections = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // tab bar
            Constraint::Length(2), // instruction
            Constraint::Length(3), // search box
            Constraint::Min(8),    // model list
            Constraint::Length(1), // footer help
        ])
        .split(inner);

        // --- Title ---
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

        // --- Tab bar ---
        self.render_model_panel_tab_bar(frame, sections[1], panel);

        // --- Instruction ---
        let instruction = "Select a model for this agent. Enter to save, Esc to close.";
        frame.render_widget(
            Paragraph::new(instruction)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );

        // --- Search box ---
        self.render_input_block_with_composer(
            frame,
            sections[3],
            "Search models",
            &panel.query,
            panel.query.placeholder(),
            false,
            false,
            false,
        );

        // --- Model list ---
        self.render_model_panel_model_list(frame, sections[4], panel, true);

        // --- Footer ---
        self.render_model_panel_footer(frame, sections[5], panel);
    }

    fn render_model_panel_tab_bar(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ModelPanelState,
    ) {
        let palette = self.palette();
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
                if idx + 1 < panel.tabs.len() {
                    spans.push(Span::styled(" │ ", Style::default().fg(palette.border)));
                }
                spans
            })
            .collect();

        frame.render_widget(
            Paragraph::new(Line::from(tab_spans))
                .style(Style::default().bg(palette.panel_alt))
                .alignment(Alignment::Left),
            area,
        );
    }

    /// Render the model list (shared between tabs).
    /// `highlight` controls whether to show a selection highlight.
    fn render_model_panel_model_list(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ModelPanelState,
        highlight: bool,
    ) {
        let palette = self.palette();
        let items = self.model_panel_items(panel);

        // Determine the "active" model index (the model currently in use / saved)
        let active_index = if panel.is_general_tab() {
            items.iter().position(|item| {
                matches!(item, ModelPanelItem::Model { summary, .. }
                    if summary.provider_id == self.runtime.active_model().provider_id
                    && summary.model_id == self.runtime.active_model().model_id)
            })
        } else {
            // Agent tab: find the model currently configured for this agent type
            panel.current_tab().and_then(|tab| {
                let current = self.runtime.config()
                    .agent
                    .models
                    .get(&tab.agent_type_str)
                    .cloned();
                current.and_then(|label| {
                    items.iter().position(|item| match item {
                        ModelPanelItem::Model { summary } => summary.label() == *label,
                        _ => false,
                    })
                })
            })
        };

        let mut rows: Vec<ListItem> = Vec::new();

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
                ModelPanelItem::Model { summary, .. } => {
                    let active_marker = if active_index == Some(index) {
                        Span::styled("✓ ", Style::default().fg(palette.accent))
                    } else {
                        Span::raw("  ")
                    };

                    let is_selected = panel
                        .current_tab()
                        .is_some_and(|t| t.selected_index == index);
                    let is_active = summary.provider_id == self.runtime.active_model().provider_id
                        && summary.model_id == self.runtime.active_model().model_id
                        && panel.is_general_tab();
                    let thinking_level_tag: Option<String> = if is_selected
                        && panel
                            .current_tab()
                            .is_some_and(|t| t.thinking_level_expanded)
                    {
                        let tl_options = thinking_options_for_model(&items, index);
                        if !tl_options.is_empty() {
                            let tl_idx = panel
                                .current_tab()
                                .map(|t| t.thinking_level_index)
                                .unwrap_or(0);
                            let opt = tl_options[tl_idx % tl_options.len()];
                            let name = opt.rsplit_once(':').map(|(_, v)| v).unwrap_or(opt);
                            Some(name.to_string())
                        } else {
                            None
                        }
                    } else if is_active && self.ui.thinking_level.is_supported() {
                        Some(self.ui.thinking_level.display_name().to_string())
                    } else if !panel.is_general_tab() {
                        if let Some(tab) = panel.current_tab() {
                            let model_label = summary.label();
                            let config = self.runtime.config();
                            if let Some(tl_str) =
                                config.agent.thinking_levels.get(&tab.agent_type_str)
                                && config
                                    .agent
                                    .models
                                    .get(&tab.agent_type_str)
                                    .is_some_and(|m| *m == model_label)
                                && tidev_config::reasoning::ThinkingMatcher::match_for_model(
                                    &summary.model_id,
                                )
                                .is_supported()
                            {
                                let tl_level =
                                    tl_str.rsplit_once(':').map(|(_, v)| v).unwrap_or(tl_str);
                                Some(tl_level.to_string())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
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

                    if let Some(tl) = &thinking_level_tag {
                        spans.push(Span::raw("  "));
                        spans.push(Span::styled(
                            format!("[{}]", tl),
                            Style::default().fg(palette.accent_soft),
                        ));
                    }

                    rows.push(ListItem::new(Line::from(spans)));

                    // If thinking level is expanded for this model, render sub-options
                    if is_selected
                        && panel
                            .current_tab()
                            .is_some_and(|t| t.thinking_level_expanded)
                    {
                        let tl_options = thinking_options_for_model(&items, index);
                        if !tl_options.is_empty() {
                            let tl_idx = panel
                                .current_tab()
                                .map(|t| t.thinking_level_index)
                                .unwrap_or(0);
                            for (oi, opt) in tl_options.iter().enumerate() {
                                let is_tl_selected = oi == tl_idx % tl_options.len();
                                let level_name =
                                    opt.rsplit_once(':').map(|(_, v)| v).unwrap_or(opt);
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

        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No connected models match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                area,
            );
        } else {
            let sel = panel
                .current_tab()
                .map(|t| t.selected_index)
                .unwrap_or(0)
                .min(items.len().saturating_sub(1));
            let mut state = ListState::default();
            state.select(Some(sel.min(rows.len().saturating_sub(1))));

            let list = List::new(rows)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(if highlight {
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    // Transparent highlight when sidebar has focus
                    Style::default().bg(palette.panel_alt).fg(palette.text)
                });

            frame.render_stateful_widget(list, area, &mut state);
        }
    }

    /// Render the footer help text.
    fn render_model_panel_footer(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ModelPanelState,
    ) {
        let palette = self.palette();
        let is_expanded = panel
            .current_tab()
            .is_some_and(|t| t.thinking_level_expanded);
        let footer = if is_expanded {
            "Enter confirm thinking · ↑ ↓ select level · Esc collapse"
        } else {
            "Enter apply / expand thinking · Ctrl+E edit provider · Tab switch tab · Esc close"
        };
        frame.render_widget(
            Paragraph::new(footer)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            area,
        );
    }

    pub(crate) fn render_search_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SearchPanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(area.width.min(60), area.height.min(20), area);
        frame.render_widget(Clear, overlay);
        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

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

        // If editing API key, show a compact input view
        if panel.editing_api_key.is_some() {
            let sections = Layout::vertical([
                Constraint::Length(3), // prompt input
                Constraint::Length(1), // footer help
            ])
            .split(body);

            frame.render_widget(
                Paragraph::new(Line::from(vec![Span::styled(
                    panel.input_buffer.placeholder(),
                    Style::default().fg(palette.muted),
                )]))
                .style(Style::default().bg(palette.panel_alt)),
                sections[0],
            );

            // Render the actual input
            let input_width = sections[0].width.saturating_sub(2);
            let text = panel.input_buffer.text();
            let display = if text.len() > input_width as usize {
                &text[text.len().saturating_sub(input_width as usize)..]
            } else {
                text
            };
            frame.render_widget(
                Paragraph::new(display.to_string())
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text)),
                sections[0],
            );

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
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt)),
                sections[1],
            );
            return;
        }

        let sections = Layout::vertical([
            Constraint::Length(1), // instruction
            Constraint::Min(4),    // provider list
            Constraint::Length(1), // footer help
        ])
        .split(body);

        // --- Instruction ---
        frame.render_widget(
            Paragraph::new("Select a web search provider. ↑↓ navigate, Enter select.")
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[0],
        );

        // --- Provider list ---
        let mut rows: Vec<ListItem> = Vec::new();
        for (i, info) in BUILTIN_PROVIDERS.iter().enumerate() {
            let status_text = panel.provider_status(i, &self.runtime.auth());

            let is_selected = i == panel.selected_index;
            let row_style = if is_selected {
                Style::default()
                    .fg(palette.selection_fg)
                    .bg(palette.selection_bg)
            } else {
                Style::default().bg(palette.panel_alt)
            };

            // Active checkmark (like model panel)
            let active_marker = if info.id == panel.active_provider {
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
            ratatui::widgets::List::new(rows).style(Style::default().bg(palette.panel_alt)),
            sections[1],
        );

        // --- Footer help ---
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
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            sections[2],
        );
    }

    /// Render the skills panel with a two-pane layout:    /// - Left: searchable list of skills
    /// - Right: markdown preview of selected skill
    pub(crate) fn render_skills_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &SkillsPanelState,
    ) {
        use crate::markdown::render_markdown_text_with_width_and_cwd;

        let palette = self.palette();

        // Main overlay - 85% width, 80% height
        let overlay = centered_rect(85, 80, area);
        self.ui.skills_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);
        let panel_block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let skills_title = if panel.is_empty() {
            " Skills ".to_string()
        } else {
            format!(
                " Skills · {}/{} ",
                panel.selected_index + 1,
                panel.filtered_count()
            )
        };

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &skills_title,
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
                Paragraph::new(empty_text).style(Style::default().bg(palette.panel_alt)),
                body,
            );
            return;
        }

        // Split into left (list) and right (preview) panes
        // Left: 35%, Right: 65%
        let panes = Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
            .split(body);

        let list_area = panes[0];
        let preview_area = panes[1];

        // --- Left Pane: Skill List ---
        // Header with search status
        let search_text = if panel.query_active {
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
                format!("  {}", search_text),
                Style::default().fg(palette.muted),
            )),
        ];

        let header_height = header_lines.len() as u16;
        frame.render_widget(
            Paragraph::new(header_lines).style(Style::default().bg(palette.panel_alt)),
            Rect::new(list_area.x, list_area.y, list_area.width, header_height),
        );

        // Divider
        let divider_y = list_area.y + header_height;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(list_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
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
            Paragraph::new(list_lines).style(Style::default().bg(palette.panel_alt)),
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
                false,
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
            Paragraph::new(preview_header).style(Style::default().bg(palette.panel_alt)),
            Rect::new(preview_area.x, preview_area.y, preview_area.width, 1),
        );

        // Divider
        let preview_divider_y = preview_area.y + 1;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(preview_area.width as usize),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel_alt)),
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
                .runtime
                .tool_registry()
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
                Paragraph::new(visible_lines).style(Style::default().bg(palette.panel_alt)),
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
                    false,
                );
            }
        } else {
            // No skill selected, just render scrollbar track if area exists
            if let Some(sb_area) = preview_scrollbar_area {
                render_scrollbar(frame, sb_area, 0, 0, palette, false);
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
            .style(Style::default().bg(palette.panel_alt)),
            Rect::new(inner.x, footer_y, inner.width, 1),
        );
    }
}
