mod content;
#[cfg(test)]
mod tests;
mod tool;
mod utils;

use crate::{
    config::{AppConfig, AuthStore},
    markdown_render::{WrapOptions, word_wrap_line},
    prompts::SessionMode,
    session::{Conversation, Message, MessageRole, ToolCall},
    theme::ThemePalette,
    tooling::canonical_tool_name,
    tui::core::state::{
        MessageRenderCacheEntry, MessageRenderCacheKey, MessageRenderCacheKind,
        MessageRenderCacheValue, SelectableRegionRange,
    },
    tui::App,
    utils::{TokenUsage, format_token_count},
};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    prelude::{Frame, Modifier, Style, Text},
    style::Color,
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};
use unicode_width::UnicodeWidthStr;
use uuid::Uuid;

use super::super::permission::{RunningSubagentExecution, SubagentStatus};
use crate::tui::render::render::{
    decorate_card_lines, line_with_prefix, line_with_style,
    shorten, shorten_single_line,
};

use crate::tui::chat_render::content::BlockComputation;

const TOOL_OUTPUT_PREVIEW_LINES: usize = 5;
const TOOL_OUTPUT_EXPANDED_MAX_LINES: usize = 100;
const MAX_VISIBLE_QUEUED_PROMPTS: usize = 4;

#[derive(Clone, Debug)]
struct ToolResultCardRange {
    message_id: Uuid,
    start_line: usize,
    end_line: usize,
}

#[derive(Clone, Debug)]
struct RunningCardRange {
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
    config: &'a AppConfig,
    auth: &'a AuthStore,
    conversation: &'a Conversation,
    mode: SessionMode,
}

impl App {
    pub(super) fn render_chat(&mut self, frame: &mut Frame<'_>) {
        let area = frame.area();
        let palette = self.palette();
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        let sidebar_visible = area.width >= self.config.ui.sidebar_width.saturating_add(70);
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

        let composer_height_raw = self
            .composer
            .preferred_height(
                main_area.width.saturating_sub(4),
                self.config.ui.max_input_lines,
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
            // inner: visible text lines + (visible-1) separator lines
            let inner = visible + (visible.saturating_sub(1));
            // +1 for "+N more" overflow, +2 for block top/bottom borders
            let overflow = if queued_count > MAX_VISIBLE_QUEUED_PROMPTS {
                1
            } else {
                0
            };
            (inner + overflow + 2)
                .min(main_area.height.saturating_sub(6) as usize / 2)
                .min(12)
        } else {
            0
        };

        let composer_height = composer_height_raw.min(
            main_area
                .height
                .saturating_sub((queued_height as u16) + 3)
                .max(3),
        );

        // Handle workspace boundary dialog (similar to question dialog)
        if let Some(dialog) = self.workspace_boundary_dialog.clone() {
            let dialog_height = dialog
                .dialog_height(main_area.width)
                .min(main_area.height.saturating_sub(3).max(6));

            let layout = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(dialog_height),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(main_area);

            self.render_messages(frame, layout[0]);
            self.render_workspace_boundary_dialog(frame, layout[1], &dialog);
            self.render_prompt_footer(frame, layout[2]);
            self.render_retrying_hint(frame, layout[3]);
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
            Constraint::Length(queued_height as u16),
            Constraint::Length(composer_height),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(main_area);

        self.render_messages(frame, layout[0]);

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
            self.render_input_block(
                frame,
                layout[2],
                &prompt_title,
                self.composer.placeholder(),
                false,
            );
            self.render_at_mention_palette(frame, layout[2]);
            self.render_snippet_palette(frame, layout[2]);
            self.render_command_palette(frame, layout[2]);
            self.render_snippet_palette(frame, layout[2]);
            self.render_shell_completion_palette(frame, layout[2]);
        }
        self.render_prompt_footer(frame, layout[3]);
        self.render_retrying_hint(frame, layout[4]);
    }

    /// Render a frozen area above the input box showing queued (pending) prompts.
    /// Each queued message is displayed as a single truncated line with a separator
    /// between items, wrapped in a top/bottom bordered block with a "QUEUE" badge.
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

        let block = Block::default()
            .borders(Borders::TOP | Borders::BOTTOM)
            .border_style(Style::default().fg(palette.muted))
            .title(title)
            .title_alignment(Alignment::Left);

        let inner = block.inner(area);
        let inner_height = inner.height as usize;
        let width = inner.width.max(1) as usize;

        let mut y_offset = 0u16;

