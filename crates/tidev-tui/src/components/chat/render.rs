//! Core rendering pipeline for the chat message list.
//!
//! Orchestrates layout index updates, cache lookups, block rendering, and
//! scroll management. Messages are rendered with markdown formatting, cached
//! in an LRU, and only re-rendered when content or width changes.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use std::time::Instant;

use fancy_regex::Regex;
use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use rayon::prelude::*;
use tidev_types::message::{Message, MessageRole, COMPACTION_MESSAGE_LABEL};
use crate::chat_context::ChatContext;
use crate::theme::ThemePalette;
use chrono::Local;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use uuid::Uuid;

use crate::components::chat::layout_index::{MessageBlock, MessageLayoutIndex};
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind, MessageRenderCacheValue,
    SelectableRegionRange,
};
use crate::markdown;
use crate::diff_render::render_unified_diff_text;

use crate::components::chat::tool;

// ---------------------------------------------------------------------------
// Badge regex patterns for user-message content
// ---------------------------------------------------------------------------

/// Regex for detecting @ file/directory references.
/// Look-behind ensures @ is not preceded by word chars or backticks.
static AT_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?<![\w`])@(\.?[^\s`.,]*(?:\.[^\s`.,]+)*)").unwrap()
});

/// Regex for image badge patterns like `[100.0 KB PNG]` produced by
/// `format_image_badge()`. The type label is uppercase (PNG, JPEG, etc.).
static IMAGE_BADGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\d[\d.]*\s+(?:B|KB|MB|GB)\s+[A-Z][A-Z0-9]*\]").unwrap()
});

/// Kind of inline badge detected in user message content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageBadgeKind {
    AtReference,
    Image,
}

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
    pub spinner: &'a str,
    pub workspace_root: &'a Path,
    pub expanded_tool_results: &'a HashSet<Uuid>,
    pub expanded_tool_outputs: &'a HashMap<Uuid, String>,
    pub hovered_card: Option<Uuid>,
    pub model_display_name: &'a str,
}

// ---------------------------------------------------------------------------
// RenderOutput
// ---------------------------------------------------------------------------

/// Every piece of data produced by the rendering pipeline.
pub(crate) struct RenderOutput {
    pub lines: Vec<Line<'static>>,
    pub total_lines: usize,
    pub render_scroll: usize,
    pub effective_scroll: usize,
    pub selectable_regions: Vec<SelectableRegionRange>,
    pub card_bounds: Vec<(Uuid, usize, usize)>,
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
    spinner_start: Instant,
    hovered_card: Option<Uuid>,
    out_card_bounds: &mut Vec<(Uuid, usize, usize)>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (content_area, scrollbar_rect) = compute_content_layout(area);

    let spinner = loading_spinner(spinner_start);
    let ctx = RenderContext {
        palette,
        spinner,
        workspace_root: Path::new(""),
        expanded_tool_results,
        expanded_tool_outputs,
        hovered_card,
        model_display_name: &chat_context.model_display_name,
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
        *follow_tail,
        current_streaming_message_id,
        render_tick,
    );

    // Sync the effective scroll (updated inside messages_text for follow_tail
    // and clamping) back to the caller so that subsequent key events see the
    // correct scroll offset without a one-frame lag.
    *scroll_offset = output.effective_scroll;
    // Export card bounds for mouse hover detection.
    *out_card_bounds = output.card_bounds;

    // Render running subagent cards (at the end of the message area)
    let total_with_subagents = {
        let mut all_lines = output.lines;
        let add_lines;
        if !running_subagents.is_empty() {
            add_lines = running_subagents.len() * 2;
            for sa in running_subagents {
                let style = Style::default().fg(palette.accent_soft).add_modifier(Modifier::BOLD);
                all_lines.push(Line::from(vec![
                    Span::styled(format!(" ▶ task [{}]", sa.subagent_type), style),
                    Span::styled(format!(" {}", sa.status_text), Style::default().fg(palette.muted)),
                ]));
                all_lines.push(Line::from(Span::styled(
                    format!("   {}", sa.description),
                    Style::default().fg(palette.text),
                )));
            }
        } else {
            add_lines = 0;
        }
        let text = ratatui::text::Text::from(all_lines);
        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(ctx.palette.background))
            .scroll((output.render_scroll as u16, 0));
        frame.render_widget(paragraph, content_area);
        output.total_lines + add_lines
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

fn render_scrollbar(frame: &mut Frame, sb: Rect, scroll_offset: usize, total_lines: usize, viewport: usize, palette: ThemePalette) {
    let max_scroll = total_lines.saturating_sub(viewport);
    let scrolled = if max_scroll > 0 { (scroll_offset as f32 / max_scroll as f32).clamp(0.0, 1.0) } else { 0.0 };
    let thumb_height = ((sb.height as f32 * sb.height as f32 / total_lines.max(1) as f32).clamp(1.0, sb.height as f32)).round() as u16;
    let track_span = sb.height.saturating_sub(thumb_height);
    let thumb_pos = if track_span == 0 {
        0
    } else {
        (scrolled * track_span as f32).round() as u16
    };

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
    follow_tail: bool,
    _current_streaming_message_id: Option<Uuid>,
    render_tick: &mut u64,
) -> RenderOutput {
    let messages = chat_context.visible_messages();
    let width = width.max(1);
    let body_width = width.saturating_sub(2).max(1);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selectable_regions: Vec<SelectableRegionRange> = Vec::new();
    let mut card_bounds: Vec<(Uuid, usize, usize)> = Vec::new();

    // Header for sub-sessions
    let header_lines = build_header_lines(chat_context.parent_session_id.is_some(), ctx.palette);
    let header_line_count = header_lines.len();

    // Empty state
    if messages.is_empty() {
        lines.extend(header_lines);
        let empty_line = Line::from(Span::styled("No messages yet.", Style::default().fg(ctx.palette.muted)));
        lines.push(empty_line);
        let total = lines.len().max(1);
        return RenderOutput { lines, total_lines: total, render_scroll: 0, effective_scroll: 0, selectable_regions, card_bounds };
    }

    // Update layout index — this renders all blocks and populates the cache
    update_layout_index(index, cache, messages, width, body_width, streaming, ctx, render_tick);

    // Calculate visible range (clamp scroll and respect follow_tail)
    let total_overall_lines = header_line_count + index.total_lines;
    let viewport = viewport.max(1);
    let max_scroll = total_overall_lines.saturating_sub(viewport);
    let scroll = if follow_tail { max_scroll } else { scroll.min(max_scroll) };
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
    } else {
        render_scroll += header_line_count;
    }

    for _ in 0..padding_lines {
        lines.push(Line::from(""));
    }

    let mut current_line_offset = lines.len();
    for block in &visible_blocks {
        let next_idx = block.message_start_idx + block.message_count;
        let is_round_end = next_idx >= messages.len()
            || matches!(messages[next_idx].role, MessageRole::User);
        let block_lines = render_block_from_cache(
            block, cache, width, is_round_end, &mut selectable_regions, ctx, &current_line_offset,
            messages, &mut card_bounds,
        );
        lines.extend(block_lines);
        current_line_offset = lines.len();
    }

    RenderOutput { lines, total_lines: total_overall_lines, render_scroll, effective_scroll: scroll, selectable_regions, card_bounds }
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
            let cards = render_assistant_cards(ctx, message, messages, body_width, is_round_end);
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
                let (tool_lines, tool_regions) = tool::render_tool_call_with_result(
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
                    value: MessageRenderCacheValue::ToolResult(tool_lines.clone(), tool_regions),
                    last_used_tick: render_tick,
                }));

                line_count += tool_lines.len();
            }

            line_count += 1; // trailing empty line
            (count, line_count, cache_entries)
        }
        MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
            let cards = render_single_card(ctx, message, messages, body_width, is_round_end);
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
            let cards = render_assistant_cards(ctx, message, messages, body_width, is_round_end);
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
                let (tool_lines, tool_regions) = tool::render_tool_call_with_result(
                    tc, tool_result, body_width, message.streaming, ctx, is_expanded,
                );

                let tool_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: body_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                };
                cache.put(tool_key, MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::ToolResult(tool_lines.clone(), tool_regions),
                    last_used_tick: *render_tick,
                });

                line_count += tool_lines.len();
            }

            line_count += 1; // trailing empty line
            (count, line_count, 0)
        }
        MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
            let cards = render_single_card(ctx, message, messages, body_width, is_round_end);
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
    is_round_end: bool,
    selectable_regions: &mut Vec<SelectableRegionRange>,
    ctx: &RenderContext,
    current_line_offset: &usize,
    messages: &[Message],
    card_bounds: &mut Vec<(Uuid, usize, usize)>,
) -> Vec<Line<'static>> {
    let body_width = width.saturating_sub(2).max(1);
    let cards_key = MessageRenderCacheKey {
        session_id: Uuid::default(),
        message_id: block.message_id,
        width: body_width,
        is_round_end,
        kind: MessageRenderCacheKind::Cards,
    };

    let mut lines = Vec::new();

    // First user message: add extra blank line before it for spacing
    if messages.get(block.message_start_idx).is_some_and(|m| {
        m.role == MessageRole::User && is_first_user_message(messages, block.message_start_idx)
    }) {
        lines.push(Line::from(""));
    }

    // Render Cards entry (assistant/user/shell card)
    if let Some(entry) = cache.peek(&cards_key) {
        match &entry.value {
            MessageRenderCacheValue::Cards(cards) => {
                for (bg, card_lines) in cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();
                        track_selectable_region(selectable_regions, card_lines, start_line);
                        let adjusted_bg = if ctx.hovered_card == Some(block.message_id) {
                            ctx.palette.hover_bg(*bg)
                        } else {
                            *bg
                        };
                        lines.extend(decorate_card_lines(card_lines.clone(), adjusted_bg, 2, width));
                        let end_line = current_line_offset + lines.len();
                        card_bounds.push((block.message_id, start_line, end_line));
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

    // Render ToolResult entries (tool call output) for assistant blocks
    if block.message_count > 1 {
        let msg = &messages[block.message_start_idx];
        if matches!(msg.role, MessageRole::Assistant) {
            for tc in &msg.tool_calls {
                let tool_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: block.message_id,
                    width: body_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                };
                if let Some(entry) = cache.peek(&tool_key) {
                    match &entry.value {
                        MessageRenderCacheValue::ToolResult(tool_lines, tool_regions) => {
                            if !tool_lines.is_empty() {
                                let start_line = current_line_offset + lines.len();
                                // Determine background: hover if a tool result in this block is hovered
                                let tool_result_msg = messages[block.message_start_idx..block.message_start_idx + block.message_count]
                                    .iter()
                                    .find(|m| m.tool_call_id.as_deref() == Some(&tc.id));
                                let bg = if tool_result_msg.is_some_and(|m| ctx.hovered_card == Some(m.id)) {
                                    ctx.palette.hover_bg(ctx.palette.panel_light)
                                } else {
                                    ctx.palette.panel_light
                                };
                                lines.extend(decorate_card_lines(tool_lines.clone(), bg, 2, width));
                                // Add selectable regions from tool result, offset by the current line
                                for r in tool_regions {
                                    selectable_regions.push(SelectableRegionRange {
                                        start_line: current_line_offset + lines.len() + r.start_line,
                                        end_line: current_line_offset + lines.len() + r.end_line,
                                        min_x: r.min_x,
                                        max_x: r.max_x,
                                    });
                                }
                                let end_line = current_line_offset + lines.len();
                                if let Some(tool_msg) = tool_result_msg {
                                    card_bounds.push((tool_msg.id, start_line, end_line));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    lines.push(Line::from(""));
    lines
}

// ---------------------------------------------------------------------------
// Reasoning rendering (ported from v0.6.x utils.rs)
// ---------------------------------------------------------------------------

/// Render reasoning content with ┃ prefix, dimmed colours, and the
/// Thinking:/Thought: label.  Matches the old implementation exactly.
fn render_reasoning_lines(
    ctx: &RenderContext,
    reasoning: &str,
    body_width: usize,
    is_streaming: bool,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    let dimmed_color = crate::theme::mix_colors(palette.muted, palette.background, 0.5);
    let label_style = Style::default().fg(dimmed_color);
    let label_italic_style = Style::default().fg(dimmed_color).add_modifier(Modifier::ITALIC);
    let body_style = Style::default().fg(dimmed_color);

    // Label line: ┃ Thinking: or ┃ Thought:
    let label = if is_streaming { "Thinking:" } else { "Thought:" };
    lines.push(Line::from(vec![
        Span::styled("┃ ", label_style),
        Span::styled(label, label_italic_style),
    ]));

    if reasoning.trim().is_empty() {
        return lines;
    }

    let content_width = body_width.saturating_sub(2).max(1);
    let rendered = markdown::render_markdown_text_with_width_and_cwd(
        reasoning,
        Some(content_width),
        Some(ctx.workspace_root),
    );

    // Skip leading blank lines
    let mut rendered_lines = rendered.lines.into_iter();
    let mut first_line = rendered_lines.next();
    while let Some(ref line) = first_line {
        if line.spans.iter().all(|s| s.content.trim().is_empty() && s.style == Style::default()) {
            first_line = rendered_lines.next();
        } else {
            break;
        }
    }

    // First content line
    if let Some(line) = first_line {
        let mut spans = vec![Span::styled("┃ ", label_style)];
        for mut span in line.spans {
            if let Some(fg) = span.style.fg {
                span.style = span.style.fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            spans.push(span);
        }
        lines.push(Line::from(spans));
    }

    // Subsequent lines
    for line in rendered_lines {
        let mut spans = vec![Span::styled("┃ ", label_style)];
        for mut span in line.spans {
            if let Some(fg) = span.style.fg {
                span.style = span.style.fg(crate::theme::mix_colors(fg, palette.background, 0.4));
            } else {
                span.style = span.style.patch(body_style);
            }
            spans.push(span);
        }
        lines.push(Line::from(spans));
    }

    lines
}

// ---------------------------------------------------------------------------
// Card rendering
// ---------------------------------------------------------------------------

/// Render assistant message cards with reasoning, content (diff or markdown),
/// and a metadata footer at round end.  No title bar — the body lines begin
/// directly.  Margin blank lines are added before and after the body.
fn render_assistant_cards(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    body_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let body_lines = render_assistant_body_lines(ctx, message, messages, body_width, is_round_end);

    let mut lines_with_margin = Vec::new();
    lines_with_margin.push(Line::from(""));
    lines_with_margin.extend(body_lines);
    lines_with_margin.push(Line::from(""));

    vec![(palette.background, lines_with_margin)]
}

/// Render the inner body lines of an assistant message card.
/// No title bar, no margin lines — just reasoning, content, footer.
fn render_assistant_body_lines(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    body_width: usize,
    is_round_end: bool,
) -> Vec<Line<'static>> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    // 1. Reasoning (with ┃ prefix, dimmed colours, exactly like old code)
    if !message.reasoning.trim().is_empty() {
        lines.extend(render_reasoning_lines(ctx, &message.reasoning, body_width, message.streaming));
        if !message.content.trim().is_empty() {
            lines.push(Line::from(""));
        }
    }

    // 2. Content — try unified diff first, fall back to markdown
    if !message.content.is_empty() {
        if let Some((diff_lines, _)) =
            render_unified_diff_text(&message.content, body_width, palette, 4)
        {
            for dl in &diff_lines {
                lines.push(dl.clone());
            }
        } else {
            let md = markdown::render_markdown_text_with_width_and_cwd(
                &message.content,
                Some(body_width),
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

    // 4. Metadata footer at round end (model · duration · t/s · time · mode)
    if is_round_end && !message.streaming && message.tool_calls.is_empty() {
        let mut parts: Vec<String> = Vec::new();

        // Model display name (resolve via config in old code — use model_id as fallback)
        if let Some(ref model_id) = message.model_id {
            parts.push(ctx.model_display_name.to_string());
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
            parts.push(completed.with_timezone(&Local).format("%H:%M:%S").to_string());
        }

        // Mode
        if let Some(mode) = message.mode {
            parts.push(mode.title().to_string());
        }

        if !parts.is_empty() {
            let suffix = parts.join(" · ");
            let text_width = UnicodeWidthStr::width(suffix.as_str());
            let padding = body_width.saturating_sub(text_width);
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
    body_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;

    let display_content = strip_system_reminder_tags(&message.content);
    let mut content_lines = render_text_body_lines(ctx, &display_content, body_width.saturating_sub(2));
    apply_badge_styling(&mut content_lines, palette);

    let mode_color = message.mode.map_or(palette.accent, |m| match m {
        tidev_types::prompts::SessionMode::Build => palette.mode_build,
        tidev_types::prompts::SessionMode::Plan => palette.mode_plan,
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

/// Render an error message card with ! prefix, reasoning (if any),
/// and panel_light background.  Wrapped with leading/trailing empty lines.
fn render_error_card(
    ctx: &RenderContext,
    message: &Message,
    body_width: usize,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let mut lines = Vec::new();

    // 1. Reasoning (if any)
    if !message.reasoning.trim().is_empty() {
        lines.extend(render_reasoning_lines(ctx, &message.reasoning, body_width, message.streaming));
        lines.push(Line::from(""));
    }

    // 2. Error text
    let error_text = if message.content.trim().is_empty() {
        "Request cancelled.".to_string()
    } else {
        message.content.clone()
    };

    let error_style = Style::default().fg(palette.error);
    let prefix_style = Style::default().fg(palette.error).add_modifier(Modifier::BOLD);
    let text_width = body_width.saturating_sub(2).max(1);

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

    // 3. Wrap with leading/trailing empty lines, use panel_light background
    let mut card = Vec::new();
    card.push(Line::from(""));
    card.extend(lines);
    card.push(Line::from(""));
    vec![(palette.panel_light, card)]
}

/// Render a system message card (handles compaction, instructions, generic).
fn render_system_card(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    body_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    let palette = ctx.palette;
    let content = &message.content;

    // Instruction loading message (single line with Nerd Font icon)
    if content.starts_with("Loaded instructions from")
        || (content.starts_with("Loaded ")
            && content.contains(" instruction files:"))
    {
        let line = Line::from(vec![
            Span::styled("󱁤 ", Style::default().fg(palette.accent_soft)),
            Span::styled(
                content.clone(),
                Style::default().fg(palette.text).add_modifier(Modifier::ITALIC),
            ),
        ]);
        return vec![(palette.background, vec![line])];
    }

    // Compaction message
    if content.starts_with(COMPACTION_MESSAGE_LABEL) {
        let summary = content.split_once("\n\n").map(|(_, s)| s).unwrap_or("").trim();
        let mut lines = Vec::new();
        lines.push(Line::from(""));
        lines.push(render_compaction_divider_line(COMPACTION_MESSAGE_LABEL, body_width, palette));
        if !summary.is_empty() {
            lines.push(Line::from(""));
            let md = markdown::render_markdown_text_with_width_and_cwd(
                summary,
                Some(body_width),
                Some(ctx.workspace_root),
            );
            for md_line in md.lines.iter() {
                lines.push(md_line.clone());
            }
        }
        // Metadata footer for compaction (same style as assistant)
        if is_round_end && !message.streaming {
            let mut parts: Vec<String> = Vec::new();
            if let Some(ref model_id) = message.model_id {
                parts.push(model_id.clone());
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
                parts.push(completed.with_timezone(&Local).format("%H:%M:%S").to_string());
            }
            if let Some(mode) = message.mode {
                parts.push(mode.title().to_string());
            }
            if !parts.is_empty() {
                let suffix = parts.join(" · ");
                let text_width = UnicodeWidthStr::width(suffix.as_str());
                let padding = body_width.saturating_sub(text_width);
                lines.push(Line::from(Span::styled(
                    format!("{}{}", " ".repeat(padding), suffix),
                    Style::default().fg(palette.accent_soft),
                )));
            }
        }
        lines.push(Line::from(""));
        return vec![(palette.background, lines)];
    }

    // Generic system message: render as markdown with margins
    let content_lines = render_text_body_lines(ctx, content, body_width);
    let mut lines = Vec::new();
    lines.push(Line::from(""));
    lines.extend(content_lines);
    lines.push(Line::from(""));
    vec![(palette.background, lines)]
}

/// Render a single-card message (dispatches by role).
fn render_single_card(
    ctx: &RenderContext,
    message: &Message,
    messages: &[Message],
    body_width: usize,
    is_round_end: bool,
) -> Vec<(Color, Vec<Line<'static>>)> {
    match message.role {
        MessageRole::User | MessageRole::Shell => render_user_shell_card(ctx, message, body_width),
        MessageRole::Error => render_error_card(ctx, message, body_width),
        MessageRole::System => render_system_card(ctx, message, messages, body_width, is_round_end),
        _ => {
            let palette = ctx.palette;
            let content_lines = render_text_body_lines(ctx, &message.content, body_width);
            vec![(palette.background, content_lines)]
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for card rendering
// ---------------------------------------------------------------------------

/// Render text body lines with markdown, returning "(empty)" if blank.
fn render_text_body_lines(
    ctx: &RenderContext,
    text: &str,
    body_width: usize,
) -> Vec<Line<'static>> {
    if text.trim().is_empty() {
        vec![Line::from(Span::styled(
            "(empty)",
            Style::default().fg(ctx.palette.muted),
        ))]
    } else {
        let md = markdown::render_markdown_text_with_width_and_cwd(
            text,
            Some(body_width),
            Some(ctx.workspace_root),
        );
        md.lines.iter().map(|l| l.clone()).collect()
    }
}

/// Check if the message at `start_idx` is the first User message in `messages`.
fn is_first_user_message(messages: &[Message], start_idx: usize) -> bool {
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
    width: usize,
) -> Vec<Line<'static>> {
    let bg_style = Style::default().bg(bg);
    let prefix = " ".repeat(indent);
    lines.into_iter().map(|line| {
        let has_visual_prefix = line.spans.first().is_some_and(|s| s.content == "┃ ");
        let mut spans = if has_visual_prefix {
            Vec::with_capacity(line.spans.len() + 1)
        } else {
            vec![Span::styled(prefix.clone(), bg_style)]
        };
        for mut span in line.spans {
            if span.style.bg.is_none() {
                span.style = span.style.bg(bg);
            }
            spans.push(span);
        }
        let used_width: usize = spans.iter().map(|s| UnicodeWidthStr::width(s.content.as_ref())).sum();
        if used_width < width {
            spans.push(Span::styled(" ".repeat(width.saturating_sub(used_width)), bg_style));
        }
        Line::from(spans).style(bg_style)
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

// ---------------------------------------------------------------------------
// Spinner
// ---------------------------------------------------------------------------

fn loading_spinner(spinner_start: Instant) -> &'static str {
    const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
    const FRAME_DURATION_MS: u128 = 100;
    let elapsed = spinner_start.elapsed().as_millis();
    let frame_index = (elapsed / FRAME_DURATION_MS) as usize;
    FRAMES[frame_index % FRAMES.len()]
}

// ---------------------------------------------------------------------------
// Text wrapping utilities (ported from v0.6.x render/render.rs)
// ---------------------------------------------------------------------------

/// Expand tab characters to spaces using configurable tab stops.
/// `unicode-width` measures `\t` as 0 columns, but terminals render it as
/// multiple spaces. Expanding tabs at parse time prevents width mismatches.
fn expand_tabs(text: &str, tab_width: usize) -> String {
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

/// Wrap `text` into at most `max_lines` lines of `max_width` columns each.
/// Newlines are collapsed into spaces. Word boundaries are preferred for
/// line breaks; hard-breaks are used when a single word exceeds max_width.
fn wrap_text_lines(text: &str, max_width: usize, max_lines: usize) -> Vec<String> {
    if max_width == 0 || max_lines == 0 {
        return vec![];
    }

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

        if lines.len() == max_lines - 1 {
            if remaining_width > max_width {
                lines.push(shorten_by_width(remaining, max_width));
            } else {
                lines.push(remaining.to_string());
            }
            break;
        }

        if remaining_width <= max_width {
            lines.push(remaining.to_string());
            break;
        }

        let mut width_so_far: usize = 0;
        let mut break_pos: Option<usize> = None;
        let mut hard_break: usize = 0;

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
                remaining = remaining[sp + 1..].trim_start();
            }
        } else if hard_break > 0 && hard_break < remaining.len() {
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

// ---------------------------------------------------------------------------
// Compaction divider (ported from v0.6.x tool.rs)
// ---------------------------------------------------------------------------

/// Render a centered divider line with the compaction label, e.g.
/// `─── COMPACTED ───`.
fn render_compaction_divider_line(label: &str, width: usize, palette: ThemePalette) -> Line<'static> {
    let label_width = UnicodeWidthStr::width(label);
    if width <= label_width.saturating_add(2) {
        return Line::from(Span::styled(
            label.to_string(),
            Style::default().fg(palette.accent_soft),
        ));
    }

    let remaining = width - label_width - 2;
    let left = remaining / 2;
    let right = remaining - left;

    let mut spans = Vec::new();
    if left > 0 {
        spans.push(Span::styled(
            "─".repeat(left),
            Style::default().fg(palette.muted),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        label.to_string(),
        Style::default().fg(palette.accent_soft),
    ));
    spans.push(Span::raw(" "));
    if right > 0 {
        spans.push(Span::styled(
            "─".repeat(right),
            Style::default().fg(palette.muted),
        ));
    }

    Line::from(spans)
}

// ---------------------------------------------------------------------------
// Badge styling (ported from v0.6.x content.rs)
// ---------------------------------------------------------------------------

/// Post-process rendered markdown lines to replace badge text with styled spans.
/// Scans each span for `@path` and `[size TYPE]` patterns and splits the span
/// at badge boundaries, applying bold accent for AtReference and white-on-teal
/// for Image badges.
fn apply_badge_styling(lines: &mut [Line<'static>], palette: ThemePalette) {
    for line in lines.iter_mut() {
        let old_spans: Vec<Span<'static>> = line.spans.drain(..).collect();
        for span in old_spans {
            let text = span.content.to_string();
            let mut parts: Vec<(String, Style)> = Vec::new();
            let mut offset = 0usize;

            let mut badges_in_span: Vec<(usize, usize, MessageBadgeKind)> = Vec::new();

            // @ references
            {
                let mut search_start = 0;
                while let Ok(Some(caps)) = AT_REF_RE.captures(&text[search_start..]) {
                    if let Some(path_match) = caps.get(1) {
                        if path_match.as_str().is_empty() {
                            break;
                        }
                        let abs_start = search_start + path_match.start() - 1;
                        let abs_end = search_start + path_match.end();
                        badges_in_span.push((abs_start, abs_end, MessageBadgeKind::AtReference));
                        search_start += path_match.end();
                    } else {
                        break;
                    }
                }
            }

            // Image badge patterns like `[100KB PNG]`
            {
                let mut search_start = 0;
                while let Ok(Some(m)) = IMAGE_BADGE_RE.find(&text[search_start..]) {
                    let abs_start = search_start + m.start();
                    let abs_end = search_start + m.end();
                    badges_in_span.push((abs_start, abs_end, MessageBadgeKind::Image));
                    search_start += m.end();
                }
            }

            badges_in_span.sort_by_key(|b| b.0);

            if badges_in_span.is_empty() {
                parts.push((text, span.style));
            } else {
                for (start, end, kind) in &badges_in_span {
                    if *start > offset {
                        parts.push((text[offset..*start].to_string(), span.style));
                    }
                    let badge_style = match kind {
                        MessageBadgeKind::AtReference => {
                            Style::default().fg(palette.accent).add_modifier(Modifier::BOLD)
                        }
                        MessageBadgeKind::Image => Style::default()
                            .bg(palette.selection_bg)
                            .fg(palette.selection_fg)
                            .add_modifier(Modifier::BOLD),
                    };
                    parts.push((text[*start..*end].to_string(), badge_style));
                    offset = *end;
                }
                if offset < text.len() {
                    parts.push((text[offset..].to_string(), span.style));
                }
            }

            for (content, style) in parts {
                line.spans.push(Span::styled(content, style));
            }
        }
    }
}


