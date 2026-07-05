use crate::input::composer::{InlineSpan, InlineSpanKind};
use crate::theme::ThemePalette;
use crate::utils::TokenUsage;
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
    hovered: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let bg = if hovered {
        palette.hover_bg(palette.background)
    } else {
        palette.background
    };

    let track_style = Style::default().bg(bg).fg(palette.border);
    let thumb_style = Style::default().bg(bg).fg(palette.accent);
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

    let paragraph = Paragraph::new(Text::from(lines)).style(Style::default().bg(bg));
    frame.render_widget(paragraph, area);
}
use std::time::Instant;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{App, Composer, Screen};

impl App {
    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        self.ui.message_content_area = None;
        self.ui.message_scrollbar_area = None;
        self.ui.sidebar_area = None;
        self.ui.input_area.set(None);
        if self.ui.at_mention.visible {
            self.refresh_at_mention_state();
        }
        if self.ui.snippet_state.visible && self.ui.snippet_state.is_enabled() {
            self.refresh_snippet_state();
        }
        match self.ui.screen {
            Screen::Welcome => self.render_welcome(frame),
            Screen::Chat => self.render_chat(frame),
        }
        let area = frame.area();
        self.render_connect_dialog(frame, area);
        self.render_panel_launcher(frame, area);
        if let Some(panel) = &self.ui.theme_panel {
            self.render_theme_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.agents_panel {
            self.render_agents_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.skills_panel {
            self.render_skills_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.settings_panel {
            self.render_settings_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.model_panel {
            self.render_model_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.search_panel {
            self.render_search_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.message_panel {
            self.render_message_panel(frame, area, panel);
        }
        if let Some(panel) = &self.ui.session_panel {
            self.render_session_panel(frame, area, panel);
            self.render_session_panel_dialog(frame, area, panel);
        }
        if let Some(dialog) = &self.ui.rename_dialog {
            self.render_rename_session_dialog(frame, area, dialog);
        }
        if let Some(dialog) = &self.ui.permission_dialog {
            self.render_permission_dialog(frame, area, dialog);
        }
        if self.ui.fork_confirm_dialog.is_some() {
            self.render_fork_confirm_dialog(frame, area);
        }
        if self.ui.undo_confirm_dialog.is_some() {
            self.render_undo_confirm_dialog(frame, area);
        }
        self.finish_mouse_selection(frame);
        // Image viewer overlay — rendered last so it's on top of everything
        if let (Some(viewer), Some(picker)) = (&mut self.ui.image_viewer, &self.ui.image_picker) {
            viewer.render(frame, frame.area(), picker);
        }
        self.render_toast(frame);
    }

    fn render_toast(&mut self, frame: &mut Frame<'_>) {
        let now = Instant::now();
        let Some((message, expires_at)) = self.ui.toast.take() else {
            return;
        };

        if now >= expires_at {
            return;
        }

        let Some(message_area) = self.ui.message_content_area else {
            return;
        };

        let palette = self.palette();
        let message_width = UnicodeWidthStr::width(message.as_str()).min(30);
        let width = (message_width + 2).min(32) as u16;
        let height = 3;

        let x = message_area.right().saturating_sub(width + 1);
        let y = message_area.top().saturating_add(1);

        let rect = Rect::new(x, y, width, height);
        let block = Block::default().style(Style::default().bg(palette.panel).fg(palette.text));
        // Prepend a newline to vertically center the single-line message in the 3-row box
        let centered = format!("\n{}", message);
        let paragraph = Paragraph::new(centered.as_str())
            .style(Style::default().bg(palette.panel).fg(palette.text))
            .alignment(Alignment::Center)
            .block(block);

        frame.render_widget(Clear, rect);
        frame.render_widget(paragraph, rect);

        self.ui.toast = Some((message, expires_at));
    }

    fn render_welcome(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let card_width = self.runtime.config()
            .ui
            .welcome_width
            .min(area.width.saturating_sub(4).max(32));
        let card_height = 20u16.min(area.height.saturating_sub(2).max(10));
        let card = centered_rect(card_width, card_height, area);

        let card_inner_width = card.width.saturating_sub(7);

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
                self.ui
                    .composer
                    .preferred_height(card_inner_width, self.runtime.config().ui.max_input_lines)
                    .saturating_add(2),
            ),
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

        let prompt_title = match self.ui.pending_mode.as_ref() {
            Some(pending) if self.ui.pending_request => {
                format!(
                    "{} (current), {} (on completion)",
                    self.ui.mode.title(),
                    pending.title()
                )
            }
            _ => self.ui.mode.title().to_string(),
        };
        let prompt_placeholder = self.ui.composer.placeholder().to_string();
        self.render_input_block_with_composer(
            frame,
            sections[2],
            &prompt_title,
            &self.ui.composer,
            &prompt_placeholder,
            false,
            true,
            true,
        );

        // Model info is now displayed inside the composer as metadata
        let workspace_path = self.runtime.workspace_root().display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );

        // Always show the workspace path on the very last row.
        let workspace_area = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &display_path,
                Style::default().fg(palette.muted),
            ))),
            workspace_area,
        );

