//! Core rendering pipeline for the chat message list.
//!
//! Orchestrates layout index updates, cache lookups, block rendering, and
//! scroll management. Messages are rendered with markdown formatting, cached
//! in an LRU, and only re-rendered when content or width changes.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use ratatui::layout::{Alignment, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use rayon::prelude::*;
use tidev_types::message::{Message, MessageRole};
use crate::chat_context::ChatContext;
use crate::theme::ThemePalette;
use uuid::Uuid;

use crate::components::chat::layout_index::{MessageBlock, MessageLayoutIndex};
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind, MessageRenderCacheValue,
    SelectableRegionRange,
};
use crate::markdown;

use crate::components::chat::tool;

// ---------------------------------------------------------------------------
// Running subagent summary (used for inline cards)
// ---------------------------------------------------------------------------

pub(crate) struct RunningSubagentInfo {
    pub tool_call_id: String,
    pub description: String,
    pub subagent_type: String,
    pub status_text: String,
    pub child_session_id: Option<uuid::Uuid>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LEFT_MARGIN: u16 = 2;
const SCROLLBAR_WIDTH: u16 = 1;
const GAP: u16 = 1;

// ---------------------------------------------------------------------------
// RenderContext
// ---------------------------------------------------------------------------

/// Shared context assembled once per frame and threaded through all rendering.
pub(crate) struct RenderContext<'a> {
    pub palette: ThemePalette,
    pub workspace_root: &'a Path,
    pub expanded_tool_results: &'a HashSet<Uuid>,
    pub expanded_tool_outputs: &'a HashMap<Uuid, String>,
}

// ---------------------------------------------------------------------------
// RenderOutput
// ---------------------------------------------------------------------------

