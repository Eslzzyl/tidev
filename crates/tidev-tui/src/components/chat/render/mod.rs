//! Core rendering pipeline for the chat message list.
//!
//! Orchestrates layout index updates, cache lookups, block rendering, and
//! scroll management. Messages are rendered with markdown formatting, cached
//! in an LRU, and only re-rendered when content or width changes. Hyperlink
//! annotations travel alongside the rendered lines (`HyperlinkLine`) and are
//! injected into the frame buffer as OSC 8 sequences after the paragraph is
//! drawn.

mod blocks;
mod cards;
mod layout;
mod subagent;
mod thinking;
mod utils;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

use crate::chat_context::ChatContext;
use crate::hyperlink::{HyperlinkLine, HyperlinkRange, mark_buffer_hyperlinks};
use crate::theme::ThemePalette;
use ratatui::layout::Rect;
use ratatui::prelude::{Frame, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use uuid::Uuid;

use blocks::messages_text;
use cards::{render_assistant_cards, render_single_card, render_tool_card};
pub use layout::{CardGeom, LEFT_MARGIN, SCROLLBAR_WIDTH};
use layout::{compute_content_layout, render_scrollbar};
pub use subagent::{InlineRunningCardRange, RunningSubagentInfo};
pub use utils::wrap_text_lines;
use utils::{ImageBadgeInfo, loading_spinner};

use crate::components::chat::layout_index::MessageLayoutIndex;
use crate::components::chat::render_cache::{
    MessageRenderCacheEntry, MessageRenderCacheKey, SelectableRegionRange,
};

// ---------------------------------------------------------------------------
// RenderContext
// ---------------------------------------------------------------------------

/// Shared context assembled once per frame and threaded through all rendering.
pub(crate) struct RenderContext<'a> {
    pub palette: ThemePalette,
    pub spinner: &'a str,
    pub workspace_root: &'a Path,
    pub expanded_tool_results: &'a HashSet<Uuid>,
    pub hovered_card: Option<Uuid>,
    pub model_display_name: &'a str,
    pub running_subagents: &'a [RunningSubagentInfo],
    pub hovered_inline_subagent: Option<usize>,
    /// Messages whose thinking/reasoning fold state has been manually toggled.
    pub thinking_collapsed_overrides: &'a HashSet<Uuid>,
    /// Default collapse state for thinking content (from config).
    pub default_collapse_thinking: bool,
    pub message_app_data: Option<&'a HashMap<Uuid, tidev_core::MessageAppData>>,
}

// ---------------------------------------------------------------------------
// RenderOutput
// ---------------------------------------------------------------------------

