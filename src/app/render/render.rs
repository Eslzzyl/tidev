use crate::theme::ThemePalette;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::time::Instant;
use unicode_width::UnicodeWidthStr;

use super::{App, Screen};

impl App {
    pub(crate) fn palette(&self) -> ThemePalette {
        self.theme.palette()
    }

    pub(crate) fn render(&mut self, frame: &mut Frame<'_>) {
        self.message_content_area = None;
        self.sidebar_area = None;
        self.input_area.set(None);
        if self.at_mention.visible {
            self.refresh_at_mention_state();
        }
        match self.screen {
            Screen::Welcome => self.render_welcome(frame),
            Screen::Chat => self.render_chat(frame),
        }
        let area = frame.area();
        self.render_connect_dialog(frame, area);
        if let Some(panel) = &self.theme_panel {
            self.render_theme_panel(frame, area, panel);
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
        if let Some(panel) = &self.message_panel {
            self.render_message_panel(frame, area, panel);
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
        if let Some(dialog) = &self.rename_dialog {
            self.render_rename_session_dialog(frame, area, dialog);
        }
        if let Some(dialog) = &self.permission_dialog {
            self.render_permission_dialog(frame, area, dialog);
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

        let prompt_title = self.mode.title().to_string();
        let prompt_placeholder = self.composer.placeholder().to_string();
        self.render_input_block(
            frame,
            sections[2],
            &prompt_title,
            &prompt_placeholder,
            false,
        );

        let model_line = Line::from(vec![Span::styled(
            self.active_model.label(),
            Style::default().fg(palette.accent),
        )]);
        frame.render_widget(
            Paragraph::new(model_line).style(Style::default().fg(palette.text)),
            sections[3],
        );

        self.render_at_mention_palette(frame, sections[2]);
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
        let palette = self.palette();
        let border_style = if self.pending_request {
            Style::default().fg(palette.warning)
        } else {
            Style::default().fg(palette.border_mode_color(self.mode))
        };

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        self.input_area.set(Some(inner));

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
            let width = inner.width as usize;
            let mut lines = Vec::new();
            for range in self.composer.visual_lines(width) {
                lines.push(Line::from(&self.composer.text()[range]));
            }
            Text::from(lines)
        };

        let visible_lines = inner.height.max(1) as usize;
        let total_lines = self.composer.display_line_count(inner.width as usize);
        let scroll = total_lines.saturating_sub(visible_lines) as u16;

        let mut paragraph = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(title),
            )
            .style(Style::default().fg(palette.text))
            .scroll((scroll, 0));

        if self.composer.is_empty() || mask_input {
            paragraph = paragraph.wrap(Wrap { trim: false });
        }

        frame.render_widget(paragraph, area);

        if inner.width > 0 && inner.height > 0 {
            let (cursor_line, cursor_col) = self.composer.cursor_position(inner.width);
            let mut cursor_line = cursor_line.saturating_sub(scroll);
            let mut cursor_col = cursor_col;

            if self.composer.cursor_wraps_to_next_row(inner.width as usize) {
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

pub(super) fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(8);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    Rect::new(x, y, width, height)
}

pub(super) fn shorten(value: &str, max_chars: usize) -> String {
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
    pub(super) fn render_prompt_footer(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let status_text = self.footer_status_text();
        let status_width = status_text.width().min(area.width as usize).max(1) as u16;
        let chunks =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(status_width)]).split(area);

        let model_line = Line::from(vec![Span::styled(
            self.active_model.label(),
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

        let Some((attempt, max_attempts, reason, retry_after_secs)) = self.retrying_hint.as_ref()
        else {
            // Clear any existing content
            frame.render_widget(
                Paragraph::new("").style(Style::default().fg(palette.text)),
                area,
            );
            return;
        };

        let retry_after_str = retry_after_secs
            .map(|s| format!("Retrying in {s}s"))
            .unwrap_or_else(|| "Retrying...".to_string());

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
            let max_context = self.active_model.context_window as u32;
            let percent = usage.input_tokens as f64 / max_context as f64 * 100.0;
            let used_k = usage.input_tokens / 1000;
            let max_k = max_context / 1000;
            format!("{:.1}% ({}K/{}K)", percent, used_k, max_k)
        });

        if self.pending_request {
            let spinner = self.loading_spinner();

            let status = if self.conversation.parent_session_id.is_some() {
                format!("{} Thinking...", spinner)
            } else if !self.running_subagent_executions.is_empty() {
                format!(
                    "{} Waiting for {} subagent(s)",
                    spinner,
                    self.running_subagent_executions.len()
                )
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

pub(super) fn line_with_style(text: &str, fg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        text.to_string(),
        Style::default().fg(fg),
    )])
}

pub(super) fn line_with_style_right_aligned(text: &str, width: usize, fg: Color) -> Line<'static> {
    let text_width = UnicodeWidthStr::width(text);
    let padding = width.saturating_sub(text_width);
    let padded_text = format!("{}{}", " ".repeat(padding), text);
    Line::from(vec![Span::styled(padded_text, Style::default().fg(fg))])
}

pub(super) fn line_with_prefix(
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

pub(super) fn decorate_card_lines(
    lines: Vec<Line<'static>>,
    width: usize,
    background: Color,
) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| decorate_card_line(line, width, background))
        .collect()
}

pub(super) fn decorate_card_line(
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

pub(super) fn shorten_single_line(value: &str, max_chars: usize) -> String {
    let single_line = value.replace('\n', " ").replace('\r', "");
    shorten(&single_line, max_chars)
}