/// Every piece of data produced by the rendering pipeline.
pub(crate) struct RenderOutput {
    pub lines: Vec<Line<'static>>,
    pub total_lines: usize,
    pub selectable_regions: Vec<SelectableRegionRange>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub(crate) fn render_messages(
    frame: &mut Frame,
    area: Rect,
    layout_index: &mut MessageLayoutIndex,
    render_cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    chat_context: &ChatContext,
    palette: ThemePalette,
    scroll_offset: &mut usize,
    follow_tail: &mut bool,
    expanded_tool_results: &mut HashSet<Uuid>,
    expanded_tool_outputs: &mut HashMap<Uuid, String>,
    streaming: bool,
    current_streaming_message_id: Option<Uuid>,
    render_tick: &mut u64,
    running_subagents: &[RunningSubagentInfo],
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (content_area, scrollbar_rect) = compute_content_layout(area);

    let ctx = RenderContext {
        palette,
        workspace_root: Path::new(""),
        expanded_tool_results,
        expanded_tool_outputs,
    };

    let output = messages_text(
        chat_context,
        layout_index,
        render_cache,
        &ctx,
        content_area.width as usize,
        *scroll_offset,
        area.height as usize,
        streaming,
        current_streaming_message_id,
        render_tick,
    );

    update_scroll_state(scroll_offset, follow_tail, output.total_lines, area.height as usize);

    // Render running subagent cards (at the end of the message area)
    let total_with_subagents = if !running_subagents.is_empty() {
        let mut subagent_lines: Vec<Line<'static>> = Vec::new();
        for sa in running_subagents {
            let style = Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD);
            subagent_lines.push(Line::from(vec![
                Span::styled(format!(" ▶ task [{}]", sa.subagent_type), style),
                Span::styled(format!(" {}", sa.status_text), Style::default().fg(palette.muted)),
            ]));
            subagent_lines.push(Line::from(Span::styled(
                format!("   {}", sa.description),
                Style::default().fg(palette.text),
            )));
        }
        // Total includes subagent cards
        let add_lines = subagent_lines.len();
        let combined: Vec<Line> = output.lines.into_iter().chain(subagent_lines).collect();
        let text = ratatui::text::Text::from(combined);
        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(ctx.palette.background));
        frame.render_widget(paragraph, content_area);
        output.total_lines + add_lines
    } else {
        let text = ratatui::text::Text::from(output.lines);
        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(ctx.palette.background));
        frame.render_widget(paragraph, content_area);
        output.total_lines
    };

    // Scrollbar (using total_with_subagents as the adjusted total)
    if let Some(sb) = scrollbar_rect {
        render_scrollbar(frame, sb, *scroll_offset, total_with_subagents, area.height as usize, ctx.palette);
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

fn compute_content_layout(area: Rect) -> (Rect, Option<Rect>) {
    if area.width > LEFT_MARGIN + GAP + SCROLLBAR_WIDTH {
        let content_width = area.width - LEFT_MARGIN - GAP - SCROLLBAR_WIDTH;
        (Rect { x: area.x + LEFT_MARGIN, y: area.y, width: content_width, height: area.height },
         Some(Rect { x: area.x + area.width - SCROLLBAR_WIDTH, y: area.y, width: SCROLLBAR_WIDTH, height: area.height }))
    } else if area.width > LEFT_MARGIN + SCROLLBAR_WIDTH {
        let content_width = area.width - LEFT_MARGIN - SCROLLBAR_WIDTH;
        (Rect { x: area.x + LEFT_MARGIN, y: area.y, width: content_width, height: area.height },
         Some(Rect { x: area.x + area.width - SCROLLBAR_WIDTH, y: area.y, width: SCROLLBAR_WIDTH, height: area.height }))
    } else if area.width > LEFT_MARGIN {
        (Rect { x: area.x + LEFT_MARGIN, y: area.y, width: area.width - LEFT_MARGIN, height: area.height }, None)
    } else {
        (area, None)
    }
}

fn update_scroll_state(scroll_offset: &mut usize, follow_tail: &mut bool, total_lines: usize, viewport: usize) {
    let max_scroll = total_lines.saturating_sub(viewport);
    if *follow_tail {
        *scroll_offset = max_scroll;
    } else {
        *scroll_offset = (*scroll_offset).min(max_scroll);
    }
}

fn render_scrollbar(frame: &mut Frame, sb: Rect, scroll_offset: usize, total_lines: usize, viewport: usize, palette: ThemePalette) {
    let max_scroll = total_lines.saturating_sub(viewport);
    let scrolled = if max_scroll > 0 { (scroll_offset as f32 / max_scroll as f32).clamp(0.0, 1.0) } else { 0.0 };
    let thumb_pos = ((sb.height as f32 - 1.0).max(0.0) * scrolled).round() as u16;
    let thumb_height = ((sb.height as f32 * sb.height as f32 / total_lines.max(1) as f32).clamp(1.0, sb.height as f32)).round() as u16;

    let lines: Vec<Line> = (0..sb.height).map(|row| {
        if row >= thumb_pos && row < thumb_pos + thumb_height {
            Line::from(Span::styled("█", Style::default().fg(palette.accent)))
        } else {
            Line::from(Span::styled("░", Style::default().fg(palette.border)))
        }
    }).collect();
    frame.render_widget(Paragraph::new(lines), sb);
}

// ---------------------------------------------------------------------------
// messages_text — the core pipeline
// ---------------------------------------------------------------------------

fn messages_text(
    chat_context: &ChatContext,
    index: &mut MessageLayoutIndex,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    ctx: &RenderContext,
    width: usize,
    scroll: usize,
    viewport: usize,
    streaming: bool,
    _current_streaming_message_id: Option<Uuid>,
    render_tick: &mut u64,
) -> RenderOutput {
    let messages = chat_context.visible_messages();
    let width = width.max(1);
    let body_width = width.saturating_sub(2).max(1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selectable_regions: Vec<SelectableRegionRange> = Vec::new();

    // Header for sub-sessions
    let header_lines = build_header_lines(chat_context.parent_session_id.is_some(), ctx.palette);
    let header_line_count = header_lines.len();

    // Empty state
    if messages.is_empty() {
        lines.extend(header_lines);
        let empty_line = Line::from(Span::styled("No messages yet.", Style::default().fg(ctx.palette.muted)));
        lines.push(empty_line);
        let total = lines.len().max(1);
        return RenderOutput { lines, total_lines: total, selectable_regions };
    }

    // Update layout index — this renders all blocks and populates the cache
    update_layout_index(index, cache, messages, width, body_width, streaming, ctx, render_tick);

    // Calculate visible range
    let total_overall_lines = header_line_count + index.total_lines;
    let message_scroll = scroll.saturating_sub(header_line_count);

    // Find visible blocks
    let visible_blocks = index.find_visible_blocks(message_scroll, viewport);

    lines.extend(header_lines);

    // Render visible blocks from cache
    let first_block_start = visible_blocks.first().map(|b| b.start_line).unwrap_or(0);
    let (mut render_scroll, padding_lines) = if first_block_start < message_scroll {
        (message_scroll - first_block_start, 0)
    } else if first_block_start > message_scroll {
        (0, first_block_start - message_scroll)
    } else {
        (0, 0)
    };
    if scroll < header_line_count {
        render_scroll = scroll;
    }

    for _ in 0..padding_lines {
        lines.push(Line::from(""));
    }

    let mut current_line_offset = lines.len();
    for block in &visible_blocks {
        let block_lines = render_block_from_cache(
            block, cache, width, &mut selectable_regions, ctx, &current_line_offset,
        );
        let block_lines = skip_rendered_lines(block_lines, &mut render_scroll);
        lines.extend(block_lines);
        current_line_offset = lines.len();
    }

    RenderOutput { lines, total_lines: total_overall_lines, selectable_regions }
}

// ---------------------------------------------------------------------------
// Layout index update — renders blocks and caches them
// ---------------------------------------------------------------------------

fn update_layout_index(
    index: &mut MessageLayoutIndex,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    messages: &[Message],
    width: usize,
    body_width: usize,
    streaming: bool,
    ctx: &RenderContext,
    render_tick: &mut u64,
) {
    let needs_full = index.needs_full_rebuild(messages.len(), width, streaming);

    if !needs_full {
        // Incremental update: only recompute dirty blocks
        if index.dirty_messages.is_empty() {
            return;
        }
        for i in 0..index.blocks.len() {
            if index.dirty_messages.contains(&index.blocks[i].message_id) {
                let block = &index.blocks[i];
                let message = &messages[block.message_start_idx];
                let start_idx = block.message_start_idx;
                // Find is_round_end for this block
                let next_idx = start_idx + block.message_count;
                let is_round_end = next_idx >= messages.len()
                    || matches!(messages[next_idx].role, MessageRole::User);
                let old_line_count = block.line_count;

                let (_msg_count, new_line_count, _) = compute_and_cache_block(
                    message, messages, start_idx, is_round_end, width, body_width,
                    cache, ctx, render_tick,
                );

                let diff = new_line_count as isize - old_line_count as isize;
                if diff != 0 {
                    index.blocks[i].line_count = new_line_count;
                    for j in (i + 1)..index.blocks.len() {
                        index.blocks[j].start_line =
                            (index.blocks[j].start_line as isize + diff) as usize;
                    }
                    index.total_lines = (index.total_lines as isize + diff) as usize;
                }
            }
        }
        index.dirty_messages.clear();
        return;
    }

    // Full rebuild
    index.reset(width, streaming);

    if messages.is_empty() {
        return;
    }

    // Determine block boundaries
    let mut blocks_info: Vec<(usize, bool)> = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let count = if matches!(messages[i].role, MessageRole::Assistant) {
            let mut c = 1;
            while i + c < messages.len() && matches!(messages[i + c].role, MessageRole::Tool) {
                c += 1;
            }
            c
        } else {
            1
        };
        let next_idx = i + count;
        let is_round_end = next_idx >= messages.len()
            || matches!(messages[next_idx].role, MessageRole::User);
        blocks_info.push((i, is_round_end));
        i += count;
    }

/// Data produced by computing a single block (used for parallel computation).
struct BlockComputation {
    message_id: Uuid,
    message_count: usize,
    line_count: usize,
    cache_entries: Vec<(MessageRenderCacheKey, MessageRenderCacheEntry)>,
}

/// Compute block data without mutating the cache.
/// This is safe to call from rayon parallel iterators.
fn compute_block_data(
    message: &Message,
    messages: &[Message],
    start_idx: usize,
    is_round_end: bool,
    width: usize,
    body_width: usize,
    ctx: &RenderContext,
    render_tick: u64,
) -> BlockComputation {
    let (message_count, line_count, cache_entries) = match message.role {
        MessageRole::Assistant => {
            let mut count = 1;
            while start_idx + count < messages.len()
                && matches!(messages[start_idx + count].role, MessageRole::Tool)
            {
                count += 1;
            }
            let cards = render_assistant_cards(ctx, message, body_width, is_round_end);
            let mut line_count = 0;
            let mut cache_entries = Vec::new();

            let cards_key = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache_entries.push((cards_key, MessageRenderCacheEntry {
                value: MessageRenderCacheValue::Cards(cards.clone()),
                last_used_tick: render_tick,
            }));

            for (_, card_lines) in &cards {
                line_count += card_lines.len();
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

            for tc in &message.tool_calls {
                let tool_result = tool_results_by_id.get(&tc.id).copied();
                let is_expanded = ctx.expanded_tool_results.contains(&message.id);
                let (tool_lines, _regions) = tool::render_tool_call_with_result(
                    tc, tool_result, body_width, message.streaming, ctx, is_expanded,
                );

                let tool_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: body_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                };
                cache_entries.push((tool_key, MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::ToolResult(tool_lines.clone(), Vec::new()),
                    last_used_tick: render_tick,
                }));

                line_count += tool_lines.len();
            }

            line_count += 1; // trailing empty line
            (count, line_count, cache_entries)
        }
        MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
            let cards = render_single_card(ctx, message, body_width);
            let mut line_count = 0;
            for (_, card_lines) in &cards {
                line_count += card_lines.len();
            }
            line_count += 1;

            let kind = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            let cache_entries = vec![(kind, MessageRenderCacheEntry {
                value: MessageRenderCacheValue::Cards(cards),
                last_used_tick: render_tick,
            })];

            (1, line_count, cache_entries)
        }
        MessageRole::Tool => (1, 0, Vec::new()),
    };

    BlockComputation { message_id: message.id, message_count, line_count, cache_entries }
}

    // Compute and cache each block (parallelised via rayon when >4 blocks).
    let mut current_line = 0usize;
    let computations: Vec<BlockComputation> = if blocks_info.len() > 4 {
        blocks_info
            .par_iter()
            .map(|(start_idx, is_round_end)| {
                compute_block_data(
                    &messages[*start_idx], messages, *start_idx, *is_round_end,
                    width, body_width, ctx, *render_tick,
                )
            })
            .collect()
    } else {
        blocks_info
            .iter()
            .map(|(start_idx, is_round_end)| {
                compute_block_data(
                    &messages[*start_idx], messages, *start_idx, *is_round_end,
                    width, body_width, ctx, *render_tick,
                )
            })
            .collect()
    };

    // Apply computations sequentially — populate index and cache
    for comp in computations {
        for (key, entry) in &comp.cache_entries {
            cache.put(key.clone(), entry.clone());
        }
        index.blocks.push(MessageBlock {
            message_id: comp.message_id,
            message_start_idx: 0, // will be fixed below
            message_count: comp.message_count,
            start_line: current_line,
            line_count: comp.line_count,
        });
        current_line += comp.line_count;
    }

    // Fix message_start_idx for each block (not computed in parallel)
    let mut msg_idx = 0usize;
    for block in &mut index.blocks {
        block.message_start_idx = msg_idx;
        msg_idx += block.message_count;
    }

    index.total_lines = current_line;
}