/// Every piece of data produced by the rendering pipeline.
pub(crate) struct RenderOutput {
    /// Rendered lines with hyperlink annotations, index-aligned with the
    /// visible window (line `i` is drawn at row `area.y + i - render_scroll`).
    pub hyperlink_lines: Vec<HyperlinkLine>,
    pub total_lines: usize,
    pub render_scroll: usize,
    pub effective_scroll: usize,
    pub selectable_regions: Vec<SelectableRegionRange>,
    pub card_bounds: Vec<(Uuid, usize, usize)>,
    pub inline_running_card_ranges: Vec<InlineRunningCardRange>,
    pub image_badge_infos: Vec<ImageBadgeInfo>,
    /// Absolute line numbers of thinking headers (message_id, line).
    pub thinking_header_infos: Vec<(Uuid, usize)>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_messages(
    frame: &mut Frame,
    area: Rect,
    workspace_root: &Path,
    layout_index: &mut MessageLayoutIndex,
    render_cache: &mut lru::LruCache<MessageRenderCacheKey, MessageRenderCacheEntry>,
    chat_context: &ChatContext,
    palette: ThemePalette,
    scroll_offset: &mut usize,
    follow_tail: &mut bool,
    expanded_tool_results: &mut HashSet<Uuid>,
    running_subagents: &[RunningSubagentInfo],
    spinner_start: Instant,
    hovered_card: Option<Uuid>,
    hovered_inline_subagent: Option<usize>,
    retrying_hint: &Option<(Uuid, u32, u32, String, Instant)>,
    thinking_collapsed_overrides: &HashSet<Uuid>,
    default_collapse_thinking: bool,
    out_card_bounds: &mut Vec<(Uuid, usize, usize)>,
    out_selectable_regions: &mut Vec<SelectableRegionRange>,
    out_inline_running_card_ranges: &mut Vec<InlineRunningCardRange>,
    out_image_badge_infos: &mut Vec<ImageBadgeInfo>,
    out_thinking_header_infos: &mut Vec<(Uuid, usize)>,
    out_render_content_area: &mut Rect,
    out_render_scroll: &mut usize,
    scrollbar_hovered: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (content_area, scrollbar_rect) = compute_content_layout(area);

    let spinner = loading_spinner(spinner_start);
    let ctx = RenderContext {
        palette,
        spinner,
        workspace_root,
        expanded_tool_results,
        hovered_card,
        model_display_name: &chat_context.model_display_name,
        running_subagents,
        hovered_inline_subagent,
        thinking_collapsed_overrides,
        default_collapse_thinking,
        message_app_data: Some(&chat_context.message_app_data),
    };

    let output = messages_text(
        chat_context,
        layout_index,
        render_cache,
        &ctx,
        CardGeom::new(content_area.width as usize),
        *scroll_offset,
        area.height as usize,
        *follow_tail,
        retrying_hint,
    );

    // Sync the effective scroll (updated inside messages_text for follow_tail
    // and clamping) back to the caller so that subsequent key events see the
    // correct scroll offset without a one-frame lag.
    *scroll_offset = output.effective_scroll;
    // Export card bounds for mouse hover detection.
    *out_card_bounds = output.card_bounds;
    // Export selectable regions for mouse selection clamping.
    *out_selectable_regions = output.selectable_regions;
    // Export inline running card ranges for mouse interaction.
    *out_inline_running_card_ranges = output.inline_running_card_ranges;
    // Export image badge infos for mouse hit-testing.
    *out_image_badge_infos = output.image_badge_infos;
    // Export thinking header infos for mouse hit-testing.
    *out_thinking_header_infos = output.thinking_header_infos;
    // Export the rendered content area and render scroll for coordinate
    // conversion in selectable_region_rects().
    *out_render_content_area = content_area;
    *out_render_scroll = output.render_scroll;

    // Render the message text as a Paragraph widget, then inject OSC 8
    // hyperlinks into the frame buffer. Lines are pre-wrapped, so each
    // logical line occupies exactly one row at `area.y + i - render_scroll`.
    let (lines, line_links): (Vec<Line<'static>>, Vec<Vec<HyperlinkRange>>) = output
        .hyperlink_lines
        .into_iter()
        .map(|hl| (hl.line, hl.hyperlinks))
        .unzip();
    let text = ratatui::text::Text::from(lines);
    let paragraph = Paragraph::new(text)
        .style(Style::default().bg(ctx.palette.background))
        .scroll((output.render_scroll as u16, 0));
    frame.render_widget(paragraph, content_area);
    mark_buffer_hyperlinks(
        frame.buffer_mut(),
        content_area,
        &line_links,
        output.render_scroll,
    );

    // Scrollbar
    if let Some(sb) = scrollbar_rect {
        render_scrollbar(
            frame,
            sb,
            *scroll_offset,
            output.total_lines,
            area.height as usize,
            ctx.palette,
            scrollbar_hovered,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blocks::update_layout_index;
    use cards::render_system_card;
    use ratatui::style::Color;
    use std::path::Path;
    use tidev_llm::message::{Message, MessageRole};
    use tidev_llm::reasoning::ThinkingLevelType;

    fn user_msg(content: &str, id: u128) -> Message {
        let mut msg = Message::new(MessageRole::User, content);
        msg.id = Uuid::from_u128(id);
        msg
    }

    fn assistant_msg(reasoning: &str, content: &str, id: u128) -> Message {
        let mut msg = Message::new(MessageRole::Assistant, content);
        msg.id = Uuid::from_u128(id);
        msg.reasoning = reasoning.to_string();
        msg.completed_at = Some(chrono::Utc::now());
        msg.model_id = Some("test-model".into());
        msg
    }

    fn test_palette() -> ThemePalette {
        ThemePalette {
            is_dark: true,
            background: Color::Rgb(0, 0, 0),
            panel: Color::Rgb(10, 10, 10),
            panel_alt: Color::Rgb(20, 20, 20),
            panel_light: Color::Rgb(30, 30, 30),
            text: Color::Rgb(255, 255, 255),
            muted: Color::Rgb(128, 128, 128),
            border: Color::Rgb(64, 64, 64),
            accent: Color::Rgb(0, 200, 200),
            accent_soft: Color::Rgb(100, 150, 150),
            success: Color::Rgb(0, 200, 0),
            warning: Color::Rgb(200, 200, 0),
            error: Color::Rgb(200, 0, 0),
            diff_add: Color::Rgb(0, 200, 0),
            diff_delete: Color::Rgb(200, 0, 0),
            selection_bg: Color::Rgb(0, 200, 200),
            selection_fg: Color::Rgb(255, 255, 255),
            mode_build: Color::Rgb(0, 200, 200),
            mode_plan: Color::Rgb(100, 150, 150),
        }
    }

    fn test_render_ctx<'a>(
        palette: &'a ThemePalette,
        expanded: &'a HashSet<Uuid>,
        subagents: &'a [RunningSubagentInfo],
        collapsed: &'a HashSet<Uuid>,
    ) -> RenderContext<'a> {
        RenderContext {
            palette: *palette,
            spinner: "|",
            workspace_root: Path::new("."),
            expanded_tool_results: expanded,
            hovered_card: None,
            model_display_name: "test",
            running_subagents: subagents,
            hovered_inline_subagent: None,
            thinking_collapsed_overrides: collapsed,
            default_collapse_thinking: false,
            message_app_data: None,
        }
    }

    #[test]
    fn assistant_card_trailing_lines() {
        let palette = test_palette();
        let expanded = HashSet::new();
        let subagents = Vec::new();
        let collapsed = HashSet::new();
        let ctx = test_render_ctx(&palette, &expanded, &subagents, &collapsed);

        let msgs = vec![
            user_msg("hello", 1),
            assistant_msg("thinking about code", "here is my response", 2),
            user_msg("next", 3),
        ];

        let chat_ctx = ChatContext::new(
            Uuid::from_u128(100),
            "test".into(),
            msgs.clone(),
            None,
            "test-model".into(),
            "test-provider".into(),
        );

        let geom = CardGeom::new(80);
        let mut index = MessageLayoutIndex::new();
        let mut cache = lru::LruCache::new(std::num::NonZeroUsize::new(64).unwrap());

        update_layout_index(&mut index, &mut cache, &chat_ctx.messages, &geom, &ctx);

        let retrying_hint: Option<(Uuid, u32, u32, String, Instant)> = None;
        let output = messages_text(
            &chat_ctx,
            &mut index,
            &mut cache,
            &ctx,
            geom,
            0,
            200,
            false,
            &retrying_hint,
        );

        // Print rendered lines for manual inspection
        eprintln!("\n=== Rendered lines ===");
        for (i, line) in output.hyperlink_lines.iter().enumerate() {
            let text: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
            let display = if text.trim().is_empty() {
                "(empty)".to_string()
            } else {
                text.escape_debug().to_string()
            };
            eprintln!("  [{:3}] {}", i, display);
        }

        // Count blank lines between the assistant block's last rendered
        // content line (the card trailing) and the next user block's
        // first opening line.
        let mut blank_count = 0usize;
        let mut passed_footer = false;

        for line in &output.hyperlink_lines {
            let text: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
            let trimmed = text.trim();

            if trimmed.contains("test · 0s") || trimmed.contains("test-model") {
                passed_footer = true;
                continue;
            }

            if passed_footer && trimmed == "┃" {
                // Next user card opening — stop counting
                break;
            }

            if passed_footer {
                let is_empty = trimmed.is_empty();
                if is_empty {
                    blank_count += 1;
                }
            }
        }

        eprintln!("\n=== Blank lines between assistant footer and next user ===");
        eprintln!("  Count: {}", blank_count);

        // After removing the unconditional `lines.push(Line::from(""))`
        // at the end of render_block_from_cache, there should be exactly
        // 1 blank line (from render_assistant_cards line 1305).
        assert_eq!(
            blank_count, 1,
            "Expected 1 blank line between assistant footer and next user, got {}",
            blank_count
        );
    }

    #[test]
    fn user_card_hyperlink_ranges_survive_decoration() {
        let palette = test_palette();
        let expanded = HashSet::new();
        let subagents = Vec::new();
        let collapsed = HashSet::new();
        let ctx = test_render_ctx(&palette, &expanded, &subagents, &collapsed);

        let msgs = vec![user_msg("See https://example.com/a", 1)];
        let chat_ctx = ChatContext::new(
            Uuid::from_u128(100),
            "test".into(),
            msgs,
            None,
            "test-model".into(),
            "test-provider".into(),
        );

        let geom = CardGeom::new(80);
        let mut index = MessageLayoutIndex::new();
        let mut cache = lru::LruCache::new(std::num::NonZeroUsize::new(64).unwrap());
        update_layout_index(&mut index, &mut cache, &chat_ctx.messages, &geom, &ctx);

        let output = messages_text(
            &chat_ctx, &mut index, &mut cache, &ctx, geom, 0, 200, false, &None,
        );

        // The user card renders as "┃ See https://example.com/a"; the link
        // range recorded by the markdown writer must be shifted by the "┃ "
        // prefix (via decorate_card_lines) so columns match the visible text.
        let mut found = false;
        for hl in &output.hyperlink_lines {
            for link in &hl.hyperlinks {
                let text: String = hl.line.spans.iter().map(|s| s.content.as_ref()).collect();
                assert_eq!(link.destination, "https://example.com/a");
                let byte_start = text.find("https://example.com/a").expect("visible URL");
                let col_start: usize = text[..byte_start]
                    .chars()
                    .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
                    .sum();
                assert_eq!(link.columns.start, col_start, "line text: {text:?}");
                found = true;
            }
        }
        assert!(found, "expected a hyperlink in the user card");
    }

    #[test]
    fn assistant_footer_includes_request_thinking_level() {
        let palette = test_palette();
        let expanded = HashSet::new();
        let subagents = Vec::new();
        let collapsed = HashSet::new();
        let ctx = test_render_ctx(&palette, &expanded, &subagents, &collapsed);

        let mut request = user_msg("hello", 1);
        request.thinking_level = Some(ThinkingLevelType::from_string("gpt5:high"));
        let msgs = vec![request, assistant_msg("", "response", 2)];
        let chat_ctx = ChatContext::new(
            Uuid::from_u128(100),
            "test".into(),
            msgs,
            None,
            "test".into(),
            "test-provider".into(),
        );

        let geom = CardGeom::new(80);
        let mut index = MessageLayoutIndex::new();
        let mut cache = lru::LruCache::new(std::num::NonZeroUsize::new(64).unwrap());
        update_layout_index(&mut index, &mut cache, &chat_ctx.messages, &geom, &ctx);

        let output = messages_text(
            &chat_ctx, &mut index, &mut cache, &ctx, geom, 0, 200, false, &None,
        );
        let rendered: String = output
            .hyperlink_lines
            .iter()
            .flat_map(|line| line.line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect();

        assert!(rendered.contains("test · High · 0s"), "{rendered}");
    }

    #[test]
    fn test_instruction_message_wrapping() {
        let palette = test_palette();
        let expanded = HashSet::new();
        let collapsed = HashSet::new();
        let ctx = test_render_ctx(&palette, &expanded, &[], &collapsed);

        // Short path: should NOT wrap
        let short_content = "Loaded instructions from AGENTS.md".to_string();
        let short_msg = Message::new(MessageRole::System, &short_content);
        let cards = render_system_card(&ctx, &short_msg, 80, false);
        // cards: [(bg, [line(s)...])]
        let lines = &cards[0].1;
        // Should be exactly 1 line (the instruction text itself)
        assert_eq!(lines.len(), 1, "short instruction should not wrap");
        let rendered: String = lines[0]
            .line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(rendered.starts_with("󱁤"), "should start with icon");
        assert!(rendered.contains("AGENTS.md"), "should contain path");

        // Long path: SHOULD wrap at narrow width
        let long_content = "Loaded 3 instruction files: a/very/long/path/that/should/definitely/wrap/AGENTS.md, another/long/path/CLAUDE.md, yet/another/CONTEXT.md".to_string();
        let long_msg = Message::new(MessageRole::System, &long_content);
        let cards2 = render_system_card(&ctx, &long_msg, 30, false);
        let lines2 = &cards2[0].1;

        // Should wrap to multiple lines
        assert!(
            lines2.len() > 1,
            "long instruction should wrap at narrow width (got {} lines)",
            lines2.len()
        );

        // First line should start with icon
        let first: String = lines2[0]
            .line
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(first.starts_with("󱁤"), "first line should start with icon");

        // Continuation lines should be indented (have leading whitespace)
        for (i, line) in lines2[1..].iter().enumerate() {
            let text: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(
                text.starts_with("   "),
                "continuation line {} should be indented with 3 spaces, got: {:?}",
                i + 1,
                text
            );
        }

        // None of the lines should exceed the content_width in display width
        use unicode_width::UnicodeWidthStr;
        for (i, line) in lines2.iter().enumerate() {
            let text: String = line.line.spans.iter().map(|s| s.content.as_ref()).collect();
            let w = UnicodeWidthStr::width(text.as_str());
            assert!(
                w <= 30,
                "line {} exceeds width: display_width={} content={:?}",
                i,
                w,
                text
            );
        }
    }
}