        for (i, queued) in self.pending_prompt_queue.iter().take(visible).enumerate() {
            if y_offset as usize >= inner_height {
                break;
            }

            // Truncate prompt to a single line
            let text = shorten_single_line(&queued.prompt, width);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(palette.muted)
                        .add_modifier(Modifier::ITALIC),
                )))
                .wrap(Wrap { trim: false }),
                Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
            );
            y_offset += 1;

            // Separator line (not after last visible item)
            if i + 1 < visible && (y_offset as usize) < inner_height {
                let sep = "─".repeat(width.saturating_sub(2));
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        sep,
                        Style::default().fg(palette.border),
                    ))),
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
        frame.render_widget(block, area);
    }

    fn render_subsession_navigation(&self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(palette.border_idle()))
            .title(" Subsession ");

        frame.render_widget(block, area);

        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Center the navigation hints
        let hint = Line::from(vec![
            Span::styled("Up", Style::default().fg(palette.accent_soft)),
            Span::styled(": return to parent  ", Style::default().fg(palette.muted)),
            Span::styled("Left", Style::default().fg(palette.accent_soft)),
            Span::styled("/", Style::default().fg(palette.muted)),
            Span::styled("Right", Style::default().fg(palette.accent_soft)),
            Span::styled(": switch subagent", Style::default().fg(palette.muted)),
        ]);

        let paragraph = Paragraph::new(hint)
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.text));

        frame.render_widget(paragraph, inner);
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

        let scrollbar_area = if inner.width > 2 {
            let chunks = Layout::horizontal([
                Constraint::Min(1),
                Constraint::Length(1), // gap between content and scrollbar
                Constraint::Length(1),
            ])
            .split(inner);
            (chunks[0], Some(chunks[2]))
        } else if inner.width > 1 {
            let chunks =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(1)]).split(inner);
            (chunks[0], Some(chunks[1]))
        } else {
            (inner, None)
        };

        let content_area = scrollbar_area.0;
        self.message_content_area = Some(content_area);
        self.message_viewport_lines = content_area.height as usize;
        let content_width = content_area.width.max(1) as usize;
        let (
            text,
            total_lines,
            card_ranges,
            selectable_regions_ranges,
            rendered_virtualized,
            virtualized_render_scroll,
            running_card_ranges,
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

        // Calculate screen positions for running subagent cards
        // running_card_ranges contain positions within the running_lines block,
        // which starts at (header_line_count + total_message_lines) in the full text.
        let header_line_count = if self.conversation.parent_session_id.is_some() {
            3
        } else {
            0
        };
        let total_msg_lines = self.message_layout_index.borrow().total_lines;
        let running_block_start = header_line_count + total_msg_lines;

        self.running_subagent_card_bounds.clear();
        for card_range in &running_card_ranges {
            let abs_start = running_block_start + card_range.start_line;
            let abs_end = running_block_start + card_range.end_line;

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
                self.running_subagent_card_bounds
                    .push((card_range.execution_index, card_rect));
            }
        }

        let paragraph = Paragraph::new(text)
            .style(Style::default().bg(palette.background).fg(palette.text))
            .scroll((render_scroll as u16, 0));

        frame.render_widget(paragraph, content_area);

        if let Some(scrollbar_area) = scrollbar_area.1 {
            self.message_scrollbar_area = Some(scrollbar_area);
            self.render_scrollbar(frame, scrollbar_area, scroll, max_scroll);
        }
    }

    fn render_sidebar(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = self.palette();
        let mut lines = Vec::new();

        // Workspace directory (top)
        let workspace_path = self.workspace_root.display().to_string();
        let display_path = workspace_path.replace(
            &dirs::home_dir().unwrap_or_default().display().to_string(),
            "~",
        );
        lines.push(Line::from(vec![Span::styled(
            "Workspace",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            display_path,
            Style::default().fg(palette.muted),
        )]));

        lines.push(Line::from(""));

        // Model info
        lines.push(Line::from(vec![Span::styled(
            "Model",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(vec![Span::styled(
            self.active_model.label(),
            Style::default().fg(palette.text),
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
        lines.push(Line::from(vec![Span::styled(
            "Changed Files",
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        )]));

        {
            let mut all_diffs = Vec::new();
            let mut seen_files = std::collections::HashSet::new();
            for msg in self.conversation.visible_messages() {
                if let Some(diffs_json) = &msg.file_diffs
                    && let Ok(diffs) =
                        serde_json::from_str::<Vec<crate::snapshot::FileDiff>>(diffs_json)
                {
                    for d in &diffs {
                        if seen_files.insert(d.file.clone()) {
                            all_diffs.push(d.clone());
                        }
                    }
                }
            }

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

                for d in &all_diffs {
                    let (status_icon, style) = match d.status.as_deref() {
                        Some("added") => ("+ ", Style::default().fg(palette.success)),
                        Some("deleted") => ("- ", Style::default().fg(palette.error)),
                        _ => ("~ ", Style::default().fg(palette.warning)),
                    };

                    let filename = Path::new(&d.file)
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| d.file.clone());

                    let summary = format!(
                        "{}{} (+{}/-{})",
                        status_icon, filename, d.additions, d.deletions
                    );
                    lines.push(Line::from(vec![Span::styled(summary, style)]));
                }
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
                "cancelled" => ("✗ ", Style::default().fg(palette.muted)),
                _ => ("○ ", Style::default().fg(palette.text)),
            };

            let priority_marker = if todo.priority == "high" { "⚠ " } else { "" };

            let content = &todo.content;
            lines.push(Line::from(vec![
                Span::styled(format!("{priority_marker}{checkbox}"), style),
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

        // Estimate total lines for scroll max (accounts for word wrapping)
        let sidebar_content_width = (area.width.saturating_sub(2)) as usize;
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

        let sidebar_viewport_lines = area.height.saturating_sub(2) as usize;
        let max_scroll = self
            .sidebar_total_lines
            .saturating_sub(sidebar_viewport_lines);
        self.sidebar_scroll_offset = self.sidebar_scroll_offset.min(max_scroll);

        let paragraph = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(palette.border_idle()))
                    .title("Sidebar"),
            )
            .style(Style::default().fg(palette.text))
            .wrap(Wrap { trim: false })
            .scroll((self.sidebar_scroll_offset as u16, 0));

        frame.render_widget(paragraph, area);
    }

    fn messages_text(
        &mut self,
        content_width: Option<usize>,
    ) -> (
        Text<'static>,
        usize,
        Vec<ToolResultCardRange>,
        Vec<SelectableRegionRange>,
        bool,
        usize,
        Vec<RunningCardRange>,
    ) {
        let started_at = Instant::now();
        let palette = self.palette();
        let width = content_width.unwrap_or(1).max(1);
        let body_width = width.saturating_sub(2).max(1);
        let messages = self.conversation.visible_messages();

        let mut lines = Vec::new();
        let mut card_ranges = Vec::new();
        let mut selectable_regions_ranges = Vec::new();
        let mut running_card_ranges = Vec::new();

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
            ));
            let total_lines = lines.len().max(1);
            return (
                Text::from(lines),
                total_lines,
                card_ranges,
                selectable_regions_ranges,
                false,
                0,
                running_card_ranges,
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

        let mut running_lines = Vec::new();
        if self.conversation.parent_session_id.is_none() {
            for (index, running_subagent) in self.running_subagent_executions.iter().enumerate() {
                let card_lines = self.render_running_subagent_lines(running_subagent, width);
                if card_lines.is_empty() {
                    continue;
                }

                let card_start = running_lines.len();
                let decorated_lines =
                    super::render::decorate_card_lines(card_lines, width, palette.panel);
                running_lines.extend(decorated_lines);
                let card_end = running_lines.len();

                running_card_ranges.push(RunningCardRange {
                    execution_index: index,
                    start_line: card_start,
                    end_line: card_end,
                });
            }
        }
        let total_running_lines = running_lines.len();

        // Calculate visible range based on scroll position
        let viewport = self.message_viewport_lines.max(1);
        let total_message_lines = self.message_layout_index.borrow().total_lines;
        let total_overall_lines = total_message_lines + total_running_lines;
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
            config: &self.config,
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
                &mut selectable_regions_ranges,
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

        lines.extend(running_lines);

        // Calculate total lines from layout index
        let total_lines = header_line_count + total_overall_lines;

        let elapsed = started_at.elapsed();
        if elapsed > Duration::from_millis(12) {
            let (hits, misses, entries) = self.message_render_cache_stats();
            crate::log_debug!(
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
            selectable_regions_ranges,
            true,
            render_scroll,
            running_card_ranges,
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
        let result =
            tool::render_tool_call_with_result(tool_call, tool_result, body_width, is_streaming, ctx);

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
        let mut outputs = HashMap::new();

        for message in messages {
            if !self.expanded_tool_results.contains(&message.id) {
                continue;
            }

            if let Ok(Some(output)) = self
                .store
                .load_tool_event_output(self.conversation.session_id, message.id)
            {
                outputs.insert(message.id, output);
            }
        }

        outputs
    }

    fn render_running_subagent_lines(
        &self,
        execution: &RunningSubagentExecution,
        body_width: usize,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();

        // Title line: description (@subagent_type)
        let description = shorten(&execution.task_description, body_width.saturating_sub(30));
        let subagent_type = execution.subagent_type.clone();

        let mut lines = Vec::new();

        // Header line with description and subagent type
        lines.push(Line::from(vec![
            Span::styled(
                description,
                Style::default()
                    .fg(palette.text)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" (@{})", subagent_type),
                Style::default().fg(palette.muted),
            ),
        ]));

        // Status line - always shown to maintain consistent height
        let status_text = execution.status.display();
        let status_line = match &execution.status {
            SubagentStatus::Tool => {
                if let Some(tool_call) = &execution.current_tool_call {
                    let tool_summary = if tool::tool_call_arguments_are_complete(&tool_call.arguments) {
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

        // Navigation hint
        lines.push(Line::from(vec![
            Span::styled("  ".to_string(), Style::default()),
            Span::styled(
                "Ctrl+X then ".to_string(),
                Style::default().fg(palette.muted),
            ),
            Span::styled("Up".to_string(), Style::default().fg(palette.accent_soft)),
            Span::styled("/".to_string(), Style::default().fg(palette.muted)),
            Span::styled("Down".to_string(), Style::default().fg(palette.accent_soft)),
            Span::styled(
                " to navigate".to_string(),
                Style::default().fg(palette.muted),
            ),
        ]));

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

    #[allow(dead_code)]
    fn render_output_preview_lines(
        &self,
        output: &str,
        body_width: usize,
        is_error: bool,
        message_id: Option<Uuid>,
    ) -> Vec<Line<'static>> {
        let palette = self.palette();
        let mut lines = Vec::new();

        let is_expanded = message_id.is_some_and(|id| self.expanded_tool_results.contains(&id));
        let max_lines = if is_expanded {
            TOOL_OUTPUT_EXPANDED_MAX_LINES
        } else if is_error {
            4
        } else {
            TOOL_OUTPUT_PREVIEW_LINES
        };

        let prefix = if is_error { "!" } else { "↳" };
        let fg = if is_error {
            palette.error
        } else {
            palette.text
        };
        let prefix_style = Style::default().fg(if is_error {
            palette.error
        } else {
            palette.accent_soft
        });

        let total_output_lines = output.lines().count();
        let wrap_width = body_width.saturating_sub(2);

        for line in output.lines().take(max_lines) {
            if is_expanded {
                let owned_line = Line::from(line.to_string());
                let wrapped =
                    word_wrap_line(&owned_line, WrapOptions::new(wrap_width).break_words(true));
                for (wrap_idx, wrapped_line) in wrapped.iter().enumerate() {
                    let effective_prefix = if wrap_idx == 0 { prefix } else { " " };
                    let mut spans =
                        vec![Span::styled(format!("{} ", effective_prefix), prefix_style)];
                    spans.extend(wrapped_line.spans.iter().map(|span| {
                        Span::styled(span.content.to_string(), Style::default().fg(fg))
                    }));
                    lines.push(Line::from(spans));
                }
            } else {
                lines.push(line_with_prefix(
                    prefix,
                    &shorten_single_line(line, wrap_width),
                    prefix_style,
                    Style::default().fg(fg),
                ));
            }
        }

        if total_output_lines > max_lines {
            lines.push(line_with_prefix(
                prefix,
                &format!("... {} more line(s)", total_output_lines - max_lines),
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        } else if total_output_lines > TOOL_OUTPUT_PREVIEW_LINES && message_id.is_some() {
            let hint = if is_expanded {
                "▲ Click to collapse"
            } else {
                "▼ Click to expand"
            };
            lines.push(line_with_prefix(
                prefix,
                hint,
                Style::default().fg(palette.muted),
                Style::default().fg(palette.muted),
            ));
        }

        if lines.is_empty() {
            lines.push(line_with_style("(no output)", palette.muted));
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
    /// For incremental updates, only the tail (last few messages) is rebuilt,
    /// preserving existing block positions for unchanged messages.
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
                config: &self.config,
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
                let computations: Vec<BlockComputation> = blocks_info
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
                    .collect();

                // Step 3: Build layout index and insert cache entries sequentially
                let mut current_line = 0;
                let mut cache = self.message_render_cache.borrow_mut();
                for (comp_idx, comp) in computations.iter().enumerate() {
                    let block = super::MessageBlock {
                        message_id: comp.message_id,
                        message_start_idx: blocks_info[comp_idx].start_idx,
                        message_count: comp.message_count,
                        start_line: current_line,
                        line_count: comp.line_count,
                    };
                    current_line += comp.line_count;
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
            config: &self.config,
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
        width: usize,
        body_width: usize,
        ctx: &RenderContext<'_>,
        is_round_end: bool,
    ) -> (Uuid, usize, usize) {
        let message = &messages[start_idx];
        let message_id = message.id;
        let palette = self.palette();

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
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.background).len();
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
                            lines +=
                                decorate_card_lines(card_lines, width, palette.panel_light).len();
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
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_alt).len();
                }
                lines += 1; // Empty line after user message
                (1, lines)
            }
            MessageRole::System => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.background).len();
                }
                (1, lines)
            }
            MessageRole::Error => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_light).len();
                }
                (1, lines)
            }
            MessageRole::Shell => {
                let cards =
                    self.cached_render_message_cards(ctx, message, body_width, is_round_end);
                let mut lines = 0;
                for (_, card_lines) in &cards {
                    lines +=
                        decorate_card_lines(card_lines.clone(), width, palette.panel_alt).len();
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
        selectable_regions_ranges: &mut Vec<SelectableRegionRange>,
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

                        let mut block_start = start_line;
                        let mut current_min_x = 1;
                        for (i, line) in card_lines.iter().enumerate() {
                            let is_reasoning =
                                line.spans.first().is_some_and(|s| s.content == "┃ ");
                            let line_min_x = if is_reasoning { 3 } else { 1 };

                            if line_min_x != current_min_x {
                                if i > 0 {
                                    selectable_regions_ranges.push(SelectableRegionRange {
                                        start_line: block_start,
                                        end_line: start_line + i,
                                        min_x: current_min_x,
                                        max_x: None,
                                    });
                                }
                                block_start = start_line + i;
                                current_min_x = line_min_x;
                            }
                        }
                        if block_start < start_line + card_lines.len() {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: block_start,
                                end_line: start_line + card_lines.len(),
                                min_x: current_min_x,
                                max_x: None,
                            });
                        }

                        lines.extend(decorate_card_lines(card_lines, width, card_bg));
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
                                r.min_x += 1; // decorate_card_lines left padding
                                if let Some(max_x) = &mut r.max_x {
                                    *max_x += 1;
                                }
                                selectable_regions_ranges.push(r.clone());
                            }

                            // Calculate fallback region for bash or non-diff output
                            if regions.is_empty() {
                                selectable_regions_ranges.push(SelectableRegionRange {
                                    start_line,
                                    end_line: start_line + tool_card_lines.len(),
                                    min_x: 1,
                                    max_x: None,
                                });
                            }

                            let card_bg = if canonical_tool_name(&tool_call.name) == Some("task") {
                                palette.panel
                            } else {
                                palette.panel_light
                            };
                            let decorated = decorate_card_lines(tool_card_lines, width, card_bg);
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
                let bg = match message.role {
                    MessageRole::User => palette.panel_alt,
                    MessageRole::Error => palette.panel_light,
                    _ => palette.background,
                };
                for (_, card_lines) in cards {
                    if !card_lines.is_empty() {
                        let start_line = current_line_offset + lines.len();

                        let mut block_start = start_line;
                        let mut current_min_x = 1;
                        for (i, line) in card_lines.iter().enumerate() {
                            let is_reasoning =
                                line.spans.first().is_some_and(|s| s.content == "┃ ");
                            let line_min_x = if is_reasoning { 3 } else { 1 };

                            if line_min_x != current_min_x {
                                if i > 0 {
                                    selectable_regions_ranges.push(SelectableRegionRange {
                                        start_line: block_start,
                                        end_line: start_line + i,
                                        min_x: current_min_x,
                                        max_x: None,
                                    });
                                }
                                block_start = start_line + i;
                                current_min_x = line_min_x;
                            }
                        }
                        if block_start < start_line + card_lines.len() {
                            selectable_regions_ranges.push(SelectableRegionRange {
                                start_line: block_start,
                                end_line: start_line + card_lines.len(),
                                min_x: current_min_x,
                                max_x: None,
                            });
                        }

                        lines.extend(decorate_card_lines(card_lines, width, bg));
                    }
                }
                if matches!(message.role, MessageRole::User | MessageRole::Shell) {
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
        );
    }
}