/// Render a block and store results in the LRU cache. Returns (message_count, line_count, cache_entries_count).
fn compute_and_cache_block(
    message: &Message,
    messages: &[Message],
    start_idx: usize,
    is_round_end: bool,
    width: usize,
    body_width: usize,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    ctx: &RenderContext,
    render_tick: &mut u64,
) -> (usize, usize, usize) {
    match message.role {
        MessageRole::Assistant => {
            let mut count = 1;
            while start_idx + count < messages.len()
                && matches!(messages[start_idx + count].role, MessageRole::Tool)
            {
                count += 1;
            }

            // Render card
            let cards = render_assistant_cards(ctx, message, body_width, is_round_end);
            let mut line_count = 0;

            // Store card cache
            let cards_key = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache.put(cards_key, MessageRenderCacheEntry {
                value: MessageRenderCacheValue::Cards(cards.clone()),
                last_used_tick: *render_tick,
            });

            for (_, card_lines) in &cards {
                line_count += card_lines.len();
            }

            // Render tool calls
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

            for tc in &message.tool_calls {
                let tool_result = tool_results_by_id.get(&tc.id).copied();
                let is_expanded = ctx.expanded_tool_results.contains(&message.id);
                let (tool_lines, _regions) = tool::render_tool_call_with_result(
                    tc, tool_result, body_width, message.streaming, ctx, is_expanded,
                );

                let tool_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: body_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                };
                let regions = Vec::new();
                cache.put(tool_key, MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::ToolResult(tool_lines.clone(), regions),
                    last_used_tick: *render_tick,
                });

                line_count += tool_lines.len();
            }

            line_count += 1; // trailing empty line
            (count, line_count, 0)
        }
        MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
            let cards = render_single_card(ctx, message, body_width);
            let mut line_count = 0;
            for (_, card_lines) in &cards {
                line_count += card_lines.len();
            }
            line_count += 1; // trailing empty line

            let kind = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: body_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache.put(kind, MessageRenderCacheEntry {
                value: MessageRenderCacheValue::Cards(cards),
                last_used_tick: *render_tick,
            });

            (1, line_count, 0)
        }
        MessageRole::Tool => (1, 0, 0),
    }
}

