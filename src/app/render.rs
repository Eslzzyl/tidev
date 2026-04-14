use crate::{prompts::SessionMode, theme::ThemePalette};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Position, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use super::{App, Screen};

impl App {
    pub(super) fn palette(&self) -> ThemePalette {
        self.theme.palette()
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
        if let Some(panel) = &self.mcp_panel {
            self.render_mcp_panel(frame, area, panel);
        }
        if let Some(panel) = &self.model_panel {
            self.render_model_panel(frame, area, panel);
        }
        if let Some(panel) = &self.session_panel {
            self.render_session_panel(frame, area, panel);
        }
        if let Some(dialog) = &self.permission_dialog {
            self.render_permission_dialog(frame, area, dialog);
        }
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
        let card_height = 13u16.min(area.height.saturating_sub(2).max(10));
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

        let prompt_title = format!("{} prompt", self.mode.title());
        self.render_input_block(
            frame,
            sections[2],
            &prompt_title,
            self.composer.placeholder(),
            false,
        );

        let hint = Paragraph::new(
            "Enter to send · /session to switch sessions · Shift+Enter/Ctrl+J newline",
        )
        .alignment(Alignment::Center)
        .style(Style::default().fg(palette.accent_soft));
        frame.render_widget(hint, sections[3]);

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

    pub(crate) fn help_message(&self) -> String {
        let mut lines = vec![
            "Commands:",
            "/help - show this message",
            "/connect - open the provider picker",
            "/mcp - open the MCP panel",
            "/mcp add - create a new MCP server",
            "/mcp edit <server-name> - edit an MCP server",
            "/mcp remove <server-name> - remove an MCP server",
            "/model - open the model panel",
            "/model <query> - prefilter the model panel",
            "/session - open the session panel",
            "/session <query> - prefilter the session panel",
            "/theme [dark|light|nord|one-dark|catppuccin|solarized|orng|github|material] - switch theme",
            "/clear - start a fresh session",
            "/undo - revert the previous user message",
            "/redo - move one step forward in the undo history",
            "/exit - exit TiDev",
            "",
            "Keys:",
            "Enter - send prompt or execute the highlighted slash command",
            "Shift+Enter / Ctrl+J - insert newline",
            "PageUp / PageDown / mouse wheel - scroll conversation",
            "Tab - switch mode (when no command is being entered)",
            "Up/Down - move through command suggestions",
                "Ctrl+V - paste clipboard text or image",
            "Ctrl+P / Ctrl+N - navigate input history",
            "Ctrl+C - exit",
            "Permission prompt - Y allow · N deny · R allow and remember · X deny and remember",
            "Connect picker - type to filter providers, Enter to select, Esc to cancel",
            "MCP panel - Enter connect/disconnect · a add · e edit · d remove · R refresh · Esc close",
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

impl App {
    pub(super) fn render_prompt_footer(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let status_text = self.footer_status_text();
        let status_width = status_text.width().min(area.width as usize).max(1) as u16;
        let chunks =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(status_width)]).split(area);

        let model_line = Line::from(vec![
            Span::styled("model ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                self.active_model.label(),
                Style::default().fg(palette.accent),
            ),
        ]);

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
        if self.pending_request
            && self
                .abort_confirmation_deadline
                .is_some_and(|deadline| deadline > std::time::Instant::now())
        {
            return "Esc again to stop".to_string();
        }

        if self.pending_request {
            let spinner = self.loading_spinner();

            if let Some(running_tool_execution) = self.running_tool_execution.as_ref() {
                let tool_name = running_tool_execution.tool_call.name.clone();
                return format!("{} Running {}", spinner, tool_name);
            }

            if self.pending_tool_execution.is_some() {
                return format!("{} Running tools", spinner);
            }

            return format!("{} {}", spinner, self.mode.title());
        }

        if let Some(message) = self.last_notice.as_deref() {
            return message.to_string();
        }

        "Ready".to_string()
    }

    fn loading_spinner(&mut self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        let frame = FRAMES[self.loading_frame % FRAMES.len()];
        self.loading_frame = self.loading_frame.wrapping_add(1);
        frame
    }
}

pub(super) fn line_with_style(text: &str, fg: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        text.to_string(),
        Style::default().fg(fg),
    )])
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
        span.style = span.style.patch(bg_style);
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
