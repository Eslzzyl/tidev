mod content;
#[cfg(test)]
mod tests;
pub(crate) mod tool;
mod utils;

pub(crate) use content::strip_system_reminder_tags;

use content::IMAGE_BADGE_RE;

use tidev_types::prompts::SessionMode;

use crate::theme::ThemePalette;
use crate::{
    App,
    core::state::{
        MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
        MessageRenderCacheValue, SelectableRegionRange,
    },
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use tidev_engine::{
    config::{AuthStore, SharedConfig},
    tooling::canonical_tool_name,
};
use tidev_session::session::{Conversation, Message, MessageRole, ToolCall};
use tidev_session::utils::{TokenUsage, format_token_count};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::super::permission::{RunningSubagentExecution, SubagentStatus};
use crate::render::render::{
    decorate_card_lines, line_with_prefix, line_with_style, shorten, shorten_single_line,
    wrap_text_lines,
};

use crate::chat_render::content::BlockComputation;

use crate::markdown::{WrapOptions, word_wrap_line};

const TOOL_OUTPUT_PREVIEW_LINES: usize = 5;
const MAX_VISIBLE_QUEUED_PROMPTS: usize = 4;
const MAX_QUEUED_PROMPT_LINES: usize = 3;

#[derive(Clone, Debug)]
struct ToolResultCardRange {
    message_id: Uuid,
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Debug)]
struct InlineRunningCardRange {
    execution_index: usize,
    start_line: usize,
    end_line: usize,
}

struct RenderContext<'a> {
    palette: ThemePalette,
    spinner: &'a str,
    workspace_root: &'a Path,
    expanded_tool_results: &'a HashSet<Uuid>,
    expanded_tool_outputs: &'a HashMap<Uuid, String>,
    config: SharedConfig,
    auth: &'a AuthStore,
    conversation: &'a Conversation,
    mode: SessionMode,
}

/// Return type of [App::messages_text].
type MessagesTextResult = (
    Text<'static>,
    usize,
    Vec<ToolResultCardRange>,
    Vec<(Uuid, usize, usize)>,
    Vec<SelectableRegionRange>,
    bool,
    usize,
    Vec<InlineRunningCardRange>,
);

impl App {
    pub(super) fn render_chat(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        const SIDEBAR_GAP: u16 = 2;
        let sidebar_visible = area.width
            >= self
                .config
                .read()
                .unwrap()
                .ui
                .sidebar_width
                .saturating_add(70)
                .saturating_add(SIDEBAR_GAP);
        let main_area = if sidebar_visible {
            let split = Layout::horizontal([
                Constraint::Min(20),
                Constraint::Length(SIDEBAR_GAP),
                Constraint::Length(self.config.read().unwrap().ui.sidebar_width),
            ])
            .split(area);
            self.sidebar_area = Some(split[2]);
            self.render_sidebar(frame, split[2]);
            split[0]
        } else {
            area
        };

        let composer_height_raw = self
            .composer
            .preferred_height(
                main_area.width.saturating_sub(5),
                self.config.read().unwrap().ui.max_input_lines,
            )
            .min(main_area.height.saturating_sub(3).max(3));

        // Calculate queued messages area height (frozen area above input box)
        let queued_count = if self.conversation.parent_session_id.is_some() {
            0
        } else {
            self.pending_prompt_queue.len()
        };
        let queued_height = if queued_count > 0 {
            let visible = queued_count.min(MAX_VISIBLE_QUEUED_PROMPTS);
            // Compute actual wrapped line count per visible queued prompt
            let text_width = main_area.width.saturating_sub(5).max(1) as usize;
            let mut inner: usize = 0;
            for (i, queued) in self.pending_prompt_queue.iter().take(visible).enumerate() {
                let wrapped = wrap_text_lines(&queued.prompt, text_width, MAX_QUEUED_PROMPT_LINES);
                inner += wrapped.len();
                // Separator between items (not after last)
                if i + 1 < visible {
                    inner += 1;
                }
            }
            // +1 for "+N more" overflow, +2 for block top/bottom borders
            let overflow = if queued_count > MAX_VISIBLE_QUEUED_PROMPTS {
                1
            } else {
                0
            };
            (inner + overflow + 2)
                .min(main_area.height.saturating_sub(6) as usize / 2)
                .min(15)
        } else {
            0
        };

        // Extra lines for metadata display inside the composer
        const METADATA_EXTRA: u16 = 2;

        let composer_height = (composer_height_raw + METADATA_EXTRA).min(
            main_area
                .height
                .saturating_sub((queued_height as u16) + 3)
                .max(3),
        );

        // In subsession, the navigation area needs only 3 rows (1 content + 1 padding above/below)
        let subsession_nav_height: u16 = 3;

        // Handle workspace boundary confirm dialog (shown before the boundary dialog)
        if let Some(dialog) = self.workspace_boundary_confirm_dialog.clone() {
            let dialog_height = dialog
                .dialog_height(main_area.width)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(dialog_height),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_workspace_boundary_confirm_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            return;
        }

        // Handle workspace boundary dialog (similar to question dialog)
        if let Some(dialog) = self.workspace_boundary_dialog.clone() {
            let dialog_height = dialog
                .dialog_height(main_area.width)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(dialog_height),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_workspace_boundary_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            return;
        }

        // Handle sensitive file confirm dialog (shown before the sensitive file dialog)
        if let Some(dialog) = self.sensitive_file_confirm_dialog.clone() {
            let dialog_height = dialog
                .dialog_height(main_area.width)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(dialog_height),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_sensitive_file_confirm_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            return;
        }

        // Handle sensitive file dialog
        if let Some(dialog) = self.sensitive_file_dialog.clone() {
            let dialog_height = dialog
                .dialog_height(main_area.width)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(dialog_height),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_sensitive_file_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            return;
        }

        if let Some(dialog) = self.question_dialog.clone() {
            let question_height = dialog
                .prompt_height(main_area.width, composer_height)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(question_height),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_question_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            return;
        }

        let layout = Layout::vertical([
            Constraint::Min(6),
            Constraint::Length(queued_height as u16),
            Constraint::Length(if self.conversation.parent_session_id.is_some() {
                subsession_nav_height
            } else {
                composer_height
            }),
            Constraint::Length(1),
        ])
        .split(main_area);

        self.render_messages(frame, layout[0]);

        self.queued_card_bounds.clear();
        if queued_height > 0 {
            self.render_queued_prompts(frame, layout[1]);
        }

        // In subsession, show navigation panel instead of input box
        if self.conversation.parent_session_id.is_some() {
            self.render_subsession_navigation(frame, layout[2]);
        } else {
            let prompt_title = if self.shell_mode {
                "Shell".to_string()
            } else {
                match self.pending_mode.as_ref() {
                    Some(pending) if self.pending_request => {
                        format!(
                            "{} (current), {} (on completion)",
                            self.mode.title(),
                            pending.title()
                        )
                    }
                    _ => self.mode.title().to_string(),
                }
            };
            self.render_input_block_with_composer(
                frame,
                layout[2],
                &prompt_title,
                &self.composer,
                self.composer.placeholder(),
                false,
                true,
                true,
            );
            // Palettes should align with the composer's visual left edge (offset by 2 columns)
            let palette_area = Rect {
                x: layout[2].x + 2,
                ..layout[2]
            };
            self.render_at_mention_palette(frame, palette_area);
            self.render_snippet_palette(frame, palette_area);
            self.render_command_palette(frame, palette_area);
            self.render_snippet_palette(frame, palette_area);
            self.render_shell_completion_palette(frame, palette_area);
        }
        self.render_prompt_footer(frame, layout[3]);
    }

