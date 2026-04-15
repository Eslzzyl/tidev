use crate::{
    markdown_render::render_markdown_text_with_width_and_cwd,
    session::{Message, MessageAttachment, MessageRole, ToolCall},
    theme::ThemePalette,
    tooling::canonical_tool_name,
};
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use super::diff_render::render_unified_diff_text;
use super::permission::RunningSubagentExecution;
use super::{
    App, MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
    MessageRenderCacheValue, render::*,
};

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
            self.sidebar_area = Some(split[1]);
            self.render_sidebar(frame, split[1]);
            split[0]
        } else {
            area
        };

        let composer_height = self
            .composer
            .preferred_height(
                main_area.width.saturating_sub(4),
                self.config.ui.max_input_lines,
            )
            .min(main_area.height.saturating_sub(3).max(3));

        if let Some(dialog) = self.question_dialog.clone() {
            let question_height = dialog
                .prompt_height(composer_height)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(question_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_question_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            self.render_retrying_hint(frame, layout[3]);
            return;
        }

        let layout = Layout::vertical([
            Constraint::Min(6),
            Constraint::Length(composer_height),
            Constraint::Length(1),
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
        self.render_at_mention_palette(frame, layout[1]);
        self.render_prompt_footer(frame, layout[2]);
        self.render_retrying_hint(frame, layout[3]);
        self.render_command_palette(frame, layout[1]);
    }

    pub(super) fn render_messages(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut title = format!("Conversation · {}", shorten(&self.conversation.title, 32),);
        if !self.message_follow_tail {
            title.push_str(" · history");
        }
        if self.conversation.parent_session_id.is_some() {
            title.push_str(" · SUBSESSION");
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(title);
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
        self.message_content_area = Some(content_area);
        let content_width = content_area.width.max(1) as usize;
        let (mut text, mut total_lines) = self.messages_text(Some(content_width));

        // Add tool running state
        if let Some(running) = &self.running_tool_execution {
            let canonical_name =
                canonical_tool_name(&running.tool_call.name).unwrap_or(&running.tool_call.name);
            let action = match canonical_name {
                "edit" | "write" => "Editing",
                "read" => "Reading",
                "bash" => "Running",
                "grep" | "glob" | "list" => "Searching",
                _ => "Executing",
            };

            let fields =
                summarize_tool_arguments(&running.tool_call.name, &running.tool_call.arguments);
            let target = fields
                .iter()
                .find(|(k, _)| k == "path" || k == "filePath" || k == "command")
                .map(|(_, v)| v.as_str())
                .unwrap_or(&running.tool_call.name);

            let line = Line::from(vec![
                Span::styled(
                    format!("{} ", action),
                    Style::default().fg(palette.accent_soft),
                ),
                Span::styled(
                    shorten(target, 64),
                    Style::default()
                        .fg(palette.text)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("...", Style::default().fg(palette.muted)),
            ]);

            let card_lines = decorate_card_lines(
                vec![Line::from(""), line, Line::from("")],
                content_width,
                palette.panel_alt,
            );
            text.lines.extend(card_lines);
            total_lines += 3;
        }

        if self.conversation.parent_session_id.is_none() {
            for running_subagent in &self.running_subagent_executions {
                let card_lines =
                    self.render_running_subagent_lines(running_subagent, content_width);
                if card_lines.is_empty() {
                    continue;
                }

                let decorated_lines = decorate_card_lines(card_lines, content_width, palette.panel);
                total_lines += decorated_lines.len();
                text.lines.extend(decorated_lines);
            }
        }

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

        // Model info
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "{} / {}",
                self.active_model.provider_id, self.active_model.model_id
            ),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            if self.active_model.api_key_present() {
                "✓ API key present"
            } else {
                "✗ API key missing"
            },
            if self.active_model.api_key_present() {
                Style::default().fg(palette.success)
            } else {
                Style::default().fg(palette.error)
            },
        )]));

        lines.push(Line::from(""));

        // Working directory
        lines.push(Line::from(shorten(
            &self.workspace_root.display().to_string(),
            32,
        )));

        // Undo state (only when active)
        if self.conversation.is_reverted() {
            lines.push(Line::from(vec![Span::styled(
                "⚠ Undo active",
                Style::default().fg(palette.warning),
            )]));
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
        let started_at = Instant::now();
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);
        let messages = self.conversation.visible_messages();

        let mut lines = Vec::new();
        if self.conversation.parent_session_id.is_some() {
            lines.push(line_with_style(
                "SUBSESSION active — viewing a child session.",
                palette.accent_soft,
            ));
            lines.push(line_with_style(
                "Press Ctrl+X then Up arrow to return to the parent session.",
                palette.muted,
            ));
            lines.push(Line::from(""));
        }

        if messages.is_empty() {
            lines.extend(decorate_card_lines(
                vec![
                    line_with_style("No messages yet.", palette.muted),
                    line_with_style("Start with a prompt in the input box below.", palette.muted),
                ],
                width,
                palette.panel,
            ));
            let total_lines = lines.len().max(1);
            return (Text::from(lines), total_lines);
        }

        let mut i = 0;
        while i < messages.len() {
            let message = &messages[i];

            // Handle Assistant messages and their subsequent Tool messages together
            if matches!(message.role, MessageRole::Assistant) {
                let mut assistant_cards = self.cached_render_message_cards(message, body_width);

                // Peek ahead for tool results that belong to this assistant's tool calls
                let mut next_i = i + 1;
                while next_i < messages.len() && matches!(messages[next_i].role, MessageRole::Tool)
                {
                    let tool_msg = &messages[next_i];
                    // Render tool result
                    let tool_lines = self.cached_render_tool_result_lines(tool_msg, body_width);
                    if !tool_lines.is_empty() {
                        let mut lines_with_margin = Vec::new();
                        lines_with_margin.push(Line::from(""));
                        lines_with_margin.extend(tool_lines);
                        lines_with_margin.push(Line::from(""));
                        // Tool results use panel_light
                        assistant_cards.push((palette.panel_light, lines_with_margin));
                    }
                    next_i += 1;
                }

                for (card_bg, card_lines) in assistant_cards {
                    if card_lines.is_empty() {
                        continue;
                    }
                    lines.extend(decorate_card_lines(card_lines, width, card_bg));
                    lines.push(Line::from(""));
                }

                i = next_i;
                continue;
            }

            // Fallback for other message types (User, System, etc.)
            // Note: Standalone Tool messages (if any) will still be rendered here
            for (card_bg, card_lines) in self.cached_render_message_cards(message, body_width) {
                if card_lines.is_empty() {
                    continue;
                }

                lines.extend(decorate_card_lines(card_lines, width, card_bg));
                lines.push(Line::from(""));
            }
            i += 1;
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
        let elapsed = started_at.elapsed();
        if elapsed > Duration::from_millis(12) {
            let (hits, misses, entries) = self.message_render_cache_stats();
            crate::log_debug!(
                "messages_text slow: messages={}, width={}, took={:?}, cache_hits={}, cache_misses={}, cache_entries={}",
                messages.len(),
                width,
                elapsed,
                hits,
                misses,
                entries
            );
        }
        (Text::from(lines), total_lines)
    }

    fn cached_render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        if message.streaming && matches!(message.role, MessageRole::Assistant) {
            self.record_message_render_cache_miss();
            return self.render_message_cards(message, body_width);
        }

        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id,
            width: body_width,
            kind: MessageRenderCacheKind::Cards,
        };
        let fingerprint = message_render_fingerprint(message);
        let tick = self.next_message_render_cache_tick();

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key)
                && entry.fingerprint == fingerprint
                && let MessageRenderCacheValue::Cards(cards) = &entry.value
            {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                return cards.clone();
            }
        }

        self.record_message_render_cache_miss();
        let cards = self.render_message_cards(message, body_width);

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    fingerprint,
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: tick,
                },
            );
        }

        self.prune_message_render_cache_if_needed();
        cards
    }

    fn cached_render_tool_result_lines(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id,
            width: body_width,
            kind: MessageRenderCacheKind::ToolResultLines,
        };
        let fingerprint = message_render_fingerprint(message);
        let tick = self.next_message_render_cache_tick();

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key)
                && entry.fingerprint == fingerprint
                && let MessageRenderCacheValue::ToolResultLines(lines) = &entry.value
            {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                return lines.clone();
            }
        }

        self.record_message_render_cache_miss();
        let lines = self.render_tool_result_lines(message, body_width);

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    fingerprint,
                    value: MessageRenderCacheValue::ToolResultLines(lines.clone()),
                    last_used_tick: tick,
                },
            );
        }

        self.prune_message_render_cache_if_needed();
        lines
    }

    fn render_message_cards(
        &self,
        message: &Message,
        body_width: usize,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        let palette = self.palette();

        match message.role {
            MessageRole::User => vec![(palette.panel_alt, {
                let mut content_lines = self.render_text_body_lines(
                    &message.content,
                    body_width.saturating_sub(2),
                    Some(self.workspace_root.as_path()),
                );

                for attachment in &message.attachments {
                    content_lines.push(line_with_style(&attachment.summary(), palette.accent_soft));
                }

                let mut lines = Vec::new();
                lines.push(Line::from(""));
                for line in content_lines {
                    let mut spans = vec![Span::styled(
                        "┃ ",
                        Style::default()
                            .fg(palette.accent)
                            .add_modifier(Modifier::BOLD),
                    )];
                    spans.extend(line.spans);
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(""));

                lines
            })],
            MessageRole::Assistant => {
                let mut cards = Vec::new();

                let body_lines = self.render_assistant_body_lines(message, body_width);
                if !body_lines.is_empty() {
                    let mut lines_with_margin = Vec::new();
                    lines_with_margin.push(Line::from(""));
                    lines_with_margin.extend(body_lines);
                    lines_with_margin.push(Line::from(""));
                    cards.push((palette.background, lines_with_margin));
                }

                for tool_call in &message.tool_calls {
                    let call_lines = self.render_tool_call_lines(tool_call, body_width);
                    if !call_lines.is_empty() {
                        let mut lines_with_margin = Vec::new();
                        lines_with_margin.push(Line::from(""));
                        lines_with_margin.extend(call_lines);
                        lines_with_margin.push(Line::from(""));
                        cards.push((palette.panel_light, lines_with_margin));
                    }
                }

                cards
            }
            MessageRole::Tool => {
                // Return nothing here; handled by the loop in messages_text
                Vec::new()
            }
            MessageRole::System => {
                let content_lines = self.render_text_body_lines(
                    &message.content,
                    body_width,
                    Some(self.workspace_root.as_path()),
                );
                let mut lines = Vec::new();
                lines.push(Line::from(""));
                lines.extend(content_lines);
                lines.push(Line::from(""));
                vec![(palette.background, lines)]
            }
            MessageRole::Error => {
                let error_lines = self.render_error_body_lines(message, body_width);
                let mut lines = Vec::new();
                lines.push(Line::from(""));
                lines.extend(error_lines);
                lines.push(Line::from(""));
                vec![(palette.panel_light, lines)]
            }
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
            for line in message.content.lines() {
                lines.push(Line::from(line.to_string()));
            }
        } else if !message.content.is_empty() {
            if let Some(diff_lines) =
                render_unified_diff_text(&message.content, body_width, self.palette())
            {
                lines.extend(diff_lines);
            } else {
                let rendered = render_markdown_text_with_width_and_cwd(
                    &message.content,
                    Some(body_width),
                    Some(self.workspace_root.as_path()),
                );
                lines.extend(rendered.lines);
            }
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
        let palette = self.palette();
        let canonical_name = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);

        // Extraction for special rendering (e.g., bash command)
        let fields = summarize_tool_arguments(&tool_call.name, &tool_call.arguments);
        let get_field = |name: &str| {
            fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };

        if canonical_name == "bash" {
            if let Some(cmd) = get_field("command") {
                let mut lines = Vec::new();
                lines.push(Line::from(vec![
                    Span::styled("Run ", Style::default().fg(palette.accent_soft)),
                    Span::styled(
                        "shell command",
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));

                // Opencode style: command in its own block/lines
                for line in cmd.lines() {
                    lines.push(line_with_style(&format!("  {}", line), palette.text));
                }
                return lines;
            }
        }

        let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);

        vec![Line::from(vec![Span::styled(
            summary,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )])]
    }

    fn render_reasoning_lines(&self, reasoning: &str, body_width: usize) -> Vec<Line<'static>> {
        render_reasoning_markdown_lines(
            reasoning,
            body_width,
            Some(self.workspace_root.as_path()),
            self.palette(),
        )
    }

    fn render_tool_result_lines(&self, message: &Message, body_width: usize) -> Vec<Line<'static>> {
        let palette = self.palette();
        let tool_name = message.tool_name.as_deref().unwrap_or(message.role.label());
        let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);

        let mut header_lines = Vec::new();
        let output = message.content.trim_end();
        let attachment_lines = message
            .attachments
            .iter()
            .map(MessageAttachment::summary)
            .collect::<Vec<_>>();

        if canonical_name == "task" {
            let summary = if output.is_empty() {
                "Subagent finished".to_string()
            } else {
                let first_line = output
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .unwrap_or("Subagent finished");
                shorten_single_line(first_line, body_width.saturating_sub(2))
            };

            let mut lines = vec![
                line_with_style("Subagent complete", palette.accent_soft),
                line_with_prefix(
                    "↳",
                    &summary,
                    Style::default().fg(palette.accent_soft),
                    Style::default().fg(palette.text),
                ),
                line_with_style(
                    "Open the child session to inspect the full transcript.",
                    palette.muted,
                ),
            ];

            if !attachment_lines.is_empty() {
                lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
            }

            return lines;
        }

        if matches!(canonical_name, "grep" | "glob") {
            let count = if output.is_empty() {
                0
            } else {
                output.lines().count()
            };
            let status_text = if tool_output_is_error(output) {
                format!("Search failed with {} output lines", count)
            } else {
                format!("Found {} matches", count)
            };
            header_lines.push(line_with_style(&status_text, palette.accent_soft));
        } else if canonical_name == "list" {
            let count = if output.trim() == "(empty)" {
                0
            } else {
                output
                    .lines()
                    .skip(1)
                    .filter(|line| !line.trim().is_empty())
                    .count()
            };
            header_lines.push(line_with_style(
                &format!("Listed {} items", count),
                palette.accent_soft,
            ));
        }

        if output.is_empty() && attachment_lines.is_empty() {
            return header_lines;
        }

        let mut lines = header_lines;

        if let Some(diff_lines) = render_unified_diff_text(output, body_width, palette) {
            lines.extend(diff_lines);
            lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
            return lines;
        }

        if matches!(canonical_name, "write" | "edit") {
            if tool_output_is_error(output) {
                let error_lines = self.render_output_preview_lines(output, body_width, true);
                lines.extend(error_lines);
                lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
                return lines;
            }

            let out_lines = self.render_output_preview_lines(output, body_width, false);
            lines.extend(out_lines);
            lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
            return lines;
        }

        if matches!(canonical_name, "read" | "list" | "todowrite") {
            if tool_output_is_error(output) {
                let error_lines = self.render_output_preview_lines(output, body_width, true);
                lines.extend(error_lines);
                lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
                return lines;
            }

            lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
            return lines;
        }

        let preview_lines =
            self.render_output_preview_lines(output, body_width, tool_output_is_error(output));
        lines.extend(preview_lines);
        lines.extend(self.render_attachment_preview_lines(&attachment_lines, body_width));
        lines
    }

    fn render_running_subagent_lines(
        &self,
        execution: &RunningSubagentExecution,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let task_summary = summarize_tool_call(
            &execution.tool_call.name,
            &execution.tool_call.arguments,
            body_width,
        );
        let child_session_label = self
            .store
            .load_session_record(execution.child_session_id)
            .ok()
            .flatten()
            .map(|record| {
                format!(
                    "{} · {}",
                    shorten(&record.title, 44),
                    execution.child_session_id.simple()
                )
            })
            .unwrap_or_else(|| execution.child_session_id.simple().to_string());

        let mut lines = vec![line_with_style("Subagent running", palette.accent_soft)];
        lines.push(line_with_prefix(
            "↳",
            &task_summary,
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));
        lines.push(line_with_prefix(
            "↳",
            &execution.status_text,
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));

        if let Some(tool_call) = &execution.current_tool_call {
            let current_tool =
                summarize_tool_call(&tool_call.name, &tool_call.arguments, body_width);
            lines.push(line_with_prefix(
                "↳",
                &format!("Tool: {current_tool}"),
                Style::default().fg(palette.accent_soft),
                Style::default().fg(palette.text),
            ));
        }

        lines.push(line_with_prefix(
            "↳",
            &format!("Session {child_session_label}"),
            Style::default().fg(palette.accent_soft),
            Style::default().fg(palette.text),
        ));
        lines.push(line_with_style(
            "Ctrl+X then arrows to inspect the child session.",
            palette.muted,
        ));

        lines
    }

    fn render_attachment_preview_lines(
        &self,
        attachments: &[String],
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        for attachment in attachments {
            lines.push(line_with_prefix(
                "↳",
                &shorten_single_line(attachment, body_width.saturating_sub(2)),
                Style::default().fg(palette.accent_soft),
                Style::default().fg(palette.text),
            ));
        }

        lines
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

fn render_reasoning_markdown_lines(
    reasoning: &str,
    body_width: usize,
    cwd: Option<&std::path::Path>,
    palette: ThemePalette,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let label_style = Style::default()
        .fg(palette.muted)
        .add_modifier(Modifier::BOLD);
    let body_style = Style::default().fg(palette.muted);

    lines.push(Line::from(vec![
        Span::styled("┃ ", label_style),
        Span::styled("Thinking:", body_style),
    ]));

    if reasoning.trim().is_empty() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", label_style),
            Span::styled(String::new(), body_style),
        ]));
        return lines;
    }

    let content_width = body_width.saturating_sub(2).max(1);
    let rendered = render_markdown_text_with_width_and_cwd(reasoning, Some(content_width), cwd);

    if rendered.lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("┃ ", label_style),
            Span::styled(String::new(), body_style),
        ]));
        return lines;
    }

    for line in rendered.lines {
        let mut spans = Vec::with_capacity(line.spans.len().saturating_add(1));
        spans.push(Span::styled("┃ ", label_style));
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }

    lines
}

