use super::*;

use std::collections::HashMap;
use std::time::Instant;

use crate::chat_context::ChatContext;
use ratatui::prelude::Style;
use ratatui::text::{Line, Span};
use rayon::prelude::*;
use tidev_types::message::{Message, MessageAttachment, MessageRole};
use tidev_types::tools::canonical_tool_name;
use uuid::Uuid;

use crate::components::chat::layout_index::{MessageBlock, MessageLayoutIndex};
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
    MessageRenderCacheValue, SelectableRegionRange,
};
use crate::components::chat::render_mod::cards::is_first_user_message;
use crate::components::chat::render_mod::subagent::{
    count_running_subagent_card_lines, render_running_subagent_lines,
};
use crate::components::chat::render_mod::utils::build_header_lines;
use crate::components::chat::render_mod::utils::{
    IMAGE_BADGE_RE, decorate_card_lines, track_selectable_region, wrap_text_lines,
};
use crate::components::chat::tool;

/// Render a block and store results in the LRU cache. Returns (message_count, line_count, cache_entries_count).
fn compute_and_cache_block(
    message: &Message,
    messages: &[Message],
    start_idx: usize,
    is_round_end: bool,
    content_width: usize,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    ctx: &RenderContext,
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
            let cards = render_assistant_cards(ctx, message, messages, content_width, is_round_end);
            let mut line_count = 0;

            // Store card cache
            let cards_key = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: content_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache.put(
                cards_key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                },
            );

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
                    tc,
                    tool_result,
                    content_width,
                    message.streaming,
                    ctx,
                    is_expanded,
                );

                let tool_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: content_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                };
                cache.put(
                    tool_key,
                    MessageRenderCacheEntry {
                        value: MessageRenderCacheValue::ToolResult(
                            tool_lines.clone(),
                            tool_regions,
                        ),
                    },
                );

                // Use the generic tool call line count by default, but adjust
                // for running subagent cards which are taller at render time.
                let mut tc_line_count = tool_lines.len();
                if tc.name == "task"
                    && tool_result.is_none()
                    && let Some(info) = ctx
                        .running_subagents
                        .iter()
                        .find(|s| s.tool_call_id == tc.id)
                {
                    tc_line_count = count_running_subagent_card_lines(info, content_width);
                }
                line_count += tc_line_count;
            }

            line_count += 1; // trailing empty line
            (count, line_count, 0)
        }
        MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
            let cards = render_single_card(ctx, message, content_width, is_round_end);
            let mut line_count = 0;
            for (_, card_lines) in &cards {
                line_count += card_lines.len();
            }
            line_count += 1; // trailing empty line

            let kind = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: content_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache.put(
                kind,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards),
                },
            );

            (1, line_count, 0)
        }
        MessageRole::Tool => {
            let cards = render_single_card(ctx, message, content_width, is_round_end);
            let mut line_count = 0;
            for (_, card_lines) in &cards {
                line_count += card_lines.len();
            }
            line_count += 1; // trailing empty line

            let kind = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: message.id,
                width: content_width,
                is_round_end,
                kind: MessageRenderCacheKind::Cards,
            };
            cache.put(
                kind,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards),
                },
            );

            (1, line_count, 0)
        }
    }
}