        // When there is a notice, show it on the row directly above the
        // workspace path so the two always sit together at the bottom.
        if let Some(message) = self.ui.last_notice.as_deref() {
            let notice_y = area.bottom().saturating_sub(2);
            if notice_y < workspace_area.y {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        message,
                        Style::default().fg(palette.muted),
                    ))),
                    Rect::new(area.x + 1, notice_y, area.width.saturating_sub(2), 1),
                );
            }
        }

        // Palettes should align with the composer's visual left edge (offset by 2 columns)
        let palette_area = Rect {
            x: sections[2].x + 2,
            ..sections[2]
        };
        self.render_at_mention_palette(frame, palette_area);
        self.render_snippet_palette(frame, palette_area);
        self.render_command_palette(frame, palette_area);
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
            &self.ui.composer,
            placeholder,
            mask_input,
            true,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_input_block_with_composer(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        _title: &str,
        composer: &Composer,
        placeholder: &str,
        mask_input: bool,
        register_input_area: bool,
        show_left_accent: bool,
    ) {
        let palette = self.palette();

        // Background fill for the input area — start at column 2 to align with message card content area
        let left_inset: u16 = if show_left_accent { 2 } else { 1 };
        let bg_rect = Rect {
            x: area.x + left_inset,
            y: area.y,
            width: area.width.saturating_sub(left_inset),
            height: area.height,
        };
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel)),
            bg_rect,
        );

        // Inner text area (with 2-column margin from accent bar, 1-column vertical margin)
        let inner_margin: u16 = if show_left_accent { 2 } else { 1 };
        let inner = Rect {
            x: bg_rect.x + inner_margin,
            y: area.y + 1,
            width: area.width.saturating_sub(left_inset + inner_margin + 1),
            height: area.height.saturating_sub(2),
        };

        // When showing the accent bar (main composer), reserve space for metadata at the bottom
        let metadata_height: u16 = if show_left_accent { 2 } else { 0 };
        let (text_area, metadata_area) = if show_left_accent && inner.height > metadata_height {
            let split = Layout::vertical([Constraint::Min(1), Constraint::Length(metadata_height)])
                .split(inner);
            (split[0], split[1])
        } else {
            (inner, Rect::default())
        };

        if register_input_area {
            self.ui.input_area.set(Some(text_area));
        }

        let visible_lines = text_area.height.max(1) as usize;
        let total_lines = composer.display_line_count(text_area.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        // Use stored scroll offset, clamped to valid range
        let scroll = if register_input_area {
            self.ui.input_scroll_offset.min(max_scroll) as u16
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
            let width = text_area.width as usize;
            let selection = composer.selection_range();
            let composer_spans = composer.spans();

            // Build lines with selection highlighting and inline span badges
            let visual_lines = composer.visual_lines(width);
            let mut lines = Vec::new();

            for range in visual_lines.iter() {
                let line_start = range.start;
                let line_end = range.end;

                // Collect composer spans that overlap this visual line
                let overlapping: Vec<&InlineSpan> = composer_spans
                    .iter()
                    .filter(|s| s.start < line_end && s.end > line_start)
                    .collect();

                if overlapping.is_empty() {
                    // No spans on this line — render plain (with selection if active)
                    lines.push(render_composer_line_plain(
                        &composer.text()[line_start..line_end],
                        line_start,
                        line_end,
                        selection,
                        palette,
                    ));
                } else {
                    // Build segments split at span boundaries
                    let mut segments: Vec<(usize, usize, Option<&InlineSpan>)> = Vec::new();
                    let mut pos = line_start;

                    for span in &overlapping {
                        let span_start = span.start.max(line_start);
                        let span_end = span.end.min(line_end);
                        if pos < span_start {
                            segments.push((pos, span_start, None));
                        }
                        segments.push((span_start, span_end, Some(span)));
                        pos = span_end;
                    }
                    if pos < line_end {
                        segments.push((pos, line_end, None));
                    }

                    // Render each segment
                    let mut line_spans = Vec::new();
                    for (seg_start, seg_end, opt_span) in &segments {
                        let text = &composer.text()[*seg_start..*seg_end];
                        match opt_span {
                            Some(span) => {
                                // Inline badge segment
                                let badge_style = match span.kind {
                                    InlineSpanKind::AtReference => {
                                        if selection_overlaps(*seg_start, *seg_end, selection) {
                                            Style::default()
                                                .fg(palette.accent)
                                                .bg(palette.panel_light)
                                                .add_modifier(Modifier::BOLD)
                                        } else {
                                            Style::default()
                                                .fg(palette.accent)
                                                .add_modifier(Modifier::BOLD)
                                        }
                                    }
                                    InlineSpanKind::Image => {
                                        if selection_overlaps(*seg_start, *seg_end, selection) {
                                            Style::default()
                                                .bg(palette.accent)
                                                .fg(palette.selection_fg)
                                                .add_modifier(Modifier::BOLD)
                                        } else {
                                            Style::default()
                                                .bg(palette.selection_bg)
                                                .fg(palette.selection_fg)
                                                .add_modifier(Modifier::BOLD)
                                        }
                                    }
                                };
                                line_spans.push(Span::styled(text.to_string(), badge_style));
                            }
                            None => {
                                // Plain text segment — apply selection if active
                                let plain_spans = render_plain_segments(
                                    text, *seg_start, *seg_end, selection, palette,
                                );
                                line_spans.extend(plain_spans);
                            }
                        }
                    }
                    lines.push(Line::from(line_spans));
                }
            }

            Text::from(lines)
        };

        // Left accent bar (mode-colored) — only for the main composer
        if show_left_accent {
            let accent_color = if self.ui.shell_mode {
                palette.success
            } else if let Some(pending) = self.ui.pending_mode {
                // Pending mode switch — show future mode's color immediately
                palette.border_mode_color(pending)
            } else {
                palette.border_mode_color(self.ui.mode)
            };
            for row in 0..area.height {
                frame.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        "┃",
                        Style::default().fg(accent_color).bg(palette.panel),
                    )]))
                    .style(Style::default().bg(palette.panel)),
                    Rect::new(bg_rect.x, area.y + row, 1, 1),
                );
            }

            // Render metadata row below the text content (2nd row of metadata_area, 1st row is blank)
            if metadata_area.width > 0 && metadata_area.height > 1 {
                let mut meta_spans: Vec<Span> = Vec::new();

                // Mode label (Build / Plan / Shell)
                let (mode_label, mode_style) = if self.ui.shell_mode {
                    (
                        "Shell".to_string(),
                        Style::default()
                            .fg(palette.success)
                            .add_modifier(Modifier::BOLD),
                    )
                } else if let Some(pending) = self.ui.pending_mode {
                    (
                        format!("{} → {}", self.ui.mode.title(), pending.title()),
                        Style::default()
                            .fg(palette.border_mode_color(pending))
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    (
                        self.ui.mode.title().to_string(),
                        Style::default()
                            .fg(palette.border_mode_color(self.ui.mode))
                            .add_modifier(Modifier::BOLD),
                    )
                };
                meta_spans.push(Span::styled(mode_label, mode_style));

                // · separator
                meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));

                // Model label
                let active_model = self.runtime.active_model();
                meta_spans.push(Span::styled(
                    &active_model.display_name,
                    Style::default().fg(palette.text),
                ));

                // · separator
                meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));

                // Provider
                meta_spans.push(Span::styled(
                    &active_model.provider_display_name,
                    Style::default().fg(palette.muted),
                ));

                // Thinking level (if supported)
                if self.ui.thinking_level.is_supported() {
                    meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
                    meta_spans.push(Span::styled(
                        format!("[{}]", self.ui.thinking_level.display_name()),
                        Style::default().fg(palette.accent_soft),
                    ));
                }

                let meta_paragraph = Paragraph::new(Line::from(meta_spans))
                    .style(Style::default().bg(palette.panel));
                // Render on the second row of metadata_area, aligned with text content
                let meta_rect = Rect::new(area.x + 4, metadata_area.y + 1, metadata_area.width, 1);
                frame.render_widget(meta_paragraph, meta_rect);
            }
        }

        let mut paragraph = Paragraph::new(content)
            .style(Style::default().fg(palette.text))
            .scroll((scroll, 0));

        paragraph = paragraph.wrap(Wrap { trim: false });

        frame.render_widget(paragraph, text_area);

        if text_area.width > 0 && text_area.height > 0 {
            let (cursor_line, cursor_col) = composer.cursor_position(text_area.width);
            let mut cursor_line = cursor_line.saturating_sub(scroll);
            let mut cursor_col = cursor_col;

            if composer.cursor_wraps_to_next_row(text_area.width as usize) {
                cursor_line = cursor_line.saturating_add(1);
                cursor_col = 0;
            }

            let cursor_x = text_area.x.saturating_add(cursor_col);
            let cursor_y = text_area
                .y
                .saturating_add(cursor_line.min(text_area.height.saturating_sub(1)));

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
    if highlight_indices.is_empty() {
        return vec![Span::styled(text.to_string(), normal_style)];
    }

    let mut spans = Vec::new();
    let mut hi_iter = highlight_indices.iter().copied().peekable();
    let mut current_run = String::new();
    let mut current_style = normal_style;

    macro_rules! flush_run {
        () => {
            if !current_run.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current_run),
                    current_style,
                ));
            }
        };
    }

    for (index, ch) in text.chars().enumerate() {
        let style = if hi_iter.peek().is_some_and(|next| *next == index) {
            hi_iter.next();
            highlighted_style
        } else {
            normal_style
        };

        if style != current_style {
            flush_run!();
            current_style = style;
        }
        current_run.push(ch);
    }
    flush_run!();

    spans
}

