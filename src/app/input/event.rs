use super::*;
use crate::agent::AgentType;
use crate::session::{BackendEvent, Message, MessageRole, ToolExecutionResult};

/// Determine the shell command and argument for each platform.
fn shell_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("powershell", "-Command")
    } else {
        ("sh", "-c")
    }
}

impl App {
    pub(crate) fn handle_event(&mut self, event: Event, runtime: &Runtime) -> Result<()> {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key, runtime)?;
            }
            Event::Paste(text) => {
                if self.model_panel.is_some() {
                    self.handle_model_panel_paste(&text)?;
                } else {
                    self.handle_text_paste(&text)?;
                }
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse, runtime);
            }
            Event::Resize(_, _) => {
                self.clear_mouse_selection();
                self.message_content_area = None;
                self.sidebar_area = None;
                self.clear_message_render_cache();
            }
            Event::FocusGained => {
                crate::log_debug!("Event::FocusGained received");
                self.notifications.set_focused(true);
            }
            Event::FocusLost => {
                crate::log_debug!("Event::FocusLost received");
                self.notifications.set_focused(false);
            }
            _ => {}
        }

        Ok(())
    }

    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent, runtime: &Runtime) {
        if self.model_panel.is_some() {
            return;
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);

                // Check if clicking on scrollbar
                if self.handle_scrollbar_mouse_down(position) {
                    return;
                }

                if self.handle_input_area_mouse_down(position) {
                    return;
                }
                if let Some(bounds) = self.selection_bounds_for_position(position) {
                    self.mouse_selection.press_with_bounds(
                        position,
                        Some(bounds),
                        self.message_scroll_offset,
                    );
                } else {
                    self.clear_mouse_selection();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                if self.handle_scrollbar_drag(position) {
                    return;
                }
                // Always update pointer position for auto-scroll
                self.mouse_selection.drag(position);
                self.handle_input_area_drag(position);
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);

                // Clear scrollbar drag state
                self.scrollbar_drag_state = None;

                // Handle input area mouse up for selection
                if self.handle_input_area_mouse_up(position) {
                    return;
                }

                if !self.mouse_selection.is_dragging() {
                    let is_ctrl = mouse.modifiers.contains(KeyModifiers::CONTROL);

                    // Ctrl+Click on a completed task card → enter subsession
                    if is_ctrl {
                        let hit_message_id = self
                            .tool_result_card_bounds
                            .iter()
                            .find(|(_, rect)| rect.contains(position))
                            .map(|(id, _)| *id);

                        if let Some(message_id) = hit_message_id {
                            if !self.try_navigate_to_subagent_subsession(message_id, runtime) {
                                self.toggle_tool_result_expanded(message_id);
                            }
                            return;
                        }
                    }

                    // Click on running subagent card → enter subsession
                    let hit_running = self
                        .running_subagent_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(idx, _)| *idx);

                    if let Some(execution_index) = hit_running
                        && let Some(execution) =
                            self.running_subagent_executions.get(execution_index)
                    {
                        let child_id = execution.child_session_id;
                        self.switch_session(child_id, runtime).ok();
                        return;
                    }

                    // Plain click on tool result card → toggle expand
                    let hit_message_id = self
                        .tool_result_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(id, _)| *id);

                    if let Some(message_id) = hit_message_id {
                        self.toggle_tool_result_expanded(message_id);
                        return;
                    }
                }

                self.mouse_selection
                    .release(position, self.message_scroll_offset);
            }
            MouseEventKind::ScrollUp => {
                let position = Position::new(mouse.column, mouse.row);
                if self.handle_input_area_scroll_up(position) {
                    return;
                }
                if self.handle_sidebar_scroll_up(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    self.scroll_messages_up(self.config.ui.scroll_speed as usize);
                }
            }
            MouseEventKind::ScrollDown => {
                let position = Position::new(mouse.column, mouse.row);
                if self.handle_input_area_scroll_down(position) {
                    return;
                }
                if self.handle_sidebar_scroll_down(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    self.scroll_messages_down(self.config.ui.scroll_speed as usize);
                }
            }
            _ => {}
        }
    }

    /// Handle mouse down on scrollbar area.
    /// Returns true if the event was handled by the scrollbar.
    fn handle_scrollbar_mouse_down(&mut self, position: Position) -> bool {
        let Some(scrollbar_area) = self.message_scrollbar_area else {
            return false;
        };

        if !scrollbar_area.contains(position) {
            return false;
        }

        let max_scroll = self.message_scroll_max();
        if max_scroll == 0 {
            return false;
        }

        // Calculate target scroll position based on click position (click-to-jump)
        let track_height = scrollbar_area.height as usize;
        let click_y = position.y.saturating_sub(scrollbar_area.y) as f32;
        let scroll_delta = (click_y / track_height as f32) * max_scroll as f32;
        let target_scroll = scroll_delta.round() as usize;

        self.message_scroll_offset = target_scroll.min(max_scroll);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;

        // Initialize drag state for continuous dragging after click
        self.scrollbar_drag_state = Some(state::ScrollbarDragState {
            start_scroll: self.message_scroll_offset,
            start_mouse_y: position.y,
            max_scroll,
        });

        // Clear mouse selection when scrolling via scrollbar
        self.clear_mouse_selection();

        true
    }

    /// Handle mouse drag on scrollbar.
    /// Returns true if the event was handled by the scrollbar.
    fn handle_scrollbar_drag(&mut self, position: Position) -> bool {
        let Some(ref state) = self.scrollbar_drag_state else {
            return false;
        };

        let Some(scrollbar_area) = self.message_scrollbar_area else {
            return false;
        };

        if !scrollbar_area.contains(position) {
            return false;
        }

        // Calculate the new scroll position based on mouse position
        let track_height = scrollbar_area.height as usize;
        let delta_y = position.y as i32 - state.start_mouse_y as i32;

        if delta_y == 0 {
            return true;
        }

        // Calculate scroll delta: each row change in scrollbar = max_scroll / track_height
        let scroll_per_pixel = state.max_scroll as f32 / track_height.max(1) as f32;
        let scroll_delta = (delta_y as f32 * scroll_per_pixel).round() as i32;

        let new_scroll =
            (state.start_scroll as i32 + scroll_delta).clamp(0, state.max_scroll as i32);

        self.message_scroll_offset = new_scroll as usize;
        self.message_follow_tail = self.message_scroll_offset >= state.max_scroll;

        true
    }

    fn handle_input_area_mouse_down(&mut self, position: Position) -> bool {
        let Some(inner) = self.input_area.get() else {
            return false;
        };

        if !inner.contains(position) || inner.width == 0 || inner.height == 0 {
            return false;
        }

        let scroll = self.input_scroll_offset as u16;
        let local_line = position.y.saturating_sub(inner.y);
        let local_column = position.x.saturating_sub(inner.x);
        let target_line = scroll.saturating_add(local_line);

        self.composer
            .set_cursor_at_visual_position(inner.width, target_line, local_column);
        // Start a new selection at the current cursor position
        self.composer.start_selection();
        self.input_dragging = true;
        self.clear_mouse_selection();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
        true
    }

    /// Handle mouse drag in input area for text selection.
    fn handle_input_area_drag(&mut self, position: Position) -> bool {
        if !self.input_dragging {
            return false;
        }

        let Some(inner) = self.input_area.get() else {
            return false;
        };

        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        // Allow dragging outside the input area for auto-scroll
        // Clamp position to input area for cursor positioning
        let clamped_y = position
            .y
            .clamp(inner.y, inner.y + inner.height.saturating_sub(1));
        let clamped_x = position
            .x
            .clamp(inner.x, inner.x + inner.width.saturating_sub(1));

        let scroll = self.input_scroll_offset as u16;
        let local_line = clamped_y.saturating_sub(inner.y);
        let local_column = clamped_x.saturating_sub(inner.x);
        let target_line = scroll.saturating_add(local_line);

        self.composer
            .set_cursor_at_visual_position(inner.width, target_line, local_column);
        // Selection anchor is already set, cursor movement extends selection
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
        true
    }

    /// Handle mouse up in input area - finalize selection and auto-copy.
    fn handle_input_area_mouse_up(&mut self, _position: Position) -> bool {
        if !self.input_dragging {
            return false;
        }

        self.input_dragging = false;

        // If there's a selection, auto-copy it to clipboard
        let selected_text = self.composer.selected_text().map(|s| s.to_string());
        if let Some(selected_text) = selected_text
            && !selected_text.is_empty()
        {
            self.copy_input_selection_to_clipboard(&selected_text);
        }

        true
    }

    /// Copy input area selection to clipboard.
    fn copy_input_selection_to_clipboard(&mut self, text: &str) {
        use mouse_selection::copy_to_clipboard;

        match copy_to_clipboard(text) {
            Ok(lease) => {
                self.selection_clipboard_lease = lease;
                self.toast = Some((
                    "Selection copied to clipboard".to_string(),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
            Err(error) => {
                self.toast = Some((
                    format!("Failed to copy selection: {error}"),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
        }
    }

    /// Handle mouse scroll up in input area.
    fn handle_input_area_scroll_up(&mut self, position: Position) -> bool {
        let Some(inner) = self.input_area.get() else {
            return false;
        };

        if !inner.contains(position) {
            return false;
        }

        if self.input_scroll_offset > 0 {
            self.input_scroll_offset -= 1;
        }
        true
    }

    /// Handle mouse scroll down in input area.
    fn handle_input_area_scroll_down(&mut self, position: Position) -> bool {
        let Some(inner) = self.input_area.get() else {
            return false;
        };

        if !inner.contains(position) {
            return false;
        }

        let visible_lines = inner.height as usize;
        let total_lines = self.composer.display_line_count(inner.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        if self.input_scroll_offset < max_scroll {
            self.input_scroll_offset += 1;
        }
        true
    }

    pub(crate) fn toggle_tool_result_expanded(&mut self, message_id: Uuid) {
        if self.expanded_tool_results.contains(&message_id) {
            self.expanded_tool_results.remove(&message_id);
        } else {
            self.expanded_tool_results.insert(message_id);
        }
        self.clear_message_render_cache();
    }

    /// Attempts to navigate to a subagent's child session from a tool result message.
    /// Returns true if navigation was performed.
    fn try_navigate_to_subagent_subsession(&mut self, message_id: Uuid, runtime: &Runtime) -> bool {
        // Find the message and its tool_call_id
        let tool_call_id = self
            .conversation
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .and_then(|m| m.tool_call_id.as_deref())
            .map(|id| id.to_string());

        let Some(tool_call_id) = tool_call_id else {
            return false;
        };

        // Look up the child session from the subagent_task_map
        if let Some(&child_session_id) = self.subagent_task_map.get(&tool_call_id) {
            self.switch_session(child_session_id, runtime).ok();
            true
        } else {
            false
        }
    }

    pub(crate) fn update_mouse_selection_auto_scroll(&mut self) {
        // Handle input area auto-scroll first
        if self.update_input_area_auto_scroll() {
            return;
        }

        if !self.mouse_selection.is_dragging() || !self.can_scroll_conversation() {
            return;
        }

        let Some(area) = self.message_content_area else {
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

        let top_threshold = area.y.saturating_add(1);
        let bottom_threshold = area.y.saturating_add(area.height.saturating_sub(2));

        if pointer.y <= top_threshold {
            self.scroll_messages_up_internal(self.config.ui.scroll_speed as usize);
        } else if pointer.y >= bottom_threshold {
            self.scroll_messages_down_internal(self.config.ui.scroll_speed as usize);
        }
    }

    /// Update input area scroll based on mouse drag position.
    /// Returns true if auto-scroll was performed.
    fn update_input_area_auto_scroll(&mut self) -> bool {
        if !self.input_dragging {
            return false;
        }

        let Some(inner) = self.input_area.get() else {
            return false;
        };

        // Get current mouse position from the last drag event
        let Some(pointer) = self.mouse_selection.pointer() else {
            return false;
        };

        let top_threshold = inner.y.saturating_add(1);
        let bottom_threshold = inner.y.saturating_add(inner.height.saturating_sub(2));

        let scrolled = if pointer.y <= top_threshold && self.input_scroll_offset > 0 {
            self.input_scroll_offset -= 1;
            true
        } else if pointer.y >= bottom_threshold {
            let visible_lines = inner.height as usize;
            let total_lines = self.composer.display_line_count(inner.width as usize);
            let max_scroll = total_lines.saturating_sub(visible_lines);
            if self.input_scroll_offset < max_scroll {
                self.input_scroll_offset += 1;
                true
            } else {
                false
            }
        } else {
            false
        };

        // If we scrolled, update cursor position to follow
        if scrolled {
            let scroll = self.input_scroll_offset as u16;
            let clamped_y = pointer
                .y
                .clamp(inner.y, inner.y + inner.height.saturating_sub(1));
            let local_line = clamped_y.saturating_sub(inner.y);
            let target_line = scroll.saturating_add(local_line);
            self.composer.set_cursor_at_visual_position(
                inner.width,
                target_line,
                pointer.x.saturating_sub(inner.x),
            );
        }

        scrolled
    }

    pub(crate) fn clear_mouse_selection(&mut self) {
        self.mouse_selection.clear();
    }

    pub(crate) fn selection_bounds_for_position(&self, position: Position) -> Option<Rect> {
        if let Some(area) = self.message_content_area
            && area.contains(position)
        {
            for rect in &self.selectable_regions {
                if rect.contains(position) {
                    return Some(Rect {
                        x: rect.x,
                        y: area.y,
                        width: rect.width,
                        height: area.height,
                    });
                }
            }
            return Some(area);
        }

        if let Some(area) = self.sidebar_area
            && area.contains(position)
        {
            return Some(area.inner(ratatui::layout::Margin {
                horizontal: 1,
                vertical: 1,
            }));
        }

        None
    }

    pub(crate) fn register_selection_region(&self, _area: Rect) {}

    pub(crate) fn can_scroll_conversation(&self) -> bool {
        self.screen == Screen::Chat
            && self.permission_dialog.is_none()
            && self.workspace_boundary_dialog.is_none()
            && self.fork_confirm_dialog.is_none()
            && self.connect_dialog.is_none()
            && self.theme_panel.is_none()
            && self.model_panel.is_none()
            && self.mcp_panel.is_none()
            && self.agents_panel.is_none()
            && !self.command_palette.visible
    }

    pub(crate) fn scroll_messages_to_bottom(&mut self) {
        self.clear_mouse_selection();
        self.message_scroll_offset = 0;
        self.message_follow_tail = true;
    }

    pub(crate) fn message_scroll_max(&self) -> usize {
        self.message_total_lines
            .saturating_sub(self.message_viewport_lines)
    }

    pub(crate) fn message_scroll_page(&self) -> usize {
        self.message_viewport_lines.saturating_sub(1).max(1)
    }

    pub(crate) fn scroll_messages_up(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_up_internal(lines);
    }

    pub(crate) fn scroll_messages_up_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_sub(lines);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    pub(crate) fn scroll_messages_down(&mut self, lines: usize) {
        self.clear_mouse_selection();
        self.scroll_messages_down_internal(lines);
    }

    pub(crate) fn scroll_messages_down_internal(&mut self, lines: usize) {
        let max_scroll = self.message_scroll_max();
        let current = if self.message_follow_tail {
            max_scroll
        } else {
            self.message_scroll_offset.min(max_scroll)
        };

        self.message_scroll_offset = current.saturating_add(lines).min(max_scroll);
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
    }

    fn sidebar_scroll_max(&self) -> usize {
        if let Some(area) = self.sidebar_area {
            let viewport = area.height.saturating_sub(2) as usize;
            self.sidebar_total_lines.saturating_sub(viewport)
        } else {
            0
        }
    }

    fn scroll_sidebar_up(&mut self, lines: usize) {
        self.sidebar_scroll_offset = self.sidebar_scroll_offset.saturating_sub(lines);
    }

    fn scroll_sidebar_down(&mut self, lines: usize) {
        let max_scroll = self.sidebar_scroll_max();
        self.sidebar_scroll_offset = self
            .sidebar_scroll_offset
            .saturating_add(lines)
            .min(max_scroll);
    }

    fn handle_sidebar_scroll_up(&mut self, position: Position) -> bool {
        if let Some(area) = self.sidebar_area
            && area.contains(position)
        {
            if self.sidebar_scroll_offset > 0 {
                self.scroll_sidebar_up(self.config.ui.scroll_speed as usize);
            }
            true // Always consume scroll event when in sidebar area
        } else {
            false
        }
    }

    fn handle_sidebar_scroll_down(&mut self, position: Position) -> bool {
        if let Some(area) = self.sidebar_area
            && area.contains(position)
        {
            let max_scroll = self.sidebar_scroll_max();
            if self.sidebar_scroll_offset < max_scroll {
                self.scroll_sidebar_down(self.config.ui.scroll_speed as usize);
            }
            true // Always consume scroll event when in sidebar area
        } else {
            false
        }
    }

    pub(crate) fn handle_message_scroll_key(&mut self, key: KeyEvent) -> bool {
        if !self.can_scroll_conversation() {
            return false;
        }

        match key.code {
            KeyCode::PageUp => {
                self.scroll_messages_up(self.message_scroll_page());
                true
            }
            KeyCode::PageDown => {
                self.scroll_messages_down(self.message_scroll_page());
                true
            }
            _ => false,
        }
    }

    pub(crate) fn handle_request_abort_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<bool> {
        if key.code != KeyCode::Esc || !self.pending_request {
            return Ok(false);
        }

        if self
            .abort_confirmation_deadline
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.abort_current_request();
            self.drain_queued_prompts(runtime);
            return Ok(true);
        }

        self.abort_confirmation_deadline = Some(Instant::now() + Duration::from_secs(3));
        self.last_notice =
            Some("Press Esc again within 3 seconds to stop the current request".to_string());
        Ok(true)
    }

    pub(crate) fn is_active_request(&self, request_id: u64) -> bool {
        request_id == self.active_request_id
    }

    pub(crate) fn cancel_running_subagents(&mut self) {
        for execution in &self.running_subagent_executions {
            execution.cancel_requested.store(true, Ordering::SeqCst);
        }
        self.running_subagent_executions.clear();
    }

    pub(crate) fn abort_current_request(&mut self) {
        self.active_request_id = self.active_request_id.wrapping_add(1);
        self.abort_confirmation_deadline = None;
        self.pending_request = false;
        self.pending_tool_execution = None;
        self.permission_dialog = None;
        self.question_dialog = None;
        self.fork_confirm_dialog = None;

        // Handle subagent cancellations: record "User cancelled" tool results
        // so the parent agent's tool call has a matching tool message.
        // This prevents orphaned tool calls that some providers (e.g. OpenAI) reject.
        if !self.running_subagent_executions.is_empty() {
            let parent_session_id = self.running_subagent_executions[0].parent_session_id;
            let current_session_id = self.conversation.session_id;
            let is_in_subsession = current_session_id != parent_session_id;
            let cancel_output = "User cancelled the request".to_string();

            for execution in self.running_subagent_executions.drain(..) {
                execution.cancel_requested.store(true, Ordering::SeqCst);

                let result = ToolExecutionResult::new(cancel_output.clone());
                let msg = Message::tool_result(
                    execution.tool_call.id.clone(),
                    execution.tool_call.name.clone(),
                    result,
                );

                // Store in DB for parent session
                let _ = self.store.append_tool_event(
                    parent_session_id,
                    msg.id,
                    &execution.tool_call.name,
                    &execution.tool_call.arguments,
                    &cancel_output,
                );
                let _ = self.store.append_message(parent_session_id, &msg);

                if is_in_subsession {
                    // Also push to cached parent conversation so it's in-memory when restored
                    if let Some(cached) = self.cached_sessions.get_mut(&parent_session_id) {
                        cached.conversation.messages.push(msg.clone());
                    }
                } else {
                    // We're on the parent session: push to in-memory conversation
                    self.conversation.messages.push(msg.clone());
                }
            }

            if is_in_subsession {
                self.pending_assistant_turns.insert(parent_session_id);
            }
        }

        self.cancel_running_subagents();

        // Clean up per-batch boundary approvals
        self.workspace_boundary_approved.clear();

        // Handle running tool execution cancellations: same orphan prevention
        if !self.running_tool_executions.is_empty() {
            let session_id = self.conversation.session_id;
            let cancel_output = "User cancelled the request".to_string();

            for running in self.running_tool_executions.drain(..) {
                running.cancel_requested.store(true, Ordering::SeqCst);

                let result = ToolExecutionResult::new(cancel_output.clone());
                let msg = Message::tool_result(
                    running.tool_call.id.clone(),
                    running.tool_call.name.clone(),
                    result,
                );

                let _ = self.store.append_tool_event(
                    session_id,
                    msg.id,
                    &running.tool_call.name,
                    &running.tool_call.arguments,
                    &cancel_output,
                );
                let _ = self.store.append_message(session_id, &msg);
                self.conversation.messages.push(msg);
            }
        }

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.role = MessageRole::Error;
            message.streaming = false;
            // Keep original reasoning and content intact to preserve thinking at interruption point
            let persisted = message.clone();
            let message_id = message.id;
            let _ = self
                .store
                .append_message(self.conversation.session_id, &persisted);
            self.invalidate_active_message_render_cache_for(message_id);
        }

        self.last_notice = Some("Request cancelled".to_string());
    }

    pub(crate) fn handle_theme_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.theme_panel {
            match key.code {
                KeyCode::Up => {
                    let previous_theme = panel.preview_theme;
                    panel.move_up();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                        self.clear_message_render_cache();
                    }
                }
                KeyCode::Down => {
                    let previous_theme = panel.preview_theme;
                    panel.move_down();
                    if panel.preview_theme != previous_theme {
                        self.theme.set_mode(panel.preview_theme);
                        self.clear_message_render_cache();
                    }
                }
                KeyCode::Enter => {
                    let _ = self.close_theme_panel(true);
                }
                KeyCode::Esc => {
                    let _ = self.close_theme_panel(false);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_agents_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.agents_panel.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.agents_panel = None;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_skills_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        // Handle close actions first to avoid borrowing issues
        let should_close = matches!(key.code, KeyCode::Esc | KeyCode::Char('q'));
        if should_close {
            self.skills_panel = None;
            return Ok(());
        }

        if let Some(panel) = &mut self.skills_panel {
            // When query is active, handle text input first
            if panel.query_active {
                match key.code {
                    KeyCode::Esc => {
                        panel.query_active = false;
                    }
                    KeyCode::Enter => {
                        panel.query_active = false;
                    }
                    KeyCode::Backspace => {
                        panel.backspace_query();
                    }
                    KeyCode::Char(c) => {
                        panel.append_to_query(c);
                    }
                    _ => {}
                }
                return Ok(());
            }

            // Normal navigation mode
            match key.code {
                KeyCode::Char('/') | KeyCode::Char('s') => {
                    panel.query_active = true;
                }
                KeyCode::Char('c') => {
                    // Copy selected skill name to composer and close
                    if let Some(name) = panel.selected_skill_name() {
                        let name = name.to_string();
                        self.composer.set_text(format!("/skill {}", name));
                        self.skills_panel = None;
                        self.last_notice = Some(format!("Skill '{}' selected", name));
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    panel.move_up(10);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    panel.move_down(10);
                }
                KeyCode::PageUp => {
                    panel.page_up(10);
                }
                KeyCode::PageDown => {
                    panel.page_down(10);
                }
                KeyCode::Home => {
                    panel.selected_index = 0;
                    panel.list_scroll = 0;
                }
                KeyCode::End if !panel.filtered_indices.is_empty() => {
                    panel.selected_index = panel.filtered_indices.len() - 1;
                }
                KeyCode::Left => {
                    panel.scroll_preview_up(5);
                }
                KeyCode::Right => {
                    panel.scroll_preview_down(5);
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_settings_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        if let Some(panel) = &mut self.settings_panel {
            match key.code {
                KeyCode::Up => {
                    panel.move_up();
                }
                KeyCode::Down => {
                    panel.move_down();
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    panel.toggle_selected(self.config.rtk.installed);
                }
                KeyCode::Left => {
                    panel.decrease_selected();
                }
                KeyCode::Right => {
                    panel.increase_selected();
                }
                KeyCode::Esc => {
                    self.close_settings_panel(true)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn handle_model_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(mut panel) = self.model_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let items = self.model_panel_items(&panel);
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let items = self.model_panel_items(&panel);
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let items = self.model_panel_items(&panel);
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    if panel.is_general_tab() {
                        // General tab: switch main session model
                        self.switch_model(Some(&summary.label()))?;
                        // Update the tab's current_label to reflect the active model
                        let mut next_panel = panel;
                        if let Some(t) = next_panel.current_tab_mut() {
                            t.current_label = summary.label();
                        }
                        self.model_panel = Some(next_panel);
                    } else {
                        // Agent tab: save to agent.models
                        let agent_type_str = panel
                            .current_tab()
                            .map(|t| t.agent_type_str.clone())
                            .unwrap_or_default();
                        let model_str = summary.label(); // "provider/model_id"
                        // Update config and persist
                        self.config
                            .set_agent_model(&self.paths, &agent_type_str, &model_str)?;
                        // Update the tab's current_label
                        let mut next_panel = panel;
                        if let Some(t) = next_panel.current_tab_mut() {
                            t.current_label = model_str.clone();
                        }
                        self.model_panel = Some(next_panel);
                        self.last_notice = Some(format!(
                            "Agent '{}' model set to {}",
                            agent_type_str, model_str
                        ));
                    }
                }
                // Stay open so the user can continue configuring; close with Esc.
            }
            KeyCode::Esc => {
                self.close_model_panel();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let items = self.model_panel_items(&panel);
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.close_model_panel();
                    self.begin_provider_edit_for_model(summary.provider_id, summary.model_id)?;
                }
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                let mut next_panel = panel;
                next_panel.next_tab();
                // Reset selection to the new tab's saved position
                let items = self.model_panel_items(&next_panel);
                let active = agent_tab_active_model(&next_panel, &self.active_model);
                if let Some((p, m)) = active {
                    next_panel.reset_selection(&items, Some((&p, &m)));
                } else {
                    next_panel.reset_selection(&items, None);
                }
                self.model_panel = Some(next_panel);
            }
            KeyCode::BackTab | KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                let mut next_panel = panel;
                next_panel.prev_tab();
                let items = self.model_panel_items(&next_panel);
                let active = agent_tab_active_model(&next_panel, &self.active_model);
                if let Some((p, m)) = active {
                    next_panel.reset_selection(&items, Some((&p, &m)));
                } else {
                    next_panel.reset_selection(&items, None);
                }
                self.model_panel = Some(next_panel);
            }
            _ => {
                let previous_query = panel.query.text().to_string();
                let _ = panel.query.handle_key_with_history(key, false);
                if panel.query.text() != previous_query {
                    let items = self.model_panel_items(&panel);
                    let mut next_panel = panel;
                    // On query change, reset the current tab's selection to its configured model
                    let active = agent_tab_active_model(&next_panel, &self.active_model);
                    if let Some((p, m)) = active {
                        next_panel.reset_selection(&items, Some((&p, &m)));
                    } else {
                        next_panel.reset_selection(&items, None);
                    }
                    self.model_panel = Some(next_panel);
                } else {
                    self.model_panel = Some(panel);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn handle_model_panel_paste(&mut self, text: &str) -> Result<()> {
        let Some(mut panel) = self.model_panel.clone() else {
            return Ok(());
        };

        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        let previous_query = panel.query.text().to_string();
        panel.query.insert_str(&normalized);

        if panel.query.text() != previous_query {
            let items = self.model_panel_items(&panel);
            let active = agent_tab_active_model(&panel, &self.active_model);
            if let Some((p, m)) = active {
                panel.reset_selection(&items, Some((&p, &m)));
            } else {
                panel.reset_selection(&items, None);
            }
        }

        self.model_panel = Some(panel);
        Ok(())
    }

    pub(crate) fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        crate::log_debug!(
            "KeyEvent: code={:?}, modifiers={:?}",
            key.code,
            key.modifiers
        );
        if self.leader_key_pending {
            self.leader_key_pending = false;
            let _ = self.handle_leader_key(key, runtime)?;
            return Ok(());
        }

        // In subsession, arrow keys work directly for navigation
        if self.conversation.parent_session_id.is_some() {
            match key.code {
                KeyCode::Up => {
                    if let Some(parent_id) = self.conversation.parent_session_id {
                        self.switch_session(parent_id, runtime)?;
                        return Ok(());
                    }
                }
                KeyCode::Down | KeyCode::Left | KeyCode::Right => {
                    let _ = self.handle_leader_key(key, runtime)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        if matches!(key.code, KeyCode::Char('x')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.leader_key_pending = true;
            self.last_notice =
                Some("Up: parent session, Down/Left/Right: switch subagent".to_string());
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            if !self.composer.text().is_empty() {
                self.composer.clear();
                self.at_mention.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                self.last_notice = Some("Input cleared".to_string());
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('d')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('s')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.toggle_stats_panel();
            return Ok(());
        }

        if self.permission_dialog.is_some() {
            return self.handle_permission_dialog_key(key, runtime);
        }

        if self.workspace_boundary_dialog.is_some() {
            return self.handle_workspace_boundary_dialog_key(key, runtime);
        }

        if self.question_dialog.is_some() {
            return self.handle_question_dialog_key(key, runtime);
        }

        if self.fork_confirm_dialog.is_some() {
            return self.handle_fork_confirm_dialog_key(key, runtime);
        }

        if self.undo_confirm_dialog.is_some() {
            return self.handle_undo_confirm_dialog_key(key, runtime);
        }

        if let Some(dialog) = self.connect_dialog.clone() {
            self.handle_connect_dialog_key(key, dialog)?;
            return Ok(());
        }

        if self.rename_dialog.is_some() {
            return self.handle_rename_session_dialog_key(key, runtime);
        }

        if self.theme_panel.is_some() {
            return self.handle_theme_panel_key(key);
        }

        if self.agents_panel.is_some() {
            return self.handle_agents_panel_key(key);
        }

        if self.skills_panel.is_some() {
            return self.handle_skills_panel_key(key);
        }

        if self.mcp_panel.is_some() {
            return self.handle_mcp_panel_key(key, runtime);
        }

        if self.settings_panel.is_some() {
            return self.handle_settings_panel_key(key);
        }

        if self.model_panel.is_some() {
            return self.handle_model_panel_key(key);
        }

        if self.message_panel.is_some() {
            return self.handle_message_panel_key(key);
        }

        if self.memory_panel.is_some() {
            return self.handle_memory_panel_key(key, runtime);
        }

        if self.session_panel.is_some() {
            return self.handle_session_panel_key(key, runtime);
        }

        if self.stats_panel.as_ref().is_some_and(|p| p.active) {
            return self.handle_stats_panel_key(key);
        }

        if self
            .balance_panel
            .lock()
            .map(|guard| guard.as_ref().is_some_and(|p| p.active))
            .unwrap_or(false)
        {
            return self.handle_balance_panel_key(key, runtime);
        }

        if self.handle_request_abort_key(key, runtime)? {
            return Ok(());
        }

        // Ctrl+P: 打开模型选择面板（无论当前是否有面板）
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_model_panel(String::new());
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('v'))
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
            && !key.modifiers.contains(KeyModifiers::SUPER)
        {
            self.handle_clipboard_paste()?;
            return Ok(());
        }

        // 自动补全弹窗的 Tab 处理优先于模式切换
        if self.snippet_state.visible && !self.snippet_state.snippets.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.snippet_state.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.snippet_state.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.snippet_state.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab => {
                    self.accept_snippet();
                    return Ok(());
                }
                _ => {}
            }
        }

        if self.at_mention.visible && !self.at_mention.suggestions.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.at_mention.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.at_mention.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.at_mention.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_at_mention();
                    return Ok(());
                }
                _ => {}
            }
        }

        // Shell mode completion popup navigation
        if self.shell_completion.visible {
            match key.code {
                KeyCode::Esc => {
                    self.shell_completion.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.shell_completion.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.shell_completion.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Enter => {
                    self.accept_shell_completion();
                    return Ok(());
                }
                _ => {}
            }
        }

        // In shell mode, Tab triggers command completion
        if self.shell_mode && key.code == KeyCode::Tab {
            let prefix = self.composer.text().trim();
            if !prefix.is_empty() {
                self.shell_completion.fetch_completions(prefix);
            }
            return Ok(());
        }

        if !self.command_palette.visible && key.code == KeyCode::Tab {
            if self.pending_mode.is_some() {
                // Cancel pending mode switch if user toggles again
                self.pending_mode = None;
                self.last_notice = Some("Mode switch cancelled".to_string());
            } else if self.pending_request {
                // Request in progress: defer mode switch to next message
                let new_mode = self.mode.toggle();
                self.pending_mode = Some(new_mode);
                self.last_notice = Some(format!(
                    "Mode will switch to {} on next message",
                    new_mode.as_str()
                ));
            } else {
                // No request: switch mode immediately
                self.mode = self.mode.toggle();
                self.refresh_tools();
                self.last_notice = Some(format!("Mode switched to {}", self.mode.as_str()));
            }
            return Ok(());
        }

        if !self.command_palette.visible
            && key.code == KeyCode::Tab
            && key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.thinking_level = self.thinking_level.next();
            self.last_notice = Some(format!("Thinking: {}", self.thinking_level.display_name()));
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('t')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.thinking_level = self.thinking_level.next();
            self.last_notice = Some(format!("Thinking: {}", self.thinking_level.display_name()));
            return Ok(());
        }

        if self.command_palette.visible {
            match key.code {
                KeyCode::Esc => {
                    self.command_palette.clear();
                    return Ok(());
                }
                KeyCode::Up => {
                    self.command_palette.move_selection(-1);
                    return Ok(());
                }
                KeyCode::Down => {
                    self.command_palette.move_selection(1);
                    return Ok(());
                }
                KeyCode::Tab => {
                    if let Some(completion) = self.command_palette.completion() {
                        self.composer.set_text(completion);
                    }
                    self.command_palette
                        .sync(self.composer.text(), &self.commands);
                    return Ok(());
                }
                KeyCode::Enter
                    if !key.modifiers.contains(KeyModifiers::SHIFT)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(selected) = self.command_palette.selected() {
                        let command_line = format!("/{}", selected.spec.name);
                        self.composer.remember_submission(&command_line);
                        self.composer.clear();
                        self.command_palette.clear();
                        self.execute_command_line(&command_line, runtime)?;
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        if !self.command_palette.visible && self.handle_message_scroll_key(key) {
            return Ok(());
        }

        if matches!(key.code, KeyCode::Up | KeyCode::Down) {
            let Some(input_area) = self.input_area.get() else {
                return Ok(());
            };

            let input_width = input_area.width;
            let visible_lines = input_area.height as usize;
            let (cursor_line, _) = self.composer.cursor_position(input_width);
            let cursor_line = cursor_line as usize;
            let total_lines = self.composer.display_line_count(input_width as usize);
            let max_scroll = total_lines.saturating_sub(visible_lines);

            match key.code {
                KeyCode::Up => {
                    // If cursor is at the first visible line and we can scroll up
                    if cursor_line == self.input_scroll_offset && self.input_scroll_offset > 0 {
                        self.input_scroll_offset -= 1;
                    }
                    self.composer.move_up(input_width);
                }
                KeyCode::Down => {
                    // If cursor is at the last visible line and we can scroll down
                    let last_visible_line =
                        self.input_scroll_offset + visible_lines.saturating_sub(1);
                    if cursor_line >= last_visible_line && self.input_scroll_offset < max_scroll {
                        self.input_scroll_offset += 1;
                    }
                    self.composer.move_down(input_width);
                }
                _ => {}
            }

            self.refresh_at_mention_state();
            self.command_palette
                .sync(self.composer.text(), &self.commands);
            return Ok(());
        }

        // Check for shell mode activation: '!' at the beginning of empty input
        if !self.shell_mode
            && key.code == KeyCode::Char('!')
            && key.modifiers.is_empty()
            && self.composer.is_empty()
        {
            self.shell_mode = true;
            self.last_notice = Some("Shell mode (enter a shell command)".to_string());
            return Ok(());
        }

        // Handle shell mode Esc before composer processing
        if self.shell_mode && key.code == KeyCode::Esc {
            self.shell_mode = false;
            self.composer.clear();
            self.last_notice = Some("Exited shell mode".to_string());
            self.command_palette
                .sync(self.composer.text(), &self.commands);
            return Ok(());
        }

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            self.handle_submission(submission, runtime)?;
            self.at_mention.clear();
            self.snippet_state.clear();
        } else {
            if key.code == KeyCode::Enter && !self.draft_attachments.is_empty() {
                self.handle_submission(String::new(), runtime)?;
                self.at_mention.clear();
                self.snippet_state.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                return Ok(());
            }
            self.refresh_at_mention_state();
            self.refresh_snippet_state();
        }

        // Exit shell mode if the composer becomes empty (e.g., user cleared it)
        if self.shell_mode && self.composer.is_empty() {
            self.shell_mode = false;
        }

        // Ensure cursor is visible after any key handling
        self.ensure_input_cursor_visible();

        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    /// Ensure the cursor in the input area is visible by adjusting scroll offset.
    pub(crate) fn ensure_input_cursor_visible(&mut self) {
        let Some(input_area) = self.input_area.get() else {
            return;
        };

        let input_width = input_area.width;
        let visible_lines = input_area.height as usize;
        let (cursor_line, _) = self.composer.cursor_position(input_width);
        let cursor_line = cursor_line as usize;
        let total_lines = self.composer.display_line_count(input_width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        // If cursor is above the visible area
        if cursor_line < self.input_scroll_offset {
            self.input_scroll_offset = cursor_line;
        }
        // If cursor is below the visible area
        else if cursor_line >= self.input_scroll_offset + visible_lines {
            self.input_scroll_offset = (cursor_line + 1).saturating_sub(visible_lines);
        }

        // Clamp to valid range
        self.input_scroll_offset = self.input_scroll_offset.min(max_scroll);
    }

    pub(crate) fn handle_leader_key(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<bool> {
        let current_session_id = self.conversation.session_id;
        let parent_session_id = self
            .conversation
            .parent_session_id
            .unwrap_or(current_session_id);

        match key.code {
            KeyCode::Up if parent_session_id != current_session_id => {
                self.switch_session(parent_session_id, runtime)?;
                return Ok(true);
            }
            KeyCode::Down | KeyCode::Right | KeyCode::Left => {
                let children = self.store.load_child_sessions(parent_session_id)?;
                if children.is_empty() {
                    return Ok(false);
                }

                let step = if matches!(key.code, KeyCode::Left) {
                    -1
                } else {
                    1
                };
                let index = children
                    .iter()
                    .position(|session| session.session_id == current_session_id)
                    .unwrap_or(usize::MAX);
                let next_index = if index == usize::MAX {
                    0
                } else {
                    (index as isize + step).rem_euclid(children.len() as isize) as usize
                };

                if let Some(target) = children.get(next_index) {
                    self.switch_session(target.session_id, runtime)?;
                    return Ok(true);
                }
            }
            _ => {}
        }

        Ok(false)
    }

    pub(crate) fn handle_submission(
        &mut self,
        submission: String,
        runtime: &Runtime,
    ) -> Result<()> {
        if self.shell_mode {
            self.shell_mode = false;
            return self.execute_shell_command(submission.trim(), runtime);
        }

        let trimmed = submission.trim();
        if trimmed.starts_with('/') {
            self.execute_command_line(trimmed, runtime)?;
            self.at_mention.clear();
            self.draft_attachments.clear();
        } else {
            self.submit_prompt(submission, runtime)?;
        }

        Ok(())
    }

    pub(crate) fn execute_shell_command(
        &mut self,
        command: &str,
        runtime: &Runtime,
    ) -> Result<()> {
        let command = command.trim();
        if command.is_empty() {
            self.last_notice = Some("No command entered".to_string());
            return Ok(());
        }

        // Add user message showing the shell command
        let user_message = Message::new(MessageRole::Shell, format!("$ {command}"));
        self.conversation.push(user_message.clone());
        self.store
            .append_message(self.conversation.session_id, &user_message)?;

        // Add a streaming assistant message that will receive the output
        let mut assistant_message = Message::streaming(MessageRole::Shell, "");
        assistant_message.streaming = true;
        self.conversation.push(assistant_message);
        // Don't persist yet; will be persisted when output finishes
        self.scroll_messages_to_bottom();

        // Clone what we need for the async task
        let session_id = self.conversation.session_id;
        let tx = self.backend_tx.clone();
        let command_owned = command.to_string();

        runtime.spawn(async move {
            let (shell, arg) = shell_command();
            let output = match std::process::Command::new(shell)
                .arg(arg)
                .arg(&command_owned)
                .output()
            {
                Ok(output) => output,
                Err(error) => {
                    let _ = tx.send(BackendEvent::ShellOutput {
                        session_id,
                        content: format!("Failed to execute command: {error}"),
                        finished: true,
                        exit_code: None,
                    });
                    return;
                }
            };
            let exit_code = output.status.code();
            let mut content = String::new();
            if output.status.success() {
                content = String::from_utf8_lossy(&output.stdout).trim_end().to_string();
                if content.is_empty() {
                    content = String::from_utf8_lossy(&output.stderr).trim_end().to_string();
                }
            } else {
                if !output.stdout.is_empty() {
                    content.push_str(String::from_utf8_lossy(&output.stdout).trim_end());
                }
                if !output.stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(String::from_utf8_lossy(&output.stderr).trim_end());
                }
            }

            // Format as code block
            let formatted = if !content.is_empty() {
                match exit_code {
                    Some(0) => format!("```\n{content}\n```"),
                    Some(code) => format!("```\n{content}\n```\n\nExit code: {code}"),
                    None => format!("```\n{content}\n```"),
                }
            } else {
                match exit_code {
                    Some(0) => "Command completed successfully (no output)".to_string(),
                    Some(code) => format!("Exit code: {code}"),
                    None => "Command completed (no output)".to_string(),
                }
            };

            let _ = tx.send(BackendEvent::ShellOutput {
                session_id,
                content: formatted,
                finished: true,
                exit_code,
            });
        });

        Ok(())
    }

    pub(crate) fn handle_text_paste(&mut self, text: &str) -> Result<()> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.composer.insert_str(&normalized);
        self.ensure_input_cursor_visible();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
    }

    pub(crate) fn handle_clipboard_paste(&mut self) -> Result<()> {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                self.last_notice = Some(format!("Clipboard unavailable: {error}"));
                return Ok(());
            }
        };

        if let Ok(text) = clipboard.get_text()
            && !text.is_empty()
        {
            return self.handle_text_paste(&text);
        }

        let image = match clipboard.get_image() {
            Ok(image) => image,
            Err(_) => {
                self.last_notice =
                    Some("Clipboard does not contain pasteable text or image".to_string());
                return Ok(());
            }
        };

        if !self.active_model.supports_images {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let data_url = match png_data_url_from_clipboard_image(image) {
            Ok(value) => value,
            Err(error) => {
                self.last_notice = Some(format!("Failed to decode clipboard image: {error}"));
                return Ok(());
            }
        };

        self.draft_attachments.push(MessageAttachment::Image {
            filename: format!("pasted-image-{}.png", Uuid::new_v4()),
            mime: "image/png".to_string(),
            data_url,
        });
        self.composer.insert_str("[Image]");
        self.last_notice = Some("Image pasted into draft".to_string());
        Ok(())
    }

    pub(crate) fn refresh_at_mention_state(&mut self) {
        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.message_panel.is_some()
            || self.session_panel.is_some()
            || self.mcp_panel.is_some()
            || self.agents_panel.is_some()
            || self.question_dialog.is_some()
        {
            self.at_mention.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.at_mention
            .sync(self.workspace_root.as_path(), text, cursor);
    }

    pub(crate) fn accept_at_mention(&mut self) {
        let Some((start, _query)) =
            current_at_fragment(self.composer.text(), self.composer.cursor())
        else {
            self.at_mention.clear();
            return;
        };

        let Some(selection) = self.at_mention.selected().cloned() else {
            self.at_mention.clear();
            return;
        };

        let replacement = match selection.kind {
            AtMentionKind::Directory => format!("@{}/", selection.path.trim_end_matches('/')),
            _ => format!("@{}", selection.path),
        };
        self.composer
            .replace_range(start, self.composer.cursor(), &replacement);
        self.at_mention.clear();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn refresh_snippet_state(&mut self) {
        // If snippets haven't been loaded yet, we need to load them first
        // to determine if they are available
        if self.snippet_state.needs_load() {
            self.snippet_state
                .load_snippets(self.workspace_root.as_path(), &self.paths.config_dir);
        }

        // If no snippets available, skip entirely
        if !self.snippet_state.is_enabled() {
            return;
        }

        if self.command_palette.visible
            || self.connect_dialog.is_some()
            || self.theme_panel.is_some()
            || self.model_panel.is_some()
            || self.message_panel.is_some()
            || self.session_panel.is_some()
            || self.mcp_panel.is_some()
            || self.agents_panel.is_some()
            || self.question_dialog.is_some()
            || self.at_mention.visible
        {
            self.snippet_state.clear();
            return;
        }

        let text = self.composer.text();
        let cursor = self.composer.cursor();
        self.snippet_state.sync(
            self.workspace_root.as_path(),
            &self.paths.config_dir,
            text,
            cursor,
        );
    }

    pub(crate) fn accept_snippet(&mut self) {
        let Some(completion) = self.snippet_state.apply_completion() else {
            self.snippet_state.clear();
            return;
        };

        // Get current word range and replace it
        let cursor = self.composer.cursor();
        let query = self.snippet_state.query.clone();

        // Since query is the exact substring extracted going backwards from cursor,
        // its byte length exactly corresponds to the byte offset of the word start.
        let actual_start = cursor.saturating_sub(query.len());

        self.composer
            .replace_range(actual_start, cursor, &completion);
        self.snippet_state.clear();
        self.refresh_snippet_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn accept_shell_completion(&mut self) {
        let Some(completion) = self.shell_completion.accept() else {
            return;
        };

        // Replace the entire input text with the completed command
        let current = self.composer.text();
        // Extract the prefix (everything before the cursor, or the entire text)
        let cursor = self.composer.cursor();
        let word_start = current[..cursor]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        self.composer.replace_range(word_start, cursor, &completion);
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn execute_command_line(&mut self, line: &str, runtime: &Runtime) -> Result<()> {
        let Some((name, args)) = self.commands.parse_invocation(line) else {
            // Not a valid command format, treat as regular message
            return self.submit_prompt(line.to_string(), runtime);
        };

        let Some(spec) = self.commands.command(&name).cloned() else {
            // Unknown command, treat as regular message
            return self.submit_prompt(line.to_string(), runtime);
        };

        self.run_command(spec.name, spec.action, &args, runtime)?;
        self.commands.mark_used(spec.name);
        Ok(())
    }

    pub(crate) fn run_command(
        &mut self,
        _command_name: &str,
        action: CommandAction,
        args: &[String],
        runtime: &Runtime,
    ) -> Result<()> {
        if self.pending_request {
            match action {
                CommandAction::Theme
                | CommandAction::Quit
                | CommandAction::Undo
                | CommandAction::Redo
                | CommandAction::Session
                | CommandAction::Clear
                | CommandAction::Balance
                | CommandAction::Connect
                | CommandAction::Mcp
                | CommandAction::Model
                | CommandAction::Message
                | CommandAction::Rename
                | CommandAction::Settings
                | CommandAction::Stats
                | CommandAction::Init
                | CommandAction::Memory
                | CommandAction::Agents
                | CommandAction::Skills => {}
                _ => {
                    self.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
            CommandAction::Balance => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /balance".to_string());
                }
                self.open_balance_panel(runtime)?;
            }
            CommandAction::Connect => {
                if !args.is_empty() {
                    self.last_notice = Some("Ignoring arguments to /connect".to_string());
                }
                self.open_connect_dialog()?;
            }
            CommandAction::Mcp => match args.first().map(|value| value.as_str()) {
                Some("add") | Some("new") | Some("create") => {
                    self.open_mcp_panel(String::new());
                    self.open_new_mcp_server_editor(String::new());
                }
                Some("edit") => {
                    if let Some(server_name) = args.get(1) {
                        self.open_mcp_panel(server_name.clone());
                        self.open_existing_mcp_server_editor(String::new(), server_name.clone())?;
                    } else {
                        self.last_notice = Some("Usage: /mcp edit <server-name>".to_string());
                    }
                }
                Some("remove") | Some("delete") | Some("rm") => {
                    if let Some(server_name) = args.get(1) {
                        if let Err(error) = self.remove_mcp_server_from_editor(runtime, server_name)
                        {
                            self.last_notice = Some(error.to_string());
                        }
                    } else {
                        self.last_notice = Some("Usage: /mcp remove <server-name>".to_string());
                    }
                }
                _ => {
                    self.open_mcp_panel(args.join(" "));
                }
            },
            CommandAction::Model => {
                self.open_model_panel(args.join(" "));
            }
            CommandAction::Session => {
                self.open_session_panel(args.join(" "))?;
            }
            CommandAction::Compact => {
                self.active_request_id = self.active_request_id.wrapping_add(1);
                let request_id = self.active_request_id;
                let msg = crate::session::Message::streaming(
                    crate::session::MessageRole::System,
                    format!("{}\n\n", crate::session::COMPACTION_MESSAGE_LABEL),
                );
                self.conversation.push(msg);

                self.schedule_context_compaction_for_session(
                    self.conversation.session_id,
                    runtime,
                    Some(request_id),
                );
            }
            CommandAction::Message => {
                self.open_message_panel(args.join(" "))?;
            }
            CommandAction::Rename => {
                self.open_rename_session_dialog()?;
            }
            CommandAction::Clear => {
                self.start_new_session()?;
            }
            CommandAction::Undo => {
                self.undo_last_user_message(runtime)?;
            }
            CommandAction::Redo => {
                self.redo_last_user_message(runtime)?;
            }
            CommandAction::Theme => {
                self.apply_theme_command(args)?;
            }
            CommandAction::Settings => {
                self.open_settings_panel();
            }
            CommandAction::Stats => {
                self.toggle_stats_panel();
            }
            CommandAction::Quit => {
                self.should_quit = true;
            }
            CommandAction::Init => {
                self.composer.set_text(init_command().to_string());
                self.last_notice = Some("Init prompt loaded".to_string());
            }
            CommandAction::Memory => {
                self.open_memory_panel()?;
            }
            CommandAction::Agents => {
                self.agents_panel = Some(ui::agents_panel::AgentsPanelState::new());
            }
            CommandAction::Skills => {
                self.open_skills_panel();
            }
        }

        Ok(())
    }

    pub(crate) fn apply_theme_command(&mut self, args: &[String]) -> Result<()> {
        let direct_theme = args.first().and_then(|v| ThemeName::parse(v));

        if let Some(theme) = direct_theme {
            self.apply_theme(theme)?;
            Ok(())
        } else {
            self.open_theme_panel();
            Ok(())
        }
    }

    pub(crate) fn open_theme_panel(&mut self) {
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = Some(ThemePanelState::new(self.theme.palette().name));
    }

    pub(crate) fn open_memory_panel(&mut self) -> Result<()> {
        self.command_palette.clear();
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;
        let mut panel = MemoryPanelState::new();
        panel.load(
            &self.memory_store,
            &self.workspace_root.display().to_string(),
        )?;
        self.memory_panel = Some(panel);
        Ok(())
    }

    pub(crate) fn open_settings_panel(&mut self) {
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = Some(SettingsPanelState::new(&self.config));
    }

    pub(crate) fn open_model_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;

        let mut panel = ModelPanelState::new();
        panel.query.set_text(initial_query);

        // Build tabs: General first, then each agent type
        let mut tabs = Vec::new();
        // General tab — main session model
        tabs.push(crate::app::model_panel::ModelPanelTab::new(
            "general",
            "General",
            &self.active_model.label(),
        ));
        // Agent tabs
        for agent_type in AgentType::all() {
            if *agent_type == AgentType::General {
                continue;
            }
            let ty = agent_type.display_name();
            let label = self.config.agent_model_display(ty);
            tabs.push(crate::app::model_panel::ModelPanelTab::new(
                ty,
                agent_type.display_name(),
                &label,
            ));
        }
        panel.tabs = tabs;
        panel.selected_tab_index = 0;

        // Initialize selection for the general tab
        let items = self.model_panel_items(&panel);
        panel.reset_selection(
            &items,
            Some((&self.active_model.provider_id, &self.active_model.model_id)),
        );
        self.model_panel = Some(panel);
    }

    pub(crate) fn close_model_panel(&mut self) {
        self.model_panel = None;
    }

    pub(crate) fn open_skills_panel(&mut self) {
        // Close other panels
        self.mcp_panel = None;
        self.agents_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;
        self.memory_panel = None;

        // Build skill items from the catalog
        let skill_items: Vec<ui::skills_panel::SkillItem> = self
            .tools
            .skills()
            .all()
            .iter()
            .map(|skill| ui::skills_panel::SkillItem {
                name: skill.name.clone(),
                description: skill.description.clone(),
                location: skill.location.clone(),
            })
            .collect();

        self.skills_panel = Some(ui::skills_panel::SkillsPanelState::new(skill_items));
    }

    pub(crate) fn open_message_panel(&mut self, initial_query: String) -> Result<()> {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search user messages in the current session");
        self.composer.set_text(initial_query);

        let messages = self
            .conversation
            .visible_messages()
            .iter()
            .filter(|message| matches!(message.role, MessageRole::User))
            .map(|message| crate::app::message_panel::MessagePanelMessage {
                message_id: message.id,
                content: message.content.clone(),
                created_at: message.created_at,
            })
            .collect();

        self.message_panel = Some(MessagePanelState::new(messages));
        self.reset_message_panel_selection();
        Ok(())
    }

    pub(crate) fn close_message_panel(&mut self) {
        if self.message_panel.take().is_some() {
            self.composer.clear();
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_message_panel_selection(&mut self) {
        if let Some(panel) = &mut self.message_panel {
            panel.reset_selection(self.composer.text());
        }
    }

    pub(crate) fn handle_message_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(panel) = self.message_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    self.scroll_messages_to_message(message.message_id);
                    self.close_message_panel();
                }
            }
            KeyCode::Esc => {
                self.close_message_panel();
            }
            KeyCode::Char('f') => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    // 计算要复制的消息数量
                    let message_count = self
                        .get_message_index(message.message_id)
                        .map(|idx| idx + 1)
                        .unwrap_or(1);

                    self.fork_confirm_dialog =
                        Some(crate::app::ui::fork_confirm::ForkConfirmDialogState::new(
                            message.message_id,
                            message_count,
                        ));
                }
            }
            KeyCode::Char('u') => {
                let query = self.composer.text().to_string();
                if let Some(message) = panel.selected_message(&query) {
                    self.undo_confirm_dialog =
                        Some(crate::app::ui::undo_confirm::UndoConfirmDialogState::new(
                            message.message_id,
                            message.content.clone(),
                        ));
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, -1);
                self.message_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let query = self.composer.text().to_string();
                let mut next_panel = panel;
                next_panel.move_selection(&query, 1);
                self.message_panel = Some(next_panel);
            }
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    self.reset_message_panel_selection();
                }
            }
        }

        Ok(())
    }

    /// 获取消息在 conversation.messages 中的索引
    fn get_message_index(&self, message_id: Uuid) -> Option<usize> {
        self.conversation
            .messages
            .iter()
            .position(|m| m.id == message_id)
    }

    pub(crate) fn handle_fork_confirm_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.confirm_fork_session(runtime)?;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.fork_confirm_dialog = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn confirm_fork_session(&mut self, _runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.fork_confirm_dialog.take() else {
            return Ok(());
        };

        // 获取要复制的消息索引
        let message_index = match self.get_message_index(dialog.selected_message_id) {
            Some(idx) => idx,
            None => {
                self.last_notice = Some("Selected message not found".to_string());
                return Ok(());
            }
        };

        // 保存当前 session 状态
        self.cache_active_session_runtime();

        // 创建新 session（独立的，没有 parent）
        let new_session_id = Uuid::new_v4();

        let _record = self.store.create_session(
            new_session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
            &format!("Fork of {}", self.conversation.title),
        )?;

        // 复制消息（从开头到选中的消息），为每条消息生成新的 ID
        let original_messages: Vec<_> = self.conversation.messages[..=message_index].to_vec();

        let mut id_mapping: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();

        for original in &original_messages {
            let mut new_message = original.clone();
            let new_id = Uuid::new_v4();
            id_mapping.insert(original.id, new_id);
            new_message.id = new_id;

            // 更新 tool_call_id 引用（如果有）
            if let Some(ref tool_call_id) = new_message.tool_call_id
                && let Some(&new_tool_call_id) =
                    id_mapping.get(&Uuid::parse_str(tool_call_id).unwrap_or_else(|_| Uuid::nil()))
            {
                new_message.tool_call_id = Some(new_tool_call_id.to_string());
            }

            self.store.append_message(new_session_id, &new_message)?;
        }

        // 加载新 session
        let conversation = self
            .store
            .load_conversation(new_session_id)?
            .context("Failed to load forked conversation")?;
        self.conversation = conversation;
        self.reset_active_runtime();

        // 关闭所有面板和状态
        self.message_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.scroll_messages_to_bottom();

        self.last_notice = Some(format!(
            "Forked session with {} messages",
            original_messages.len()
        ));

        Ok(())
    }

    pub(crate) fn handle_undo_confirm_dialog_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                self.confirm_undo_to_message(runtime)?;
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                self.undo_confirm_dialog = None;
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn confirm_undo_to_message(&mut self, runtime: &Runtime) -> Result<()> {
        let Some(dialog) = self.undo_confirm_dialog.take() else {
            return Ok(());
        };

        // 查找选中的消息并克隆所需数据
        let message_data = self
            .conversation
            .messages
            .iter()
            .find(|m| m.id == dialog.selected_message_id)
            .map(|m| (m.id, m.content.clone()));

        let Some((message_id, message_content)) = message_data else {
            self.last_notice = Some("Selected message not found".to_string());
            return Ok(());
        };

        // 关闭 message_panel
        self.close_message_panel();

        // 调用 revert_to_message 执行 undo
        self.revert_to_message(message_id, message_content, runtime)?;
        self.last_notice = Some("Undo complete".to_string());

        Ok(())
    }

    pub(crate) fn scroll_messages_to_message(&mut self, message_id: Uuid) {
        self.message_scroll_target = Some(message_id);
        self.message_follow_tail = false;
    }

    pub(crate) fn close_theme_panel(&mut self, apply: bool) -> Result<()> {
        if let Some(panel) = self.theme_panel.take() {
            if apply {
                self.apply_theme(panel.preview_theme)?;
            } else {
                self.theme.set_mode(panel.original_theme);
                self.clear_message_render_cache();
            }
        }
        Ok(())
    }

    pub(crate) fn close_settings_panel(&mut self, _apply: bool) -> Result<()> {
        if let Some(panel) = self.settings_panel.take() {
            panel.apply_to_config(&mut self.config);
            self.config.save(&self.paths)?;
        }
        Ok(())
    }

    pub(crate) fn apply_theme(&mut self, theme: ThemeName) -> Result<()> {
        self.theme.set_mode(theme);
        self.clear_message_render_cache();
        self.config.set_theme(theme);
        self.config.save(&self.paths)?;
        self.last_notice = Some(format!("Theme switched to {}", self.theme.name()));
        Ok(())
    }

    pub(crate) fn open_rename_session_dialog(&mut self) -> Result<()> {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();

        self.rename_dialog = Some(RenameSessionDialogState::new(
            self.conversation.title.clone(),
        ));
        self.composer.set_text(self.conversation.title.clone());
        self.composer
            .set_placeholder("Type the new session title and press Enter");
        self.last_notice = Some("Rename the current session title".to_string());
        Ok(())
    }

    pub(crate) fn close_rename_session_dialog(&mut self) {
        self.rename_dialog = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
    }

    pub(crate) fn confirm_rename_session(&mut self) -> Result<()> {
        let mut title = self.composer.text().trim().to_string();
        if title.is_empty() {
            title = "Untitled session".to_string();
        }

        self.conversation.title = title.clone();
        self.store
            .update_session_title(self.conversation.session_id, &title)?;
        self.last_notice = Some("Session title updated".to_string());
        self.close_rename_session_dialog();
        Ok(())
    }

    pub(crate) fn handle_rename_session_dialog_key(
        &mut self,
        key: KeyEvent,
        _runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.close_rename_session_dialog();
                Ok(())
            }
            KeyCode::Enter
                if !key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.confirm_rename_session()
            }
            _ => {
                let _ = self.composer.handle_key_with_history(key, false);
                Ok(())
            }
        }
    }

    pub(crate) fn switch_model(&mut self, selector: Option<&str>) -> Result<()> {
        let model = self.config.resolve_model(&self.auth, selector)?;
        self.active_model = model.clone();
        self.thinking_level = model.thinking_level.clone();
        self.tools.set_active_model(model.clone());
        self.conversation.set_model(
            model.provider_id.clone(),
            model.provider_display_name.clone(),
            model.model_id.clone(),
            model.display_name.clone(),
        );
        self.store.update_session_model(
            self.conversation.session_id,
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
        )?;
        self.config.default_provider = model.provider_id.clone();
        self.config.default_model = model.model_id.clone();
        self.config.save(&self.paths)?;
        self.last_notice = Some(format!("Switched to {}", model.label()));
        Ok(())
    }

    pub(crate) fn start_new_session(&mut self) -> Result<()> {
        self.cache_active_session_runtime();

        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            self.active_model.provider_id.clone(),
            self.active_model.provider_display_name.clone(),
            self.active_model.model_id.clone(),
            self.active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
            &conversation.title,
        )?;

        self.conversation = conversation;
        self.reset_active_runtime();
        self.active_request_id = 0;
        self.screen = Screen::Welcome;
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.mcp_panel = None;
        self.agents_panel = None;
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
        self.scroll_messages_to_bottom();
        self.last_notice = Some("Started a fresh session".to_string());

        Ok(())
    }

    pub(crate) fn submit_prompt(&mut self, prompt: String, runtime: &Runtime) -> Result<()> {
        let prompt = prompt.trim().to_string();
        if prompt.is_empty() && self.draft_attachments.is_empty() {
            return Ok(());
        }

        let attachments = self.build_prompt_attachments(&prompt)?;
        if attachments.iter().any(MessageAttachment::is_image) && !self.active_model.supports_images
        {
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        if self.pending_request || !self.pending_prompt_queue.is_empty() {
            self.queue_prompt(prompt, attachments);
            self.draft_attachments.clear();
            return Ok(());
        }

        self.submit_prompt_now(prompt, attachments, runtime)
    }

    pub(crate) fn handle_stats_panel_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.active = false;
                }
            }
            KeyCode::Tab => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.next_chart();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::BackTab => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.prev_chart();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.prev_granularity();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('l') | KeyCode::Right => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.next_granularity();
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('1') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Hour;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('2') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Day;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('3') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Week;
                    self.refresh_stats_panel();
                }
            }
            KeyCode::Char('4') => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.granularity = crate::stats::Granularity::Month;
                    self.refresh_stats_panel();
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn handle_balance_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &Runtime,
    ) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.close();
                }
            }
            KeyCode::Char('r') => {
                self.refresh_balance_panel(runtime);
            }
            KeyCode::Tab => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.next_provider();
                    // Query balance for the new provider
                    let provider_id = match panel.selected_provider {
                        crate::app::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
                        crate::app::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
                    };
                    if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
                        panel.set_loading(true);
                        let http = self.http_client.clone();
                        let panel_ptr = self.balance_panel.clone();
                        let panel_ptr_clone = panel_ptr.clone();
                        let api_key_clone = api_key.clone();
                        let selected_provider = panel.selected_provider;

                        runtime.spawn(async move {
                            match selected_provider {
                                crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                                    match crate::balance::query_deepseek_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                                crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                                    match crate::balance::query_siliconflow_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_siliconflow_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    } else {
                        let error_msg = match panel.selected_provider {
                            crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                                "DeepSeek API key not configured"
                            }
                            crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                                "SiliconFlow API key not configured"
                            }
                        };
                        panel.set_error(error_msg.to_string());
                    }
                }
            }
            KeyCode::BackTab => {
                if let Ok(mut guard) = self.balance_panel.lock()
                    && let Some(panel) = &mut *guard
                {
                    panel.prev_provider();
                    // Query balance for the new provider
                    let provider_id = match panel.selected_provider {
                        crate::app::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
                        crate::app::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
                    };
                    if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
                        panel.set_loading(true);
                        let http = self.http_client.clone();
                        let panel_ptr = self.balance_panel.clone();
                        let panel_ptr_clone = panel_ptr.clone();
                        let api_key_clone = api_key.clone();
                        let selected_provider = panel.selected_provider;

                        runtime.spawn(async move {
                            match selected_provider {
                                crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                                    match crate::balance::query_deepseek_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                                crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                                    match crate::balance::query_siliconflow_balance(
                                        &http,
                                        &api_key_clone,
                                    )
                                    .await
                                    {
                                        Ok(balance) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_siliconflow_balance(balance);
                                            }
                                        }
                                        Err(e) => {
                                            if let Ok(mut guard) = panel_ptr_clone.lock()
                                                && let Some(panel) = &mut *guard
                                            {
                                                panel.set_error(e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    } else {
                        let error_msg = match panel.selected_provider {
                            crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                                "DeepSeek API key not configured"
                            }
                            crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                                "SiliconFlow API key not configured"
                            }
                        };
                        panel.set_error(error_msg.to_string());
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub(crate) fn toggle_stats_panel(&mut self) {
        if let Some(panel) = &mut self.stats_panel {
            panel.toggle();
            if panel.active {
                self.refresh_stats_panel();
            }
        } else {
            let mut panel = crate::app::ui::stats_panel::StatsPanelState::new();
            panel.active = true;
            self.stats_panel = Some(panel);
            self.refresh_stats_panel();
        }
    }

    fn refresh_stats_panel(&mut self) {
        if let Some(panel) = &mut self.stats_panel
            && panel.needs_refresh()
        {
            let (start, end) = panel.granularity.default_range();
            match self
                .store
                .get_time_range_stats(panel.granularity, start, end)
            {
                Ok(stats) => {
                    panel.cached_stats = Some(stats);
                    panel.last_refresh = Some(chrono::Utc::now());
                }
                Err(e) => {
                    crate::log_error!("Failed to refresh stats: {}", e);
                }
            }
        }
    }

    pub(crate) fn open_balance_panel(&mut self, runtime: &Runtime) -> Result<()> {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = None;
        self.agents_panel = None;

        let mut panel = crate::app::ui::balance_panel::BalancePanelState::new();
        let selected_provider = panel.selected_provider;
        panel.open();
        *self.balance_panel.lock().unwrap() = Some(panel);

        // Query balance based on selected provider
        let provider_id = match selected_provider {
            crate::app::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
            crate::app::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
        };

        if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
            // Set loading state
            if let Ok(mut guard) = self.balance_panel.lock()
                && let Some(panel) = &mut *guard
            {
                panel.set_loading(true);
            }

            let http = self.http_client.clone();
            let panel_ptr = self.balance_panel.clone();
            let panel_ptr_clone = panel_ptr.clone();
            let api_key_clone = api_key.clone();

            runtime.spawn(async move {
                match selected_provider {
                    crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                        match crate::balance::query_deepseek_balance(&http, &api_key_clone).await {
                            Ok(balance) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_balance(balance);
                                }
                            }
                            Err(e) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_error(e.to_string());
                                }
                            }
                        }
                    }
                    crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                        match crate::balance::query_siliconflow_balance(&http, &api_key_clone).await
                        {
                            Ok(balance) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_siliconflow_balance(balance);
                                }
                            }
                            Err(e) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_error(e.to_string());
                                }
                            }
                        }
                    }
                }
            });
        } else {
            // Set error state
            let error_msg = match selected_provider {
                crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                    "DeepSeek API key not configured"
                }
                crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                    "SiliconFlow API key not configured"
                }
            };
            if let Ok(mut guard) = self.balance_panel.lock()
                && let Some(panel) = &mut *guard
            {
                panel.set_error(error_msg.to_string());
            }
        }

        Ok(())
    }

    fn refresh_balance_panel(&mut self, runtime: &Runtime) {
        let mut guard = match self.balance_panel.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        let panel = match &mut *guard {
            Some(panel) => panel,
            None => return,
        };

        if !panel.active || panel.loading {
            return;
        }

        // Determine provider based on selected tab
        let provider_id = match panel.selected_provider {
            crate::app::ui::balance_panel::ProviderTab::DeepSeek => "deepseek",
            crate::app::ui::balance_panel::ProviderTab::SiliconFlow => "siliconflow-cn",
        };

        if let Some(api_key) = self.auth.api_key(provider_id).map(|s| s.to_string()) {
            panel.set_loading(true);

            let http = self.http_client.clone();
            let panel_ptr = self.balance_panel.clone();
            let panel_ptr_clone = panel_ptr.clone();
            let api_key_clone = api_key.clone();
            let selected_provider = panel.selected_provider;

            runtime.spawn(async move {
                match selected_provider {
                    crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                        match crate::balance::query_deepseek_balance(&http, &api_key_clone).await {
                            Ok(balance) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_balance(balance);
                                }
                            }
                            Err(e) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_error(e.to_string());
                                }
                            }
                        }
                    }
                    crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                        match crate::balance::query_siliconflow_balance(&http, &api_key_clone).await
                        {
                            Ok(balance) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_siliconflow_balance(balance);
                                }
                            }
                            Err(e) => {
                                if let Ok(mut guard) = panel_ptr_clone.lock()
                                    && let Some(panel) = &mut *guard
                                {
                                    panel.set_error(e.to_string());
                                }
                            }
                        }
                    }
                }
            });
        } else {
            let error_msg = match panel.selected_provider {
                crate::app::ui::balance_panel::ProviderTab::DeepSeek => {
                    "DeepSeek API key not configured"
                }
                crate::app::ui::balance_panel::ProviderTab::SiliconFlow => {
                    "SiliconFlow API key not configured"
                }
            };
            panel.set_error(error_msg.to_string());
        }
    }
}

/// Helper: given a model panel, parse the current tab's `current_label` into
/// `(provider_id, model_id)` for use with `reset_selection`.
/// Returns owned strings to avoid borrow conflicts with the panel.
fn agent_tab_active_model(
    panel: &crate::app::model_panel::ModelPanelState,
    default: &crate::config::ActiveModel,
) -> Option<(String, String)> {
    let tab = panel.current_tab()?;
    let label = &tab.current_label;
    if label == "<inherit>" || label.is_empty() {
        Some((default.provider_id.clone(), default.model_id.clone()))
    } else if let Some(slash_pos) = label.find('/') {
        let p = &label[..slash_pos];
        let m = &label[slash_pos + 1..];
        Some((p.to_string(), m.to_string()))
    } else {
        None
    }
}
