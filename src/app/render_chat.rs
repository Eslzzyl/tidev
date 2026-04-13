use crate::{
    markdown_render::{WrapOptions, adaptive_wrap_lines, render_markdown_text_with_width_and_cwd},
    session::{Message, MessageRole, ToolCall},
    tooling::canonical_tool_name,
};
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use super::{App, render::*};

impl App {
    pub(super) fn render_chat(&mut self, frame: &mut Frame<'_>) {
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
            Constraint::Length(composer_height),
            Constraint::Length(1),
        ])
        .split(main_area);

        self.render_messages(frame, layout[0]);
        let prompt_title = format!("{} prompt", self.mode.title());
        self.render_input_block(
            frame,
            layout[1],
            &prompt_title,
            self.composer.placeholder(),
            false,
        );
        self.render_prompt_footer(frame, layout[2]);
        self.render_command_palette(frame, layout[1]);
    }

    pub(super) fn render_messages(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(format!(
                "Conversation · {}{}",
                shorten(&self.conversation.title, 32),
                if !self.message_follow_tail {
                    " · history"
                } else {
                    ""
                }
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
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(inner);
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
        if self.conversation.is_reverted() {
            lines.push(Line::from(vec![Span::styled(
                "Undo: active",
                Style::default().fg(palette.warning),
            )]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "cwd",
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
        lines.push(Line::from("/undo - revert the previous user message"));
        lines.push(Line::from("/redo - move one step forward in the undo history"));
        lines.push(Line::from("/model - open the model panel"));
        lines.push(Line::from("/model <query> - prefilter the model panel"));
        lines.push(Line::from("/session - open the session panel"));
        lines.push(Line::from("/session <query> - prefilter the session panel"));
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

    fn messages_text(&self, content_width: Option<usize>) -> (Text<'static>, usize) {
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);

        if self.conversation.visible_messages().is_empty() {
            let lines = decorate_card_lines(
                vec![
                    line_with_style("No messages yet.", palette.muted),
                    line_with_style("Start with a prompt in the input box below.", palette.muted),
                ],
                width,
                palette.panel,
            );
            let total_lines = lines.len().max(1);
            return (Text::from(lines), total_lines);
        }

        let mut lines = Vec::new();

        for message in self.conversation.visible_messages() {
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
                self.render_text_body_lines(
                    &message.content,
                    body_width,
                    Some(self.workspace_root.as_path()),
                ),
            )],
            MessageRole::Assistant => {
                let mut cards = Vec::new();
                let body_lines = self.render_assistant_body_lines(message, body_width);
                if !body_lines.is_empty() {
                    cards.push((palette.panel, body_lines));
                }

                for tool_call in &message.tool_calls {
                    cards.push((
                        palette.panel_alt,
                        self.render_tool_call_lines(tool_call, body_width),
                    ));
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
                self.render_text_body_lines(
                    &message.content,
                    body_width,
                    Some(self.workspace_root.as_path()),
                ),
            )],
            MessageRole::Error => vec![(
                palette.panel_alt,
                self.render_error_body_lines(message, body_width),
            )],
        }
    }

    fn render_assistant_body_lines(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<Line<'static>> {
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
                        WrapOptions::new(width),
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
            let rendered = render_markdown_text_with_width_and_cwd(
                &message.content,
                Some(body_width),
                Some(self.workspace_root.as_path()),
            );
            lines.extend(rendered.lines);
        }

        if lines.is_empty() && message.reasoning.trim().is_empty() && message.tool_calls.is_empty()
        {
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
            let rendered = render_markdown_text_with_width_and_cwd(text, Some(body_width), cwd);
            lines.extend(rendered.lines);
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

    fn render_tool_call_lines(
        &self,
        tool_call: &ToolCall,
        body_width: usize,
    ) -> Vec<Line<'static>> {
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

        if matches!(
            canonical_name,
            "read" | "write" | "edit" | "list" | "todowrite"
        ) {
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
        let fg = if is_error {
            palette.error
        } else {
            palette.text
        };

        for line in output.lines().take(max_lines) {
            lines.push(line_with_prefix(
                prefix,
                &shorten_single_line(line, body_width.saturating_sub(2)),
                Style::default().fg(if is_error {
                    palette.error
                } else {
                    palette.accent_soft
                }),
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

        let paragraph =
            Paragraph::new(Text::from(lines)).style(Style::default().bg(palette.background));
        frame.render_widget(paragraph, area);
    }
}

fn tool_output_is_error(output: &str) -> bool {
    let first_line = output.lines().next().unwrap_or("").trim_start();

    first_line.starts_with("Tool failed:")
        || first_line.starts_with("Tool '")
        || first_line.starts_with("Request failed:")
        || (first_line.starts_with("[exit ") && !first_line.starts_with("[exit 0]"))
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
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();
    let mut fields = Vec::new();

    let string_field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(serde_json::Value::as_str)
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
                .and_then(serde_json::Value::as_array)
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