fn message_render_fingerprint(message: &Message) -> u64 {
    let mut hasher = DefaultHasher::new();

    message.id.hash(&mut hasher);
    message.role.label().hash(&mut hasher);
    message.content.hash(&mut hasher);
    message.reasoning.hash(&mut hasher);
    message.tool_call_id.hash(&mut hasher);
    message.tool_name.hash(&mut hasher);
    message.streaming.hash(&mut hasher);

    for attachment in &message.attachments {
        match attachment {
            MessageAttachment::FileReference { path, content } => {
                1u8.hash(&mut hasher);
                path.hash(&mut hasher);
                content.hash(&mut hasher);
            }
            MessageAttachment::DirectoryReference { path, tree } => {
                2u8.hash(&mut hasher);
                path.hash(&mut hasher);
                tree.hash(&mut hasher);
            }
            MessageAttachment::Image {
                filename,
                mime,
                data_url,
            } => {
                3u8.hash(&mut hasher);
                filename.hash(&mut hasher);
                mime.hash(&mut hasher);
                data_url.hash(&mut hasher);
            }
        }
    }

    for tool_call in &message.tool_calls {
        tool_call.id.hash(&mut hasher);
        tool_call.name.hash(&mut hasher);
        tool_call.arguments.hash(&mut hasher);
    }

    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::render_reasoning_markdown_lines;
    use crate::session::{Message, MessageRole};
    use crate::theme::ThemePalette;
    use ratatui::style::Style;
    use ratatui::text::Line;

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    #[test]
    fn reasoning_lines_render_markdown_code_blocks() {
        let lines = render_reasoning_markdown_lines(
            "```rust\nfn main() { println!(\"hi\"); }\n```\n",
            80,
            None,
            ThemePalette::dark(),
        );

        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
        assert_eq!(line_text(&lines[1]), "┃ fn main() { println!(\"hi\"); }");
        assert!(
            lines[1].spans.len() > 2,
            "expected highlighted spans in code line"
        );
        assert!(
            lines[1]
                .spans
                .iter()
                .skip(1)
                .any(|span| span.style != Style::default()),
            "expected syntax highlighting styles on code spans"
        );
    }

    #[test]
    fn reasoning_lines_preserve_empty_state() {
        let lines = render_reasoning_markdown_lines("", 80, None, ThemePalette::dark());

        assert_eq!(lines.len(), 2);
        assert_eq!(line_text(&lines[0]), "┃ Thinking:");
        assert_eq!(line_text(&lines[1]), "┃ ");
    }

    #[test]
    fn render_tool_result_lines_list_counts_items_from_output() {
        use crate::session::{Message, ToolExecutionResult};

        let message = Message::tool_result(
            "tool-call-id",
            "list",
            ToolExecutionResult::new("./\nfile1.txt\nfile2.txt"),
        );

        let app = super::App::new().unwrap();
        let lines = app.render_tool_result_lines(&message, 80);
        assert!(
            lines
                .iter()
                .any(|line| line_text(line).contains("Listed 2 items"))
        );
    }

    #[test]
    fn message_render_cache_hits_on_second_render_same_width() {
        let mut app = super::App::new().unwrap();
        app.conversation
            .push(Message::new(MessageRole::User, "show file list"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "Summary with **markdown** and `inline code`.",
        ));

        let _ = app.messages_text(Some(80));
        let (hits_before, misses_before, entries_before) = app.message_render_cache_stats();

        let _ = app.messages_text(Some(80));
        let (hits_after, misses_after, entries_after) = app.message_render_cache_stats();

        assert_eq!(hits_before, 0);
        assert!(misses_before >= 2);
        assert!(entries_before >= 2);
        assert!(hits_after > hits_before);
        assert_eq!(misses_after, misses_before);
        assert_eq!(entries_after, entries_before);
    }

    #[test]
    fn message_render_cache_width_change_causes_miss() {
        let mut app = super::App::new().unwrap();
        app.conversation
            .push(Message::new(MessageRole::User, "open README"));
        app.conversation.push(Message::new(
            MessageRole::Assistant,
            "A longer paragraph that should wrap differently at another width.",
        ));

        let _ = app.messages_text(Some(72));
        let (_, misses_before, entries_before) = app.message_render_cache_stats();

        let _ = app.messages_text(Some(100));
        let (_, misses_after, entries_after) = app.message_render_cache_stats();

        assert!(misses_after > misses_before);
        assert!(entries_after > entries_before);
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
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();

    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };

    let summary = match canonical_name {
        "read" => field("path")
            .map(|path| format!("Read {path}"))
            .unwrap_or_else(|| "Read file".to_string()),
        "write" => field("path")
            .map(|path| format!("Write {path}"))
            .unwrap_or_else(|| "Write file".to_string()),
        "edit" => field("path")
            .map(|path| format!("Edit {path}"))
            .unwrap_or_else(|| "Edit file".to_string()),
        "list" => field("path")
            .map(|path| format!("List {path}"))
            .unwrap_or_else(|| "List items".to_string()),
        "glob" => {
            let pattern = field("pattern").unwrap_or("*");
            let path = field("path").unwrap_or(".");
            format!("Find {pattern} in {path}")
        }
        "grep" => {
            let pattern = field("pattern").unwrap_or("");
            let path = field("path").unwrap_or(".");
            if pattern.is_empty() {
                format!("Search in {path}")
            } else {
                format!("Search \"{pattern}\" in {path}")
            }
        }
        "bash" => field("command")
            .map(|command| format!("Run shell command: {command}"))
            .unwrap_or_else(|| "Run shell command".to_string()),
        "task" => {
            let description = field("description").unwrap_or("task");
            let subagent_type = field("subagent_type").unwrap_or("general");
            format!("Spawn {subagent_type} subagent: {description}")
        }
        "question" => {
            let count = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .map(|questions| questions.len())
                .unwrap_or(0);

            if count == 1 {
                field("question")
                    .map(|question| format!("Ask: {question}"))
                    .unwrap_or_else(|| "Ask 1 question".to_string())
            } else {
                format!("Ask {count} question{}", if count == 1 { "" } else { "s" })
            }
        }
        "todowrite" => "Update todo list".to_string(),
        _ => {
            let mut summary = display_tool_name(tool_name);
            summary = summary[0..1].to_uppercase() + &summary[1..];
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
        "task" => {
            if let Some(description) = string_field("description") {
                fields.push(("description".to_string(), description));
            }
            if let Some(subagent_type) = string_field("subagent_type") {
                fields.push(("subagent_type".to_string(), subagent_type));
            }
        }
        "question" => {
            let question_count = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .map(|questions| questions.len())
                .unwrap_or(0);

            fields.push((
                "questions".to_string(),
                format!("{question_count} question(s)"),
            ));

            if let Some(first_question) = parsed
                .as_ref()
                .and_then(|value| value.get("questions"))
                .and_then(serde_json::Value::as_array)
                .and_then(|questions| questions.first())
                .and_then(|question| question.get("question"))
                .and_then(serde_json::Value::as_str)
            {
                fields.push((
                    "question".to_string(),
                    shorten_single_line(first_question, 96),
                ));
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
