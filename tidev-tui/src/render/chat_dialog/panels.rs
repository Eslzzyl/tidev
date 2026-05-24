use crate::App;
use crate::render::render::{centered_rect, render_scrollbar, shorten};
use crate::{

mcp_panel::McpPanelState,
    memory_panel::{EditField, MemoryPanelMode, MemoryPanelState, PanelFocus},
    message_panel::MessagePanelState,
    model_panel::{ModelPanelItem, ModelPanelState, thinking_options_for_model},
    session_panel::{SessionPanelState, SessionViewMode},
    settings_panel::SettingsPanelState,
    theme_panel::{DisplayItem, ThemePanelState},
    ui::agents_panel::AgentsPanelState,
    ui::search_panel::{BUILTIN_PROVIDERS, SearchPanelState},
    ui::skills_panel::SkillsPanelState,

};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Position, Style},
    text::{Line, Span},
    widgets::{
        Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState, Wrap,
    },
};

impl App {
    pub(crate) fn render_theme_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &ThemePanelState,
    ) {
        let palette = self.palette();
        let overlay = centered_rect(36, 22, area);
        self.theme_panel_overlay.set(Some(overlay));

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
        self.agents_panel_overlay.set(Some(overlay));

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
        self.settings_panel_overlay.set(Some(overlay));

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
        self.session_panel_overlay.set(Some(overlay));
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
            self.composer.placeholder(),
            false,
        );

        let query = self.composer.text().to_string();
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

        let help_text = if panel.operation_mode
            == crate::session_panel::OperationMode::MultiSelect
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
        self.message_panel_overlay.set(Some(overlay));
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
            self.composer.placeholder(),
            false,
        );

        let query = self.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        if matches.is_empty() {
            frame.render_widget(
                Paragraph::new("No user messages match this search.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                sections[3],
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
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD),
                );

            frame.render_stateful_widget(list, sections[3], &mut state);
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
        self.model_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);
        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        if panel.is_memory_tab() {
            self.render_model_panel_memory(frame, inner, panel);
        } else {
            self.render_model_panel_standard(frame, inner, panel);
        }
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

    /// Two-column layout for Memory tab (sidebar + model list).
    fn render_model_panel_memory(
        &self,
        frame: &mut Frame<'_>,
        inner: Rect,
        panel: &ModelPanelState,
    ) {
        let palette = self.palette();

        let sections = Layout::vertical([
            Constraint::Length(1), // title
            Constraint::Length(1), // tab bar
            Constraint::Min(8),    // main content area (sidebar | right)
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

        // Split main area horizontally: left sidebar, separator, right content
        let main = sections[2];
        let cols = Layout::horizontal([
            Constraint::Length(28), // sidebar
            Constraint::Length(1),  // separator
            Constraint::Min(30),    // model content (instruction + search + list + footer)
        ])
        .split(main);

        // --- Left sidebar: sub-tab list ---
        self.render_memory_sidebar(frame, cols[0], panel);

        // --- Separator ---
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.border)),
            cols[1],
        );

        // --- Right content area ---
        let right = cols[2];
        let right_sections = Layout::vertical([
            Constraint::Length(3), // search box
            Constraint::Min(8),    // model list
            Constraint::Length(1), // footer
        ])
        .split(right);

        // --- Search box ---
        self.render_input_block_with_composer(
            frame,
            right_sections[0],
            "Search models",
            &panel.query,
            panel.query.placeholder(),
            false,
            false,
            false,
        );

        // --- Model list ---
        let list_highlight = panel.memory_focus == crate::model_panel::MemoryFocus::List;
        self.render_model_panel_model_list(frame, right_sections[1], panel, list_highlight);

        // --- Footer ---
        let footer = "↑↓ navigate · ← → switch focus · Enter confirm · Esc close";
        frame.render_widget(
            Paragraph::new(footer)
                .alignment(Alignment::Center)
                .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
            right_sections[2],
        );
    }

    /// Render the tab bar at the top of the model panel.
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

    /// Render the left sidebar for the Memory tab showing the sub-tab list.
    fn render_memory_sidebar(&self, frame: &mut Frame<'_>, area: Rect, panel: &ModelPanelState) {
        let palette = self.palette();
        use crate::model_panel::MEMORY_ROLES;

        let sidebar_focused = panel.memory_focus == crate::model_panel::MemoryFocus::Sidebar;

        let rows: Vec<ListItem> = MEMORY_ROLES
            .iter()
            .enumerate()
            .map(|(idx, role)| {
                let is_selected = idx == panel.memory_sub_selection;
                let label = self.config.memory_model_display(role);

                let role_display = match *role {
                    "consolidation" => "Consolidation",
                    _ => role,
                };

                let indicator = if is_selected && sidebar_focused {
                    "▸"
                } else if is_selected {
                    "✓"
                } else {
                    " "
                };

                let role_style = if is_selected && sidebar_focused {
                    Style::default()
                        .bg(palette.selection_bg)
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default()
                        .fg(palette.selection_fg)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.text)
                };

                let display = format!("{} {} ", indicator, role_display);
                let spans = vec![
                    Span::styled(display, role_style),
                    Span::raw("\n"),
                    Span::styled(format!("   {}", label), Style::default().fg(palette.muted)),
                ];
                ListItem::new(Line::from(spans))
            })
            .collect();

        if rows.is_empty() {
            frame.render_widget(
                Paragraph::new("No memory roles")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                area,
            );
            return;
        }

        // Title
        let header = Paragraph::new(" Roles ")
            .style(Style::default().bg(palette.panel_alt).fg(palette.accent));
        let sidebar_layout =
            Layout::vertical([Constraint::Length(1), Constraint::Min(4)]).split(area);
        frame.render_widget(header, sidebar_layout[0]);

        let mut state = ListState::default();
        state.select(Some(
            panel.memory_sub_selection.min(rows.len().saturating_sub(1)),
        ));
        let list = List::new(rows)
            .style(Style::default().bg(palette.panel_alt).fg(palette.text))
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(list, sidebar_layout[1], &mut state);
    }

    /// Render the model list (shared between standard and memory layouts).
    /// `highlight` controls whether to show a selection highlight (false when sidebar has focus).
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
                    if summary.provider_id == self.active_model.provider_id
                    && summary.model_id == self.active_model.model_id)
            })
        } else if panel.is_memory_tab() {
            // For memory tab, find the model currently saved for the active role
            let role = panel.active_memory_role();
            let current = self.config.memory_model_label(role);
            current.and_then(|label| {
                items.iter().position(|item| match item {
                    ModelPanelItem::Model { summary } => summary.label() == label,
                    _ => false,
                })
            })
        } else {
            None
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
                    let is_active = summary.provider_id == self.active_model.provider_id
                        && summary.model_id == self.active_model.model_id
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
                    } else if is_active && self.thinking_level.is_supported() {
                        Some(self.thinking_level.display_name().to_string())
                    } else if panel.is_memory_tab() {
                        let model_label = summary.label();
                        if let Some(tl_str) =
                            self.config.memory.thinking_levels.get("consolidation")
                            && self.config.memory.consolidation_model.as_deref()
                                == Some(&model_label)
                        {
                            let tl_level =
                                tl_str.rsplit_once(':').map(|(_, v)| v).unwrap_or(tl_str);
                            Some(tl_level.to_string())
                        } else {
                            None
                        }
                    } else if !panel.is_general_tab() {
                        if let Some(tab) = panel.current_tab() {
                            if let Some(tl_str) =
                                self.config.agent.thinking_levels.get(&tab.agent_type_str)
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
            let status_text = panel.provider_status(i, &self.auth);

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
        self.mcp_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);
        let title = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(title, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.register_selection_region(inner);

        let mcp_title = panel
            .editor
            .as_ref()
            .map(|e| e.title())
            .unwrap_or_else(|| " MCP servers ".to_string());

        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                &mcp_title,
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

        if let Some(editor) = &panel.editor {
            let sections = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(body);

            frame.render_widget(
                Paragraph::new(editor.help())
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                    .wrap(Wrap { trim: false }),
                sections[2],
            );

            frame.render_widget(
                Paragraph::new("Enter advance/save · Tab advance/save · Esc cancel")
                    .alignment(Alignment::Center)
                    .style(
                        Style::default()
                            .bg(palette.panel_alt)
                            .fg(palette.accent_soft),
                    ),
                sections[3],
            );
        } else {
            let sections = Layout::vertical([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(body);

            frame.render_widget(
                Paragraph::new("Type to filter by server name, transport, or status. Enter toggles connect/disconnect.")
                    .alignment(Alignment::Center)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[2],
                );
            } else {
                let mut state = ListState::default();
                state.select(Some(
                    panel.selected_index.min(items.len().saturating_sub(1)),
                ));

                let list = List::new(rows)
                    .style(Style::default().bg(palette.panel_alt).fg(palette.text))
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
                .style(
                    Style::default()
                        .bg(palette.panel_alt)
                        .fg(palette.accent_soft),
                ),
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

        // Larger overlay like skills panel
        let overlay = centered_rect(85, 80, area);
        // Store overlay rect for mouse hit-testing
        self.memory_panel_overlay.set(Some(overlay));
        frame.render_widget(Clear, overlay);

        let panel_block = Block::default().style(Style::default().bg(palette.panel_alt));
        frame.render_widget(panel_block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let count = panel.filtered_indices().len();
        let memory_title = format!(" Memories · {}/{} ", count, panel.memories.len());

        match panel.mode {
            MemoryPanelMode::Browse => {
                // Two-pane layout: left (list) + right (preview) with footer
                let sections = Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Min(3),    // main two-pane area
                    Constraint::Length(1), // footer help
                ])
                .split(inner);

                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        &memory_title,
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]))
                    .style(Style::default().bg(palette.panel_alt)),
                    sections[0],
                );

                // ── Memories Mode ──
                let filtered = panel.filtered_indices();
                if filtered.is_empty() {
                    // Empty state centered in main area
                    frame.render_widget(
                        Paragraph::new("No memories yet. Press 'a' to add one.")
                            .alignment(Alignment::Center)
                            .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                        sections[1],
                    );
                } else {
                    // Split main area into left (35%) and right (65%)
                    let panes = Layout::horizontal([
                        Constraint::Percentage(35),
                        Constraint::Percentage(65),
                    ])
                    .split(sections[1]);

                    let list_area = panes[0];
                    let preview_area = panes[1];

                    // ── Left Pane: Memory List ──

                    // Header
                    let filter_text = match panel.filter_type {
                        None => "All types".to_string(),
                        Some(t) => format!("Filter: {}", t.as_str()),
                    };
                    let mut header_lines: Vec<Line<'static>> = Vec::new();
                    header_lines.push(Line::from(vec![Span::styled(
                        "  Name",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )]));
                    if panel.search_active {
                        let cursor = if (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                            / 500)
                            .is_multiple_of(2)
                        {
                            "|"
                        } else {
                            " "
                        };
                        header_lines.push(Line::from(vec![
                            Span::styled("  🔍 ", Style::default().fg(palette.accent)),
                            Span::styled(
                                format!("{}{}", panel.query, cursor),
                                Style::default().fg(palette.text),
                            ),
                        ]));
                    } else {
                        header_lines.push(Line::from(Span::styled(
                            format!("  {}  ·  / to search", filter_text),
                            Style::default().fg(palette.muted),
                        )));
                    }
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

                    // List items with scrollbar
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
                        let chunks =
                            Layout::horizontal([Constraint::Min(1), Constraint::Length(1)])
                                .split(list_content_area);
                        (chunks[0], Some(chunks[1]))
                    } else {
                        (list_content_area, None)
                    };

                    // Compute visible range such that selected_index is visible
                    let total_filtered = filtered.len();
                    let visible_height = list_content_height as usize;
                    let visible_start = if visible_height == 0 || total_filtered <= visible_height {
                        0
                    } else {
                        // Keep selection visible; prefer showing above selection
                        let half = visible_height / 2;
                        if panel.selected_index < half {
                            0
                        } else if panel.selected_index + half >= total_filtered {
                            total_filtered.saturating_sub(visible_height)
                        } else {
                            panel.selected_index.saturating_sub(half)
                        }
                    };
                    let visible_end = (visible_start + visible_height).min(total_filtered);

                    let mut list_lines: Vec<Line<'_>> = Vec::new();
                    for (i, &mem_idx) in filtered
                        .iter()
                        .enumerate()
                        .take(visible_end)
                        .skip(visible_start)
                    {
                        let entry = &panel.memories[mem_idx];
                        let is_selected = i == panel.selected_index;

                        // Single-line format: "▸ [proj] Title"
                        let type_label = entry.memory_type.short_label();
                        let prefix = if is_selected { "▸" } else { " " };
                        let base = format!("{} [{}] {}", prefix, type_label, entry.title);
                        // Truncate to fit width
                        let max_w = list_content_area.width.saturating_sub(3).max(10) as usize;
                        let display = if base.chars().count() > max_w {
                            format!(
                                "{}…",
                                base.chars()
                                    .take(max_w.saturating_sub(1))
                                    .collect::<String>()
                            )
                        } else {
                            base
                        };

                        let style = if is_selected {
                            Style::default()
                                .bg(palette.selection_bg)
                                .fg(palette.selection_fg)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(palette.text)
                        };

                        list_lines.push(Line::from(Span::styled(display, style)));
                    }

                    // Fill remaining space
                    while list_lines.len() < visible_height {
                        list_lines.push(Line::from(""));
                    }

                    frame.render_widget(
                        Paragraph::new(list_lines).style(Style::default().bg(palette.panel_alt)),
                        list_content_area,
                    );

                    // List scrollbar
                    if let Some(sb_area) = list_scrollbar_area {
                        render_scrollbar(
                            frame,
                            sb_area,
                            visible_start,
                            total_filtered,
                            palette,
                            false,
                        );
                    }

                    // ── Right Pane: Content Preview / Editor ──
                    match panel.focus {
                        PanelFocus::List => {
                            // Browse mode — render markdown preview with auto-wrap
                            if let Some(entry) = panel.selected_entry() {
                                let (preview_content_area, preview_scrollbar_area) =
                                    if preview_area.width > 2 {
                                        let chunks = Layout::horizontal([
                                            Constraint::Min(1),
                                            Constraint::Length(1),
                                            Constraint::Length(1),
                                        ])
                                        .split(preview_area);
                                        (chunks[0], Some(chunks[2]))
                                    } else if preview_area.width > 1 {
                                        let chunks = Layout::horizontal([
                                            Constraint::Min(1),
                                            Constraint::Length(1),
                                        ])
                                        .split(preview_area);
                                        (chunks[0], Some(chunks[1]))
                                    } else {
                                        (preview_area, None)
                                    };

                                use tidev_engine::markdown_render::render_markdown_text_with_width_and_cwd;
                                let content_width =
                                    preview_content_area.width.saturating_sub(2) as usize;

                                // Build metadata header lines
                                let mut header_lines: Vec<Line<'_>> = Vec::new();

                                // Line 1: [type] Title (with RO marker for system types)
                                let is_system = matches!(
                                    entry.memory_type,
                                    tidev_engine::memory::MemoryType::Fact
                                        | tidev_engine::memory::MemoryType::Pattern
                                        | tidev_engine::memory::MemoryType::Insight
                                        | tidev_engine::memory::MemoryType::Lesson
                                );
                                let title_prefix = if is_system { " [RO]" } else { "" };
                                header_lines.push(Line::from(vec![
                                    Span::styled(
                                        format!(
                                            " [{}]{} ",
                                            entry.memory_type.as_str(),
                                            title_prefix
                                        ),
                                        Style::default()
                                            .fg(palette.accent)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::styled(
                                        &entry.title,
                                        Style::default()
                                            .fg(palette.text)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                ]));

                                // Line 2: tags
                                if !entry.tags.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!(" Tags: {}", entry.tags.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Line 3: concepts
                                if !entry.concepts.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!(" Concepts: {}", entry.concepts.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Line 4: files
                                if !entry.files.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!(" Files: {}", entry.files.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Line 5: importance, strength, version
                                let mut meta_parts = Vec::new();
                                meta_parts.push(format!("Importance: {}/10", entry.importance));
                                if entry.strength > 0.0 {
                                    meta_parts.push(format!("Strength: {:.2}", entry.strength));
                                }
                                meta_parts.push(format!("v{}", entry.version));
                                header_lines.push(Line::from(Span::styled(
                                    format!(" {}", meta_parts.join("  ")),
                                    Style::default().fg(palette.accent_soft),
                                )));

                                // Line 6: created / updated timestamps
                                header_lines.push(Line::from(Span::styled(
                                    format!(
                                        " Created: {}  Updated: {}",
                                        entry.created_at.format("%Y-%m-%d %H:%M"),
                                        entry.updated_at.format("%Y-%m-%d %H:%M")
                                    ),
                                    Style::default().fg(palette.muted),
                                )));

                                // Line 7: version chain info
                                let mut chain_parts: Vec<String> = Vec::new();
                                if let Some(pid) = entry.parent_id {
                                    chain_parts.push(format!("Parent: {}", pid));
                                }
                                if !entry.supersedes.is_empty() {
                                    chain_parts.push(format!(
                                        "Supersedes: {} version(s)",
                                        entry.supersedes.len()
                                    ));
                                }
                                if !entry.related_ids.is_empty() {
                                    chain_parts.push(format!(
                                        "Related: {} memory(ies)",
                                        entry.related_ids.len()
                                    ));
                                }
                                if !chain_parts.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!(" {}", chain_parts.join("  ")),
                                        Style::default().fg(palette.warning),
                                    )));
                                }

                                // Separator
                                let sep_w = content_width.clamp(10, 80);
                                header_lines.push(Line::from(Span::styled(
                                    format!(" {}", "─".repeat(sep_w)),
                                    Style::default().fg(palette.border),
                                )));

                                // Render markdown content
                                let rendered = render_markdown_text_with_width_and_cwd(
                                    &entry.content,
                                    Some(content_width),
                                    None,
                                );

                                // Combine header + markdown content
                                let mut all_lines: Vec<Line<'_>> = Vec::new();
                                all_lines.extend(header_lines);
                                all_lines.extend(rendered);

                                let total_lines = all_lines.len();
                                let scroll =
                                    panel.preview_scroll.min(total_lines.saturating_sub(1));

                                let visible_lines: Vec<Line<'_>> = all_lines
                                    .into_iter()
                                    .skip(scroll)
                                    .take(preview_content_area.height as usize)
                                    .collect();

                                frame.render_widget(
                                    Paragraph::new(visible_lines)
                                        .style(Style::default().bg(palette.panel_alt)),
                                    preview_content_area,
                                );

                                if let Some(sb_area) = preview_scrollbar_area {
                                    render_scrollbar(
                                        frame,
                                        sb_area,
                                        panel.preview_scroll,
                                        total_lines,
                                        palette,
                                        false,
                                    );
                                }
                            } else {
                                frame.render_widget(
                                    Paragraph::new("No memory selected")
                                        .alignment(Alignment::Center)
                                        .style(
                                            Style::default()
                                                .bg(palette.panel_alt)
                                                .fg(palette.muted),
                                        ),
                                    preview_area,
                                );
                            }
                        }
                        PanelFocus::ContentEdit => {
                            // Edit mode — show entry metadata header + editable content
                            if let Some(entry) = panel.selected_entry() {
                                // Split preview area for scrollbar
                                let (editor_area, _edit_scrollbar_area) = if preview_area.width > 2
                                {
                                    let chunks = Layout::horizontal([
                                        Constraint::Min(1),
                                        Constraint::Length(1),
                                        Constraint::Length(1),
                                    ])
                                    .split(preview_area);
                                    (chunks[0], Some(chunks[2]))
                                } else if preview_area.width > 1 {
                                    let chunks = Layout::horizontal([
                                        Constraint::Min(1),
                                        Constraint::Length(1),
                                    ])
                                    .split(preview_area);
                                    (chunks[0], Some(chunks[1]))
                                } else {
                                    (preview_area, None)
                                };

                                // Build header info lines
                                let mut header_lines: Vec<Line<'_>> = Vec::new();

                                // Type badge + title
                                header_lines.push(Line::from(vec![
                                    Span::styled(
                                        format!("  [{}] ", entry.memory_type.as_str()),
                                        Style::default()
                                            .fg(palette.accent)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                    Span::styled(
                                        &entry.title,
                                        Style::default()
                                            .fg(palette.text)
                                            .add_modifier(Modifier::BOLD),
                                    ),
                                ]));

                                // Tags
                                if !entry.tags.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!("  Tags: {}", entry.tags.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Concepts
                                if !entry.concepts.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!("  Concepts: {}", entry.concepts.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Files
                                if !entry.files.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!("  Files: {}", entry.files.join(", ")),
                                        Style::default().fg(palette.muted),
                                    )));
                                }

                                // Importance / strength / version
                                let mut meta_parts = Vec::new();
                                meta_parts.push(format!("Importance: {}/10", entry.importance));
                                if entry.strength > 0.0 {
                                    meta_parts.push(format!("Strength: {:.2}", entry.strength));
                                }
                                meta_parts.push(format!("v{}", entry.version));
                                header_lines.push(Line::from(Span::styled(
                                    format!("  {}", meta_parts.join("  ")),
                                    Style::default().fg(palette.accent_soft),
                                )));

                                // Version chain info
                                let mut chain_parts: Vec<String> = Vec::new();
                                if let Some(pid) = entry.parent_id {
                                    chain_parts.push(format!("Parent: {}", pid));
                                }
                                if !entry.supersedes.is_empty() {
                                    chain_parts.push(format!(
                                        "Supersedes: {} version(s)",
                                        entry.supersedes.len()
                                    ));
                                }
                                if !entry.related_ids.is_empty() {
                                    chain_parts.push(format!(
                                        "Related: {} memory(ies)",
                                        entry.related_ids.len()
                                    ));
                                }
                                if !chain_parts.is_empty() {
                                    header_lines.push(Line::from(Span::styled(
                                        format!("  {}", chain_parts.join("  ")),
                                        Style::default().fg(palette.warning),
                                    )));
                                }

                                // EDITING indicator + separator
                                let sep_w = editor_area.width.saturating_sub(4) as usize;
                                header_lines.push(Line::from(Span::styled(
                                    format!(
                                        "  ── EDITING ──{}",
                                        "─".repeat(sep_w.saturating_sub(14).min(40))
                                    ),
                                    Style::default()
                                        .fg(palette.accent)
                                        .add_modifier(Modifier::BOLD),
                                )));

                                // Render header
                                let header_h = header_lines.len() as u16;
                                frame.render_widget(
                                    Paragraph::new(header_lines)
                                        .style(Style::default().bg(palette.panel_alt)),
                                    Rect::new(
                                        editor_area.x,
                                        editor_area.y,
                                        editor_area.width,
                                        header_h.min(editor_area.height),
                                    ),
                                );

                                // Editor content area (below header)
                                let edit_content_y = editor_area.y + header_h;
                                let edit_content_height =
                                    editor_area.height.saturating_sub(header_h).max(1);
                                let edit_content_area = Rect::new(
                                    editor_area.x,
                                    edit_content_y,
                                    editor_area.width,
                                    edit_content_height,
                                );

                                // Store editor width for cursor movement
                                panel.editor_width.set(editor_area.width);

                                if edit_content_area.width > 0 && edit_content_height > 0 {
                                    // Build lines from composer
                                    let editor_width = edit_content_area.width as usize;
                                    let visual_lines =
                                        panel.content_editor.visual_lines(editor_width);
                                    let editor_scroll = 0;
                                    let mut lines: Vec<Line<'_>> = Vec::new();

                                    for range in visual_lines.iter() {
                                        let line_text = &panel.content_editor.text()[range.clone()];
                                        lines.push(Line::from(Span::styled(
                                            line_text.to_string(),
                                            Style::default().fg(palette.text),
                                        )));
                                    }

                                    let visible_lines: Vec<Line<'_>> = lines
                                        .into_iter()
                                        .skip(editor_scroll)
                                        .take(edit_content_height as usize)
                                        .collect();

                                    frame.render_widget(
                                        Paragraph::new(visible_lines)
                                            .style(Style::default().bg(palette.panel_alt)),
                                        edit_content_area,
                                    );

                                    // Set cursor position
                                    let (cursor_line, cursor_col) =
                                        panel.content_editor.cursor_position(editor_area.width);
                                    let cursor_y = edit_content_y.saturating_add(
                                        cursor_line.min(edit_content_height.saturating_sub(1)),
                                    );
                                    let cursor_x = editor_area.x.saturating_add(cursor_col);
                                    frame.set_cursor_position(Position::new(cursor_x, cursor_y));
                                }
                            } else {
                                frame.render_widget(
                                    Paragraph::new("No memory selected")
                                        .alignment(Alignment::Center)
                                        .style(
                                            Style::default()
                                                .bg(palette.panel_alt)
                                                .fg(palette.muted),
                                        ),
                                    preview_area,
                                );
                            }
                        }
                    }
                }

                // Footer help
                let footer_y = sections[2].y;
                let help_text = match panel.focus {
                    PanelFocus::List if panel.search_active => {
                        "  Type to search  Esc: clear/exit  Enter: confirm  Up/Down: navigate"
                    }
                    PanelFocus::List => {
                        "  Up/Down: navigate  Left/Right: scroll  /: search  Enter: edit  a: add  e: edit content  d: delete  r: filter  Esc: close"
                    }
                    PanelFocus::ContentEdit => {
                        "  Arrow keys: move cursor  Enter: save  Esc: cancel  Shift+Enter: newline"
                    }
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        help_text,
                        Style::default().fg(palette.accent_soft),
                    )))
                    .style(Style::default().bg(palette.panel_alt)),
                    Rect::new(inner.x, footer_y, inner.width, 1),
                );
            }

            MemoryPanelMode::Add | MemoryPanelMode::Edit => {
                let label = match panel.mode {
                    MemoryPanelMode::Add => " Add Memory ",
                    MemoryPanelMode::Edit => " Edit Memory ",
                    _ => unreachable!(),
                };

                let sections = Layout::vertical([
                    Constraint::Length(1), // label
                    Constraint::Length(1), // type
                    Constraint::Length(1), // title
                    Constraint::Min(6),    // content
                    Constraint::Length(1), // tags
                    Constraint::Length(1), // concepts
                    Constraint::Length(1), // files
                    Constraint::Length(1), // importance
                    Constraint::Length(1), // hints
                ])
                .split(inner);

                // Label
                frame.render_widget(
                    Paragraph::new(label)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.accent)),
                    sections[0],
                );

                // Helper: style a field line with active highlight
                let field_style = |is_active: bool| -> Style {
                    if is_active {
                        Style::default()
                            .bg(palette.selection_bg)
                            .fg(palette.selection_fg)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().bg(palette.panel_alt).fg(palette.text)
                    }
                };

                let active_field = panel.edit_field.as_ref();

                // Type
                frame.render_widget(
                    Paragraph::new(format!(
                        "Type: {}   (Tab to change)",
                        panel.edit_type.as_str()
                    ))
                    .style(field_style(active_field == Some(&EditField::Type))),
                    sections[1],
                );

                // Title
                let title_display = if panel.edit_title.is_empty() {
                    "Title: (type here)".to_string()
                } else {
                    format!("Title: {}", panel.edit_title)
                };
                frame.render_widget(
                    Paragraph::new(title_display)
                        .style(field_style(active_field == Some(&EditField::Title))),
                    sections[2],
                );

                // Content
                let content_display = if panel.edit_content.is_empty() {
                    "Content: (type here)".to_string()
                } else {
                    format!("Content: {}", panel.edit_content)
                };
                frame.render_widget(
                    Paragraph::new(content_display)
                        .style(field_style(active_field == Some(&EditField::Content)))
                        .wrap(Wrap { trim: false }),
                    sections[3],
                );

                // Tags
                let tags_display = if panel.edit_tags.is_empty() {
                    "Tags: (comma separated)".to_string()
                } else {
                    format!("Tags: {}", panel.edit_tags)
                };
                frame.render_widget(
                    Paragraph::new(tags_display)
                        .style(field_style(active_field == Some(&EditField::Tags))),
                    sections[4],
                );

                // Concepts
                let concepts_display = if panel.edit_concepts.is_empty() {
                    "Concepts: (comma separated)".to_string()
                } else {
                    format!("Concepts: {}", panel.edit_concepts)
                };
                frame.render_widget(
                    Paragraph::new(concepts_display)
                        .style(field_style(active_field == Some(&EditField::Concepts))),
                    sections[5],
                );

                // Files
                let files_display = if panel.edit_files.is_empty() {
                    "Files: (comma separated paths)".to_string()
                } else {
                    format!("Files: {}", panel.edit_files)
                };
                frame.render_widget(
                    Paragraph::new(files_display)
                        .style(field_style(active_field == Some(&EditField::Files))),
                    sections[6],
                );

                // Importance
                frame.render_widget(
                    Paragraph::new(format!(
                        "Importance: {} /10   (type a digit 1-9)",
                        panel.edit_importance
                    ))
                    .style(field_style(active_field == Some(&EditField::Importance))),
                    sections[7],
                );

                // Hints
                let hint_text = if active_field.is_none() {
                    "Tab: cycle fields  Enter: save  Esc: cancel"
                } else {
                    "Tab: cycle fields  Type to edit  Enter: save  Esc: cancel"
                };
                frame.render_widget(
                    Paragraph::new(hint_text)
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
                    sections[8],
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
                            .style(Style::default().bg(palette.panel_alt).fg(palette.warning)),
                        sections[0],
                    );
                }

                frame.render_widget(
                    Paragraph::new("Press Y to confirm, N or Esc to cancel")
                        .alignment(Alignment::Center)
                        .style(Style::default().bg(palette.panel_alt).fg(palette.muted)),
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
        use tidev_engine::markdown_render::render_markdown_text_with_width_and_cwd;

        let palette = self.palette();

        // Main overlay - 85% width, 80% height
        let overlay = centered_rect(85, 80, area);
        self.skills_panel_overlay.set(Some(overlay));
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
