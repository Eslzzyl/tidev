use super::*;

use std::time::Instant;

use crate::component::Component;
use crate::components::chat::render::wrap_text_lines;
use crate::components::selection::copy_to_clipboard;
use crate::context::DrawContext;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::prelude::{Frame, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use tidev_utils::path::display_path_with_tilde;
use unicode_width::UnicodeWidthStr;

impl App {
    pub(crate) fn footer_status_text(&self) -> String {
        let queued_count = self.pending_inputs.iter().filter(|p| !p.steered).count();
        let steered_count = self.pending_inputs.len() - queued_count;

        // 1. Esc again to stop (abort confirmation)
        if self.has_active_request()
            && self
                .abort_confirmation_deadline
                .is_some_and(|deadline| deadline > Instant::now())
        {
            return "Esc again to stop".to_string();
        }

        // 2. Token usage helper
        let token_status = self.context_usage.as_ref().map(|usage| {
            let max_context = self.runtime.active_model().context_window;
            let total = usage.input_tokens as u64 + usage.output_tokens as u64;
            let pct = if max_context > 0 {
                (total as f64 / max_context as f64 * 100.0).min(100.0)
            } else {
                0.0
            };
            let used_k = usage.input_tokens / 1000;
            let max_k = (max_context as u32) / 1000;
            format!("{pct:.1}% ({used_k}K/{max_k}K)")
        });

        // 3. Active request — show spinner + status
        if self.has_active_request() {
            let spinner = self.loading_spinner();

            // Check for subsession
            let parent_session_id = self
                .message_list
                .as_ref()
                .and_then(|ml| ml.active_chat_context())
                .and_then(|ctx| ctx.parent_session_id);

            let status = if parent_session_id.is_some() {
                // Check if this subsession's own subagent is still running.
                let session_id = self
                    .message_list
                    .as_ref()
                    .and_then(|ml| ml.active_chat_context())
                    .map(|ctx| ctx.session_id);
                let subagent_running = session_id.is_some_and(|sid| {
                    self.message_list
                        .as_ref()
                        .is_some_and(|ml| ml.is_subagent_running(sid))
                });
                if subagent_running {
                    format!("{spinner} Thinking...")
                } else {
                    "Subsession active · Up: parent  Left/Right: switch subagent".to_string()
                }
            } else if let Some(ref ml) = self.message_list {
                let sub_count = ml.running_subagents_count();
                if sub_count > 0 {
                    let label = if sub_count == 1 {
                        "subagent"
                    } else {
                        "subagents"
                    };
                    format!("{spinner} Waiting for {sub_count} {label}")
                } else if ml.running_tools_count() > 0 {
                    let counts = ml.running_tool_counts();
                    let total = ml.running_tools_count();
                    if counts.len() == 1 {
                        let (name, n) = &counts[0];
                        if *n == 1 {
                            format!("{spinner} Running {name}")
                        } else {
                            format!("{spinner} Running {n}× {name}")
                        }
                    } else {
                        let items: Vec<String> = counts
                            .iter()
                            .map(|(name, n)| {
                                if *n == 1 {
                                    name.clone()
                                } else {
                                    format!("{n}× {name}")
                                }
                            })
                            .collect();
                        format!("{spinner} Running {} tools ({})", total, items.join(", "))
                    }
                } else if ml.is_streaming() {
                    let pending_mode = self
                        .current_session_id
                        .and_then(|sid| self.pending_modes.get(&sid));
                    match pending_mode {
                        Some(pending) => {
                            format!(
                                "{spinner} {} → {} (on completion)",
                                self.mode.title(),
                                pending.title()
                            )
                        }
                        None => format!("{spinner} {}", self.mode.title()),
                    }
                } else if self
                    .current_session_id
                    .is_some_and(|sid| self.pending_approvals.contains_key(&sid))
                {
                    format!("{spinner} Running tools")
                } else {
                    let pending_mode = self
                        .current_session_id
                        .and_then(|sid| self.pending_modes.get(&sid));
                    match pending_mode {
                        Some(pending) => {
                            format!(
                                "{spinner} {} → {} (on completion)",
                                self.mode.title(),
                                pending.title()
                            )
                        }
                        None => format!("{spinner} {}", self.mode.title()),
                    }
                }
            } else {
                let pending_mode = self
                    .current_session_id
                    .and_then(|sid| self.pending_modes.get(&sid));
                match pending_mode {
                    Some(pending) => {
                        format!(
                            "{spinner} {} → {} (on completion)",
                            self.mode.title(),
                            pending.title()
                        )
                    }
                    None => format!("{spinner} {}", self.mode.title()),
                }
            };

            let is_pending_compact = self
                .current_session_id
                .is_some_and(|sid| self.pending_compacts.contains(&sid));
            let extra = match (queued_count, steered_count, is_pending_compact) {
                (0, 0, false) => String::new(),
                (1, 0, false) => " · queued 1".to_string(),
                (q, 0, false) => format!(" · queued {q}"),
                (0, 1, false) => " · steer 1".to_string(),
                (0, s, false) => format!(" · steer {s}"),
                (q, s, false) => format!(" · queued {q} · steer {s}"),
                (0, 0, true) => " · compact pending".to_string(),
                (q, 0, true) => format!(" · queued {q} · compact pending"),
                (0, s, true) => format!(" · steer {s} · compact pending"),
                (q, s, true) => format!(" · queued {q} · steer {s} · compact pending"),
            };
            let status = format!("{status}{extra}");

            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 3b. Compacting in progress — show spinner + status
        if self.is_compacting() {
            let spinner = self.loading_spinner();
            let status = format!("{spinner} Compacting...");
            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 4. Pending messages or compact pending (not streaming)
        let has_pending = queued_count > 0
            || steered_count > 0
            || self
                .current_session_id
                .is_some_and(|sid| self.pending_compacts.contains(&sid));
        if has_pending {
            let is_pending_compact = self
                .current_session_id
                .is_some_and(|sid| self.pending_compacts.contains(&sid));
            let compact_part = if is_pending_compact {
                " · compact pending"
            } else {
                ""
            };
            let mut parts: Vec<String> = Vec::new();
            if queued_count > 0 {
                parts.push(if queued_count == 1 {
                    "1 queued message".to_string()
                } else {
                    format!("{queued_count} queued messages")
                });
            }
            if steered_count > 0 {
                parts.push(if steered_count == 1 {
                    "1 steering message".to_string()
                } else {
                    format!("{steered_count} steering messages")
                });
            }
            let status = format!("{}{compact_part}", parts.join(" · "));
            if let Some(ref t) = token_status {
                return format!("{status} · {t}");
            }
            return status;
        }

        // 5. Token usage only
        if let Some(t) = token_status {
            return t;
        }

        // 6. Last notice
        if let Some((msg, _)) = &self.last_notice
            && !msg.is_empty()
        {
            return msg.clone();
        }

        // 7. Subsession navigation hint
        let is_subsession = self
            .message_list
            .as_ref()
            .and_then(|ml| ml.active_chat_context())
            .and_then(|ctx| ctx.parent_session_id)
            .is_some();
        if is_subsession {
            return "Subsession active · Up: parent  Left/Right: switch subagent".to_string();
        }

        // 8. Ready
        "Ready".to_string()
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();
        self.terminal_area = area;
        self.cursor_rendered = false;
        self.composer_cursor_position = None;

        // Background
        frame.render_widget(
            Block::default().style(Style::default().bg(palette.background)),
            area,
        );

        if self.screen == AppScreen::Welcome {
            self.draw_welcome(frame);
            if self.overlays.is_empty() {
                self.composer_cursor_position = self
                    .composer
                    .as_ref()
                    .and_then(|composer| composer.last_cursor_position);
                self.cursor_rendered = self.composer_cursor_position.is_some();
            }
            // Draw overlays on top of welcome content.
            let draw_ctx = DrawContext {
                palette,
                focused: true,
                mode: self.mode,
                pending_mode: self
                    .current_session_id
                    .and_then(|sid| self.pending_modes.get(&sid).copied()),
                model_display: None,
                provider_display: None,
                thinking_level: None,
                subagent_disabled: !self.runtime.config().subagent.enabled,
                collapse_thinking: self.runtime.config().ui.collapse_thinking,
                collapse_diffs: self.runtime.config().ui.collapse_diffs,
                workspace_root: self.runtime.workspace_root(),
            };
            self.overlays.draw(frame, area, area, &draw_ctx);
            return;
        }

        // Determine sidebar visibility and split the layout.
        // Use the same threshold as the old TUI.
        const SIDEBAR_GAP: u16 = 2;
        let sidebar_width = self.runtime.config().ui.sidebar_width;
        let sidebar_visible =
            area.width >= sidebar_width.saturating_add(70).saturating_add(SIDEBAR_GAP);
        let (main_area, sidebar_area) = if sidebar_visible {
            let split = ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Min(20),
                ratatui::layout::Constraint::Length(SIDEBAR_GAP),
                ratatui::layout::Constraint::Length(sidebar_width),
            ])
            .split(area);
            (split[0], Some(split[2]))
        } else {
            self.sidebar_area = None;
            (area, None)
        };

        // Determine if in a subsession.
        let is_subsession = self
            .message_list
            .as_ref()
            .and_then(|ml| ml.active_chat_context())
            .and_then(|ctx| ctx.parent_session_id)
            .is_some();
        const SUBSESSION_NAV_HEIGHT: u16 = 3;

        // Calculate bottom-bar height: subsession nav or composer.
        let bottom_height = if is_subsession {
            SUBSESSION_NAV_HEIGHT
        } else {
            self.composer
                .as_ref()
                .map(|c| {
                    let width = main_area.width.saturating_sub(5);
                    c.preferred_height(width, 6)
                        .saturating_add(2)
                        .min(main_area.height.saturating_sub(2))
                })
                .unwrap_or(0)
        };

        // Calculate queued prompts area height (frozen area above input box).
        let queued_height = if !is_subsession {
            let count = self.pending_inputs.len();
            if count > 0 {
                let visible = count.min(MAX_VISIBLE_QUEUED_PROMPTS);
                let text_width = main_area.width.saturating_sub(5).max(1) as usize;
                let mut inner: usize = 0;
                for (i, q) in self.pending_inputs.iter().take(visible).enumerate() {
                    // +1 for mode header line
                    let wrapped =
                        wrap_text_lines(&q.message.content, text_width, MAX_QUEUED_PROMPT_LINES);
                    inner += 1 + wrapped.len();
                    // Separator between items (not after last)
                    if i + 1 < visible {
                        inner += 1;
                    }
                }
                // +1 for "+N more" overflow, +2 for block top/bottom borders
                let overflow = if count > MAX_VISIBLE_QUEUED_PROMPTS {
                    1
                } else {
                    0
                };
                (inner + overflow + 2)
                    .min(main_area.height.saturating_sub(6) as usize / 2)
                    .min(15)
            } else {
                0
            }
        } else {
            0
        };

        // Split: message area + queued area + bottom bar + notice line.
        let notice_height: u16 = 1;
        let (content_area, queued_area, bottom_area, notice_line) = if bottom_height > 0 {
            let split = Layout::vertical([
                Constraint::Min(1),
                Constraint::Length(queued_height as u16),
                Constraint::Length(bottom_height),
                Constraint::Length(notice_height),
            ])
            .split(main_area);
            (split[0], split[1], split[2], split[3])
        } else {
            let split = Layout::vertical([Constraint::Min(1), Constraint::Length(notice_height)])
                .split(main_area);
            (split[0], Rect::default(), Rect::default(), split[1])
        };

        // Chat message area (when session is active)
        if let Some(ref mut chat) = self.message_list {
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                mode: self.mode,
                pending_mode: self
                    .current_session_id
                    .and_then(|sid| self.pending_modes.get(&sid).copied()),
                model_display: None,
                provider_display: None,
                thinking_level: None,
                subagent_disabled: !self.runtime.config().subagent.enabled,
                collapse_thinking: self.runtime.config().ui.collapse_thinking,
                collapse_diffs: self.runtime.config().ui.collapse_diffs,
                workspace_root: self.runtime.workspace_root(),
            };
            chat.draw(frame, content_area, &draw_ctx);
        }

        // Render queued prompts above the composer
        self.queued_card_bounds.clear();
        if queued_height > 0 {
            self.render_queued_prompts(frame, queued_area);
        }

        // ── Bottom bar ───────────────────────────────────────────────
        // Subsession: navigation hints.  Normal session: composer.
        if is_subsession {
            // Match the background with the message panel area.
            let bg_rect = Rect {
                x: main_area.x + 2,
                y: bottom_area.y,
                width: bottom_area.width.saturating_sub(2),
                height: bottom_area.height,
            };
            frame.render_widget(
                Block::default().style(Style::default().bg(palette.panel)),
                bg_rect,
            );
            let hint = Line::from(vec![
                Span::styled("Up", Style::default().fg(palette.accent_soft)),
                Span::styled(": return to parent  ", Style::default().fg(palette.muted)),
                Span::styled("Left", Style::default().fg(palette.accent_soft)),
                Span::styled("/", Style::default().fg(palette.muted)),
                Span::styled("Right", Style::default().fg(palette.accent_soft)),
                Span::styled(": switch subagent", Style::default().fg(palette.muted)),
            ]);
            let y_offset = bg_rect.height.saturating_sub(1) / 2;
            let content_rect = Rect {
                x: bg_rect.x,
                y: bg_rect.y + y_offset,
                width: bg_rect.width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(hint)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(palette.text)),
                content_rect,
            );
        } else if let Some(ref mut composer) = self.composer {
            if composer.has_popup() {
                composer.sync_autocomplete();
            }
            self.cursor_rendered = self.overlays.is_empty();
            let active_model = self.runtime.active_model();
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                mode: self.mode,
                pending_mode: self
                    .current_session_id
                    .and_then(|sid| self.pending_modes.get(&sid).copied()),
                model_display: Some(&active_model.display_name),
                provider_display: Some(&active_model.provider_display_name),
                thinking_level: Some(&active_model.thinking_level),
                subagent_disabled: !self.runtime.config().subagent.enabled,
                collapse_thinking: self.runtime.config().ui.collapse_thinking,
                collapse_diffs: self.runtime.config().ui.collapse_diffs,
                workspace_root: self.runtime.workspace_root(),
            };
            composer.draw(frame, bottom_area, &draw_ctx);
            self.composer_cursor_position = composer.last_cursor_position;
        }

        // Build DrawContext for overlays
        let draw_ctx = DrawContext {
            palette,
            focused: true,
            mode: self.mode,
            pending_mode: self
                .current_session_id
                .and_then(|sid| self.pending_modes.get(&sid).copied()),
            model_display: None,
            provider_display: None,
            thinking_level: None,
            subagent_disabled: !self.runtime.config().subagent.enabled,
            collapse_thinking: self.runtime.config().ui.collapse_thinking,
            collapse_diffs: self.runtime.config().ui.collapse_diffs,
            workspace_root: self.runtime.workspace_root(),
        };
        // ── Sidebar ───────────────────────────────────────────────────
        if let Some(sidebar_area) = sidebar_area {
            self.sidebar_area = Some(sidebar_area);
            let chat_ctx = self
                .message_list
                .as_ref()
                .and_then(|ml| ml.active_chat_context());
            self.sidebar.draw(
                frame,
                sidebar_area,
                palette,
                self.runtime.workspace_root(),
                chat_ctx,
                self.context_usage.as_ref(),
                &self.todos,
            );
        }

        // Draw overlays.
        // - Composer-style overlays (question, sensitive, workspace) use
        //   main_area so they don't spill over the sidebar.
        // - Centered panels (session, message, settings, …) use the full
        //   terminal area so they appear properly centered across the screen.
        self.overlays.draw(frame, area, main_area, &draw_ctx);

        // ── Footer status line (right-aligned, matching v0.6.x) ──
        let status_text = self.footer_status_text();
        let status_width = status_text
            .width()
            .min(notice_line.width.saturating_sub(2) as usize) as u16;
        let status_x = notice_line.x
            + notice_line
                .width
                .saturating_sub(2)
                .saturating_sub(status_width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &status_text,
                Style::default().fg(palette.muted),
            )))
            .style(Style::default().bg(palette.background)),
            Rect::new(status_x, notice_line.y, status_width, 1),
        );

        // ── Toast notification ──
        // Small popup at the top-right of the message area, auto-expires.
        // Mirrors old TUI's render_toast: positioned relative to message content area.
        if let Some((msg, expires_at)) = &self.toast.clone() {
            if Instant::now() < *expires_at {
                let chat_area = self.message_list.as_ref().and_then(|ml| ml.content_area);
                if let Some(chat_area) = chat_area {
                    let toast_width = (msg.len() as u16).min(32).saturating_add(2);
                    let toast_rect = Rect::new(
                        chat_area.right().saturating_sub(toast_width + 1),
                        chat_area.y + 1,
                        toast_width,
                        3,
                    );
                    frame.render_widget(Clear, toast_rect);
                    let block =
                        Block::default().style(Style::default().bg(palette.panel).fg(palette.text));
                    let centered = format!("\n{}", msg);
                    frame.render_widget(
                        Paragraph::new(centered)
                            .style(Style::default().bg(palette.panel).fg(palette.text))
                            .alignment(Alignment::Center)
                            .block(block),
                        toast_rect,
                    );
                }
            } else {
                self.toast = None;
            }
        }

        // ── Mouse selection overlay ──
        // Apply after all widgets have been drawn, so the selection style
        // paints on top of the rendered content.
        let scroll_offset = self
            .message_list
            .as_ref()
            .map(|ml| ml.scroll_offset)
            .unwrap_or(0);
        let selectable_rects = self
            .message_list
            .as_ref()
            .map(|ml| ml.selectable_region_rects())
            .unwrap_or_default();
        let sel_style = Style::default()
            .bg(palette.selection_bg)
            .fg(palette.selection_fg);

        if self.mouse_selection.has_selection(scroll_offset) {
            self.mouse_selection.apply_overlay(
                frame.buffer_mut(),
                scroll_offset,
                &selectable_rects,
                sel_style,
            );
        }

        // Handle pending clipboard copy (set by mouse up in handle_mouse_event).
        if self
            .mouse_selection
            .take_pending_copy(scroll_offset)
            .is_some()
            && let Some(text) = self.mouse_selection.selected_text(
                frame.buffer_mut(),
                scroll_offset,
                &selectable_rects,
            )
            && !text.is_empty()
        {
            match copy_to_clipboard(&text) {
                Ok(()) => {
                    self.mouse_selection.clear();
                    self.set_toast(
                        "Selection copied to clipboard",
                        std::time::Duration::from_secs(3),
                    );
                }
                Err(e) => {
                    self.mouse_selection.clear();
                    self.set_toast(
                        format!("Copy failed: {e}"),
                        std::time::Duration::from_secs(5),
                    );
                }
            }
        }

        // Handle pending clipboard copy from composer input area.
        if let Some(text) = self.pending_input_copy.take()
            && !text.is_empty()
        {
            match copy_to_clipboard(&text) {
                Ok(()) => {
                    self.set_toast(
                        "Selection copied to clipboard",
                        std::time::Duration::from_secs(3),
                    );
                }
                Err(e) => {
                    self.set_toast(
                        format!("Copy failed: {e}"),
                        std::time::Duration::from_secs(5),
                    );
                }
            }
        }
    }

    /// Render the welcome screen with logo, subtitle, and composer.
    fn draw_welcome(&mut self, frame: &mut Frame) {
        let palette = self.current_palette;
        let area = frame.area();

        // Centered card — exact match to old TUI's render_welcome
        let card_width = self
            .runtime
            .config()
            .ui
            .welcome_width
            .min(area.width.saturating_sub(4).max(32));
        let card_height = 20u16.min(area.height.saturating_sub(2).max(10));
        let card = Rect::new(
            (area.width - card_width) / 2,
            (area.height - card_height) / 2,
            card_width,
            card_height,
        );

        let card_inner_width = card.width.saturating_sub(7);

        let inner = card.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let composer_height = self
            .composer
            .as_ref()
            .map(|c| {
                c.preferred_height(card_inner_width, self.runtime.config().ui.max_input_lines)
                    .saturating_add(2)
            })
            .unwrap_or(5);

        let sections = Layout::vertical([
            Constraint::Length(8),
            Constraint::Length(1),
            Constraint::Length(composer_height),
        ])
        .split(inner);

        // ASCII art logo
        // #[rustfmt::skip]
        let ascii_art = Paragraph::new(
            r#"░▒▓████████▓▒░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░      ░▒▓█▓▒░░▒▓█▓▒░ 
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░       ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓██████▓▒░  ░▒▓█▓▒▒▓█▓▒░  
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓█▓▒░░▒▓█▓▒░▒▓█▓▒░        ░▒▓█▓▓█▓▒░   
   ░▒▓█▓▒░   ░▒▓█▓▒░▒▓███████▓▒░░▒▓████████▓▒░  ░▒▓██▓▒░    "#,
        )
        .alignment(Alignment::Center)
        .style(
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        );
        frame.render_widget(ascii_art, sections[0]);

        // Subtitle
        let subtitle = Paragraph::new("Terminal AI assistant for focused coding work")
            .alignment(Alignment::Center)
            .style(Style::default().fg(palette.muted));
        frame.render_widget(subtitle, sections[1]);

        // Composer input block — pass section area directly, exactly as old TUI
        if let Some(ref mut composer) = self.composer {
            if composer.has_popup() {
                composer.sync_autocomplete();
            }
            let active_model = self.runtime.active_model();
            let draw_ctx = DrawContext {
                palette,
                focused: self.overlays.is_empty(),
                mode: self.mode,
                pending_mode: self
                    .current_session_id
                    .and_then(|sid| self.pending_modes.get(&sid).copied()),
                model_display: Some(&active_model.display_name),
                provider_display: Some(&active_model.provider_display_name),
                thinking_level: Some(&active_model.thinking_level),
                subagent_disabled: !self.runtime.config().subagent.enabled,
                collapse_thinking: self.runtime.config().ui.collapse_thinking,
                collapse_diffs: self.runtime.config().ui.collapse_diffs,
                workspace_root: self.runtime.workspace_root(),
            };
            composer.draw(frame, sections[2], &draw_ctx);
        }

        // Workspace path on the very last row
        let display_path = display_path_with_tilde(self.runtime.workspace_root());
        let workspace_area = Rect::new(
            area.x + 1,
            area.bottom() - 1,
            area.width.saturating_sub(2),
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                &display_path,
                Style::default().fg(palette.muted),
            ))),
            workspace_area,
        );

        // Notice, if any, on the row directly above workspace path
        if let Some((message, _)) = &self.last_notice
            && !message.is_empty()
        {
            let notice_y = area.bottom().saturating_sub(2);
            if notice_y < workspace_area.y {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        message,
                        Style::default().fg(palette.muted),
                    ))),
                    Rect::new(area.x + 1, notice_y, area.width.saturating_sub(2), 1),
                );
            }
        }
    }

    /// Render a frozen area above the composer showing pending (queued or
    /// steered) user messages. Each message is word-wrapped into up to
    /// [`MAX_QUEUED_PROMPT_LINES`] lines. Cards are separated by a thin rule.
    /// Each card is independently hover-highlighted.
    fn render_queued_prompts(&mut self, frame: &mut Frame<'_>, area: Rect) {
        let palette = &self.current_palette;
        let count = self.pending_inputs.len();
        let visible = count.min(MAX_VISIBLE_QUEUED_PROMPTS);

        // Build title: " PENDING " badge with background color + count
        let title = Line::from(vec![
            Span::styled(
                " PENDING ",
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

        for (i, pending) in self.pending_inputs.iter().take(visible).enumerate() {
            if y_offset as usize >= inner_height {
                break;
            }

            // Strip system-reminder tags from steering messages for display.
            let display_text = crate::utils::strip_system_reminder_tags(&pending.message.content);
            // Word-wrap the prompt into up to MAX_QUEUED_PROMPT_LINES lines
            let wrapped_lines = wrap_text_lines(&display_text, width, MAX_QUEUED_PROMPT_LINES);
            // +1 for mode header line
            let row_text_height = 1 + wrapped_lines.len();
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

            // ── Mode header: delivery type + mode ─────────────────────
            let delivery_label = if pending.steered { "STEER" } else { "QUEUE" };
            let mode_color = palette.border_mode_color(pending.mode);
            let header_line = Line::from(vec![
                Span::styled(
                    format!("{delivery_label} "),
                    Style::default()
                        .fg(if pending.steered {
                            palette.accent
                        } else {
                            palette.muted
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    pending.mode.title(),
                    Style::default().fg(mode_color).add_modifier(Modifier::BOLD),
                ),
            ]);
            frame.render_widget(
                Paragraph::new(header_line),
                Rect::new(inner.x, inner.y + y_offset, inner.width, 1),
            );
            y_offset += 1;

            // ── Render each wrapped line of the prompt ───────────────
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
                        .wrap(ratatui::widgets::Wrap { trim: false }),
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
}