// ---------------------------------------------------------------------------
// Render block from cache
// ---------------------------------------------------------------------------

fn render_block_from_cache(
    block: &MessageBlock,
    cache: &lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    width: usize,
    selectable_regions: &mut Vec<SelectableRegionRange>,
    ctx: &RenderContext,
    current_line_offset: &usize,
) -> Vec<Line<'static>> {
    // Find card cache entry for this block
    let cards_key = MessageRenderCacheKey {
        session_id: Uuid::default(),
        message_id: block.message_id,
        width: width.saturating_sub(2).max(1),
        is_round_end: false,
        kind: MessageRenderCacheKind::Cards,
    };

    let mut lines = Vec::new();

    if let Some(entry) = cache.peek(&cards_key) {
        match &entry.value {
            MessageRenderCacheValue::Cards(cards) => {
                for (bg, card_lines) in cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();
                        track_selectable_region(selectable_regions, card_lines, start_line);
                        lines.extend(decorate_card_lines(card_lines.clone(), *bg, 2));
                    }
                }
            }
            _ => {}
        }
    } else {
        // Cache miss — render placeholder
        lines.push(Line::from(Span::styled(
            "[cache miss]",
            Style::default().fg(ctx.palette.muted),
        )));
    }

    lines.push(Line::from(""));
    lines
}

