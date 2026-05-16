use crate::{theme::ThemePalette, utils::TokenUsage};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Render a vertical scrollbar (1 column wide) into the given area.
/// Draws a track (░) with a thumb (█) proportional to the visible fraction.
/// `content_height` is the total number of lines in the scrollable content.
pub(crate) fn render_scrollbar(
    frame: &mut Frame<'_>,
    area: Rect,
    scroll: usize,
    content_height: usize,
    palette: ThemePalette,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let track_style = Style::default().bg(palette.background).fg(palette.border);
    let thumb_style = Style::default().bg(palette.background).fg(palette.accent);
    let height = area.height as usize;
    let mut lines = Vec::with_capacity(height);

    if content_height <= height || height == 0 {
        for _ in 0..height {
            lines.push(Line::from(vec![Span::styled(" ", track_style)]));
        }
    } else {
        let max_scroll = content_height.saturating_sub(height);
        let thumb_height = ((height * height) / content_height.max(1))
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

    let paragraph =
        Paragraph::new(Text::from(lines)).style(Style::default().bg(palette.background));
    frame.render_widget(paragraph, area);
}
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

use super::{App, Composer, Screen};

impl App {
    pub(crate) fn palette(&self) -> ThemePalette {
        self.theme.palette()
    }

    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        self.message_content_area = None;
        self.message_scrollbar_area = None;
        self.sidebar_area = None;
        self.input_area.set(None);
        if self.at_mention.visible {
            self.refresh_at_mention_state();
        }
        if self.snippet_state.visible && self.snippet_state.is_enabled() {
            self.refresh_snippet_state();
        }
        match self.screen {
            Screen::Welcome => self.render_welcome(frame),
            Screen::Chat => self.render_chat(frame),
        }
        let area = frame.area();
        self.render_connect_dialog(frame, area);
        self.render_panel_launcher(frame, area);
        if let Some(panel) = &self.theme_panel {
            self.render_theme_panel(frame, area, panel);
        }
        if let Some(panel) = &self.agents_panel {
            self.render_agents_panel(frame, area, panel);
        }
        if let Some(panel) = &self.skills_panel {
            self.render_skills_panel(frame, area, panel);
        }
        if let Some(panel) = &self.sandbox_panel {
            self.render_sandbox_panel(frame, area, panel);
        }
        if let Some(panel) = &self.settings_panel {
            self.render_settings_panel(frame, area, panel);
        }
        if let Some(panel) = &self.mcp_panel {
            self.render_mcp_panel(frame, area, panel);
        }
        if let Some(panel) = &self.model_panel {
            self.render_model_panel(frame, area, panel);
        }
        if let Some(panel) = &self.search_panel {
            self.render_search_panel(frame, area, panel);
        }
        if let Some(panel) = &self.message_panel {
            self.render_message_panel(frame, area, panel);
        }
        if let Some(panel) = &self.memory_panel {
            self.render_memory_panel(frame, area, panel);
        }
        if let Some(panel) = &self.session_panel {
            self.render_session_panel(frame, area, panel);
            self.render_session_panel_dialog(frame, area, panel);
        }
        if let Some(panel) = &self.stats_panel
            && panel.active
        {
            self.render_stats_panel(frame, area);
        }
        let balance_active = self
            .balance_panel
            .lock()
            .map(|guard| guard.as_ref().is_some_and(|p| p.active))
            .unwrap_or(false);
        if balance_active {
            self.render_balance_panel(frame, area);
        }
        if let Some(dialog) = &self.rename_dialog {
            self.render_rename_session_dialog(frame, area, dialog);
        }
        if let Some(dialog) = &self.permission_dialog {
            self.render_permission_dialog(frame, area, dialog);
        }
        if self.sandbox_elevation.is_some() {
            self.render_sandbox_elevation_dialog(frame, area);
        }
        if self.fork_confirm_dialog.is_some() {
            self.render_fork_confirm_dialog(frame, area);
        }
        if self.undo_confirm_dialog.is_some() {
            self.render_undo_confirm_dialog(frame, area);
        }
        self.finish_mouse_selection(frame);
        self.render_toast(frame);
    }

    fn render_toast(&mut self, frame: &mut Frame<'_>) {
        let now = Instant::now();
        let Some((message, expires_at)) = self.toast.take() else {
            return;
        };

        if now >= expires_at {
            return;
        }

        let Some(message_area) = self.message_content_area else {
            return;
        };

        let palette = self.palette();
        let message_width = UnicodeWidthStr::width(message.as_str()).min(30);
        let width = (message_width + 2).min(32) as u16;
        let height = 3;

        let x = message_area.right().saturating_sub(width + 1);
        let y = message_area.top().saturating_add(1);

        let rect = Rect::new(x, y, width, height);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.accent))
            .style(Style::default().bg(palette.background).fg(palette.text));
        let paragraph = Paragraph::new(message.as_str())
            .style(Style::default().bg(palette.background).fg(palette.text))
            .alignment(Alignment::Center)
            .block(block);

        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);

        self.toast = Some((message, expires_at));
    }

    fn render_welcome(&mut self, frame: &mut Frame<'_>) {
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
        let card_height = 20u16.min(area.height.saturating_sub(2).max(10));
        let card = centered_rect(card_width, card_height, area);

        let card_inner_width = card.width.saturating_sub(4);

        let block = Block::default().borders(Borders::NONE);
        frame.render_widget(block, card);

        let inner = card.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let sections = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Length(
                self.composer
                    .preferred_height(card_inner_width, self.config.ui.max_input_lines),
            ),
            Constraint::Length(1),
        ])
        .split(inner);

        // https://patorjk.com/software/taag/#p=display&f=BlurVision+ASCII&t=tidev&x=none&v=4&h=4&w=80&we=false
        let ascii_art = Paragraph::new(
            r#"░▒▓████████▓▒░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░       ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓██████▓▒░  ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░  ░▒▓██▓▒░    "#,
        )
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(ascii_art, sections[0]);

        let subtitle = Paragraph::new("Terminal AI assistant for focused coding work")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.muted));
        frame.render_widget(subtitle, sections[1]);

        let prompt_title = match self.pending_mode.as_ref() {
            Some(pending) if self.pending_request => {
                format!(
                    "{} (current), {} (on completion)",
                    self.mode.title(),
                    pending.title()
                )
            }
            _ => self.mode.title().to_string(),
        };
        let prompt_placeholder = self.composer.placeholder().to_string();
        self.render_input_block(
            frame,
            sections[2],
            &prompt_title,
            &prompt_placeholder,
            false,
        );

        let model_label = self.active_model.label();
        let sandbox_label = self
            .tools
            .sandbox_policy()
            .map(|p| p.label())
            .unwrap_or_else(|| self.mode.sandbox_policy(&self.config.sandbox).label());
        let model_display = if self.thinking_level.is_supported() {
            format!(
                "{} [{}]  Sandbox: {}",
                model_label,
                self.thinking_level.display_name(),
                sandbox_label
            )
        } else {
            format!("{}  Sandbox: {}", model_label, sandbox_label)
        };
        let model_line = Line::from(vec![Span::styled(
            model_display,
            Style::default().fg(palette.accent),
        )]);
        frame.render_widget(
            Paragraph::new(model_line).style(Style::default().fg(palette.text)),
            sections[3],
        );

        let workspace_path = self.workspace_root.display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );
        let workspace_line = Line::from(Span::styled(
            display_path,
            Style::default().fg(palette.muted),
        ));

        let workspace_area = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(Paragraph::new(workspace_line), workspace_area);

        self.render_at_mention_palette(frame, sections[2]);
        self.render_snippet_palette(frame, sections[2]);
        self.render_command_palette(frame, sections[2]);
    }

    pub(super) fn render_input_block(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        placeholder: &str,
        mask_input: bool,
    ) {
        self.render_input_block_with_composer(
            frame,
            area,
            title,
            &self.composer,
            placeholder,
            mask_input,
            true,
        );
    }

    pub(super) fn render_input_block_with_composer(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        title: &str,
        composer: &Composer,
        placeholder: &str,
        mask_input: bool,
        register_input_area: bool,
    ) {
        let palette = self.palette();
        let border_style = if self.shell_mode {
            Style::default().fg(palette.success)
        } else if self.pending_request {
            // Request was made in current mode — keep its color while waiting
            Style::default().fg(palette.border_mode_color(self.mode))
        } else if let Some(pending) = self.pending_mode {
            // Mode switch queued without pending request — preview the future mode
            Style::default().fg(palette.border_mode_color(pending))
        } else {
            Style::default().fg(palette.border_mode_color(self.mode))
        };

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        if register_input_area {
            self.input_area.set(Some(inner));
        }

        let visible_lines = inner.height.max(1) as usize;
        let total_lines = composer.display_line_count(inner.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        // Use stored scroll offset, clamped to valid range
        let scroll = if register_input_area {
            self.input_scroll_offset.min(max_scroll) as u16
        } else {
            0
        };

        let content = if composer.is_empty() {
            Text::from(Line::from(Span::styled(
                placeholder.to_string(),
                Style::default().fg(palette.muted),
            )))
        } else if mask_input {
            Text::from(Line::from(Span::styled(
                "•".repeat(composer.text().chars().count().max(1)),
                Style::default().fg(palette.text),
            )))
        } else {
            let width = inner.width as usize;
            let selection = composer.selection_range();

            // Build lines with selection highlighting
            let visual_lines = composer.visual_lines(width);
            let mut lines = Vec::new();

            for range in visual_lines.iter() {
                let line_text = &composer.text()[range.clone()];

                if let Some((sel_start, sel_end)) = selection {
                    // This line may have selection
                    let line_start = range.start;
                    let line_end = range.end;

                    // Calculate intersection of selection with this line
                    let sel_in_line_start = sel_start.max(line_start);
                    let sel_in_line_end = sel_end.min(line_end);

                    if sel_in_line_start < sel_in_line_end {
                        // Selection overlaps this line
                        let mut spans = Vec::new();

                        // Before selection
                        if sel_in_line_start > line_start {
                            let before = &composer.text()[line_start..sel_in_line_start];
                            spans.push(Span::styled(
                                before.to_string(),
                                Style::default().fg(palette.text),
                            ));
                        }

                        // Selection
                        let selected = &composer.text()[sel_in_line_start..sel_in_line_end];
                        spans.push(Span::styled(
                            selected.to_string(),
                            Style::default().fg(palette.text).bg(palette.accent),
                        ));

                        // After selection
                        if sel_in_line_end < line_end {
                            let after = &composer.text()[sel_in_line_end..line_end];
                            spans.push(Span::styled(
                                after.to_string(),
                                Style::default().fg(palette.text),
                            ));
                        }

                        lines.push(Line::from(spans));
                    } else {
                        // No selection on this line
                        lines.push(Line::from(line_text.to_string()));
                    }
                } else {
                    lines.push(Line::from(line_text.to_string()));
                }
            }

            Text::from(lines)
        };

        let mut paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .style(Style::default().fg(palette.text))
            .scroll((scroll, 0));

        paragraph = paragraph.wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);

        if inner.width > 0 && inner.height > 0 {
            let (cursor_line, cursor_col) = composer.cursor_position(inner.width);
            let mut cursor_line = cursor_line.saturating_sub(scroll);
            let mut cursor_col = cursor_col;

            if composer.cursor_wraps_to_next_row(inner.width as usize) {
                cursor_line = cursor_line.saturating_add(1);
                cursor_col = 0;
            }

            let cursor_x = inner.x.saturating_add(cursor_col);
            let cursor_y = inner
                .y
                .saturating_add(cursor_line.min(inner.height.saturating_sub(1)));

            frame.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
    }
}