/// Scan card lines for image badge patterns and record their positions
/// along with the associated message attachment index.
fn scan_image_badges(
    card_lines: &[Line<'static>],
    msg: &Message,
    card_start_line: usize,
) -> Vec<ImageBadgeInfo> {
    let image_attachments: Vec<&MessageAttachment> = msg
        .attachments
        .iter()
        .filter(|a| matches!(a, MessageAttachment::Image { .. }))
        .collect();
    if image_attachments.is_empty() {
        return Vec::new();
    }

    let mut infos = Vec::new();
    let mut url_idx = 0;
    for (line_offset, line) in card_lines.iter().enumerate() {
        let line_text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let mut search_start = 0;
        while let Ok(Some(m)) = IMAGE_BADGE_RE.find(&line_text[search_start..]) {
            let abs_start = search_start + m.start();
            let abs_end = search_start + m.end();
            if url_idx < image_attachments.len() {
                infos.push(ImageBadgeInfo {
                    card_start_line,
                    badge_line_offset: line_offset,
                    badge_col: abs_start,
                    badge_width: abs_end - abs_start,
                    message_id: msg.id,
                    attachment_index: url_idx,
                });
            }
            url_idx += 1;
            search_start += m.end();
        }
    }
    infos
}

#[allow(clippy::too_many_arguments)]
fn render_block_from_cache(
    block: &MessageBlock,
    cache: &lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    geom: &CardGeom,
    is_round_end: bool,
    selectable_regions: &mut Vec<SelectableRegionRange>,
    ctx: &RenderContext,
    current_line_offset: &usize,
    messages: &[Message],
    card_bounds: &mut Vec<(Uuid, usize, usize)>,
    inline_running_card_ranges: &mut Vec<InlineRunningCardRange>,
    image_badge_infos: &mut Vec<ImageBadgeInfo>,
    thinking_header_infos: &mut Vec<(Uuid, usize)>,
) -> Vec<Line<'static>> {
    let content_width = geom.content();
    let cards_key = MessageRenderCacheKey {
        session_id: Uuid::default(),
        message_id: block.message_id,
        width: content_width,
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

    let role = messages[block.message_start_idx].role.clone();

    // Render Cards entry (assistant/user/shell card)
    if let Some(entry) = cache.peek(&cards_key) {
        if let MessageRenderCacheValue::Cards(cards) = &entry.value {
            for (bg, card_lines) in cards {
                if !card_lines.is_empty() {
                    let start_line = current_line_offset + lines.len();
                    // Track thinking header position for assistant messages with reasoning.
                    if matches!(role, MessageRole::Assistant) {
                        let msg = &messages[block.message_start_idx];
                        if !msg.reasoning.trim().is_empty() {
                            thinking_header_infos.push((msg.id, start_line));
                        }
                    }
                    track_selectable_region(selectable_regions, card_lines, start_line);
                    let show_hover = ctx.hovered_card == Some(block.message_id)
                        && matches!(role, MessageRole::User | MessageRole::Shell);
                    let adjusted_bg = if show_hover {
                        ctx.palette.hover_bg(*bg)
                    } else {
                        *bg
                    };
                    lines.extend(decorate_card_lines(card_lines.clone(), adjusted_bg, geom));
                    let end_line = current_line_offset + lines.len();
                    if !matches!(role, MessageRole::Assistant) {
                        card_bounds.push((block.message_id, start_line, end_line));
                        let msg = &messages[block.message_start_idx];
                        image_badge_infos.extend(scan_image_badges(card_lines, msg, start_line));
                    }
                }
            }
        }
    } else {
        // Cache miss — render directly instead of showing a placeholder.
        let msg = &messages[block.message_start_idx];
        let cards = match msg.role {
            MessageRole::Assistant => {
                render_assistant_cards(ctx, msg, messages, content_width, is_round_end)
            }
            _ => render_single_card(ctx, msg, content_width, is_round_end),
        };
        for (bg, card_lines) in &cards {
            if !card_lines.is_empty() {
                let start_line = current_line_offset + lines.len();
                track_selectable_region(selectable_regions, card_lines, start_line);
                let show_hover = ctx.hovered_card == Some(block.message_id)
                    && matches!(role, MessageRole::User | MessageRole::Shell);
                let adjusted_bg = if show_hover {
                    ctx.palette.hover_bg(*bg)
                } else {
                    *bg
                };
                lines.extend(decorate_card_lines(card_lines.clone(), adjusted_bg, geom));
                let end_line = current_line_offset + lines.len();
                if !matches!(role, MessageRole::Assistant) {
                    card_bounds.push((block.message_id, start_line, end_line));
                    image_badge_infos.extend(scan_image_badges(card_lines, msg, start_line));
                }
            }
        }
    }

    // Render ToolResult entries (tool call output) for assistant blocks
    let msg = &messages[block.message_start_idx];
    if matches!(msg.role, MessageRole::Assistant) && !msg.tool_calls.is_empty() {
        // Blank line between assistant body and tool calls
        lines.push(Line::from(""));
        // Build tool_results_by_id for completion checks
        let tool_results_by_id: HashMap<String, &Message> = {
            let mut map = HashMap::new();
            let mut j = block.message_start_idx + 1;
            while j < messages.len() && j < block.message_start_idx + block.message_count {
                if matches!(messages[j].role, MessageRole::Tool)
                    && let Some(id) = &messages[j].tool_call_id
                {
                    map.insert(id.clone(), &messages[j]);
                }
                j += 1;
            }
            map
        };

        for tc in &msg.tool_calls {
            // Check for running subagent (pending task tool call) — render inline card
            if tc.name == "task"
                && !tool_results_by_id.contains_key(&tc.id)
                && let Some(exec_index) = ctx
                    .running_subagents
                    .iter()
                    .position(|s| s.tool_call_id == tc.id)
            {
                let info = &ctx.running_subagents[exec_index];
                let running_lines = render_running_subagent_lines(info, content_width, ctx.palette);
                let start_line = current_line_offset + lines.len();
                let mut card_bg = ctx.palette.panel;
                if ctx.hovered_inline_subagent == Some(exec_index) {
                    card_bg = ctx.palette.hover_bg(card_bg);
                }
                let decorated = decorate_card_lines(running_lines, card_bg, geom);
                lines.extend(decorated);
                let end_line = current_line_offset + lines.len();
                inline_running_card_ranges.push(InlineRunningCardRange {
                    execution_index: exec_index,
                    start_line,
                    end_line,
                });
                continue;
            }

            let tool_key = MessageRenderCacheKey {
                session_id: Uuid::default(),
                message_id: block.message_id,
                width: content_width,
                is_round_end,
                kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
            };
            let (tool_lines, tool_regions): (Vec<Line<'static>>, Vec<SelectableRegionRange>) =
                if let Some(entry) = cache.peek(&tool_key) {
                    match &entry.value {
                        MessageRenderCacheValue::ToolResult(tl, tr) => (tl.clone(), tr.clone()),
                        _ => (Vec::new(), Vec::new()),
                    }
                } else {
                    // Cache miss — render directly.
                    let tool_result = tool_results_by_id.get(&tc.id).copied();
                    let is_expanded = ctx.expanded_tool_results.contains(&block.message_id);
                    tool::render_tool_call_with_result(
                        tc,
                        tool_result,
                        content_width,
                        msg.streaming,
                        ctx,
                        is_expanded,
                    )
                };
            if !tool_lines.is_empty() {
                let start_line = current_line_offset + lines.len();
                let tool_result_msg = messages
                    [block.message_start_idx..block.message_start_idx + block.message_count]
                    .iter()
                    .find(|m| m.tool_call_id.as_deref() == Some(&tc.id));
                let is_hovered = tool_result_msg.is_some_and(|m| {
                    ctx.hovered_card == Some(m.id) && {
                        let canonical = canonical_tool_name(&tc.name);
                        match canonical {
                            Some("read" | "grep" | "glob" | "skill" | "question" | "todowrite") => {
                                false
                            }
                            Some("write" | "edit" | "apply_patch") => {
                                m.metadata.diff.is_none()
                                    && m.content.lines().count() > tool::TOOL_OUTPUT_PREVIEW_LINES
                            }
                            _ => m.content.lines().count() > tool::TOOL_OUTPUT_PREVIEW_LINES,
                        }
                    }
                });
                let bg = if is_hovered {
                    ctx.palette.hover_bg(ctx.palette.panel_light)
                } else {
                    ctx.palette.panel_light
                };
                lines.extend(decorate_card_lines(tool_lines, bg, geom));
                for r in &tool_regions {
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
    }

    lines
}

pub(super) fn update_layout_index(
    index: &mut MessageLayoutIndex,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    messages: &[Message],
    geom: &CardGeom,
    ctx: &RenderContext,
) {
    let content_width = geom.content();
    let needs_full = index.needs_full_rebuild(messages.len(), geom.total);

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
                    || !matches!(messages[next_idx].role, MessageRole::Tool);
                let old_line_count = block.line_count;

                let (_msg_count, new_line_count, _) = compute_and_cache_block(
                    message,
                    messages,
                    start_idx,
                    is_round_end,
                    content_width,
                    cache,
                    ctx,
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
    index.reset(geom.total);

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
        let is_round_end =
            next_idx >= messages.len() || !matches!(messages[next_idx].role, MessageRole::Tool);
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
        content_width: usize,
        ctx: &RenderContext,
    ) -> BlockComputation {
        let (message_count, line_count, cache_entries) = match message.role {
            MessageRole::Assistant => {
                let mut count = 1;
                while start_idx + count < messages.len()
                    && matches!(messages[start_idx + count].role, MessageRole::Tool)
                {
                    count += 1;
                }
                let cards =
                    render_assistant_cards(ctx, message, messages, content_width, is_round_end);
                let mut line_count = 0;
                let mut cache_entries = Vec::new();

                let cards_key = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: content_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::Cards,
                };
                cache_entries.push((
                    cards_key,
                    MessageRenderCacheEntry {
                        value: MessageRenderCacheValue::Cards(cards.clone()),
                    },
                ));

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
                        tc,
                        tool_result,
                        content_width,
                        message.streaming,
                        ctx,
                        is_expanded,
                    );

                    let tool_key = MessageRenderCacheKey {
                        session_id: Uuid::default(),
                        message_id: message.id,
                        width: content_width,
                        is_round_end,
                        kind: MessageRenderCacheKind::ToolCall(tc.id.clone()),
                    };
                    cache_entries.push((
                        tool_key,
                        MessageRenderCacheEntry {
                            value: MessageRenderCacheValue::ToolResult(
                                tool_lines.clone(),
                                tool_regions,
                            ),
                        },
                    ));

                    // Adjust line count for running subagent cards
                    let mut tc_line_count = tool_lines.len();
                    if tc.name == "task"
                        && tool_result.is_none()
                        && let Some(info) = ctx
                            .running_subagents
                            .iter()
                            .find(|s| s.tool_call_id == tc.id)
                    {
                        tc_line_count = count_running_subagent_card_lines(info, content_width);
                    }
                    line_count += tc_line_count;
                }

                line_count += 1; // trailing empty line
                (count, line_count, cache_entries)
            }
            MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
                let cards = render_single_card(ctx, message, content_width, is_round_end);
                let mut line_count = 0;
                for (_, card_lines) in &cards {
                    line_count += card_lines.len();
                }

                let kind = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: content_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::Cards,
                };
                let cache_entries = vec![(
                    kind,
                    MessageRenderCacheEntry {
                        value: MessageRenderCacheValue::Cards(cards),
                    },
                )];

                (1, line_count, cache_entries)
            }
            MessageRole::Tool => {
                let cards = render_tool_card(ctx, message, content_width);
                let mut line_count = 0;
                for (_, card_lines) in &cards {
                    line_count += card_lines.len();
                }

                let kind = MessageRenderCacheKey {
                    session_id: Uuid::default(),
                    message_id: message.id,
                    width: content_width,
                    is_round_end,
                    kind: MessageRenderCacheKind::Cards,
                };
                let cache_entries = vec![(
                    kind,
                    MessageRenderCacheEntry {
                        value: MessageRenderCacheValue::Cards(cards),
                    },
                )];

                (1, line_count, cache_entries)
            }
        };

        BlockComputation {
            message_id: message.id,
            message_count,
            line_count,
            cache_entries,
        }
    }

    // Compute and cache each block (parallelised via rayon when >4 blocks).
    let mut current_line = 0usize;
    let computations: Vec<BlockComputation> = if blocks_info.len() > 4 {
        blocks_info
            .par_iter()
            .map(|(start_idx, is_round_end)| {
                compute_block_data(
                    &messages[*start_idx],
                    messages,
                    *start_idx,
                    *is_round_end,
                    content_width,
                    ctx,
                )
            })
            .collect()
    } else {
        blocks_info
            .iter()
            .map(|(start_idx, is_round_end)| {
                compute_block_data(
                    &messages[*start_idx],
                    messages,
                    *start_idx,
                    *is_round_end,
                    content_width,
                    ctx,
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

#[allow(clippy::too_many_arguments)]
pub(super) fn messages_text(
    chat_context: &ChatContext,
    index: &mut MessageLayoutIndex,
    cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    ctx: &RenderContext,
    geom: CardGeom,
    scroll: usize,
    viewport: usize,
    follow_tail: bool,
    retrying_hint: &Option<(u32, u32, String, Instant)>,
) -> RenderOutput {
    let messages = chat_context.visible_messages();
    let content_width = geom.content();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut selectable_regions: Vec<SelectableRegionRange> = Vec::new();
    let mut card_bounds: Vec<(Uuid, usize, usize)> = Vec::new();
    let mut inline_running_card_ranges: Vec<InlineRunningCardRange> = Vec::new();
    let mut image_badge_infos: Vec<ImageBadgeInfo> = Vec::new();
    let mut thinking_header_infos: Vec<(Uuid, usize)> = Vec::new();

    // Header for sub-sessions
    let header_lines = build_header_lines(chat_context.parent_session_id.is_some(), ctx.palette);
    let header_line_count = header_lines.len();

    // Empty state
    if messages.is_empty() {
        lines.extend(header_lines);
        let empty_line = Line::from(Span::styled(
            "No messages yet.",
            Style::default().fg(ctx.palette.muted),
        ));
        lines.push(empty_line);
        let total = lines.len().max(1);
        return RenderOutput {
            lines,
            total_lines: total,
            render_scroll: 0,
            effective_scroll: 0,
            selectable_regions,
            card_bounds,
            inline_running_card_ranges,
            image_badge_infos,
            thinking_header_infos: Vec::new(),
        };
    }

    // Update layout index — this renders all blocks and populates the cache
    update_layout_index(index, cache, messages, &geom, ctx);

    // Pre-compute retrying hint lines (if any) so we can include its height
    // in the scroll calculation before determining which blocks are visible.
    let mut precomputed_hint_lines: Vec<Line<'static>> = Vec::new();
    if let Some((attempt, max_attempts, reason, deadline)) = retrying_hint.as_ref() {
        let now = Instant::now();
        let remaining = if *deadline > now {
            deadline.duration_since(now).as_secs()
        } else {
            0
        };

        let retry_after_str = format!("Retrying in {remaining}s");
        let msg = format!("Retrying ({}/{}): {}", attempt, max_attempts, reason);

        let text_width = content_width.max(1);
        let palette = ctx.palette;
        let mut retry_lines = Vec::new();

        // Wrap the retry message with word-wrap
        let wrapped = wrap_text_lines(&msg, text_width, usize::MAX);
        for (i, line) in wrapped.iter().enumerate() {
            let prefix = if i == 0 { "⟳" } else { " " };
            retry_lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(palette.accent_soft)),
                Span::styled(format!(" {line}"), Style::default().fg(palette.text)),
            ]));
        }

        // Countdown line
        retry_lines.push(Line::from(vec![
            Span::styled("⟳", Style::default().fg(palette.accent_soft)),
            Span::styled(
                format!(" {retry_after_str}"),
                Style::default().fg(palette.muted),
            ),
        ]));

        // Wrap in card with padding (same style as error messages)
        let mut card_lines = Vec::new();
        card_lines.push(Line::from(""));
        card_lines.extend(retry_lines);
        card_lines.push(Line::from(""));

        precomputed_hint_lines = decorate_card_lines(card_lines, palette.panel_light, &geom);
    }

    let retry_hint_height = precomputed_hint_lines.len();

    // Calculate visible range (clamp scroll and respect follow_tail)
    let mut total_overall_lines = header_line_count + index.total_lines + retry_hint_height;
    let viewport = viewport.max(1);
    let max_scroll = total_overall_lines.saturating_sub(viewport);
    let mut scroll = if follow_tail {
        max_scroll
    } else {
        scroll.min(max_scroll)
    };
    let mut message_scroll = scroll.saturating_sub(header_line_count);

    // Find visible blocks
    let mut visible_blocks = index.find_visible_blocks(message_scroll, viewport);

    // Inter-block spacing: account for spacer lines in total and scroll.
    let num_spacers = visible_blocks.len().saturating_sub(1);
    total_overall_lines += num_spacers;
    if follow_tail {
        scroll = total_overall_lines.saturating_sub(viewport);
        message_scroll = scroll.saturating_sub(header_line_count);
        visible_blocks = index.find_visible_blocks(message_scroll, viewport);
    }

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
    for (block_idx, block) in visible_blocks.iter().enumerate() {
        let next_idx = block.message_start_idx + block.message_count;
        let is_round_end =
            next_idx >= messages.len() || !matches!(messages[next_idx].role, MessageRole::Tool);
        let block_lines = render_block_from_cache(
            block,
            cache,
            &geom,
            is_round_end,
            &mut selectable_regions,
            ctx,
            &current_line_offset,
            messages,
            &mut card_bounds,
            &mut inline_running_card_ranges,
            &mut image_badge_infos,
            &mut thinking_header_infos,
        );
        lines.extend(block_lines);
        current_line_offset = lines.len();
        // Neutral inter-block spacing between every pair of blocks.
        if block_idx + 1 < visible_blocks.len() {
            lines.push(Line::from(""));
            current_line_offset += 1;
        }
    }

    // Append the pre-computed retrying hint card at the bottom of the chat area
    if !precomputed_hint_lines.is_empty() {
        total_overall_lines += precomputed_hint_lines.len();
        lines.extend(precomputed_hint_lines);
    }

    RenderOutput {
        lines,
        total_lines: total_overall_lines,
        render_scroll,
        effective_scroll: scroll,
        selectable_regions,
        card_bounds,
        inline_running_card_ranges,
        image_badge_infos,
        thinking_header_infos,
    }
}
