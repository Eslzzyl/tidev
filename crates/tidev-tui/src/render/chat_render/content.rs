use crate::markdown::render_markdown_text_with_width_and_cwd;
use chrono::Local;
use ratatui::{
    prelude::{Modifier, Style},
    style::Color,
    text::{Line, Span},
};
use std::collections::HashMap;
use std::path::Path;
use tidev_session::session::{COMPACTION_MESSAGE_LABEL, Message, MessageRole};
use tidev_types::prompts::SessionMode;
use uuid::Uuid;

use super::RenderContext;
use super::tool::{render_compaction_divider_line, render_tool_call_with_result};
use super::utils::render_reasoning_markdown_lines;
use crate::core::state::{
    MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind, MessageRenderCacheValue,
};
use crate::diff_render::render_unified_diff_text;
use crate::render::render::{
    line_with_prefix, line_with_style, line_with_style_right_aligned, shorten_single_line,
};

pub(super) fn render_reasoning_lines(
    ctx: &RenderContext<'_>,
    reasoning: &str,
    body_width: usize,
    is_streaming: bool,
) -> Vec<Line<'static>> {
    render_reasoning_markdown_lines(
        reasoning,
        body_width,
        Some(ctx.workspace_root),
        ctx.palette,
        is_streaming,
    )
}

