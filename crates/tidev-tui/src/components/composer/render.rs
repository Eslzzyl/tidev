//! Composer rendering pipeline.
//!
//! Renders the input area with:
//! - Left accent bar (mode indicator colour)
//! - Text content with inline span badges
//! - Selection highlighting
//! - Cursor (handled by ratatui)
//! - Metadata row (mode label + model name)
//! - Inline popups above the input area (command palette, @-mention, snippet)

use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::prelude::{Color, Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};

use super::{Composer, InlineSpan, compute_visual_lines};
use crate::context::DrawContext;

/// Draw the composer component.
pub(crate) fn draw_composer(
    composer: &mut Composer,
    frame: &mut Frame,
    area: Rect,
    ctx: &DrawContext,
) {
    let palette = ctx.palette;

    // ── Background fill ─────────────────────────────────────────────
    let left_inset: u16 = 2;
    let bg_rect = Rect {
        x: area.x + left_inset,
        y: area.y,
        width: area.width.saturating_sub(left_inset),
        height: area.height,
    };
    frame.render_widget(
        Block::default().style(Style::default().bg(palette.panel)),
        bg_rect,
    );

    // ── Inner text area (margins from accent bar) ───────────────────
    let inner_margin: u16 = 2;
    let inner = Rect {
        x: bg_rect.x + inner_margin,
        y: area.y + 1,
        width: area.width.saturating_sub(left_inset + inner_margin + 1),
        height: area.height.saturating_sub(2),
    };

    // Reserve 2 rows at the bottom for metadata.
    let metadata_height: u16 = 2;
    let (text_area, metadata_area) = if inner.height > metadata_height {
        let split = Layout::vertical([Constraint::Min(1), Constraint::Length(metadata_height)])
            .split(inner);
        (split[0], split[1])
    } else {
        (inner, Rect::default())
    };

    let width = text_area.width as usize;
    let visible_lines = text_area.height.max(1) as usize;

    // Save input area for keyboard handler and mouse hit-testing.
    composer.last_input_width = text_area.width;
    composer.last_visible_lines = visible_lines;
    composer.last_text_area = text_area;

    // ── Content rendering ───────────────────────────────────────────
    if composer.text().is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                composer.placeholder(),
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.panel)),
            text_area,
        );
    } else {
        let scroll = composer.input_scroll_offset;
        let lines = compute_visual_lines(composer.text(), width);
        let selection = composer.selection_range();
        let spans = composer.spans().to_vec();

        let mut rendered_lines: Vec<Line> = Vec::new();
        let end_idx = (scroll + visible_lines).min(lines.len());
        for vl in lines[scroll..end_idx].iter() {
            let overlapping: Vec<&InlineSpan> = spans
                .iter()
                .filter(|s| s.start < vl.end && s.end > vl.start)
                .collect();

            if overlapping.is_empty() {
                let text_spans = render_line_with_selection(
                    &composer.text()[vl.start..vl.end],
                    vl.start,
                    selection,
                    palette.text,
                );
                rendered_lines.push(Line::from(text_spans));
            } else {
                let mut segments: Vec<Span> = Vec::new();
                let mut pos = vl.start;
                for span in &overlapping {
                    let seg_start = pos.max(vl.start);
                    let seg_end = span.start.min(vl.end);
                    if seg_end > seg_start {
                        segments.extend(render_line_with_selection(
                            &composer.text()[seg_start..seg_end],
                            seg_start,
                            selection,
                            palette.text,
                        ));
                    }
                    let span_start = span.start.max(vl.start);
                    let span_end = span.end.min(vl.end);
                    if span_end > span_start {
                        let badge_style = Style::default()
                            .fg(palette.accent)
                            .bg(palette.panel_alt)
                            .add_modifier(Modifier::BOLD);
                        let badge_text = &composer.text()[span_start..span_end];
                        segments.push(Span::styled(badge_text.to_string(), badge_style));
                    }
                    pos = span.end;
                }
                let tail_start = pos.max(vl.start);
                if tail_start < vl.end {
                    segments.extend(render_line_with_selection(
                        &composer.text()[tail_start..vl.end],
                        tail_start,
                        selection,
                        palette.text,
                    ));
                }
                rendered_lines.push(Line::from(segments));
            }
        }

        while rendered_lines.len() < visible_lines {
            rendered_lines.push(Line::from(Span::raw("")));
        }

        frame.render_widget(
            Paragraph::new(rendered_lines).style(Style::default().bg(palette.panel)),
            text_area,
        );
    }

    // ── Left accent bar (mode-colored) ──────────────────────────────
    let accent_color = if let Some(pending) = &ctx.pending_mode {
        palette.border_mode_color(*pending)
    } else {
        palette.border_mode_color(ctx.mode)
    };
    for row in 0..area.height {
        let mut bar_style = Style::default().fg(accent_color).bg(palette.panel);
        if row == 0 {
            bar_style = bar_style.add_modifier(Modifier::BOLD);
        }
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled("┃", bar_style)]))
                .style(Style::default().bg(palette.panel)),
            Rect::new(bg_rect.x, area.y + row, 1, 1),
        );
    }

    // ── Metadata row (mode + model + provider + thinking) ────────────
    if metadata_area.width > 0 && metadata_area.height > 1 {
        let mut meta_spans: Vec<Span> = Vec::new();

        // Mode label (Build / Plan)
        let mode_label = if let Some(pending) = &ctx.pending_mode {
            format!("{} → {}", ctx.mode.title(), pending.title())
        } else {
            ctx.mode.title().to_string()
        };
        meta_spans.push(Span::styled(
            mode_label,
            Style::default()
                .fg(accent_color)
                .add_modifier(Modifier::BOLD),
        ));

        // · Model display name
        if let Some(model) = ctx.model_display {
            meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
            meta_spans.push(Span::styled(model, Style::default().fg(palette.text)));
        }

        // · Provider display name
        if let Some(provider) = ctx.provider_display {
            meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
            meta_spans.push(Span::styled(
                provider,
                Style::default().fg(palette.muted),
            ));
        }

        // · [thinking level]
        if let Some(level) = ctx.thinking_level
            && level.is_supported() {
                meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
                meta_spans.push(Span::styled(
                    format!("[{}]", level.display_name()),
                    Style::default().fg(palette.accent_soft),
                ));
            }

        // · Subagent strikethrough when disabled
        if ctx.subagent_disabled {
            meta_spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
            meta_spans.push(Span::styled(
                "Subagent",
                Style::default()
                    .fg(palette.muted)
                    .add_modifier(Modifier::CROSSED_OUT),
            ));
        }

        // Render on the second row of metadata_area (first row is blank spacer)
        let meta_rect = Rect::new(metadata_area.x, metadata_area.y + 1, metadata_area.width, 1);
        frame.render_widget(
            Paragraph::new(Line::from(meta_spans))
                .style(Style::default().bg(palette.panel)),
            meta_rect,
        );
    }

    // ── Command palette popup (rendered above the input area) ──────
    if composer.command_palette.visible && !composer.command_palette.suggestions.is_empty() {
        let cp = &composer.command_palette;
        let popup_height = cp.popup_height();
        if popup_height > 0 {
            let popup_x = area.x + 2;
            let width = area.width.clamp(20, 72).min(frame.area().width.saturating_sub(popup_x));
            let popup_rect = Rect::new(
                popup_x,
                area.y.saturating_sub(popup_height),
                width,
                popup_height,
            );
            let inner = popup_rect.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });

            let items: Vec<ListItem> = cp
                .suggestions
                .iter()
                .map(|s| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            s.spec.label(),
                            Style::default()
                                .fg(palette.accent)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw("  "),
                        Span::styled(
                            s.spec.description,
                            Style::default().fg(palette.muted),
                        ),
                    ]))
                })
                .collect();

            let highlight_style = Style::default()
                .bg(palette.selection_bg)
                .fg(palette.selection_fg)
                .add_modifier(Modifier::BOLD);

            let panel = Block::default().style(Style::default().bg(palette.panel_alt));
            let list = List::new(items)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(highlight_style);

            let mut state = ListState::default();
            state.select(Some(cp.selected_index));

            frame.render_widget(Clear, popup_rect);
            frame.render_widget(panel, popup_rect);
            frame.render_stateful_widget(list, inner, &mut state);
        }
    }

    // ── @-mention popup ─────────────────────���──────────────────────
    if composer.at_mention.visible && !composer.at_mention.suggestions.is_empty() {
        let am = &composer.at_mention;
        let popup_height = am.popup_height();
        if popup_height > 0 {
            let popup_x = area.x + 2;
            let width = area.width.clamp(20, 56).min(frame.area().width.saturating_sub(popup_x));
            let popup_rect = Rect::new(
                popup_x,
                area.y.saturating_sub(popup_height),
                width,
                popup_height,
            );
            let inner = popup_rect.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });

            let items: Vec<ListItem> = am
                .suggestions
                .iter()
                .map(|s| {
                    let icon = match s.kind {
                        super::at_mention::AtMentionKind::File => "📄",
                        super::at_mention::AtMentionKind::Directory => "📁",
                        super::at_mention::AtMentionKind::Image => "🖼",
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{} ", icon)),
                        Span::styled(
                            &s.display,
                            Style::default().fg(palette.text),
                        ),
                    ]))
                })
                .collect();

            let highlight_style = Style::default()
                .bg(palette.selection_bg)
                .fg(palette.selection_fg)
                .add_modifier(Modifier::BOLD);

            let panel = Block::default().style(Style::default().bg(palette.panel_alt));
            let list = List::new(items)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(highlight_style);

            let mut state = ListState::default();
            state.select(Some(am.selected_index));

            frame.render_widget(Clear, popup_rect);
            frame.render_widget(panel, popup_rect);
            frame.render_stateful_widget(list, inner, &mut state);
        }
    }

    // ── Snippet popup ───────────────────────────────────────────────
    if composer.snippet_state.visible && !composer.snippet_state.snippets.is_empty() {
        let sn = &composer.snippet_state;
        let popup_height = sn.popup_height();
        if popup_height > 0 {
            let popup_x = area.x + 2;
            let width = area.width.clamp(20, 56).min(frame.area().width.saturating_sub(popup_x));
            let popup_rect = Rect::new(
                popup_x,
                area.y.saturating_sub(popup_height),
                width,
                popup_height,
            );
            let inner = popup_rect.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            });

            let items: Vec<ListItem> = sn
                .snippets
                .iter()
                .map(|s| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            &s.text,
                            Style::default().fg(palette.text),
                        ),
                    ]))
                })
                .collect();

            let highlight_style = Style::default()
                .bg(palette.selection_bg)
                .fg(palette.selection_fg)
                .add_modifier(Modifier::BOLD);

            let panel = Block::default().style(Style::default().bg(palette.panel_alt));
            let list = List::new(items)
                .style(Style::default().bg(palette.panel_alt).fg(palette.text))
                .highlight_style(highlight_style);

            let mut state = ListState::default();
            state.select(Some(sn.selected_index));

            frame.render_widget(Clear, popup_rect);
            frame.render_widget(panel, popup_rect);
            frame.render_stateful_widget(list, inner, &mut state);
        }
    }

    // ── Cursor positioning ───────────────────────────────────────────
    if ctx.focused && text_area.width > 0 && text_area.height > 0 {
        let (cursor_line, cursor_col) = composer.cursor_position(text_area.width);
        let mut cursor_line = cursor_line.saturating_sub(composer.input_scroll_offset as u16);
        let mut cursor_col = cursor_col;

        if composer.cursor_wraps_to_next_row(text_area.width as usize) {
            cursor_line = cursor_line.saturating_add(1);
            cursor_col = 0;
        }

        let cursor_x = text_area.x.saturating_add(cursor_col);
        let cursor_y = text_area
            .y
            .saturating_add(cursor_line.min(text_area.height.saturating_sub(1)));

        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

/// Render a text segment with optional selection highlighting.
fn render_line_with_selection<'a>(
    text: &'a str,
    byte_offset: usize,
    selection: Option<(usize, usize)>,
    text_color: Color,
) -> Vec<Span<'a>> {
    let default_style = Style::default().fg(text_color);
    if text.is_empty() {
        return vec![Span::styled(" ", default_style)];
    }
    let Some((sel_start, sel_end)) = selection else {
        return vec![Span::styled(text.to_string(), default_style)];
    };
    let local_start = sel_start.saturating_sub(byte_offset);
    let local_end = sel_end.saturating_sub(byte_offset);
    let seg_len = text.len();
    if local_start >= seg_len || local_end == 0 {
        return vec![Span::styled(text.to_string(), default_style)];
    }
    let local_start = local_start.min(seg_len);
    let local_end = local_end.min(seg_len);
    if local_start >= local_end {
        return vec![Span::styled(text.to_string(), default_style)];
    }

    let selected_style = default_style.patch(Style::default().bg(Color::Indexed(4)));

    let mut spans = Vec::new();
    if local_start > 0 {
        spans.push(Span::styled(text[..local_start].to_string(), default_style));
    }
    spans.push(Span::styled(
        text[local_start..local_end].to_string(),
        selected_style,
    ));
    if local_end < seg_len {
        spans.push(Span::styled(text[local_end..].to_string(), default_style));
    }
    spans
}
