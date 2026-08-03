use super::*;

use chrono::Local;
use ratatui::prelude::{Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use tidev_llm::message::{COMPACTION_MESSAGE_LABEL, Message, MessageRole};
use unicode_width::UnicodeWidthStr;

use crate::diff_render::render_unified_diff_text;
use crate::markdown;
use crate::markdown::{WrapOptions, word_wrap_line};

use super::thinking::{is_reasoning_collapsed, render_reasoning_lines, thinking_duration_str};
use super::utils::{apply_badge_styling, render_compaction_divider_line, wrap_text_lines};

/// Render assistant message cards with reasoning, content (diff or markdown),
/// and a metadata footer at round end.  No title bar — the body lines begin
/// directly.  Margin blank lines are added before and after the body.
pub(super) fn render_assistant_cards(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    content_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let body_lines =
        render_assistant_body_lines(ctx, message, messages, content_width, is_round_end);

    let mut lines_with_margin = Vec::new();
    lines_with_margin.extend(body_lines);
    // No trailing "" here — inter-block spacing is handled by messages_text.
    // Spacing between body and tool calls is added in render_block_from_cache.

    vec![(palette.background, lines_with_margin)]
}

/// Render the inner body lines of an assistant message card.
/// No title bar, no margin lines — just reasoning, content, footer.
fn render_assistant_body_lines(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    content_width: usize,
    is_round_end: bool,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    // 1. Reasoning (with ┃ prefix, dimmed colours, exactly like old code)
    if !message.reasoning.trim().is_empty() {
        let collapsed = is_reasoning_collapsed(
            &message.id,
            ctx.thinking_collapsed_overrides,
            ctx.default_collapse_thinking,
        );
        let duration = thinking_duration_str(message);
        lines.extend(render_reasoning_lines(
            ctx,
            &message.reasoning,
            content_width,
            message.streaming,
            collapsed,
            duration.as_deref(),
        ));
        if !message.content.trim().is_empty() {
            lines.push(Line::from(""));
        }
    }

    // 2. Content — try unified diff first, fall back to markdown
    if !message.content.is_empty() {
        if let Some((diff_lines, _)) =
            render_unified_diff_text(&message.content, content_width, palette, 4)
        {
            for dl in &diff_lines {
                lines.push(dl.clone());
            }
        } else {
            let md = markdown::render_markdown_text_with_width_and_cwd(
                &message.content,
                Some(content_width),
                Some(ctx.workspace_root),
            );
            for md_line in md.lines.iter() {
                lines.push(md_line.clone());
            }
        }
    }

    // 3. "(empty)" placeholder for empty non-streaming with no tool calls
    if lines.is_empty()
        && !message.streaming
        && message.reasoning.trim().is_empty()
        && message.tool_calls.is_empty()
    {
        lines.push(Line::from(Span::styled(
            "(empty)",
            Style::default().fg(palette.muted),
        )));
    }

    // 4. Metadata footer at round end (model · thinking level · duration · t/s · time · mode)
    if is_round_end && !message.streaming && message.tool_calls.is_empty() {
        let mut parts: Vec<String> = Vec::new();

        // Model display name (resolve via config in old code — use model_id as fallback)
        if message.model_id.is_some() {
            parts.push(ctx.model_display_name.to_string());
        }

        // The thinking level is stored on the user message for the request
        // that produced this assistant response.  Prefer a level copied onto
        // the assistant message when available for forward compatibility.
        let thinking_level = message.thinking_level.clone().or_else(|| {
            let message_index = messages.iter().position(|m| m.id == message.id)?;
            messages[..message_index]
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User))
                .and_then(|m| m.thinking_level.clone())
        });
        if let Some(level) = thinking_level.filter(|level| level.is_supported()) {
            parts.push(level.display_name().to_string());
        }

        // Duration: from previous user message created_at to this message completed_at
        if let Some(completed) = message.completed_at {
            let prev_user = messages
                .iter()
                .take_while(|m| m.id != message.id)
                .filter(|m| matches!(m.role, MessageRole::User))
                .last()
                .map(|m| m.created_at)
                .unwrap_or(message.created_at);
            let elapsed = completed - prev_user;
            let total_secs = elapsed.num_seconds().max(0) as u64;
            let hours = total_secs / 3600;
            let minutes = (total_secs % 3600) / 60;
            let seconds = total_secs % 60;
            let duration = if hours > 0 {
                format!("{}h {}min {}s", hours, minutes, seconds)
            } else if minutes > 0 {
                format!("{}min {}s", minutes, seconds)
            } else {
                format!("{}s", seconds)
            };
            parts.push(duration);
        }

        // Tokens per second
        if let Some(tps) = message.tokens_per_second {
            parts.push(format!("{:.1} t/s", tps));
        }

        // End time
        if let Some(completed) = message.completed_at {
            parts.push(
                completed
                    .with_timezone(&Local)
                    .format("%H:%M:%S")
                    .to_string(),
            );
        }

        // Mode
        if let Some(mode) = message.mode {
            parts.push(mode.title().to_string());
        }

        if !parts.is_empty() {
            let suffix = parts.join(" · ");
            let text_width = UnicodeWidthStr::width(suffix.as_str());
            let padding = content_width.saturating_sub(text_width);
            lines.push(Line::from(Span::styled(
                format!("{}{}", " ".repeat(padding), suffix),
                Style::default().fg(palette.accent_soft),
            )));
        }
    }

    lines
}