pub(crate) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect::new(x, y, width, height)
}

pub(crate) fn shorten(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }

    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

pub(super) fn spans_with_highlights(
    text: &str,
    highlight_indices: &[usize],
    normal_style: Style,
    highlighted_style: Style,
) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(text.chars().count());
    let mut highlight_indices = highlight_indices.iter().copied().peekable();

    for (index, ch) in text.chars().enumerate() {
        let style = if highlight_indices.peek().is_some_and(|next| *next == index) {
            highlight_indices.next();
            highlighted_style
        } else {
            normal_style
        };

        spans.push(Span::styled(ch.to_string(), style));
    }

    spans
}

impl App {
    /// Render the sandbox policy selection panel.
    pub(super) fn render_sandbox_panel(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        panel: &crate::tui::ui::sandbox_panel::SandboxPanelState,
    ) {
        use crate::tui::ui::sandbox_panel::SandboxPanelState as S;
        use ratatui::layout::Margin;
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Clear, List};

        let palette = self.palette();

        let overlay = centered_rect(28, S::build_items().len() as u16 + 2, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .title(" Sandbox Policy ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_active()))
            .style(Style::default().bg(palette.panel));
        frame.render_widget(block, overlay);

        let inner = overlay.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });

        let items = S::build_items();

        // Build list items
        let list_items: Vec<ratatui::widgets::ListItem> = items
            .iter()
            .map(|item| {
                ratatui::widgets::ListItem::new(Line::from(vec![Span::styled(
                    item.label,
                    Style::default().add_modifier(Modifier::BOLD),
                )]))
            })
            .collect();

        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(
            panel.selected_index.min(items.len().saturating_sub(1)),
        ));

        let list = List::new(list_items)
            .highlight_style(
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg),
            )
            .highlight_symbol("");
        frame.render_stateful_widget(list, inner, &mut list_state);
    }

    /// Render the sandbox elevation dialog.
    pub(super) fn render_sandbox_elevation_dialog(&self, frame: &mut Frame<'_>, area: Rect) {
        use ratatui::layout::{Constraint, Layout, Margin};
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Clear, Paragraph};

        let palette = self.palette();
        let overlay = centered_rect(56, 8, area);
        frame.render_widget(Clear, overlay);

        let block = Block::default()
            .title(" Sandbox Denied ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.warning))
            .style(Style::default().bg(palette.panel));
        frame.render_widget(&block, overlay);

        let inner = overlay.inner(Margin::new(2, 1));
        let sections = Layout::vertical([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

        // Main message
        let msg = Line::from(Span::styled(
            "This command was blocked by the OS sandbox.",
            Style::default().fg(palette.text),
        ));
        let hint = Line::from(Span::styled(
            "Retry with full filesystem access?",
            Style::default().fg(palette.muted),
        ));
        frame.render_widget(
            Paragraph::new(vec![msg, hint]).style(Style::default().bg(palette.panel)),
            sections[0],
        );

        // Options
        let options = Line::from(vec![
            Span::styled(
                "  [Y] Retry with full access  ",
                Style::default().fg(palette.success),
            ),
            Span::styled("[N] Cancel  ", Style::default().fg(palette.error)),
        ]);
        frame.render_widget(
            Paragraph::new(options).style(Style::default().bg(palette.panel)),
            sections[1],
        );

        // Separator
        let sep = Line::from(Span::styled(
            "\u{2500}".repeat(inner.width.saturating_sub(2) as usize),
            Style::default().fg(palette.muted),
        ));
        frame.render_widget(
            Paragraph::new(sep).style(Style::default().bg(palette.panel)),
            sections[2],
        );
    }

    pub(super) fn render_prompt_footer(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let status_text = self.footer_status_text();
        let status_width = status_text.width().min(area.width as usize).max(1) as u16;
        let chunks =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(status_width)]).split(area);

        // Build the left label: model-name [thinking-level]  Sandbox: xxxx
        // Check runtime override first, fall back to mode-based config policy
        let sandbox_label = self
            .tools
            .sandbox_policy()
            .map(|p| p.label())
            .unwrap_or_else(|| self.mode.sandbox_policy(&self.config.sandbox).label());

        let model_label = self.active_model.label();
        let full_label = if self.thinking_level.is_supported() {
            format!(
                "{} [{}]  Sandbox: {}",
                model_label,
                self.thinking_level.display_name(),
                sandbox_label
            )
        } else {
            format!("{}  Sandbox: {}", model_label, sandbox_label)
        };

        let model_line = Line::from(vec![Span::styled(
            full_label,
            Style::default().fg(palette.accent),
        )]);

        frame.render_widget(
            Paragraph::new(model_line).style(Style::default().fg(palette.text)),
            chunks[0],
        );

        frame.render_widget(
            Paragraph::new(status_text)
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette.muted)),
            chunks[1],
        );
    }

    pub(super) fn render_retrying_hint(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();

        let Some((attempt, max_attempts, reason, deadline)) = self.retrying_hint.as_ref() else {
            // Clear any existing content
            frame.render_widget(
                Paragraph::new("").style(Style::default().fg(palette.text)),
                area,
            );
            return;
        };

        let now = Instant::now();
        let remaining = if *deadline > now {
            deadline.duration_since(now).as_secs()
        } else {
            0
        };

        let retry_after_str = format!("Retrying in {remaining}s");
        let hint_text = format!(
            "Retrying ({}/{}): {} · {}",
            attempt, max_attempts, reason, retry_after_str
        );

        frame.render_widget(
            Paragraph::new(hint_text).style(Style::default().fg(palette.accent_soft)),
            area,
        );
    }

    fn footer_status_text(&mut self) -> String {
        let queued_count = self.pending_prompt_queue.len();

        if self.pending_request
            && self
                .abort_confirmation_deadline
                .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return "Esc again to stop".to_string();
        }

        let token_status = self.context_usage.as_ref().map(|usage| {
            let token_usage = TokenUsage::new(
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
            );
            let max_context = self.active_model.context_window;
            let percent = token_usage.context_usage_pct(max_context);
            let used_k = usage.input_tokens / 1000;
            let max_k = max_context as u32 / 1000;
            format!("{:.1}% ({}K/{}K)", percent, used_k, max_k)
        });

        if self.pending_request {
            let spinner = self.loading_spinner();

            let status = if self.conversation.parent_session_id.is_some() {
                format!("{} Thinking...", spinner)
            } else if !self.running_subagent_executions.is_empty() {
                let count = self.running_subagent_executions.len();
                let label = if count == 1 { "subagent" } else { "subagents" };
                format!("{} Waiting for {} {}", spinner, count, label)
            } else if !self.running_tool_executions.is_empty() {
                let tool_names: Vec<_> = self
                    .running_tool_executions
                    .iter()
                    .map(|r| r.tool_call.name.as_str())
                    .collect();
                let count = tool_names.len();
                if count == 1 {
                    format!("{} Running {}", spinner, tool_names[0])
                } else {
                    format!(
                        "{} Running {} tools ({})",
                        spinner,
                        count,
                        tool_names.join(", ")
                    )
                }
            } else if self.pending_tool_execution.is_some() {
                format!("{} Running tools", spinner)
            } else {
                format!("{} {}", spinner, self.mode.title())
            };

            let status = if queued_count > 0 {
                format!("{} · queued {}", status, queued_count)
            } else {
                status
            };

            if let Some(token_status) = token_status {
                return format!("{} · {}", status, token_status);
            }

            return status;
        }

        if queued_count > 0 {
            let status = if queued_count == 1 {
                "1 queued message".to_string()
            } else {
                format!("{queued_count} queued messages")
            };

            if let Some(token_status) = token_status {
                return format!("{} · {}", status, token_status);
            }

            return status;
        }

        if let Some(token_status) = token_status {
            return token_status;
        }

        if let Some(message) = self.last_notice.as_deref() {
            let background_running = self.background_running_count();
            let background_waiting = self.background_waiting_question_count();
            if background_running > 0 || background_waiting > 0 {
                return format!(
                    "{} · bg:{} · waiting:{}",
                    message, background_running, background_waiting
                );
            }
            return message.to_string();
        }

        let background_running = self.background_running_count();
        let background_waiting = self.background_waiting_question_count();
        if background_running > 0 || background_waiting > 0 {
            return format!(
                "Ready · bg:{} · waiting:{}",
                background_running, background_waiting
            );
        }

        if self.conversation.parent_session_id.is_some() {
            return "Subsession active · Ctrl+X then Up arrow to return".to_string();
        }

        "Ready".to_string()
    }

    pub(crate) fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        const FRAME_DURATION_MS: u128 = 100;

        let elapsed = self.spinner_start.elapsed().as_millis();
        let frame_index = (elapsed / FRAME_DURATION_MS) as usize;

        FRAMES[frame_index % FRAMES.len()]
    }
}