    /// Render a frozen area above the input box showing queued (pending) prompts.
    /// Each queued message is word-wrapped into up to [`MAX_QUEUED_PROMPT_LINES`] lines.
    /// Rows are separated by a thin rule. Each row is independently hover-highlighted.
    fn render_queued_prompts(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let count = self.pending_prompt_queue.len();
        let visible = count.min(MAX_VISIBLE_QUEUED_PROMPTS);

        // Build title: " QUEUE " badge with background color + count
        let title = Line::from(vec![
            Span::styled(
                " QUEUE ",
                Style::default()
                    .bg(palette.selection_bg)
                    .fg(palette.selection_fg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {} ", count), Style::default().fg(palette.muted)),
        ]);

        // Align with composer: left_inset=2 (bg) + inner_margin=2 (text)
        let left_inset: u16 = 2;
        let block_area = Rect {
            x: area.x + left_inset,
            y: area.y,
            width: area.width.saturating_sub(left_inset),
            height: area.height,
        };

        let block = Block::default()
            .style(Style::default().bg(palette.panel))
            .title(title)
            .title_alignment(Alignment::Left);

        // Inner content matches composer's text area (x+4, width-5).
        // Offset y by 1 to leave room for the block's title on the first row.
        let inner = Rect {
            x: block_area.x + left_inset,
            y: block_area.y + 1,
            width: block_area.width.saturating_sub(left_inset + 1),
            height: block_area.height.saturating_sub(1),
        };
        let inner_height = inner.height as usize;
        let width = inner.width.max(1) as usize;

        let mut y_offset: u16 = 0;

        for (i, queued) in self.pending_prompt_queue.iter().take(visible).enumerate() {
            if y_offset as usize >= inner_height {
                break;
            }

            // Word-wrap the prompt into up to MAX_QUEUED_PROMPT_LINES lines
            let wrapped_lines = wrap_text_lines(&queued.prompt, width, MAX_QUEUED_PROMPT_LINES);
            let row_text_height = wrapped_lines.len();
            let has_separator = i + 1 < visible;
            let row_height = row_text_height + if has_separator { 1 } else { 0 };

            // Clamp to available space
            let available = inner_height.saturating_sub(y_offset as usize);
            if available == 0 {
                break;
            }
            let render_height = row_height.min(available);

            // Record bounds for hover hit-testing
            let row_rect = Rect::new(
                inner.x,
                inner.y + y_offset,
                inner.width,
                render_height as u16,
            );
            self.queued_card_bounds.push((i, row_rect));

            // Apply hover highlight
            let is_hovered = self.hovered_queued_index == Some(i);
            if is_hovered {
                let hover_bg = palette.hover_bg(palette.panel);
                frame.render_widget(
                    Block::default().style(Style::default().bg(hover_bg)),
                    row_rect,
                );
            }

            // Render each wrapped line of the prompt
            let text_style = if is_hovered {
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::ITALIC)
            } else {
                Style::default()
                    .fg(palette.muted)
                    .add_modifier(Modifier::ITALIC)
            };

            for line_text in wrapped_lines.iter() {
                if y_offset as usize >= inner_height {
                    break;
                }
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(line_text.clone(), text_style)))
                        .wrap(Wrap { trim: false }),
                    Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
                );
                y_offset += 1;
            }