impl App {
    pub(super) fn render_prompt_footer(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let status_text = self.footer_status_text();
        let status_width = status_text.width().min(area.width as usize).max(1) as u16;
        let chunks =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(status_width)]).split(area);

        frame.render_widget(
            Paragraph::new(status_text)
                .alignment(Alignment::Right)
                .style(Style::default().fg(palette.muted)),
            chunks[1],
        );
    }

    fn footer_status_text(&mut self) -> String {
        let queued_count = self.ui.pending_prompt_queue.len();

        if self.ui.pending_request
            && self.ui.abort_confirmation_deadline
                .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return "Esc again to stop".to_string();
        }

        let token_status = {
            let token_usage = TokenUsage::new(
                self.ui.context_usage.input_tokens,
                self.ui.context_usage.output_tokens,
                self.ui.context_usage.cache_read_tokens,
                self.ui.context_usage.cache_write_tokens,
            );
            let max_context = self.runtime.active_model().context_window;
            let percent = token_usage.context_usage_pct(max_context);
            let used_k = self.ui.context_usage.input_tokens / 1000;
            let max_k = max_context as u32 / 1000;
            Some(format!("{:.1}% ({}K/{}K)", percent, used_k, max_k))
        };

        if self.ui.pending_request {
            let spinner = self.loading_spinner();

            let status = if self.ui.chat_context.parent_session_id.is_some() {
                format!("{} Thinking...", spinner)
            } else if !self.ui.running_subagent_executions.is_empty() {
                let count = self.ui.running_subagent_executions.len();
                let label = if count == 1 { "subagent" } else { "subagents" };
                format!("{} Waiting for {} {}", spinner, count, label)
            } else if !self.ui.running_tool_executions.is_empty() {
                let tool_names: Vec<_> = self.ui.running_tool_executions
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
            } else if self.ui.pending_tool_execution.is_some() {
                format!("{} Running tools", spinner)
            } else {
                match self.ui.pending_mode.as_ref() {
                    Some(pending) => {
                        format!(
                            "{} {} → {} (on completion)",
                            spinner,
                            self.ui.mode.title(),
                            pending.title()
                        )
                    }
                    None => format!("{} {}", spinner, self.ui.mode.title()),
                }
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

        if let Some(message) = self.ui.last_notice.as_deref() {
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

        if self.ui.chat_context.parent_session_id.is_some() {
            return "Subsession active · Ctrl+X then Up arrow to return".to_string();
        }

        "Ready".to_string()
    }

    pub(crate) fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        const FRAME_DURATION_MS: u128 = 100;

        let elapsed = self.ui.spinner_start.elapsed().as_millis();
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
    left_padding: usize,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| decorate_card_line(line, width, background, left_padding))
        .collect()
}