/// Strip all `<system-reminder>…</system-reminder>` blocks from the
/// given text. These tags are injected into user-message content for LLM
/// prefix cache consistency and must not be visible in the UI.
pub(crate) fn strip_system_reminder_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        if let Some(start) = rest.find("<system-reminder") {
            // Push content before the tag
            result.push_str(&rest[..start]);
            // Find the closing tag
            if let Some(end) = rest[start..].find("</system-reminder>") {
                let after_close = start + end + "</system-reminder>".len();
                rest = &rest[after_close..];
                // Skip trailing whitespace/newlines after the closing tag
                while rest.starts_with('\n') || rest.starts_with('\r') || rest.starts_with(' ') {
                    rest = &rest[1..];
                }
            } else {
                // No closing tag — keep the rest as-is
                result.push_str(&rest[start..]);
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result
}

pub(super) fn render_text_body_lines(
    ctx: &RenderContext<'_>,
    text: &str,
    body_width: usize,
    cwd: Option<&Path>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if text.trim().is_empty() {
        lines.push(line_with_style("(empty)", ctx.palette.muted));
    } else {
        let rendered = render_markdown_text_with_width_and_cwd(text, Some(body_width), cwd);
        lines.extend(rendered.lines);
    }
    lines
}

pub(super) fn render_error_body_lines(
    ctx: &RenderContext<'_>,
    message: &Message,
    body_width: usize,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    if !message.reasoning.trim().is_empty() {
        lines.extend(render_reasoning_lines(
            ctx,
            &message.reasoning,
            body_width,
            message.streaming,
        ));
        lines.push(Line::from(""));
    }

    let error_text = if message.content.trim().is_empty() {
        "Request cancelled.".to_string()
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
        lines.push(line_with_style("! Request cancelled.", palette.error));
    }

    lines
}

pub(super) fn render_assistant_body_lines(
    ctx: &RenderContext<'_>,
    message: &Message,
    body_width: usize,
    is_round_end: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if !message.reasoning.trim().is_empty() {
        lines.extend(render_reasoning_lines(
            ctx,
            &message.reasoning,
            body_width,
            message.streaming,
        ));
        if !message.content.trim().is_empty() {
            lines.push(Line::from(""));
        }
    }

    if !message.content.is_empty() {
        if let Some((diff_lines, _)) =
            render_unified_diff_text(&message.content, body_width, ctx.palette)
        {
            lines.extend(diff_lines);
        } else {
            let rendered = render_markdown_text_with_width_and_cwd(
                &message.content,
                Some(body_width),
                Some(ctx.workspace_root),
            );
            lines.extend(rendered.lines);
        }
    }

    if lines.is_empty()
        && !message.streaming
        && message.reasoning.trim().is_empty()
        && message.tool_calls.is_empty()
    {
        lines.push(line_with_style("(empty)", ctx.palette.muted));
    }

    // Add model name, duration, end time, and mode at the end (only for round end)
    if is_round_end && !message.streaming && message.tool_calls.is_empty() {
        let model_display_name = message
            .model_id
            .as_ref()
            .and_then(|model_id| {
                ctx.config
                    .read()
                    .unwrap()
                    .resolve_model_by_ids(ctx.auth, &ctx.conversation.provider_id, model_id)
                    .ok()
                    .map(|model| model.display_name)
            })
            .unwrap_or_else(|| ctx.conversation.model_display_name.clone());

        let duration = message.completed_at.map(|completed| {
            let elapsed = completed - message.created_at;
            let secs = elapsed.as_seconds_f64();
            format!("{:.1}s", secs)
        });

        let end_time = message.completed_at.map(|completed| {
            completed
                .with_timezone(&Local)
                .format("%H:%M:%S")
                .to_string()
        });

        let tps = message
            .tokens_per_second
            .map(|val| format!("{:.1} t/s", val));

        let mode_label = message
            .mode
            .or_else(|| {
                ctx.conversation
                    .messages
                    .iter()
                    .take_while(|m| m.id != message.id)
                    .filter(|m| m.role == MessageRole::User)
                    .last()
                    .and_then(|m| m.mode)
            })
            .unwrap_or(ctx.mode);

        let parts: Vec<String> = [
            Some(model_display_name),
            duration,
            tps,
            end_time,
            Some(mode_label.title().to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();

        let suffix = parts.join(" · ");
        lines.push(line_with_style_right_aligned(
            &suffix,
            body_width,
            ctx.palette.accent_soft,
        ));
    }

    lines
}

pub(super) fn render_message_cards_inner(
    ctx: &RenderContext<'_>,
    message: &Message,
    body_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;

    match message.role {
        MessageRole::User | MessageRole::Shell => {
            // Check for hidden system-injected goal continuation messages
            if message.role == MessageRole::User
                && message.content.trim_start().starts_with("<goal_context>")
            {
                vec![(palette.panel_alt, {
                    let mut lines = Vec::new();
                    let style = Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::DIM);
                    lines.push(Line::from(vec![Span::styled(
                        "┃ [Auto continue in goal mode]",
                        style,
                    )]));
                    lines
                })]
            } else {
                vec![(palette.panel_alt, {
                    let display_content = strip_system_reminder_tags(&message.content);
                    let mut content_lines = render_text_body_lines(
                        ctx,
                        &display_content,
                        body_width.saturating_sub(2),
                        Some(ctx.workspace_root),
                    );
                    for attachment in &message.attachments {
                        content_lines
                            .push(line_with_style(&attachment.summary(), palette.accent_soft));
                    }
                    let mut lines = Vec::new();
                    let mode_color = message.mode.map_or(palette.accent, |m| match m {
                        SessionMode::Build => palette.mode_build,
                        SessionMode::Plan => palette.mode_plan,
                    });
                    let prefix_style = Style::default().fg(mode_color).add_modifier(Modifier::BOLD);
                    lines.push(Line::from(vec![Span::styled("┃ ", prefix_style)]));
                    for line in content_lines {
                        let mut spans = vec![Span::styled("┃ ", prefix_style)];
                        spans.extend(line.spans);
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(vec![Span::styled("┃ ", prefix_style)]));
                    lines
                })]
            }
        }
        MessageRole::Assistant => {
            let mut cards = Vec::new();
            let body_lines = render_assistant_body_lines(ctx, message, body_width, is_round_end);
            if !body_lines.is_empty() {
                let mut lines_with_margin = Vec::new();
                lines_with_margin.push(Line::from(""));
                lines_with_margin.extend(body_lines);
                lines_with_margin.push(Line::from(""));
                cards.push((palette.background, lines_with_margin));
            }
            cards
        }
        MessageRole::Tool => Vec::new(),
        MessageRole::System => {
            if message.content.starts_with(COMPACTION_MESSAGE_LABEL) {
                let summary = message
                    .content
                    .split_once("\n\n")
                    .map(|(_, s)| s)
                    .unwrap_or("")
                    .trim();
                let mut lines = Vec::new();
                lines.push(Line::from(""));
                lines.push(render_compaction_divider_line(
                    COMPACTION_MESSAGE_LABEL,
                    body_width,
                    palette,
                ));
                if !summary.is_empty() {
                    lines.push(Line::from(""));
                    lines.extend(render_text_body_lines(
                        ctx,
                        summary,
                        body_width,
                        Some(ctx.workspace_root),
                    ));
                }
                lines.push(Line::from(""));
                return vec![(palette.background, lines)];
            }
            if message.content.starts_with("Loaded instructions from")
                || (message.content.starts_with("Loaded ")
                    && message.content.contains(" instruction files:"))
            {
                let line = Line::from(vec![
                    Span::styled("󱁤 ", Style::default().fg(palette.accent_soft)),
                    Span::styled(
                        message.content.clone(),
                        Style::default()
                            .fg(palette.text)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]);
                return vec![(palette.background, vec![line])];
            }
            let content_lines =
                render_text_body_lines(ctx, &message.content, body_width, Some(ctx.workspace_root));
            let mut lines = Vec::new();
            lines.push(Line::from(""));
            lines.extend(content_lines);
            lines.push(Line::from(""));
            vec![(palette.background, lines)]
        }
        MessageRole::Error => {
            let error_lines = render_error_body_lines(ctx, message, body_width);
            let mut lines = Vec::new();
            lines.push(Line::from(""));
            lines.extend(error_lines);
            lines.push(Line::from(""));
            vec![(palette.panel_light, lines)]
        }
    }
}

/// Result of computing block data for a single message block.
pub(super) struct BlockComputation {
    pub(super) message_id: Uuid,
    pub(super) message_count: usize,
    pub(super) line_count: usize,
    pub(super) cache_entries: Vec<(MessageRenderCacheKey, MessageRenderCacheEntry)>,
}

/// Compute block data without accessing the shared render cache.
pub(super) fn compute_block_data(
    ctx: &RenderContext<'_>,
    session_id: Uuid,
    messages: &[Message],
    start_idx: usize,
    _width: usize,
    body_width: usize,
    is_round_end: bool,
) -> BlockComputation {
    let message = &messages[start_idx];
    let message_id = message.id;

    let (message_count, line_count, cache_entries) = match message.role {
        MessageRole::Assistant => {
            let mut count = 1;
            while start_idx + count < messages.len()
                && matches!(messages[start_idx + count].role, MessageRole::Tool)
            {
                count += 1;
            }

            let cards = render_message_cards_inner(ctx, message, body_width, is_round_end);
            let mut lines = 0;
            let mut cache_entries = Vec::new();

            let cards_key = MessageRenderCacheKey {
                session_id,
                message_id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache_entries.push((
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: 0,
                },
            ));

            for (_, card_lines) in &cards {
                lines += card_lines.len();
            }

            let tool_results_by_id: HashMap<String, &Message> = {
                let mut map = HashMap::new();
                let mut j = start_idx + 1;
                while j < messages.len() && matches!(messages[j].role, MessageRole::Tool) {
                    if let Some(id) = &messages[j].tool_call_id {
                        map.insert(id.clone(), &messages[j]);
                    }
                    j += 1;
                }
                map
            };

            if !message.tool_calls.is_empty() {
                for tool_call in &message.tool_calls {
                    let tool_result = tool_results_by_id.get(&tool_call.id).copied();
                    let (card_lines, regions) = render_tool_call_with_result(
                        tool_call,
                        tool_result,
                        body_width,
                        message.streaming,
                        ctx,
                    );

                    let tool_key = MessageRenderCacheKey {
                        session_id,
                        message_id,
                        width: body_width,
                        is_round_end,
                        kind: MessageRenderCacheKind::ToolCall(tool_call.id.clone()),
                    };
                    cache_entries.push((
                        tool_key,
                        MessageRenderCacheEntry {
                            value: MessageRenderCacheValue::ToolResult(card_lines.clone(), regions),
                            last_used_tick: 0,
                        },
                    ));

                    if !card_lines.is_empty() {
                        lines += card_lines.len();
                    }
                }
                lines += 1;
            }

            (count, lines, cache_entries)
        }
        MessageRole::User => {
            let cards = render_message_cards_inner(ctx, message, body_width, is_round_end);
            let mut lines = 0;
            let cards_key = MessageRenderCacheKey {
                session_id,
                message_id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            let cache_entries = vec![(
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: 0,
                },
            )];
            for (_, card_lines) in &cards {
                lines += card_lines.len();
            }
            lines += 1;
            (1, lines, cache_entries)
        }
        MessageRole::System => {
            let cards = render_message_cards_inner(ctx, message, body_width, is_round_end);
            let mut lines = 0;
            let cards_key = MessageRenderCacheKey {
                session_id,
                message_id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            let cache_entries = vec![(
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: 0,
                },
            )];
            for (_, card_lines) in &cards {
                lines += card_lines.len();
            }
            lines += 1;
            (1, lines, cache_entries)
        }
        MessageRole::Error => {
            let cards = render_message_cards_inner(ctx, message, body_width, is_round_end);
            let mut lines = 0;
            let cards_key = MessageRenderCacheKey {
                session_id,
                message_id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            let cache_entries = vec![(
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: 0,
                },
            )];
            for (_, card_lines) in &cards {
                lines += card_lines.len();
            }
            lines += 1;
            (1, lines, cache_entries)
        }
        MessageRole::Shell => {
            let cards = render_message_cards_inner(ctx, message, body_width, is_round_end);
            let mut lines = 0;
            let cards_key = MessageRenderCacheKey {
                session_id,
                message_id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            let cache_entries = vec![(
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: 0,
                },
            )];
            for (_, card_lines) in &cards {
                lines += card_lines.len();
            }
            lines += 1;
            (1, lines, cache_entries)
        }
        MessageRole::Tool => (1, 0, Vec::new()),
    };

    BlockComputation {
        message_id,
        message_count,
        line_count,
        cache_entries,
    }
}