/// Render a user or shell message card with ┃ prefix and mode-colored accent.
fn render_user_shell_card(
    ctx: &RenderContext,
    message: &Message,
    content_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;

    let display_content = strip_system_reminder_tags(&message.content);
    let mut content_lines =
        render_text_body_lines(ctx, &display_content, content_width.saturating_sub(2)); // 2 for ┃ prefix
    apply_badge_styling(&mut content_lines, palette);

    let mode_color = message.mode.map_or(palette.accent, |m| match m {
        tidev_llm::mode::SessionMode::Build => palette.mode_build,
        tidev_llm::mode::SessionMode::Plan => palette.mode_plan,
    });
    let prefix_style = Style::default().fg(mode_color).add_modifier(Modifier::BOLD);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled("┃ ", prefix_style)]));
    for line in &content_lines {
        let mut spans = vec![Span::styled("┃ ", prefix_style)];
        spans.extend(line.spans.iter().cloned());
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(vec![Span::styled("┃ ", prefix_style)]));

    vec![(palette.panel_alt, lines)]
}

/// Render an error message with `!` prefix and optional reasoning.
/// Uses the default chat background (no card panel) with a trailing
/// empty line for inter-card spacing.
fn render_error_card(
    ctx: &RenderContext,
    message: &Message,
    content_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    // 1. Reasoning (if any)
    if !message.reasoning.trim().is_empty() {
        let collapsed = is_reasoning_collapsed(
            &message.id,
            ctx.thinking_collapsed_overrides,
            ctx.default_collapse_thinking,
        );
        let duration = thinking_duration_str(message);
        lines.extend(render_reasoning_lines(
            ctx,
            &message.reasoning,
            content_width,
            message.streaming,
            collapsed,
            duration.as_deref(),
        ));
        lines.push(Line::from(""));
    }

    // 2. Error text
    let error_text = if message.content.trim().is_empty() {
        "Request cancelled.".to_string()
    } else {
        message.content.clone()
    };

    let error_style = Style::default().fg(palette.error);
    let prefix_style = Style::default()
        .fg(palette.error)
        .add_modifier(Modifier::BOLD);
    let text_width = content_width.saturating_sub(2).max(1); // 2 for ! prefix

    for line_text in error_text.lines() {
        if line_text.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let wrapped = wrap_text_lines(line_text, text_width, usize::MAX);
        for (i, wrapped_line) in wrapped.iter().enumerate() {
            let prefix = if i == 0 { "!" } else { " " };
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", prefix), prefix_style),
                Span::styled(wrapped_line.clone(), error_style),
            ]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("! ", prefix_style),
            Span::styled("Request cancelled.", error_style),
        ]));
    }

    // 3. No card background — just the error text in error colour.
    vec![(palette.background, lines)]
}