// ---------------------------------------------------------------------------
// Card rendering
// ---------------------------------------------------------------------------

/// Render assistant message cards with markdown content and tool calls.
fn render_assistant_cards(
    ctx: &RenderContext,
    message: &Message,
    body_width: usize,
    _is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let mut card_lines = Vec::new();

    // Title bar
    let title_style = Style::default().fg(palette.accent).add_modifier(Modifier::BOLD);
    card_lines.push(Line::from(vec![
        Span::styled(" assistant ", title_style),
    ]));

    // Markdown content
    if !message.content.is_empty() {
        let md = markdown::render_markdown_text_with_width_and_cwd(
            &message.content,
            Some(body_width),
            Some(ctx.workspace_root),
        );
        for md_line in md.lines.iter() {
            card_lines.push(md_line.clone());
        }
    }

    // Reasoning content
    if !message.reasoning.is_empty() {
        card_lines.push(Line::from(Span::styled(
            "  reasoning...",
            Style::default().fg(palette.muted),
        )));
    }

    // Tool calls (shown inline, not cached separately here)
    for tc in &message.tool_calls {
        let summary = tool::render_tool_call_summary_line(tc, palette, true);
        card_lines.push(summary);
    }

    vec![(palette.panel, card_lines)]
}

/// Render a single-card message (user, system, error, shell).
fn render_single_card(
    ctx: &RenderContext,
    message: &Message,
    body_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;

    let (label, label_color, bg) = match message.role {
        MessageRole::User => (" user ", palette.text, palette.background),
        MessageRole::System => (" system ", palette.muted, palette.panel),
        MessageRole::Error => (" error ", palette.error, palette.background),
        MessageRole::Shell => (" shell ", palette.accent_soft, palette.background),
        _ => (" message ", palette.text, palette.background),
    };

    let label_style = Style::default().fg(label_color).add_modifier(Modifier::BOLD);
    let content_style = Style::default().fg(palette.text);

    let mut lines = Vec::new();
    lines.push(Line::from(vec![Span::styled(label, label_style)]));

    if !message.content.is_empty() {
        let md = markdown::render_markdown_text_with_width_and_cwd(
            &message.content,
            Some(body_width),
            Some(ctx.workspace_root),
        );
        for md_line in md.lines.iter() {
            lines.push(md_line.clone());
        }
    }

    vec![(bg, lines)]
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn build_header_lines(is_subsession: bool, palette: ThemePalette) -> Vec<Line<'static>> {
    if is_subsession {
        vec![
            Line::from(Span::styled(
                "SUBSESSION active — viewing a child session.",
                Style::default().fg(palette.accent_soft),
            )),
            Line::from(Span::styled(
                "Press Ctrl+X then Up arrow to return to the parent session.",
                Style::default().fg(palette.muted),
            )),
            Line::from(""),
        ]
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

fn decorate_card_lines(
    lines: Vec<Line<'static>>,
    bg: Color,
    indent: usize,
) -> Vec<Line<'static>> {
    let prefix = " ".repeat(indent);
    lines.into_iter().map(|line| {
        let mut new_spans = vec![Span::styled(prefix.clone(), Style::default().bg(bg))];
        for span in line.spans {
            let mut styled = span.clone();
            styled.style = styled.style.bg(bg);
            new_spans.push(styled);
        }
        Line::from(new_spans).style(Style::default().bg(bg))
    }).collect()
}

fn track_selectable_region(
    regions: &mut Vec<SelectableRegionRange>,
    card_lines: &[Line<'static>],
    start_line: usize,
) {
    let first_content = card_lines.iter().position(|l| l.spans.iter().any(|s| !s.content.is_empty()));
    let last_content = card_lines.iter().rposition(|l| l.spans.iter().any(|s| !s.content.is_empty()));
    if let (Some(first), Some(last)) = (first_content, last_content) {
        regions.push(SelectableRegionRange {
            start_line: start_line + first,
            end_line: start_line + last + 1,
            min_x: 2,
            max_x: None,
        });
    }
}

fn skip_rendered_lines(lines: Vec<Line<'static>>, render_scroll: &mut usize) -> Vec<Line<'static>> {
    if *render_scroll == 0 {
        return lines;
    }
    if *render_scroll >= lines.len() {
        *render_scroll -= lines.len();
        return Vec::new();
    }
    let skipped = lines.into_iter().skip(*render_scroll).collect();
    *render_scroll = 0;
    skipped
}