pub(crate) fn decorate_card_line(
    line: Line<'static>,
    width: usize,
    background: Color,
    left_padding: usize,
) -> Line<'static> {
    let bg_style = Style::default().bg(background);
    let mut spans = Vec::with_capacity(line.spans.len().saturating_add(2));

    // Detect if the line already has a visual prefix like "┃ " (used for thinking
    // and user message indicators). In that case, skip the extra left_padding
    // so the content text aligns with other card lines.
    let has_visual_prefix = line.spans.first().is_some_and(|s| s.content == "┃ ");

    if !has_visual_prefix {
        spans.push(Span::styled(" ".repeat(left_padding), bg_style));
    }

    for mut span in line.spans {
        if span.style.bg.is_none() {
            span.style = span.style.patch(bg_style);
        }
        spans.push(span);
    }

    let used_width = line_display_width(&Line::from(spans.clone()));
    if used_width < width {
        spans.push(Span::styled(
            " ".repeat(width.saturating_sub(used_width)),
            bg_style,
        ));
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

/// Wrap `text` into at most `max_lines` lines of `max_width` columns each.
///
/// * Newlines in the input are treated as spaces.
/// * Word boundaries are preferred for line breaks; hard-breaks are used when
///   a single word exceeds `max_width`.
/// * If the text would exceed `max_lines`, the last line is truncated with `…`.
///
/// Returns at least one (possibly empty) line.
pub(crate) fn wrap_text_lines(text: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    if max_width == 0 || max_lines == 0 {
        return vec![];
    }

    // Normalize: collapse newlines into spaces
    let normalized: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let normalized = expand_tabs(&normalized, 4);
    let trimmed = normalized.trim();

    if trimmed.is_empty() {
        return vec!["".to_string()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut remaining = trimmed;

    while !remaining.is_empty() && lines.len() < max_lines {
        let remaining_width = char_width(remaining);

        // Last allowed line — truncate if needed
        if lines.len() == max_lines - 1 {
            if remaining_width > max_width {
                lines.push(shorten_by_width(remaining, max_width));
            } else {
                lines.push(remaining.to_string());
            }
            break;
        }

        // Fits entirely on this line
        if remaining_width <= max_width {
            lines.push(remaining.to_string());
            break;
        }

        // Need to wrap. Walk characters to find the break point.
        let mut width_so_far: usize = 0;
        let mut break_pos: Option<usize> = None; // byte offset of last whitespace
        let mut hard_break: usize = 0; // byte offset where width overflows

        for (i, ch) in remaining.char_indices() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width_so_far + cw > max_width {
                hard_break = i;
                break;
            }
            width_so_far += cw;
            if ch.is_whitespace() {
                break_pos = Some(i);
            }
            hard_break = i + ch.len_utf8();
        }

        if let Some(sp) = break_pos {
            if sp > 0 {
                lines.push(remaining[..sp].to_string());
                remaining = remaining[sp..].trim_start();
            } else {
                // Leading whitespace — skip it
                remaining = remaining[sp + 1..].trim_start();
            }
        } else if hard_break > 0 && hard_break < remaining.len() {
            // No whitespace in range — hard-break
            lines.push(remaining[..hard_break].to_string());
            remaining = &remaining[hard_break..];
        } else {
            lines.push(remaining.to_string());
            break;
        }
    }

    if lines.is_empty() {
        lines.push("".to_string());
    }

    lines
}

/// Return the display width (in terminal columns) of `s`.
fn char_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Truncate `s` to fit within `max_width` columns and append `…` if truncated.
fn shorten_by_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let total = char_width(s);
    if total <= max_width {
        return s.to_string();
    }
    let ellipsis = '…';
    let ellipsis_width = UnicodeWidthChar::width(ellipsis).unwrap_or(1);
    let target = max_width.saturating_sub(ellipsis_width);
    let mut out = String::new();
    let mut w = 0;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > target {
            break;
        }
        w += cw;
        out.push(ch);
    }
    out.push(ellipsis);
    out
}