pub(crate) fn line_with_style(text: &str, fg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        text.to_string(),
        Style::default().fg(fg),
    )])
}

pub(crate) fn line_with_style_right_aligned(text: &str, width: usize, fg: Color) -> Line<'static> {
    let text_width = UnicodeWidthStr::width(text);
    let padding = width.saturating_sub(text_width);
    let padded_text = format!("{}{}", " ".repeat(padding), text);
    Line::from(vec![Span::styled(padded_text, Style::default().fg(fg))])
}

pub(crate) fn line_with_prefix(
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

pub(crate) fn decorate_card_lines(
    lines: Vec<Line<'static>>,
    width: usize,
    background: Color,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| decorate_card_line(line, width, background))
        .collect()
}

pub(crate) fn decorate_card_line(
    line: Line<'static>,
    width: usize,
    background: Color,
) -> Line<'static> {
    let bg_style = Style::default().bg(background);
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(2));
    spans.push(Span::styled(" ", bg_style));

    for mut span in line.spans {
        if span.style.bg.is_none() {
            span.style = span.style.patch(bg_style);
        }
        spans.push(span);
    }

    let used_width = line_display_width(&Line::from(spans.clone()));
    if used_width < width {
        spans.push(Span::styled(" ".repeat(width - used_width), bg_style));
    }

    Line::from(spans)
}

pub(super) fn line_display_width(line: &Line<'static>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

pub(crate) fn shorten_single_line(value: &str, max_chars: usize) -> String {
    let single_line: String = value
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    shorten(&single_line, max_chars)
}