            // Separator line (not after last visible item)
            if has_separator && (y_offset as usize) < inner_height {
                let sep_width = width.saturating_sub(2);
                let sep = "─".repeat(sep_width);
                let sep_style = if is_hovered {
                    Style::default().fg(palette.text)
                } else {
                    Style::default().fg(palette.border)
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(sep, sep_style))),
                    Rect::new(
                        inner.x + 1,
                        inner.y + y_offset,
                        inner.width.saturating_sub(2),
                        1,
                    ),
                );
                y_offset += 1;
            }
        }

        // Overflow indicator
        if count > MAX_VISIBLE_QUEUED_PROMPTS && (y_offset as usize) < inner_height {
            let more_text = format!("+{} more...", count - MAX_VISIBLE_QUEUED_PROMPTS);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    more_text,
                    Style::default().fg(palette.muted),
                ))),
                Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
            );
        }

        // Render block last so it draws borders on top
        frame.render_widget(block, block_area);
    }

    fn render_subsession_navigation(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();

        // Match the composer's left inset (2 columns) so the background aligns with the
        // main session input block.
        let left_inset: u16 = 2;
        let bg_rect = Rect {
            x: area.x + left_inset,
            y: area.y,
            width: area.width.saturating_sub(left_inset),
            height: area.height,
        };

        let block = Block::default().style(Style::default().bg(palette.panel));
        frame.render_widget(block, bg_rect);

        // Build navigation hint
        let hint = Line::from(vec![
            Span::styled("Up", Style::default().fg(palette.accent_soft)),
            Span::styled(": return to parent  ", Style::default().fg(palette.muted)),
            Span::styled("Left", Style::default().fg(palette.accent_soft)),
            Span::styled("/", Style::default().fg(palette.muted)),
            Span::styled("Right", Style::default().fg(palette.accent_soft)),
            Span::styled(": switch subagent", Style::default().fg(palette.muted)),
        ]);

        // Vertically center the single-line hint within bg_rect
        let content_height: u16 = 1;
        let y_offset = bg_rect.height.saturating_sub(content_height) / 2;
        let content_rect = Rect {
            x: bg_rect.x,
            y: bg_rect.y + y_offset,
            width: bg_rect.width,
            height: content_height.min(bg_rect.height),
        };

        let paragraph = Paragraph::new(hint)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.text));

        frame.render_widget(paragraph, content_rect);
    }

    pub(super) fn render_messages(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();

        // Use manual rect layout: 2-column left margin from screen edge,
        // content fills remaining space (right edge matches composer inner right edge),
        // scrollbar at far right with 1-column gap from content.
        const LEFT_MARGIN: u16 = 2;
        const SCROLLBAR_WIDTH: u16 = 1;
        const GAP: u16 = 1;

        if area.width == 0 || area.height == 0 {
            return;
        }

        let scrollbar_area = if area.width > LEFT_MARGIN + GAP + SCROLLBAR_WIDTH {
            let content_width = area.width - LEFT_MARGIN - GAP - SCROLLBAR_WIDTH;
            let content = Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: content_width,
                height: area.height,
            };
            let scrollbar = Rect {
                x: area.x + area.width - SCROLLBAR_WIDTH,
                y: area.y,
                width: SCROLLBAR_WIDTH,
                height: area.height,
            };
            (content, Some(scrollbar))
        } else if area.width > LEFT_MARGIN + SCROLLBAR_WIDTH {
            // No gap, content + scrollbar directly adjacent
            let content_width = area.width - LEFT_MARGIN - SCROLLBAR_WIDTH;
            let content = Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: content_width,
                height: area.height,
            };
            let scrollbar = Rect {
                x: area.x + area.width - SCROLLBAR_WIDTH,
                y: area.y,
                width: SCROLLBAR_WIDTH,
                height: area.height,
            };
            (content, Some(scrollbar))
        } else if area.width > LEFT_MARGIN {
            let content = Rect {
                x: area.x + LEFT_MARGIN,
                y: area.y,
                width: area.width - LEFT_MARGIN,
                height: area.height,
            };
            (content, None)
        } else {
            return;
        };

        let content_area = scrollbar_area.0;
        self.message_content_area = Some(content_area);
        self.message_viewport_lines = content_area.height as usize;
        let content_width = content_area.width.max(1) as usize;
        let (
            text,
            total_lines,
            card_ranges,
            user_card_ranges,
            selectable_regions_ranges,
            rendered_virtualized,
            virtualized_render_scroll,
            inline_running_card_ranges,
        ) = self.messages_text(Some(content_width));

        self.message_total_lines = total_lines;

        let max_scroll = total_lines.saturating_sub(self.message_viewport_lines);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = scroll;
        self.message_follow_tail = scroll >= max_scroll;
        let render_scroll = if rendered_virtualized {
            virtualized_render_scroll
        } else {
            scroll
        };

        self.selectable_regions.clear();
        for r in selectable_regions_ranges {
            let screen_start = r.start_line.saturating_sub(render_scroll);
            let screen_end = r.end_line.saturating_sub(render_scroll);
            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }
            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;
            if visible_start < visible_end {
                let y = content_area.y.saturating_add(visible_start);
                let height = visible_end.saturating_sub(visible_start);
                let min_x = content_area.x.saturating_add(r.min_x);
                let max_x = r
                    .max_x
                    .map(|mx| content_area.x.saturating_add(mx))
                    .unwrap_or(content_area.x.saturating_add(content_area.width));
                let width = max_x.saturating_sub(min_x);
                if width > 0 {
                    self.selectable_regions.push(Rect {
                        x: min_x,
                        y,
                        width,
                        height,
                    });
                }
            }
        }

        // Calculate screen positions for tool result cards
        self.tool_result_card_bounds.clear();
        for card_range in card_ranges {
            let screen_start = card_range.start_line.saturating_sub(render_scroll);
            let screen_end = card_range.end_line.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: content_area.x,
                    y: content_area.y.saturating_add(visible_start),
                    width: content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.tool_result_card_bounds
                    .push((card_range.message_id, card_rect));
            }
        }

        // Calculate screen positions for user message cards
        self.user_card_bounds.clear();
        for &(message_id, start_line, end_line) in &user_card_ranges {
            let screen_start = start_line.saturating_sub(render_scroll);
            let screen_end = end_line.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: content_area.x,
                    y: content_area.y.saturating_add(visible_start),
                    width: content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.user_card_bounds.push((message_id, card_rect));
            }
        }

        // Calculate screen positions for inline running subagent cards
        // inline_running_card_ranges contain absolute line positions within the full text.
        self.inline_subagent_card_bounds.clear();
        for card_range in &inline_running_card_ranges {
            let abs_start = card_range.start_line;
            let abs_end = card_range.end_line;

            let screen_start = abs_start.saturating_sub(render_scroll);
            let screen_end = abs_end.saturating_sub(render_scroll);

            if screen_end == 0 || screen_start >= self.message_viewport_lines {
                continue;
            }

            let visible_start = screen_start as u16;
            let visible_end = (screen_end.min(self.message_viewport_lines)) as u16;

            if visible_start < visible_end {
                let card_rect = Rect {
                    x: content_area.x,
                    y: content_area.y.saturating_add(visible_start),
                    width: content_area.width,
                    height: visible_end.saturating_sub(visible_start),
                };
                self.inline_subagent_card_bounds
                    .push((card_range.execution_index, card_rect));
            }
        }

        // Scan user message cards for image badge spans and record their
        // screen positions so mouse clicks can open the image viewer.
        self.user_image_badge_bounds.clear();
        {
            let messages = self.conversation.visible_messages();
            for &(message_id, start_line, end_line) in &user_card_ranges {
                // Collect data_urls from Image attachments for this message
                let mut data_urls: Vec<String> = Vec::new();
                if let Some(msg) = messages.iter().find(|m| m.id == message_id) {
                    for att in &msg.attachments {
                        if let tidev_session::session::MessageAttachment::Image {
                            data_url, ..
                        } = att
                        {
                            data_urls.push(data_url.clone());
                        }
                    }
                }
                if data_urls.is_empty() {
                    continue;
                }
                let mut url_idx = 0;
                for line_idx in start_line..end_line.min(text.lines.len()) {
                    let line = &text.lines[line_idx];
                    let mut col_offset = 0usize;
                    for span in &line.spans {
                        let span_text: &str = span.content.as_ref();
                        // Find all image badge patterns in this span
                        let mut search_start = 0;
                        while let Some(m) = IMAGE_BADGE_RE
                            .find(&span_text[search_start..])
                            .unwrap()
                        {
                            let badge_col =
                                col_offset + search_start + m.start();
                            let badge_width = m.end() - m.start();
                            let screen_line =
                                line_idx.saturating_sub(render_scroll);
                            if screen_line < self.message_viewport_lines {
                                let screen_x =
                                    content_area.x + badge_col as u16;
                                let screen_y =
                                    content_area.y + screen_line as u16;
                                let data_url =
                                    data_urls[url_idx % data_urls.len()]
                                        .clone();
                                self.user_image_badge_bounds.push((
                                    message_id,
                                    Rect {
                                        x: screen_x,
                                        y: screen_y,
                                        width: badge_width as u16,
                                        height: 1,
                                    },
                                    data_url,
                                ));
                            }
                            url_idx += 1;
                            search_start += m.end();
                        }
                        col_offset += unicode_width::UnicodeWidthStr::width(span_text);
                    }
                }
            }
        }

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(palette.background).fg(palette.text))
            .scroll((render_scroll as u16, 0));

        frame.render_widget(paragraph, content_area);

        if let Some(scrollbar_area) = scrollbar_area.1 {
            // Explicitly fill the gap between content area and scrollbar to prevent
            // stale visual artifacts (diff backgrounds, table borders, etc.) from
            // appearing in this gap column when content refreshes. Without this,
            // ratatui's frame diff optimization may skip updating these cells in
            // certain edge cases, leaving residual content visible.
            if content_area.x.saturating_add(content_area.width) < scrollbar_area.x {
                frame.render_widget(
                    Block::default().style(Style::default().bg(palette.background)),
                    Rect {
                        x: content_area.x + content_area.width,
                        y: content_area.y,
                        width: scrollbar_area.x - content_area.x - content_area.width,
                        height: content_area.height,
                    },
                );
            }
            self.message_scrollbar_area = Some(scrollbar_area);
            self.render_scrollbar(frame, scrollbar_area, scroll, max_scroll);
        }
    }

    fn render_sidebar(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Session title (top)
        lines.push(Line::from(""));
        let session_title = shorten(
            &self.conversation.title,
            (area.width.saturating_sub(4).max(1)) as usize,
        );
        lines.push(Line::from(vec![Span::styled(
            session_title,
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from(""));

        // Model info
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            &self.active_model.display_name,
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            &self.active_model.provider_display_name,
            Style::default().fg(palette.muted),
        )]));

        if let Some(usage) = &self.context_usage {
            let session_tps: Vec<f32> = self
                .conversation
                .messages
                .iter()
                .filter(|m| matches!(m.role, MessageRole::Assistant))
                .filter_map(|m| m.tokens_per_second)
                .collect();

            if !session_tps.is_empty() {
                let avg_tps = session_tps.iter().sum::<f32>() / session_tps.len() as f32;
                lines.push(Line::from(vec![Span::styled(
                    format!("Speed: {:.1} t/s (avg)", avg_tps),
                    Style::default().fg(palette.muted),
                )]));
            } else if let Some(current_tps) = usage.tokens_per_second {
                lines.push(Line::from(vec![Span::styled(
                    format!("Speed: {:.1} t/s", current_tps),
                    Style::default().fg(palette.muted),
                )]));
            }
        }

        // Token statistics (session cumulative)
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            "Tokens",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        let mut token_usage = TokenUsage::default();
        for m in self
            .conversation
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Assistant))
        {
            token_usage.add(m.token_usage());
        }

        let total = token_usage.total();
        let total_cache = token_usage.total_cache();

        lines.push(Line::from(vec![Span::styled(
            format!("Total: {}", format_token_count(total)),
            Style::default().fg(palette.text),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "In: {}",
                format_token_count(token_usage.input_tokens as u64)
            ),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!("Cache: {}", format_token_count(total_cache)),
            Style::default().fg(palette.muted),
        )]));
        lines.push(Line::from(vec![Span::styled(
            format!(
                "Out: {}",
                format_token_count(token_usage.output_tokens as u64)
            ),
            Style::default().fg(palette.muted),
        )]));

        lines.push(Line::from(""));

        // Request count
        let request_count = self
            .conversation
            .messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Assistant))
            .count();
        lines.push(Line::from(vec![Span::styled(
            format!("Requests: {request_count}"),
            Style::default().fg(palette.text),
        )]));

        // Changed Files section
        lines.push(Line::from(""));

        let mut all_diffs = Vec::new();
        let mut seen_files = std::collections::HashSet::new();
        for msg in self.conversation.visible_messages() {
            if let Some(diffs_json) = &msg.file_diffs
                && let Ok(diffs) =
                    serde_json::from_str::<Vec<tidev_engine::snapshot::FileDiff>>(diffs_json)
            {
                for d in &diffs {
                    if seen_files.insert(d.file.clone()) {
                        all_diffs.push(d.clone());
                    }
                }
            }
        }

        lines.push(Line::from(vec![
            Span::styled(
                "Changed Files",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({})", all_diffs.len()),
                Style::default().fg(palette.muted),
            ),
        ]));

        if all_diffs.is_empty() {
            lines.push(Line::from(vec![Span::styled(
                "(no changes yet)",
                Style::default().fg(palette.muted),
            )]));
        } else {
            // Sort: modified first, then added, then deleted
            all_diffs.sort_by_key(|d| match d.status.as_deref() {
                Some("modified") => 0,
                Some("added") => 1,
                Some("deleted") => 2,
                _ => 3,
            });

            // Available content width for right-alignment
            let content_width = (area.width as usize).saturating_sub(4); // 2-char padding each side

            for d in &all_diffs {
                let filename = Path::new(&d.file)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| d.file.clone());

                let show_add = d.additions > 0;
                let show_del = d.deletions > 0;

                let add_str = format!("+{}", d.additions);
                let del_str = format!("-{}", d.deletions);

                // Filename in normal text color (matching opencode's text-strong)
                let file_span = Span::styled(filename.clone(), Style::default().fg(palette.text));

                // +N in green (matching opencode's text-diff-add-base)
                let add_span = Span::styled(add_str.clone(), Style::default().fg(palette.diff_add));

                // -M in red (matching opencode's text-diff-delete-base)
                let del_span =
                    Span::styled(del_str.clone(), Style::default().fg(palette.diff_delete));

                // Calculate padding to right-align the counts (like opencode's space-between)
                let fw = UnicodeWidthStr::width(filename.as_str());
                let aw = if show_add {
                    UnicodeWidthStr::width(add_str.as_str())
                } else {
                    0
                };
                let dw = if show_del {
                    UnicodeWidthStr::width(del_str.as_str())
                } else {
                    0
                };
                // +1 for space between the two counts when both are visible
                let gap_count = if show_add && show_del { 1 } else { 0 };
                let padding = content_width.saturating_sub(fw + aw + dw + gap_count);

                let mut spans = vec![file_span, Span::raw(" ".repeat(padding))];
                if show_add {
                    spans.push(add_span);
                }
                if show_del {
                    if show_add {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(del_span);
                }
                lines.push(Line::from(spans));
            }
        }

        // Todos section
        lines.push(Line::from(""));
        lines.push(Line::from(vec![Span::styled(
            format!("Todos ({})", self.todos.len()),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        for todo in &self.todos {
            let (checkbox, style) = match todo.status.as_str() {
                "completed" => (
                    "✔ ",
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                "in_progress" => ("● ", Style::default().fg(palette.accent)),
                "pending" => ("○ ", Style::default().fg(palette.text)),
                _ => ("○ ", Style::default().fg(palette.text)),
            };

            let content = &todo.content;
            lines.push(Line::from(vec![
                Span::styled(checkbox.to_string(), style),
                Span::styled(content.as_str(), style),
            ]));
        }

        // Undo state (only when active)
        if self.conversation.is_reverted() {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![Span::styled(
                "⚠ Undo active",
                Style::default().fg(palette.warning),
            )]));
        }

        // Background fill for the full sidebar area
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.panel)),
            area,
        );

        // Split sidebar: scrollable content (top) + fixed footer with workspace (bottom)
        let sidebar_padded = area.inner(Margin {
            horizontal: 2,
            vertical: 0,
        });
        let sidebar_content_width = sidebar_padded.width as usize;

        // Build fixed footer (workspace path, always visible)
        let workspace_path = self.workspace_root.display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );
        // Truncate long workspace paths
        let display_path = shorten(&display_path, sidebar_content_width.max(1));
        let footer_lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                "Workspace",
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![Span::styled(
                display_path,
                Style::default().fg(palette.muted),
            )]),
            Line::from(""),
            Line::from(""),
        ];
        let footer_height: u16 = footer_lines.len() as u16;

        // Content area height = sidebar height - footer height
        let content_height = area.height.saturating_sub(footer_height);
        let content_area = Rect {
            height: content_height,
            ..sidebar_padded
        };
        let footer_area = Rect {
            y: area.y + area.height.saturating_sub(footer_height),
            height: footer_height,
            ..sidebar_padded
        };

        // Estimate total lines for scroll max (accounts for word wrapping)
        self.sidebar_total_lines = lines
            .iter()
            .map(|line| {
                let w: usize = line
                    .spans
                    .iter()
                    .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                    .sum();
                if w == 0 {
                    1
                } else {
                    (w + sidebar_content_width - 1) / sidebar_content_width.max(1)
                }
            })
            .sum();

        let sidebar_viewport_lines = content_height as usize;
        let max_scroll = self
            .sidebar_total_lines
            .saturating_sub(sidebar_viewport_lines);
        self.sidebar_scroll_offset = self.sidebar_scroll_offset.min(max_scroll);

        // Render scrollable content
        let paragraph = Paragraph::new(Text::from(lines))
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((self.sidebar_scroll_offset as u16, 0));
        frame.render_widget(paragraph, content_area);

        // Render fixed footer (workspace path)
        let footer_paragraph =
            Paragraph::new(Text::from(footer_lines)).style(Style::default().fg(palette.text));
        frame.render_widget(footer_paragraph, footer_area);
    }

    fn messages_text(&mut self, content_width: Option<usize>) -> MessagesTextResult {
        let started_at = Instant::now();
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);
        let messages = self.conversation.visible_messages();

        let mut lines = Vec::new();
        let mut card_ranges = Vec::new();
        let mut user_card_ranges = Vec::new();
        let mut selectable_regions_ranges = Vec::new();
        let mut inline_running_card_ranges = Vec::new();

        // Header for subsessions (always visible at top)
        let header_lines = if self.conversation.parent_session_id.is_some() {
            vec![
                line_with_style(
                    "SUBSESSION active — viewing a child session.",
                    palette.accent_soft,
                ),
                line_with_style(
                    "Press Ctrl+X then Up arrow to return to the parent session.",
                    palette.muted,
                ),
                Line::from(""),
            ]
        } else {
            Vec::new()
        };

        // Handle empty messages case
        if messages.is_empty() {
            lines.extend(header_lines);
            lines.extend(decorate_card_lines(
                vec![
                    line_with_style("No messages yet.", palette.muted),
                    line_with_style("Start with a prompt in the input box below.", palette.muted),
                ],
                width,
                palette.panel,
                2,
            ));
            let total_lines = lines.len().max(1);
            return (
                Text::from(lines),
                total_lines,
                card_ranges,
                user_card_ranges, // empty at this point
                selectable_regions_ranges,
                false,
                0,
                inline_running_card_ranges,
            );
        }

        // Update layout index
        self.update_message_layout_index(width, body_width, false);
        if let Some(scroll_offset) = self.resolve_message_scroll_target(messages, width, body_width)
        {
            self.message_scroll_offset = scroll_offset;
            self.message_follow_tail = false;
            self.message_scroll_target = None;
        }

        // Calculate visible range based on scroll position
        let viewport = self.message_viewport_lines.max(1);
        let total_message_lines = self.message_layout_index.borrow().total_lines;
        let total_overall_lines = total_message_lines;
        let header_line_count = header_lines.len();

        let max_scroll = (header_line_count + total_overall_lines).saturating_sub(viewport);
        let scroll = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };
        self.message_scroll_offset = scroll;

        // the 'scroll' includes header lines. To find the correct message block, we must
        // offset the scroll past the header
        let message_scroll = scroll.saturating_sub(header_line_count);

        // Find visible blocks using the message-relative scroll
        let visible_blocks = self.find_visible_message_blocks(message_scroll, viewport);

        lines.extend(header_lines);

        // Calculate render_scroll for virtualized rendering
        // The visible blocks may start before 'message_scroll' (due to buffer zone),
        // so we need to skip those lines when rendering.
        // Also, if first block starts after 'message_scroll', we need padding.
        let first_block_start = visible_blocks.first().map(|b| b.start_line).unwrap_or(0);

        let (mut render_scroll, padding_lines) = if first_block_start < message_scroll {
            (message_scroll - first_block_start, 0)
        } else if first_block_start > message_scroll {
            (0, first_block_start - message_scroll)
        } else {
            (0, 0)
        };

        // Important: if we are scrolled inside the header, the render_scroll applies entirely to the header.
        // Otherwise, it skips the entire header PLUS block-relative scroll.
        if scroll < header_line_count {
            render_scroll = scroll;
        } else {
            render_scroll += header_line_count;
        }

        // Add padding lines if first block starts after scroll position
        for _ in 0..padding_lines {
            lines.push(Line::from(""));
        }

        // Create render context for tool calls
        let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
        let spinner = self.loading_spinner();
        let ctx = RenderContext {
            palette,
            spinner,
            workspace_root: self.workspace_root.as_path(),
            expanded_tool_results: &self.expanded_tool_results,
            expanded_tool_outputs: &expanded_tool_outputs,
            config: self.config.clone(),
            auth: &self.auth,
            conversation: &self.conversation,
            mode: self.mode,
        };

        // Render visible blocks
        let mut current_line_offset = header_line_count + padding_lines;
        for block in &visible_blocks {
            // Round end = no next message (session end) OR next message is User (new round)
            let next_idx = block.message_start_idx + block.message_count;
            let is_round_end =
                next_idx >= messages.len() || matches!(messages[next_idx].role, MessageRole::User);
            let block_lines = self.render_message_block_to_lines(
                messages,
                block,
                width,
                body_width,
                &mut card_ranges,
                &mut user_card_ranges,
                &mut selectable_regions_ranges,
                &mut inline_running_card_ranges,
                current_line_offset,
                &ctx,
                is_round_end,
            );
            current_line_offset += block_lines.len();
            lines.extend(block_lines);
        }

        let last_block_end = visible_blocks
            .last()
            .map(|b| b.start_line + b.line_count)
            .unwrap_or(0);
        let missing_lines = total_message_lines.saturating_sub(last_block_end);
        for _ in 0..missing_lines {
            lines.push(Line::from(""));
        }

        // Calculate total lines from layout index
        let mut total_lines = header_line_count + total_overall_lines;

        // Append retrying hint as a temporary message at the bottom of chat area
        if let Some((attempt, max_attempts, reason, deadline)) = self.retrying_hint.as_ref() {
            let now = Instant::now();
            let remaining = if *deadline > now {
                deadline.duration_since(now).as_secs()
            } else {
                0
            };

            let retry_after_str = format!("Retrying in {remaining}s");
            let msg = format!("Retrying ({}/{}): {}", attempt, max_attempts, reason);

            let text_width = body_width.saturating_sub(2).max(1);
            let mut retry_lines = Vec::new();

            // Wrap the retry message with word-wrap
            let wrapped = wrap_text_lines(&msg, text_width, usize::MAX);
            for (i, line) in wrapped.iter().enumerate() {
                if i == 0 {
                    retry_lines.push(line_with_prefix(
                        "⟳",
                        line,
                        Style::default().fg(palette.accent_soft),
                        Style::default().fg(palette.text),
                    ));
                } else {
                    retry_lines.push(line_with_prefix(
                        " ",
                        line,
                        Style::default().fg(palette.accent_soft),
                        Style::default().fg(palette.text),
                    ));
                }
            }

            // Countdown line
            retry_lines.push(line_with_prefix(
                "⟳",
                &retry_after_str,
                Style::default().fg(palette.accent_soft),
                Style::default().fg(palette.muted),
            ));

            // Wrap in card with padding (same style as error messages)
            let mut card_lines = Vec::new();
            card_lines.push(Line::from(""));
            card_lines.extend(retry_lines);
            card_lines.push(Line::from(""));

            let decorated = decorate_card_lines(card_lines, width, palette.panel_light, 2);
            let hint_line_count = decorated.len();
            lines.extend(decorated);
            total_lines += hint_line_count;
        }

        let elapsed = started_at.elapsed();
        if elapsed > Duration::from_millis(12) {
            let (hits, misses, entries) = self.message_render_cache_stats();
            log::debug!(
                "messages_text: messages={}, visible_blocks={}, width={}, took={:?}, cache_hits={}, cache_misses={}, cache_entries={}",
                messages.len(),
                visible_blocks.len(),
                width,
                elapsed,
                hits,
                misses,
                entries
            );
        }

        (
            Text::from(lines),
            total_lines,
            card_ranges,
            user_card_ranges,
            selectable_regions_ranges,
            true,
            render_scroll,
            inline_running_card_ranges,
        )
    }

    fn cached_render_tool_call_with_result(
        &self,
        message: &Message,
        tool_call: &ToolCall,
        tool_result: Option<&Message>,
        body_width: usize,
        is_streaming: bool,
        ctx: &RenderContext<'_>,
    ) -> (Vec<Line<'static>>, Vec<SelectableRegionRange>) {
        if body_width == 0 {
            return (Vec::new(), Vec::new());
        }

        let tick = self.next_message_render_cache_tick();
        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id, // Binds the cache to the Assistant message hosting this tool call
            width: body_width,
            is_round_end: !is_streaming, // Approximation, cache differs when streaming is done
            kind: MessageRenderCacheKind::ToolCall(tool_call.id.clone()),
        };

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key) {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                match &entry.value {
                    MessageRenderCacheValue::ToolResult(lines, regions) => {
                        return (lines.clone(), regions.clone());
                    }
                    MessageRenderCacheValue::Cards(..) => {}
                }
            }
        }

        self.record_message_render_cache_miss();
        let result = tool::render_tool_call_with_result(
            tool_call,
            tool_result,
            body_width,
            is_streaming,
            ctx,
        );

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::ToolResult(result.0.clone(), result.1.clone()),
                    last_used_tick: tick,
                },
            );
        }

        result
    }

    fn cached_render_message_cards(
        &self,
        ctx: &RenderContext<'_>,
        message: &Message,
        body_width: usize,
        is_round_end: bool,
    ) -> Vec<(Color, Vec<Line<'static>>)> {
        let key = MessageRenderCacheKey {
            session_id: self.conversation.session_id,
            message_id: message.id,
            width: body_width,
            is_round_end,
            kind: MessageRenderCacheKind::Cards,
        };
        let tick = self.next_message_render_cache_tick();

        {
            let mut cache = self.message_render_cache.borrow_mut();
            if let Some(entry) = cache.get_mut(&key) {
                entry.last_used_tick = tick;
                self.record_message_render_cache_hit();
                match &entry.value {
                    MessageRenderCacheValue::Cards(cards) => return cards.clone(),
                    MessageRenderCacheValue::ToolResult(..) => {} // Should never happen with .Cards kind
                }
            }
        }

        self.record_message_render_cache_miss();
        let cards = content::render_message_cards_inner(ctx, message, body_width, is_round_end);

        {
            let mut cache = self.message_render_cache.borrow_mut();
            cache.insert(
                key,
                MessageRenderCacheEntry {
                    value: MessageRenderCacheValue::Cards(cards.clone()),
                    last_used_tick: tick,
                },
            );
        }

        self.prune_message_render_cache_if_needed();
        cards
    }

    fn load_expanded_tool_outputs(&self, messages: &[Message]) -> HashMap<Uuid, String> {
        let mut map = HashMap::new();
        for msg in messages
            .iter()
            .filter(|m| matches!(m.role, MessageRole::Tool))
        {
            if !self.expanded_tool_results.contains(&msg.id) {
                continue;
            }
            // Try to load the full output from the tool_outputs table.
            if let Ok(Some(output)) = self.store.load_tool_output(msg.id) {
                map.insert(msg.id, output);
            }
            // Not in the table → message.content (preview) will be used instead.
        }
        map
    }

    /// Count the number of visual lines a running subagent card will occupy.
    ///
    /// This mirrors render_running_subagent_lines() but only computes the count.
    /// Used by update_message_layout_index() to correct the layout index height
    /// for blocks containing running subagent task tool calls — the generic tool
    /// call card (2 lines) is replaced at render time by the taller running card.
    fn count_running_subagent_card_lines(
        execution: &RunningSubagentExecution,
        body_width: usize,
    ) -> usize {
        let mut count = 0;
        // Top padding
        count += 1;
        // Header line (word-wrapped)
        let header_text = format!(
            "@{} subagent: {}",
            execution.subagent_type,
            execution.task_description.trim()
        );
        count += word_wrap_line(
            &Line::from(header_text),
            WrapOptions::new(body_width).break_words(true),
        )
        .len();
        // Status line
        count += 1;
        // Streaming content preview (if any)
        if !execution.streaming_content.trim().is_empty() {
            count += 1;
        }
        // Bottom padding
        count += 1;
        count
    }

    fn render_running_subagent_lines(
        &self,
        execution: &RunningSubagentExecution,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();

        // Title line: [@type] subagent: [description]
        let description = execution.task_description.trim();
        let subagent_type = execution.subagent_type.clone();

        let mut lines = Vec::new();

        // Top padding
        lines.push(Line::from(""));

        // Header line with @type and description, word-wrapped
        let header_line = Line::from(vec![
            Span::styled(
                format!("@{}", subagent_type),
                Style::default().fg(palette.accent_soft),
            ),
            Span::styled(" subagent: ", Style::default().fg(palette.muted)),
            Span::styled(
                description.to_string(),
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        lines.extend(
            word_wrap_line(&header_line, WrapOptions::new(body_width).break_words(true))
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

        // Status line
        let status_text = execution.status.display();
        let status_line = match &execution.status {
            SubagentStatus::Tool => {
                if let Some(tool_call) = &execution.current_tool_call {
                    let tool_summary =
                        if tool::tool_call_arguments_are_complete(&tool_call.arguments) {
                            utils::summarize_tool_call(
                                &tool_call.name,
                                &tool_call.arguments,
                                body_width.saturating_sub(10),
                                self.workspace_root.as_path(),
                            )
                        } else {
                            let canonical_display = canonical_tool_name(&tool_call.name)
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| tool_call.name.clone());
                            format!("{} ...", canonical_display)
                        };
                    format!("{}: {}", status_text, tool_summary)
                } else {
                    status_text.to_string()
                }
            }
            _ => status_text.to_string(),
        };

        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(status_line, Style::default().fg(palette.accent_soft)),
        ]));

        // Streaming content preview (if any)
        let content = execution.streaming_content.trim().to_string();
        if !content.is_empty() {
            let preview = shorten_single_line(&content, body_width.saturating_sub(4));
            lines.push(Line::from(vec![
                Span::styled("  ".to_string(), Style::default()),
                Span::styled(
                    preview,
                    Style::default().fg(palette.text).add_modifier(Modifier::DIM),
                ),
            ]));
        }

        // Bottom padding
        lines.push(Line::from(""));

        lines
    }

    #[allow(dead_code)]
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

    /// Updates the message layout index to enable viewport virtualization.
    ///
    /// The layout index maintains a mapping from messages to their positions in
    /// the rendered output. This enables O(log n) binary search to find visible
    /// messages without rendering everything.
    ///
    /// The index is rebuilt when:
    /// - Width changes (line counts become invalid)
    /// - Messages are added/removed
    /// - Cache is cleared
    /// - Force rebuild is requested (for streaming messages)
    ///
    /// For incremental updates, only blocks with dirty messages are recomputed,
    /// preserving positions for unchanged blocks.
    fn update_message_layout_index(&self, width: usize, body_width: usize, force_rebuild: bool) {
        let messages = self.conversation.visible_messages();
        let mut index = self.message_layout_index.borrow_mut();

        // Check if message count changed (new messages added or removed)
        let indexed_message_count = index
            .blocks
            .last()
            .map(|b| b.message_start_idx + b.message_count)
            .unwrap_or(0);
        let message_count_changed = indexed_message_count != messages.len();
        let streaming_mode_changed = index.contains_streaming_messages != force_rebuild;

        // Check if we need a full rebuild
        let needs_full_rebuild = force_rebuild
            || streaming_mode_changed
            || !index.valid
            || index.width != width
            || message_count_changed
            || index.blocks.is_empty() && !messages.is_empty();

        if needs_full_rebuild {
            index.blocks.clear();
            index.total_lines = 0;
            index.width = width;
            index.valid = true;
            index.contains_streaming_messages = force_rebuild;
            index.dirty_messages.clear();

            if messages.is_empty() {
                return;
            }

            let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
            let spinner = self.loading_spinner();
            let ctx = RenderContext {
                palette: self.palette(),
                spinner,
                workspace_root: self.workspace_root.as_path(),
                expanded_tool_results: &self.expanded_tool_results,
                expanded_tool_outputs: &expanded_tool_outputs,
                config: self.config.clone(),
                auth: &self.auth,
                conversation: &self.conversation,
                mode: self.mode,
            };
            let session_id = self.conversation.session_id;

            // Step 1: Determine block boundaries sequentially (cheap)
            struct BlockInfo {
                start_idx: usize,
                is_round_end: bool,
            }
            let mut blocks_info = Vec::new();
            let mut i = 0;
            while i < messages.len() {
                let count = if matches!(messages[i].role, MessageRole::Assistant) {
                    let mut c = 1;
                    while i + c < messages.len()
                        && matches!(messages[i + c].role, MessageRole::Tool)
                    {
                        c += 1;
                    }
                    c
                } else {
                    1
                };
                let next_idx = i + count;
                let is_round_end = next_idx >= messages.len()
                    || matches!(messages[next_idx].role, MessageRole::User);
                blocks_info.push(BlockInfo {
                    start_idx: i,
                    is_round_end,
                });
                i += count;
            }

            // Step 2: Compute block data in parallel using rayon
            // (RenderContext is Sync, so it can be shared across threads)
            if !blocks_info.is_empty() {
                // Use sequential iteration for small message batches to avoid rayon dispatch overhead
                let computations: Vec<BlockComputation> = if blocks_info.len() > 4 {
                    blocks_info
                        .par_iter()
                        .map(|info| {
                            content::compute_block_data(
                                &ctx,
                                session_id,
                                messages,
                                info.start_idx,
                                width,
                                body_width,
                                info.is_round_end,
                            )
                        })
                        .collect()
                } else {
                    blocks_info
                        .iter()
                        .map(|info| {
                            content::compute_block_data(
                                &ctx,
                                session_id,
                                messages,
                                info.start_idx,
                                width,
                                body_width,
                                info.is_round_end,
                            )
                        })
                        .collect()
                };

                // Step 3: Build layout index and insert cache entries sequentially
                let mut current_line = 0;
                let mut cache = self.message_render_cache.borrow_mut();
                for (comp_idx, comp) in computations.iter().enumerate() {
                    // Adjust line count for running subagent task tool calls:
                    // compute_block_data uses render_tool_call_with_result which returns
                    // a small 2-line generic card for task tools with no result, but
                    // render_message_block_to_lines replaces it with a taller running
                    // subagent card (4+ lines). Correct the layout index to match.
                    let start_idx = blocks_info[comp_idx].start_idx;
                    let mut line_count = comp.line_count;
                    if let Some(msg) = messages.get(start_idx) {
                        if msg.role == MessageRole::Assistant {
                            for tool_call in &msg.tool_calls {
                                if tool_call.name == "task" {
                                    if let Some(execution) = self
                                        .running_subagent_executions
                                        .iter()
                                        .find(|e| e.tool_call.id == tool_call.id)
                                    {
                                        let running_height = Self::count_running_subagent_card_lines(
                                            execution,
                                            body_width,
                                        );
                                        // Generic tool call card: 1 empty + 1 summary = 2 lines
                                        line_count = line_count.saturating_sub(2) + running_height;
                                    }
                                }
                            }
                        }
                    }

                    let block = super::MessageBlock {
                        message_id: comp.message_id,
                        message_start_idx: blocks_info[comp_idx].start_idx,
                        message_count: comp.message_count,
                        start_line: current_line,
                        line_count,
                    };
                    current_line += line_count;
                    index.blocks.push(block);

                    // Insert cache entries with fresh ticks
                    for (key, entry) in &comp.cache_entries {
                        let tick = self.next_message_render_cache_tick();
                        cache.insert(
                            key.clone(),
                            MessageRenderCacheEntry {
                                value: entry.value.clone(),
                                last_used_tick: tick,
                            },
                        );
                    }
                }
                index.total_lines = current_line;
            }
        } else if !index.dirty_messages.is_empty() {
            // Incremental update: only recompute blocks with dirty messages
            let dirty_ids: std::collections::HashSet<Uuid> =
                index.dirty_messages.drain(..).collect();

            let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
            let spinner = self.loading_spinner();
            let ctx = RenderContext {
                palette: self.palette(),
                spinner,
                workspace_root: self.workspace_root.as_path(),
                expanded_tool_results: &self.expanded_tool_results,
                expanded_tool_outputs: &expanded_tool_outputs,
                config: self.config.clone(),
                auth: &self.auth,
                conversation: &self.conversation,
                mode: self.mode,
            };
            let session_id = self.conversation.session_id;

            // Find and recompute dirty blocks
            let mut i = 0;
            while i < index.blocks.len() {
                let block = &index.blocks[i];
                let msg = &messages[block.message_start_idx];
                if dirty_ids.contains(&msg.id)
                    || dirty_ids.iter().any(|id| {
                        messages
                            [block.message_start_idx..block.message_start_idx + block.message_count]
                            .iter()
                            .any(|m| &m.id == id)
                    })
                {
                    // Recompute this block
                    let is_round_end = {
                        let next_idx = block.message_start_idx + block.message_count;
                        next_idx >= messages.len()
                            || matches!(messages[next_idx].role, MessageRole::User)
                    };
                    let comp = content::compute_block_data(
                        &ctx,
                        session_id,
                        messages,
                        block.message_start_idx,
                        width,
                        body_width,
                        is_round_end,
                    );

                    // Apply the same running-subagent line-count adjustment as in
                    // the full rebuild path above.
                    let mut adjusted_line_count = comp.line_count;
                    if let Some(msg) = messages.get(block.message_start_idx) {
                        if msg.role == MessageRole::Assistant {
                            for tool_call in &msg.tool_calls {
                                if tool_call.name == "task" {
                                    if let Some(execution) = self
                                        .running_subagent_executions
                                        .iter()
                                        .find(|e| e.tool_call.id == tool_call.id)
                                    {
                                        let running_height =
                                            Self::count_running_subagent_card_lines(
                                                execution,
                                                body_width,
                                            );
                                        adjusted_line_count =
                                            adjusted_line_count.saturating_sub(2) + running_height;
                                    }
                                }
                            }
                        }
                    }

                    let old_line_count = index.blocks[i].line_count;
                    let line_count_diff =
                        adjusted_line_count as isize - old_line_count as isize;

                    // Update the block
                    index.blocks[i].line_count = adjusted_line_count;
                    index.blocks[i].message_count = comp.message_count;

                    // Adjust subsequent blocks' start_line
                    if line_count_diff != 0 {
                        for j in (i + 1)..index.blocks.len() {
                            index.blocks[j].start_line =
                                (index.blocks[j].start_line as isize + line_count_diff) as usize;
                        }
                        index.total_lines = (index.total_lines as isize + line_count_diff) as usize;
                    }

                    // Insert cache entries for this block
                    let mut cache = self.message_render_cache.borrow_mut();
                    for (key, entry) in &comp.cache_entries {
                        let tick = self.next_message_render_cache_tick();
                        cache.insert(
                            key.clone(),
                            MessageRenderCacheEntry {
                                value: entry.value.clone(),
                                last_used_tick: tick,
                            },
                        );
                    }
                    drop(cache);
                }
                i += 1;
            }
        }
    }

    fn resolve_message_scroll_target(
        &self,
        messages: &[Message],
        width: usize,
        body_width: usize,
    ) -> Option<usize> {
        let message_id = self.message_scroll_target?;

        // Create a minimal context for block data calculation
        let expanded_tool_outputs = self.load_expanded_tool_outputs(messages);
        let spinner = self.loading_spinner();
        let ctx = RenderContext {
            palette: self.palette(),
            spinner,
            workspace_root: self.workspace_root.as_path(),
            expanded_tool_results: &self.expanded_tool_results,
            expanded_tool_outputs: &expanded_tool_outputs,
            config: self.config.clone(),
            auth: &self.auth,
            conversation: &self.conversation,
            mode: self.mode,
        };

        let mut offset = 0;
        let mut i = 0;

        while i < messages.len() {
            if messages[i].id == message_id {
                return Some(offset);
            }

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
                next_idx >= messages.len() || matches!(messages[next_idx].role, MessageRole::User);

            let (_message_id, message_count, line_count) =
                self.build_message_block_data(messages, i, width, body_width, &ctx, is_round_end);
            offset += line_count;
            i += message_count;
        }

        None
    }

    /// Builds data for a single message block (without start_line).
    ///
    /// Returns (message_id, message_count, line_count).
    fn build_message_block_data(
        &self,
        messages: &[Message],
        start_idx: usize,
        _width: usize,
        body_width: usize,
        ctx: &RenderContext<'_>,
        is_round_end: bool,
    ) -> (Uuid, usize, usize) {
        let message = &messages[start_idx];
        let message_id = message.id;

        let (message_count, line_count) = match message.role {
            MessageRole::Assistant => {
                // Count tool result messages that follow
                let mut count = 1;
                while start_idx + count < messages.len()
                    && matches!(messages[start_idx + count].role, MessageRole::Tool)
                {
                    count += 1;
                }

                // Calculate lines for assistant message
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines += card_lines.len();
                }

                // Calculate lines for tool calls with results
                let tool_results_by_id: std::collections::HashMap<String, &Message> = {
                    let mut map = std::collections::HashMap::new();
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
                        let (card_lines, _) = self.cached_render_tool_call_with_result(
                            message,
                            tool_call,
                            tool_result,
                            body_width,
                            message.streaming,
                            ctx,
                        );
                        if !card_lines.is_empty() {
                            lines += card_lines.len();
                        }
                    }
                    lines += 1; // Empty line after tool calls
                }

                (count, lines)
            }
            MessageRole::User => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                if Self::is_first_user_message(messages, start_idx) {
                    lines += 1; // Empty line above the first user message
                }
                for (_, card_lines) in &cards {
                    lines += card_lines.len();
                }
                lines += 1; // Empty line after user message
                (1, lines)
            }
            MessageRole::System => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines += card_lines.len();
                }
                (1, lines)
            }
            MessageRole::Error => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines += card_lines.len();
                }
                (1, lines)
            }
            MessageRole::Shell => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines += card_lines.len();
                }
                lines += 1; // Empty line after shell message
                (1, lines)
            }
            MessageRole::Tool => {
                // Tool messages are included in Assistant blocks, skip
                (1, 0)
            }
        };

        (message_id, message_count, line_count)
    }

    /// Finds message blocks that intersect with the visible viewport.
    ///
    /// Uses binary search for O(log n) complexity. Returns blocks with a
    /// buffer zone to ensure smooth scrolling.
    fn find_visible_message_blocks(
        &self,
        scroll: usize,
        viewport_height: usize,
    ) -> Vec<super::MessageBlock> {
        let index = self.message_layout_index.borrow();

        if index.blocks.is_empty() {
            return Vec::new();
        }

        let viewport_height = viewport_height.max(1);
        let max_scroll = index.total_lines.saturating_sub(viewport_height);
        let clamped_scroll = scroll.min(max_scroll);

        let visible_start = clamped_scroll.saturating_sub(5); // Buffer above
        let visible_end = clamped_scroll
            .saturating_add(viewport_height)
            .saturating_add(5); // Buffer below

        // Binary search for first block that could be visible
        let first_visible = index
            .blocks
            .partition_point(|block| block.start_line + block.line_count <= visible_start);

        // Collect all visible blocks
        let mut visible_blocks = Vec::new();
        for block in index.blocks.iter().skip(first_visible) {
            if block.start_line >= visible_end {
                break;
            }
            visible_blocks.push(block.clone());
        }

        visible_blocks
    }

    fn is_first_user_message(messages: &[Message], start_idx: usize) -> bool {
        matches!(messages[start_idx].role, MessageRole::User)
            && !messages[..start_idx]
                .iter()
                .any(|m| matches!(m.role, MessageRole::User))
    }

    /// Renders a single message block to lines.
    ///
    /// This is the actual rendering logic, extracted for reuse in virtualization.
    #[allow(clippy::too_many_arguments)]
    fn render_message_block_to_lines(
        &self,
        messages: &[Message],
        block: &super::MessageBlock,
        width: usize,
        body_width: usize,
        card_ranges: &mut Vec<ToolResultCardRange>,
        user_card_ranges: &mut Vec<(Uuid, usize, usize)>,
        selectable_regions_ranges: &mut Vec<SelectableRegionRange>,
        inline_running_card_ranges: &mut Vec<InlineRunningCardRange>,
        current_line_offset: usize,
        ctx: &RenderContext<'_>,
        is_round_end: bool,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Skip Tool messages - they're rendered as part of Assistant blocks
        if block.message_count == 0 {
            return lines;
        }

        let start_idx = block.message_start_idx;
        let message = &messages[start_idx];

        match message.role {
            MessageRole::Assistant => {
                // Render assistant message cards
                let assistant_cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                for (card_bg, card_lines) in assistant_cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();

                        // Only make content lines selectable — skip leading/trailing blank spacer lines
                        let first_content = card_lines.iter().position(|l| {
                            !l.spans.is_empty() && l.spans.iter().any(|s| !s.content.is_empty())
                        });
                        let last_content = card_lines.iter().rposition(|l| {
                            !l.spans.is_empty() && l.spans.iter().any(|s| !s.content.is_empty())
                        });
                        if let (Some(first), Some(last)) = (first_content, last_content) {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: start_line + first,
                                end_line: start_line + last + 1,
                                min_x: 2,
                                max_x: None,
                            });
                        }

                        lines.extend(decorate_card_lines(card_lines, width, card_bg, 2));
                    }
                }

                // Collect tool results
                let tool_results_by_id: std::collections::HashMap<String, &Message> = {
                    let mut map = std::collections::HashMap::new();
                    let mut j = start_idx + 1;
                    while j < messages.len() && j < start_idx + block.message_count {
                        if matches!(messages[j].role, MessageRole::Tool)
                            && let Some(id) = &messages[j].tool_call_id
                        {
                            map.insert(id.clone(), &messages[j]);
                        }
                        j += 1;
                    }
                    map
                };

                // Render tool calls with results
                if !message.tool_calls.is_empty() {
                    for tool_call in &message.tool_calls {
                        let tool_result = tool_results_by_id.get(&tool_call.id).copied();

                        // For task tools with no result yet: if a subagent is still running,
                        // render the running card inline instead of the tool call header.
                        // This eliminates the dual-card problem (tool call card + running overlay).
                        if tool_result.is_none()
                            && tool_call.name == "task"
                            && let Some(exec_index) = self
                                .running_subagent_executions
                                .iter()
                                .position(|e| e.tool_call.id == tool_call.id)
                        {
                            let execution = &self.running_subagent_executions[exec_index];
                            let running_lines =
                                self.render_running_subagent_lines(execution, body_width);
                            let start_line = current_line_offset + lines.len();
                            let mut card_bg = palette.panel;
                            if self.hovered_inline_subagent == Some(exec_index) {
                                card_bg = palette.hover_bg(card_bg);
                            }
                            let decorated = decorate_card_lines(running_lines, width, card_bg, 2);
                            lines.extend(decorated);
                            let end_line = current_line_offset + lines.len();
                            inline_running_card_ranges.push(InlineRunningCardRange {
                                execution_index: exec_index,
                                start_line,
                                end_line,
                            });
                            continue;
                        }

                        let (tool_card_lines, mut regions) = self
                            .cached_render_tool_call_with_result(
                                message,
                                tool_call,
                                tool_result,
                                body_width,
                                message.streaming,
                                ctx,
                            );
                        if !tool_card_lines.is_empty() {
                            let start_line = current_line_offset + lines.len();

                            // Adjust regions mapping
                            for r in &mut regions {
                                r.start_line += start_line;
                                r.end_line += start_line;
                                r.min_x += 2; // decorate_card_lines left padding
                                if let Some(max_x) = &mut r.max_x {
                                    *max_x += 2;
                                }
                                selectable_regions_ranges.push(r.clone());
                            }

                            // Calculate fallback region for bash or non-diff output
                            if regions.is_empty() {
                                // Trim leading/trailing blank spacer lines
                                let first = tool_card_lines.iter().position(|l| {
                                    !l.spans.is_empty()
                                        && l.spans.iter().any(|s| !s.content.is_empty())
                                });
                                let last = tool_card_lines.iter().rposition(|l| {
                                    !l.spans.is_empty()
                                        && l.spans.iter().any(|s| !s.content.is_empty())
                                });
                                if let (Some(f), Some(l)) = (first, last) {
                                    selectable_regions_ranges.push(SelectableRegionRange {
                                        start_line: start_line + f,
                                        end_line: start_line + l + 1,
                                        min_x: 2,
                                        max_x: None,
                                    });
                                }
                            }

                            let mut card_bg =
                                if canonical_tool_name(&tool_call.name) == Some("task") {
                                    palette.panel
                                } else {
                                    palette.panel_light
                                };
                            // Apply hover highlight only when clicking the card actually
                            // changes its visual content — i.e., the renderer uses
                            // expanded_tool_results AND the output exceeds the preview
                            // threshold (5 lines).  Tools whose renderers never vary with
                            // expansion state are excluded entirely.
                            if let Some(result_msg) = tool_result {
                                let canonical = canonical_tool_name(&tool_call.name);
                                let has_expandable = match canonical {
                                    // These tools' renderers never use expanded_tool_results
                                    Some(
                                        "read" | "grep" | "glob" | "skill" | "question"
                                        | "todowrite",
                                    ) => false,
                                    // write/edit/apply_patch normally render diff text (no expand).
                                    // When no diff is available (e.g. output too large), they
                                    // fall through to render_output_preview_lines which IS
                                    // expandable.
                                    Some("write" | "edit" | "apply_patch") => {
                                        result_msg.metadata.diff.is_none()
                                            && result_msg.content.lines().count()
                                                > TOOL_OUTPUT_PREVIEW_LINES
                                    }
                                    // All other tools (task, websearch, webfetch, memory,
                                    // bash, MCP, etc.) use expanded_tool_results — only
                                    // meaningful if output exceeds preview threshold
                                    _ => {
                                        result_msg.content.lines().count()
                                            > TOOL_OUTPUT_PREVIEW_LINES
                                    }
                                };
                                if self.hovered_card == Some(result_msg.id) && has_expandable {
                                    card_bg = palette.hover_bg(card_bg);
                                }
                            }
                            let decorated = decorate_card_lines(tool_card_lines, width, card_bg, 2);
                            if let Some(result_msg) = tool_result {
                                lines.extend(decorated);
                                let end_line = current_line_offset + lines.len();
                                card_ranges.push(ToolResultCardRange {
                                    message_id: result_msg.id,
                                    start_line,
                                    end_line,
                                });
                            } else {
                                lines.extend(decorated);
                            }
                        }
                    }
                    lines.push(Line::from(""));
                }
            }
            MessageRole::User | MessageRole::System | MessageRole::Error | MessageRole::Shell => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut bg = match message.role {
                    MessageRole::User => palette.panel_alt,
                    MessageRole::Error => palette.panel_light,
                    _ => palette.background,
                };
                // Apply hover highlight for user message cards
                if matches!(message.role, MessageRole::User | MessageRole::Shell)
                    && self.hovered_card == Some(message.id)
                {
                    bg = palette.hover_bg(bg);
                }
                if Self::is_first_user_message(messages, start_idx) {
                    lines.push(Line::from(""));
                }
                for (_, card_lines) in cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();

                        // Only make content lines selectable — skip leading/trailing spacer lines
                        // that only have ┃ with no actual content
                        let is_other_line = |l: &Line<'static>| {
                            !l.spans.is_empty()
                                && !l
                                    .spans
                                    .iter()
                                    .any(|s| !s.content.is_empty() && s.content != "┃ ")
                        };
                        if let (Some(first), Some(last)) = (
                            card_lines.iter().position(|l| !is_other_line(l)),
                            card_lines.iter().rposition(|l| !is_other_line(l)),
                        ) {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: start_line + first,
                                end_line: start_line + last + 1,
                                min_x: 2,
                                max_x: None,
                            });
                        }

                        lines.extend(decorate_card_lines(card_lines, width, bg, 2));
                        // Record user card bounds for hover detection
                        let end_line = current_line_offset + lines.len();
                        user_card_ranges.push((message.id, start_line, end_line));
                    }
                }
                if matches!(
                    message.role,
                    MessageRole::User
                        | MessageRole::Shell
                        | MessageRole::System
                        | MessageRole::Error
                ) {
                    lines.push(Line::from(""));
                }
            }
            MessageRole::Tool => {
                // Tool messages are handled within Assistant blocks
            }
        }

        lines
    }

    fn render_scrollbar(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        scroll: usize,
        _max_scroll: usize,
    ) {
        super::render::render_scrollbar(
            frame,
            area,
            scroll,
            self.message_total_lines,
            self.palette(),
            self.scrollbar_hovered,
        );
    }
}