/// Check if a byte range `[start, end)` overlaps with an optional selection range.
fn selection_overlaps(start: usize, end: usize, selection: Option<(usize, usize)>) -> bool {
    selection
        .map(|(sel_start, sel_end)| start < sel_end && end > sel_start)
        .unwrap_or(false)
}

/// Render a plain composer line (no inline spans) with optional selection highlighting.
fn render_composer_line_plain(
    text: &str,
    line_start: usize,
    line_end: usize,
    selection: Option<(usize, usize)>,
    palette: ThemePalette,
) -> Line<'static> {
    let spans = render_plain_segments(text, line_start, line_end, selection, palette);
    Line::from(spans)
}

/// Render plain text segments with optional selection highlighting.
/// Splits text into before-selection, selection, and after-selection spans.
fn render_plain_segments(
    text: &str,
    seg_start: usize,
    seg_end: usize,
    selection: Option<(usize, usize)>,
    palette: ThemePalette,
) -> Vec<Span<'static>> {
    let mut result = Vec::new();

    if let Some((sel_start, sel_end)) = selection {
        let sel_in_seg_start = sel_start.max(seg_start);
        let sel_in_seg_end = sel_end.min(seg_end);

        if sel_in_seg_start < sel_in_seg_end {
            // Before selection
            if sel_in_seg_start > seg_start {
                let before = text[..sel_in_seg_start - seg_start].to_string();
                result.push(Span::styled(before, Style::default().fg(palette.text)));
            }
            // Selection
            let selected =
                text[sel_in_seg_start - seg_start..sel_in_seg_end - seg_start].to_string();
            result.push(Span::styled(
                selected,
                Style::default().fg(palette.text).bg(palette.accent),
            ));
            // After selection
            if sel_in_seg_end < seg_end {
                let after = text[sel_in_seg_end - seg_start..].to_string();
                result.push(Span::styled(after, Style::default().fg(palette.text)));
            }
            return result;
        }
    }

    // No selection overlap
    result.push(Span::styled(
        text.to_string(),
        Style::default().fg(palette.text),
    ));
    result
}

/// Expand tab characters to spaces using configurable tab stops.
///
/// `unicode-width` measures `\t` as 0 columns, but terminals render it as
/// multiple spaces. Expanding tabs at parse time prevents width mismatches
/// in word wrapping, card padding, and table column alignment.
pub(crate) fn expand_tabs(text: &str, tab_width: usize) -> String {
    if !text.contains('\t') {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len() + tab_width);
    let mut col = 0usize;
    for ch in text.chars() {
        if ch == '\t' {
            let spaces = tab_width - (col % tab_width);
            result.extend(std::iter::repeat_n(' ', spaces));
            col += spaces;
        } else {
            result.push(ch);
            col += 1;
        }
    }
    result
}
