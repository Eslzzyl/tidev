use super::*;
use crate::tui::memory_panel::PanelFocus;

impl App {
    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent, runtime: &Runtime) {
        if self.model_panel.is_some() {
            return;
        }

        // Route mouse events to memory panel when it's active
        if self.memory_panel.is_some()
            && self.handle_memory_panel_mouse(mouse, runtime)
        {
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

    /// Handle mouse events within the memory panel. Returns true if consumed.
    fn handle_memory_panel_mouse(
        &mut self,
        mouse: MouseEvent,
        _runtime: &Runtime,
    ) -> bool {
        let Some(overlay) = self.memory_panel_overlay.get() else {
            return false;
        };
        let position = Position::new(mouse.column, mouse.row);
        if !overlay.contains(position) {
            return false;
        }

        // Inner area (within the border)
        let inner_x = overlay.x + 1;
        let inner_y = overlay.y + 1;
        let inner_w = overlay.width.saturating_sub(2);
        let inner_h = overlay.height.saturating_sub(2);

        if inner_w < 10 || inner_h < 3 {
            return true; // too small to interact meaningfully
        }

        // The inner area is split vertically into main + footer(1)
        let main_h = inner_h.saturating_sub(1);

        // Local mouse position relative to inner area
        let local_x = position.x.saturating_sub(inner_x);
        let local_y = position.y.saturating_sub(inner_y);

        // Determine left (35%) vs right (65%) pane
        let split_x = (inner_w as usize * 35 / 100) as u16;
        let in_left = local_x < split_x && local_y < main_h;
        let in_right = local_x >= split_x && local_y < main_h;

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if in_left {
                    // Scroll up in left list → move selection up
                    if let Some(mut panel) = self.memory_panel.clone() {
                        panel.move_selection(-1);
                        self.memory_panel = Some(panel);
                    }
                    true
                } else if in_right {
                    // Scroll up in right pane → scroll preview up
                    if let Some(mut panel) = self.memory_panel.clone() {
                        panel.preview_scroll =
                            panel.preview_scroll.saturating_sub(3);
                        self.memory_panel = Some(panel);
                    }
                    true
                } else {
                    false
                }
            }
            MouseEventKind::ScrollDown => {
                if in_left {
                    // Scroll down in left list → move selection down
                    if let Some(mut panel) = self.memory_panel.clone() {
                        panel.move_selection(1);
                        self.memory_panel = Some(panel);
                    }
                    true
                } else if in_right {
                    // Scroll down in right pane → scroll preview down
                    if let Some(mut panel) = self.memory_panel.clone() {
                        panel.preview_scroll =
                            panel.preview_scroll.saturating_add(3);
                        self.memory_panel = Some(panel);
                    }
                    true
                } else {
                    false
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_left {
                    // Click on left list → select the clicked item
                    if let Some(mut panel) = self.memory_panel.clone() {
                        // Calculate which item was clicked (accounting for header)
                        let header_h = 3u16; // "Name" + filter + divider
                        if local_y >= header_h {
                            let list_offset = local_y - header_h;
                            let _half = main_h.saturating_sub(header_h) / 2;
                            let filtered = panel.filtered_indices();
                            if !filtered.is_empty() {
                                let target_idx = list_offset as usize;
                                if target_idx < filtered.len() {
                                    panel.selected_index = target_idx;
                                    panel.preview_scroll = 0;
                                    self.memory_panel = Some(panel);
                                }
                            }
                        }
                    }
                    true
                } else if in_right {
                    // Click on right pane → position cursor in edit mode
                    if let Some(panel) = self.memory_panel.clone() {
                        if panel.focus == PanelFocus::ContentEdit {
                            let mut p = panel;
                            // Account for "EDITING" header (1 line)
                            let local_line = local_y.saturating_sub(1);
                            let editor_width = p.editor_width.get().max(1);
                            p.content_editor.set_cursor_at_visual_position(
                                editor_width,
                                local_line,
                                local_x,
                            );
                            self.memory_panel = Some(p);
                        } else {
                            // In browse mode, clicking right pane does nothing special
                            // (keep focus on list)
                        }
                    }
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    pub(crate) fn register_selection_region(&self, _area: Rect) {}
}