/// Render a system message card (handles compaction, instructions, generic).
pub(super) fn render_system_card(
    ctx: &RenderContext,
    message: &Message,
    content_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let content = &message.content;

    // Instruction loading message (single line with Nerd Font icon)
    if content.starts_with("Loaded instructions from")
        || (content.starts_with("Loaded ") && content.contains(" instruction files:"))
    {
        let line = Line::from(vec![
            Span::styled("󱁤  ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                content.clone(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC),
            ),
        ]);
        let mut lines = Vec::new();
        lines.extend(
            word_wrap_line(
                &line,
                WrapOptions::new(content_width).subsequent_indent(Line::from("   ")),
            )
            .into_iter()
            .map(|l| {
                Line::from(
                    l.spans
                        .into_iter()
                        .map(|s| Span::styled(s.content.to_string(), s.style))
                        .collect::<Vec<_>>(),
                )
            }),
        );
        return vec![(palette.background, lines)];
    }

    // Compaction message
    if content.starts_with(COMPACTION_MESSAGE_LABEL) {
        let summary = content
            .split_once("\n\n")
            .map(|(_, s)| s)
            .unwrap_or("")
            .trim();
        let mut lines = Vec::new();
        lines.push(render_compaction_divider_line(
            COMPACTION_MESSAGE_LABEL,
            content_width,
            palette,
        ));
        if !summary.is_empty() {
            lines.push(Line::from(""));
            let md = markdown::render_markdown_text_with_width_and_cwd(
                summary,
                Some(content_width),
                Some(ctx.workspace_root),
            );
            for md_line in md.lines.iter() {
                lines.push(md_line.clone());
            }
        }
        // Metadata footer for compaction (same style as assistant)
        if is_round_end && !message.streaming {
            let mut parts: Vec<String> = Vec::new();
            if message.model_id.is_some() {
                parts.push(ctx.model_display_name.to_string());
            }
            if let Some(completed) = message.completed_at {
                let elapsed = completed - message.created_at;
                let total_secs = elapsed.num_seconds().max(0) as u64;
                let hours = total_secs / 3600;
                let minutes = (total_secs % 3600) / 60;
                let seconds = total_secs % 60;
                let duration = if hours > 0 {
                    format!("{}h {}min {}s", hours, minutes, seconds)
                } else if minutes > 0 {
                    format!("{}min {}s", minutes, seconds)
                } else {
                    format!("{}s", seconds)
                };
                parts.push(duration);
            }
            if let Some(tps) = message.tokens_per_second {
                parts.push(format!("{:.1} t/s", tps));
            }
            if let Some(completed) = message.completed_at {
                parts.push(
                    completed
                        .with_timezone(&Local)
                        .format("%H:%M:%S")
                        .to_string(),
                );
            }
            if let Some(mode) = message.mode {
                parts.push(mode.title().to_string());
            }
            if !parts.is_empty() {
                let suffix = parts.join(" · ");
                let text_width = UnicodeWidthStr::width(suffix.as_str());
                let padding = content_width.saturating_sub(text_width);
                lines.push(Line::from(Span::styled(
                    format!("{}{}", " ".repeat(padding), suffix),
                    Style::default().fg(palette.accent_soft),
                )));
            }
        }
        return vec![(palette.background, lines)];
    }

    // Generic system message: render as markdown
    let content_lines = render_text_body_lines(ctx, content, content_width);
    let mut lines = Vec::new();
    lines.extend(content_lines);
    vec![(palette.background, lines)]
}

/// Render a standalone Tool message that was not grouped with its parent
/// Assistant block (defensive fallback).  Shows the tool name as a header
/// followed by the output content rendered as markdown.
pub(super) fn render_tool_card(
    ctx: &RenderContext,
    message: &Message,
    content_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let tool_name = message.tool_name.clone().unwrap_or_else(|| "tool".into());
    let mut lines = Vec::new();

    // Header line
    lines.push(Line::from(vec![
        Span::styled("Tool: ", Style::default().fg(palette.accent_soft)),
        Span::styled(
            tool_name,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
    ]));

    // Separator
    lines.push(Line::from(vec![Span::styled(
        "─".repeat(content_width.min(40)),
        Style::default().fg(palette.muted),
    )]));

    // Output content (markdown-rendered), with system-reminder tags stripped
    let display_content = crate::utils::strip_system_reminder_tags(&message.content);
    let content_lines = render_text_body_lines(ctx, &display_content, content_width);
    lines.extend(content_lines);

    vec![(palette.panel_alt, lines)]
}

/// Render a single-card message (dispatches by role).
pub(super) fn render_single_card(
    ctx: &RenderContext,
    message: &Message,
    content_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    match message.role {
        MessageRole::User | MessageRole::Shell => {
            render_user_shell_card(ctx, message, content_width)
        }
        MessageRole::Error => render_error_card(ctx, message, content_width),
        MessageRole::System => render_system_card(ctx, message, content_width, is_round_end),
        MessageRole::Tool => render_tool_card(ctx, message, content_width),
        _ => {
            let palette = ctx.palette;
            let content_lines = render_text_body_lines(ctx, &message.content, content_width);
            vec![(palette.background, content_lines)]
        }
    }
}

/// Render text body lines with markdown, returning "(empty)" if blank.
fn render_text_body_lines(
    ctx: &RenderContext,
    text: &str,
    content_width: usize,
) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        vec![Line::from(Span::styled(
            "(empty)",
            Style::default().fg(ctx.palette.muted),
        ))]
    } else {
        let md = markdown::render_markdown_text_with_width_and_cwd(
            text,
            Some(content_width),
            Some(ctx.workspace_root),
        );
        md.lines.to_vec()
    }
}

/// Check if the message at `start_idx` is the first User message in `messages`.
pub(super) fn is_first_user_message(messages: &[Message], start_idx: usize) -> bool {
    matches!(messages[start_idx].role, MessageRole::User)
        && !messages[..start_idx]
            .iter()
            .any(|m| matches!(m.role, MessageRole::User))
}

/// Strip `<system-reminder>…</system-reminder>` tags from user-message
/// content. These tags are injected for LLM prefix cache consistency but
/// must never be visible in the UI.
fn strip_system_reminder_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        if let Some(start) = rest.find("<system-reminder") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</system-reminder>") {
                let after_close = start + end + "</system-reminder>".len();
                rest = &rest[after_close..];
                while rest.starts_with('\n') || rest.starts_with('\r') || rest.starts_with(' ') {
                    rest = &rest[1..];
                }
            } else {
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
