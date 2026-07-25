use super::*;
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use uuid::Uuid;

use crate::action::{Action, ChatAction, OverlayAction, OverlayKind, SessionAction};
use crate::component::Component;

impl App {
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent) {
        // 0. Esc: close any composer popup first (overrides abort confirmation).
        if key.code == KeyCode::Esc
            && self.composer.as_ref().is_some_and(|c| c.has_popup())
            && self.overlays.is_empty()
        {
            if let Some(ref mut composer) = self.composer {
                composer.handle_key_event(key);
            }
            return;
        }

        // 0. Abort confirmation: double-Esc to cancel current request.
        if key.code == KeyCode::Esc
            && self.overlays.is_empty()
            && (self.has_active_request() || !self.pending_prompt_queue.is_empty())
        {
            if self
                .abort_confirmation_deadline
                .is_some_and(|deadline| deadline > Instant::now())
            {
                self.abort_current_request();
                return;
            }
            self.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
            self.set_notice("Press Esc again within 3 seconds to stop the current request");
            return;
        }
        self.abort_confirmation_deadline = None;

        // 0. Ctrl+C: clear input (overrides quit — Ctrl+D is the quit shortcut).
        if key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL {
            if let Some(ref mut composer) = self.composer
                && !composer.is_empty()
            {
                composer.clear();
                self.set_notice("Input cleared");
            }
            return;
        }

        // 0a. Alt+E: open external editor with current composer text.
        if key.code == KeyCode::Char('e') && key.modifiers == KeyModifiers::ALT {
            self.open_external_editor();
            return;
        }

        // 1. Global shortcuts (unaffected by overlays)
        if let Some(action) = self.handle_global_key(key) {
            self.process_action(action);
            return;
        }

        // 1a. Message scrolling keys work even when overlays are open.
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
            && let Some(ref mut chat) = self.message_list
            && let Some(action) = chat.handle_key_event(key)
        {
            self.process_action(action);
            return;
        }

        // 2. OverlayStack top-first
        if let Some(action) = self.overlays.handle_key_event(key) {
            self.process_action(action);
            return;
        }

        // 2a. Subsession navigation (when parent_session_id is set).
        if let Some(ref chat) = self.message_list
            && let Some(ctx) = chat.active_chat_context()
            && ctx.parent_session_id.is_some()
        {
            match key.code {
                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                    self.handle_subsession_navigation(key);
                    return;
                }
                _ => {}
            }
        }
        // 2b. Tab: session mode switch (only when no composer popup is active).
        if key.code == KeyCode::Tab
            && key.modifiers.is_empty()
            && !self.composer.as_ref().is_some_and(|c| c.has_popup())
        {
            self.handle_tab_mode_switch();
            return;
        }

        // 2c. Shift+Tab / Ctrl+T: cycle thinking level.
        if (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT))
            || (key.code == KeyCode::Char('t') && key.modifiers == KeyModifiers::CONTROL)
        {
            self.process_action(Action::Session(SessionAction::CycleThinkingLevel));
            return;
        }

        // 3. Composer (when no overlay consumed the event)
        // Composer always consumes the key when present, even if no action
        // is produced (e.g., typing a character, moving cursor). This prevents
        // keys from leaking to the message list below.
        if let Some(ref mut composer) = self.composer {
            if let Some(action) = composer.handle_key_event(key) {
                self.process_action(action);
            }
            return;
        }

        // 4. MessageList (only when no overlay/composer consumed the event)
        if let Some(ref mut chat) = self.message_list
            && let Some(action) = chat.handle_key_event(key)
        {
            self.process_action(action);
        }
    }

    /// Handle bracketed paste text from the terminal (⌘V / Shift+Insert).
    ///
    /// Routes the pasted content to the composer when no overlay is active.
    /// When the pasted text is empty (clipboard contains only image data),
    /// falls back to direct clipboard reading for image paste.
    pub(crate) fn handle_paste(&mut self, text: String) {
        // If an overlay is open, defer paste — the overlay will handle
        // paste via its own Ctrl+V + arboard logic for now.
        if !self.overlays.is_empty() {
            return;
        }
        if let Some(ref mut composer) = self.composer
            && let Some(action) = composer.handle_paste(&text) {
                self.process_action(action);
            }
    }

    /// Single dispatch point for all crossterm events.
    ///
    /// Both the batch drain (Phase 1a) and the idle wait (Phase 3) in the
    /// event loop call this method so that every event variant is handled
    /// in exactly one place.
    pub(crate) fn handle_crossterm_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => self.handle_key_event(key),
            Event::Mouse(mouse) => self.handle_mouse_event(mouse),
            Event::Paste(text) => self.handle_paste(text),
            Event::Resize(w, h) => self.handle_resize(w, h),
            Event::FocusGained => self.handle_focus_event(true),
            Event::FocusLost => self.handle_focus_event(false),
        }
    }

    /// Handle Tab key for session mode switching.
    fn handle_tab_mode_switch(&mut self) {
        // Mode switching works even without a current session (welcome page).
        // Per-session pending modes only apply when there IS a session.
        if let Some(sid) = self.current_session_id
            && self.pending_modes.contains_key(&sid)
        {
            // Cancel pending mode switch.
            self.pending_modes.remove(&sid);
            self.set_notice("Mode switch cancelled");
            return;
        }

        let is_busy = self.current_session_id.is_some_and(|sid| {
            self.runtime.is_session_busy(sid) || self.pending_approvals.contains_key(&sid)
        });

        if is_busy || !self.pending_prompt_queue.is_empty() {
            // Request in progress: defer mode switch until request completes.
            let new_mode = self.mode.toggle();
            if let Some(sid) = self.current_session_id {
                self.pending_modes.insert(sid, new_mode);
            }
            self.set_notice(format!(
                "Mode will switch to {} on completion",
                new_mode.title()
            ));
        } else {
            // Apply immediately.
            self.mode = self.mode.toggle();
            self.set_notice(format!("Mode switched to {}", self.mode.title()));
        }
    }

    /// Navigate between subsessions.
    fn handle_subsession_navigation(&mut self, key: KeyEvent) {
        let Some(ref chat) = self.message_list else {
            return;
        };
        let Some(ctx) = chat.active_chat_context() else {
            return;
        };
        let Some(parent_id) = ctx.parent_session_id else {
            return;
        };
        let current_id = ctx.session_id;

        // Cache current session's context_usage before navigating away.
        if let Some(usage) = &self.context_usage {
            self.context_usage_cache.insert(current_id, usage.clone());
        }

        match key.code {
            KeyCode::Up => {
                // Switch to parent session in-memory (no DB load).
                if let Some(chat) = self.message_list.as_mut() {
                    chat.switch_to_session(parent_id);
                    self.current_session_id = Some(parent_id);
                }
                // Restore cached context_usage for parent session.
                self.context_usage = self.context_usage_cache.remove(&parent_id);
            }
            KeyCode::Down => {
                // Switch to the last (most recently delegated) child.
                let all = self
                    .runtime
                    .session_manager()
                    .store()
                    .list_sessions_unfiltered(1000, 0)
                    .unwrap_or_default();
                let children: Vec<_> = all
                    .into_iter()
                    .filter(|s| s.parent_session_id == Some(parent_id))
                    .collect();
                if let Some(target) = children.last() {
                    let target_id = target.session_id;
                    if let Some(chat) = self.message_list.as_mut() {
                        if chat.switch_to_session(target_id) {
                            self.current_session_id = Some(target_id);
                        } else {
                            self.switch_to_session(target_id);
                            // switch_to_session goes through SessionAction::Select
                            // which already handles context_usage_cache internally.
                            // Skip the manual restore below.
                            return;
                        }
                    }
                    // Fast path succeeded — restore cached context_usage.
                    self.context_usage = self.context_usage_cache.remove(&target_id);
                }
            }
            KeyCode::Left | KeyCode::Right => {
                let step = if key.code == KeyCode::Left {
                    -1isize
                } else {
                    1
                };
                let all = self
                    .runtime
                    .session_manager()
                    .store()
                    .list_sessions_unfiltered(1000, 0)
                    .unwrap_or_default();
                let children: Vec<_> = all
                    .into_iter()
                    .filter(|s| s.parent_session_id == Some(parent_id))
                    .collect();
                if children.is_empty() {
                    return;
                }
                let index = children
                    .iter()
                    .position(|s| s.session_id == current_id)
                    .unwrap_or(usize::MAX);
                let next_index = if index == usize::MAX {
                    0
                } else {
                    (index as isize + step).rem_euclid(children.len() as isize) as usize
                };
                if let Some(target) = children.get(next_index) {
                    let target_id = target.session_id;
                    if let Some(chat) = self.message_list.as_mut() {
                        if chat.switch_to_session(target_id) {
                            self.current_session_id = Some(target_id);
                        } else {
                            self.switch_to_session(target_id);
                            // switch_to_session goes through SessionAction::Select
                            // which already handles context_usage_cache internally.
                            return;
                        }
                    }
                    // Fast path succeeded — restore cached context_usage.
                    self.context_usage = self.context_usage_cache.remove(&target_id);
                }
            }
            _ => {}
        }
    }

    /// Switch to a different session (via SessionAction::Select).
    fn switch_to_session(&mut self, session_id: Uuid) {
        self.process_action(Action::Session(SessionAction::Select(session_id)));
    }

    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        use crossterm::event::MouseButton;
        use crossterm::event::MouseEventKind;

        let position = ratatui::layout::Position::new(mouse.column, mouse.row);

        // Route mouse events to overlays first (top overlay has priority).
        // Let scroll events fall through to the chat area when the overlay
        // blocks input but does not handle the scroll itself — mirrors the
        // PageUp/PageDown pattern for keyboard events (step 1a).
        if let Some(action) = self.overlays.handle_mouse_event(mouse, self.terminal_area) {
            if matches!(action, Action::Noop)
                && matches!(
                    mouse.kind,
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                )
            {
                // Fall through to chat scroll handling below.
            } else {
                self.process_action(action);
                return;
            }
        }

        // Sidebar scroll (scroll events in the sidebar area)
        if let Some(sidebar_area) = self.sidebar_area
            && sidebar_area.contains(position)
        {
            // Auto-scroll runs every frame (~60 fps), much more frequently than
            // discrete scroll-wheel ticks, so use a fraction of the base speed.
            let raw = self.runtime.config().ui.scroll_speed as usize;
            let speed = raw.saturating_div(3).max(1);
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.sidebar.scroll_down(speed);
                }
                MouseEventKind::ScrollUp => {
                    self.sidebar.scroll_up(speed);
                }
                _ => {}
            }
            return;
        }

        // Determine the message content area bounds for selection clamping.
        let msg_bounds = self.message_list.as_ref().and_then(|ml| ml.content_area);

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Clear scrollbar hover on any click (will be re-set on Moved).
                if let Some(ref mut chat) = self.message_list {
                    chat.set_scrollbar_hovered(false);
                }
                // Check scrollbar click first.
                if let Some(ref mut chat) = self.message_list {
                    let sb_area = chat.scrollbar_area();
                    if sb_area.is_some_and(|a| a.contains(position)) {
                        chat.start_scrollbar_drag(position.y);
                        return;
                    }
                }

                // MessageList click-to-expand or subsession navigation (non-drag).
                // Run BEFORE mouse selection so interactive elements get priority.
                if let Some(ref mut chat) = self.message_list
                    && let Some(action) = chat.handle_mouse_click(mouse.column, mouse.row)
                {
                    self.process_action(action);
                    return;
                }

                // Composer input area: set cursor and start selection.
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_down(position, text_area);
                        self.mouse_selection.clear();
                        return;
                    }
                }

                // Start mouse selection if within message area (no interactive hit).
                if msg_bounds.is_some_and(|b| b.contains(position)) {
                    let scroll_offset = self
                        .message_list
                        .as_ref()
                        .map(|ml| ml.scroll_offset)
                        .unwrap_or(0);

                    // Refine bounds to the specific selectable region under the cursor
                    // (mirrors old TUI's selection_bounds_for_position).
                    let area = msg_bounds.unwrap();
                    let refined = self.message_list.as_ref().and_then(|ml| {
                        let hit = ml
                            .selectable_region_rects()
                            .iter()
                            .find(|r| r.contains(position))
                            .copied();
                        hit.map(|r| Rect {
                            x: r.x,
                            y: area.y,
                            width: r.width,
                            height: area.height,
                        })
                        .or(Some(area))
                    });

                    self.mouse_selection.press(position, refined, scroll_offset);
                }
            }
            MouseEventKind::Moved => {
                if let Some(ref mut chat) = self.message_list {
                    chat.set_hovered_card(mouse.column, mouse.row);
                    // Update scrollbar hover state
                    let sb_area = chat.scrollbar_area();
                    let hovered = sb_area.is_some_and(|a| a.contains(position));
                    chat.set_scrollbar_hovered(hovered);
                }
                // Check queued prompt card hover.
                let pos = ratatui::layout::Position::new(mouse.column, mouse.row);
                self.hovered_queued_index = self
                    .queued_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(pos))
                    .map(|(i, _)| *i);
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                // Check scrollbar drag first.
                if let Some(ref mut chat) = self.message_list
                    && (chat.scrollbar_area().is_some_and(|a| a.contains(position))
                        || chat.is_scrollbar_dragging())
                {
                    chat.continue_scrollbar_drag(position.y);
                    return;
                }
                // Always update pointer position for auto-scroll (must happen
                // before the composer check so that update_input_area_auto_scroll
                // can read the latest pointer position).
                self.mouse_selection.drag(position);
                // Composer input area drag (extends selection).
                if let Some(ref mut composer) = self.composer
                    && composer.is_input_dragging()
                {
                    composer.handle_mouse_drag(position, composer.last_text_area);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if let Some(ref mut chat) = self.message_list {
                    chat.end_scrollbar_drag();
                    chat.set_scrollbar_hovered(false);
                }

                // Composer input area: finalize selection and queue clipboard copy.
                if let Some(ref mut composer) = self.composer
                    && let Some(selected) = composer.handle_mouse_up(position)
                {
                    self.pending_input_copy = Some(selected);
                }

                // Composer image badge click: open ImageViewer.
                if !self.mouse_selection.is_dragging()
                    && let Some(ref mut composer) = self.composer
                {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        let scroll = composer.input_scroll_offset as u16;
                        let local_y = position.y.saturating_sub(text_area.y);
                        let local_x = position.x.saturating_sub(text_area.x);
                        let target_line = scroll.saturating_add(local_y);
                        let raw_pos = composer.raw_text_position_at_visual(
                            text_area.width,
                            target_line,
                            local_x,
                        );
                        if let Some(span) = composer.span_at(raw_pos)
                            && let Some(data) = &span.image_data
                        {
                            let action =
                                Action::Overlay(OverlayAction::Open(OverlayKind::ImageViewer {
                                    data: data.clone(),
                                    filename: span.image_filename.clone().unwrap_or_default(),
                                }));
                            self.mouse_selection.release(position, 0);
                            self.process_action(action);
                            return;
                        }
                    }
                }

                let scroll_offset = self
                    .message_list
                    .as_ref()
                    .map(|ml| ml.scroll_offset)
                    .unwrap_or(0);
                self.mouse_selection.release(position, scroll_offset);
                // Clipboard copy is handled in draw() where we have access to the frame buffer.
            }
            MouseEventKind::ScrollDown => {
                // Check composer input area first (mirrors old TUI behaviour).
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_scroll_down(text_area.width, text_area.height);
                        return;
                    }
                }
                let speed = self.runtime.config().ui.scroll_speed as isize;
                // Compare effective scroll position (accounts for follow_tail)
                // so we don't shift the selection when the viewport doesn't
                // actually move (e.g. already at the bottom).
                let old_effective = self.message_list.as_ref().map(|ml| {
                    let max = ml.max_scroll();
                    if ml.follow_tail {
                        max
                    } else {
                        ml.scroll_offset.min(max)
                    }
                });
                if self.message_list.is_some() {
                    self.process_action(Action::Chat(ChatAction::ScrollDelta(speed)));
                }
                let new_effective = self.message_list.as_ref().map(|ml| {
                    let max = ml.max_scroll();
                    if ml.follow_tail {
                        max
                    } else {
                        ml.scroll_offset.min(max)
                    }
                });
                if old_effective != new_effective {
                    self.mouse_selection.shift_for_scroll(speed);
                }
            }
            MouseEventKind::ScrollUp => {
                // Check composer input area first (mirrors old TUI behaviour).
                if let Some(ref mut composer) = self.composer {
                    let text_area = composer.last_text_area;
                    if text_area.contains(position) {
                        composer.handle_mouse_scroll_up();
                        return;
                    }
                }
                let speed = self.runtime.config().ui.scroll_speed as isize;
                let old_effective = self.message_list.as_ref().map(|ml| {
                    let max = ml.max_scroll();
                    if ml.follow_tail {
                        max
                    } else {
                        ml.scroll_offset.min(max)
                    }
                });
                if self.message_list.is_some() {
                    self.process_action(Action::Chat(ChatAction::ScrollDelta(-speed)));
                }
                let new_effective = self.message_list.as_ref().map(|ml| {
                    let max = ml.max_scroll();
                    if ml.follow_tail {
                        max
                    } else {
                        ml.scroll_offset.min(max)
                    }
                });
                if old_effective != new_effective {
                    self.mouse_selection.shift_for_scroll(-speed);
                }
            }
            _ => {}
        }
    }

    /// Per-frame auto-scroll while dragging a mouse selection near the
    /// top/bottom edge of the message content area.
    ///
    /// Time-throttled: a minimum interval (50 ms) is enforced between
    /// scroll steps so the rate is consistent regardless of frame rate
    /// or event-loop iteration frequency.
    pub fn update_mouse_selection_auto_scroll(&mut self) {
        if !self.mouse_selection.is_dragging() {
            self.last_selection_auto_scroll = None;
            return;
        }
        let Some(area) = self.message_list.as_ref().and_then(|ml| ml.content_area) else {
            return;
        };
        let Some(pointer) = self.mouse_selection.pointer() else {
            return;
        };

        let left = area.x;
        let right = area.x.saturating_add(area.width);
        if pointer.x < left || pointer.x >= right {
            return;
        }

        // Throttle: at most one step per 50 ms.
        let now = Instant::now();
        let min_interval = Duration::from_millis(50);
        if let Some(last) = self.last_selection_auto_scroll
            && now - last < min_interval
        {
            return;
        }
        self.last_selection_auto_scroll = Some(now);

        let top_threshold = area.y.saturating_add(1);
        let bottom_threshold = area.y.saturating_add(area.height.saturating_sub(2));

        let speed = self.runtime.config().ui.scroll_speed as usize;
        if pointer.y <= top_threshold {
            let chat = self.message_list.as_mut().unwrap();
            let new_scroll = chat.scroll_offset.saturating_sub(speed);
            chat.scroll_offset = new_scroll.min(chat.max_scroll());
            chat.follow_tail = false;
            chat.dirty = true;
        } else if pointer.y >= bottom_threshold {
            let chat = self.message_list.as_mut().unwrap();
            let new_scroll = chat.scroll_offset.saturating_add(speed);
            chat.scroll_offset = new_scroll.min(chat.max_scroll());
            chat.follow_tail = chat.scroll_offset >= chat.max_scroll();
            chat.dirty = true;
        }
    }

    /// Per-frame auto-scroll while dragging a mouse selection in the
    /// composer input area near the top/bottom edge.
    pub fn update_input_area_auto_scroll(&mut self) {
        let Some(ref mut composer) = self.composer else {
            return;
        };
        let text_area = composer.last_text_area;
        if text_area.width == 0 || text_area.height == 0 {
            return;
        }
        let Some(pointer) = self.mouse_selection.pointer() else {
            return;
        };
        composer.update_drag_auto_scroll(pointer, text_area);
    }

    pub fn handle_resize(&mut self, _w: u16, _h: u16) {
        // Full layout rebuild on resize (width change invalidates all line counts).
        if let Some(ref mut chat) = self.message_list {
            chat.invalidate_layout();
        }
        self.sidebar_area = None;
        self.mouse_selection.clear();
    }

    /// Global shortcuts that work regardless of overlay state.
    fn handle_global_key(&self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => Some(Action::Quit),
            KeyCode::F(1) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::ThemePanel,
            ))),
            KeyCode::F(2) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::AgentsPanel,
            ))),
            KeyCode::F(3) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::SkillsPanel,
            ))),
            KeyCode::F(4) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::SettingsPanel,
            ))),
            KeyCode::F(5) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::SearchPanel,
            ))),
            KeyCode::F(6) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::MessagePanel,
            ))),
            KeyCode::F(7) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::ModelPanel,
            ))),
            KeyCode::F(8) => Some(Action::Overlay(OverlayAction::Open(
                OverlayKind::SessionPanel,
            ))),
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => Some(Action::Overlay(
                OverlayAction::Open(OverlayKind::PanelLauncher),
            )),
            _ => None,
        }
    }
}
