use super::*;
use crate::memory_panel::PanelFocus;
use crate::model_panel::{ModelPanelItem, selectable_indices, thinking_options_for_model};
use crate::theme_panel::DisplayItem;
use ratatui::layout::Margin;
use tidev_engine::mcp::McpConnectionStatus;

/// Helper: check if a position is within an overlay rect (including border).
fn in_overlay(position: Position, overlay: Option<Rect>) -> bool {
    overlay.is_some_and(|r| r.contains(position))
}

impl App {
    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent, runtime: &Runtime) {
        // Image viewer overlay: close on any click (Up), block everything else.
        if self.image_viewer.is_some() {
            if matches!(mouse.kind, MouseEventKind::Up(_)) {
                if self.image_viewer_consume_next_up {
                    // This Up belongs to the click that opened the viewer — skip.
                    self.image_viewer_consume_next_up = false;
                } else {
                    self.image_viewer = None;
                    self.dirty = true;
                }
            }
            return;
        }

        // Route mouse events to active overlay panel first.
        // Panels are mutually exclusive — only one can be open at a time.
        if self.theme_panel.is_some() {
            if self.handle_theme_panel_mouse(mouse, runtime) {
                return;
            }
            return; // Panel open but event not in its area; still consume.
        }
        if self.agents_panel.is_some() {
            if self.handle_agents_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.skills_panel.is_some() {
            if self.handle_skills_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.mcp_panel.is_some() {
            if self.handle_mcp_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.settings_panel.is_some() {
            if self.handle_settings_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.model_panel.is_some() {
            if self.handle_model_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.message_panel.is_some() {
            if self.handle_message_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.memory_panel.is_some() {
            if self.handle_memory_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }
        if self.session_panel.is_some() {
            if self.handle_session_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }

        // Stats panel
        if self.stats_panel.as_ref().is_some_and(|p| p.active) {
            if self.handle_stats_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }

        // Balance panel
        let balance_active = self
            .balance_panel
            .lock()
            .map(|guard| guard.as_ref().is_some_and(|p| p.active))
            .unwrap_or(false);
        if balance_active {
            if self.handle_balance_panel_mouse(mouse, runtime) {
                return;
            }
            return;
        }

        // Fall through to chat-area mouse handling
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                self.hovered_card = None;
                self.scrollbar_hovered = false;

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
            MouseEventKind::Moved => {
                let position = Position::new(mouse.column, mouse.row);
                let hit_id = self
                    .tool_result_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .map(|(id, _)| *id)
                    // If not on a tool card, check user message cards
                    .or_else(|| {
                        self.user_card_bounds
                            .iter()
                            .find(|(_, rect)| rect.contains(position))
                            .map(|(id, _)| *id)
                    });
                if self.hovered_card != hit_id {
                    self.hovered_card = hit_id;
                }

                // Check inline running subagent card hover
                let hit_inline = self
                    .inline_subagent_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .map(|(idx, _)| *idx);
                if self.hovered_inline_subagent != hit_inline {
                    self.hovered_inline_subagent = hit_inline;
                }

                // Check queued prompt hover
                let hit_queued = self
                    .queued_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .map(|(idx, _)| *idx);
                if self.hovered_queued_index != hit_queued {
                    self.hovered_queued_index = hit_queued;
                }

                // Check scrollbar hover
                let scrollbar_hovered = self
                    .message_scrollbar_area
                    .is_some_and(|area| area.contains(position));
                if self.scrollbar_hovered != scrollbar_hovered {
                    self.scrollbar_hovered = scrollbar_hovered;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);

                // Clear scrollbar drag state
                self.scrollbar_drag_state = None;

                // Image badge click: open viewer if this was a click (not drag)
                // on an Image span in the input area.
                if !self.mouse_selection.is_dragging() {
                    if let Some(picker) = &self.image_picker {
                        // Check composer image badges first
                        if let Some(inner) = self.input_area.get()
                            && inner.contains(position)
                        {
                            // Compute the raw text position (before span snapping)
                            // to check if the click landed inside an Image span.
                            let scroll = self.input_scroll_offset as u16;
                            let local_line = position.y.saturating_sub(inner.y);
                            let local_column = position.x.saturating_sub(inner.x);
                            let target_line = scroll.saturating_add(local_line);
                            let raw_pos = self.composer.raw_text_position_at_visual(
                                inner.width,
                                target_line,
                                local_column,
                            );
                            if let Some(span) = self.composer.span_at(raw_pos)
                                && span.kind == crate::input::composer::InlineSpanKind::Image
                                && let Some(data_url) = &span.data_url
                            {
                                let data_url = data_url.clone();
                                let filename = span.display.clone();
                                if let Some(viewer) =
                                    crate::ui::image_viewer::ImageViewerState::new(
                                        picker, &data_url, &filename,
                                    )
                                {
                                    self.image_viewer = Some(viewer);
                                    self.image_viewer_consume_next_up = true;
                                    self.dirty = true;
                                    self.mouse_selection
                                        .release(position, self.message_scroll_offset);
                                    return;
                                }
                            }
                        }

                        // Check user message card image badges
                        if let Some((_, _, data_url)) = self
                            .user_image_badge_bounds
                            .iter()
                            .find(|(_, rect, _)| rect.contains(position))
                        {
                            let data_url = data_url.clone();
                            // Derive filename from data URL mime type
                            let filename = data_url
                                .find("data:")
                                .and_then(|i| {
                                    let rest = &data_url[i + 5..];
                                    let mime_end = rest.find(';')?;
                                    let mime = &rest[..mime_end];
                                    let ext = mime
                                        .strip_prefix("image/")
                                        .unwrap_or(mime);
                                    Some(format!("image.{ext}"))
                                })
                                .unwrap_or_else(|| "image".to_string());
                            if let Some(viewer) =
                                crate::ui::image_viewer::ImageViewerState::new(
                                    picker, &data_url, &filename,
                                )
                            {
                                self.image_viewer = Some(viewer);
                                self.image_viewer_consume_next_up = true;
                                self.dirty = true;
                                self.mouse_selection
                                    .release(position, self.message_scroll_offset);
                                return;
                            }
                        }
                    }
                }

                // Handle input area mouse up for selection
                if self.handle_input_area_mouse_up(position) {
                    return;
                }

                if !self.mouse_selection.is_dragging() {
                    // Click on an inline running subagent card → enter subsession directly.
                    // If the execution was already removed (e.g. ToolCompleted fired but render
                    // hasn't caught up), fall through to tool_result_card_bounds below.
                    let hit_running = self
                        .inline_subagent_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(idx, _)| *idx);

                    if let Some(exec_index) = hit_running
                        && let Some(execution) = self.running_subagent_executions.get(exec_index)
                    {
                        self.switch_session(execution.child_session_id, runtime)
                            .ok();
                        return;
                    }
                    // Execution already gone (completed) — fall through to the
                    // completed tool result card check below.

                    // Click on a tool result card
                    let hit_message_id = self
                        .tool_result_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(id, _)| *id);

                    if let Some(message_id) = hit_message_id {
                        // For task/subagent results: click enters subsession directly
                        if self.try_navigate_to_subagent_subsession(message_id, runtime) {
                            return;
                        }
                        // For other tools: click toggles expand/collapse
                        self.toggle_tool_result_expanded(message_id);
                        return;
                    }
                }

                self.mouse_selection
                    .release(position, self.message_scroll_offset);
            }
            MouseEventKind::ScrollUp => {
                let position = Position::new(mouse.column, mouse.row);
                self.hovered_card = None;
                self.scrollbar_hovered = false;
                if self.handle_input_area_scroll_up(position) {
                    return;
                }
                if self.handle_sidebar_scroll_up(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    let speed = self.config.read().unwrap().ui.scroll_speed as usize;
                    self.scroll_messages_up(speed);
                }
            }
            MouseEventKind::ScrollDown => {
                let position = Position::new(mouse.column, mouse.row);
                self.hovered_card = None;
                self.scrollbar_hovered = false;
                if self.handle_input_area_scroll_down(position) {
                    return;
                }
                if self.handle_sidebar_scroll_down(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    let speed = self.config.read().unwrap().ui.scroll_speed as usize;
                    self.scroll_messages_down(speed);
                }
            }
            _ => {}
        }
    }

    // ── Theme Panel ──────────────────────────────────────────────────────────

    fn handle_theme_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.theme_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.theme_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.move_up();
                self.handle_theme_panel_preview_change(&panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.move_down();
                self.handle_theme_panel_preview_change(&panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // List starts at inner.y + 2 (search bar + divider above)
                let header_rows = 2u16;
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < header_rows {
                    return true; // Click on search/header — consume but no action
                }
                let row = (local_y - header_rows) as usize;
                // Compute scroll offset (same logic as in render_theme_panel)
                let list_height = inner.height.saturating_sub(2) as usize;
                let scroll = if panel.selected_index < list_height {
                    0
                } else {
                    let target = panel.selected_index.saturating_sub(list_height / 2);
                    target.min(panel.display_items.len().saturating_sub(list_height))
                };
                let idx = scroll + row;
                if idx < panel.display_items.len()
                    && matches!(panel.display_items[idx], DisplayItem::Theme(_))
                {
                    // Click on a theme: select and confirm (same as Enter)
                    panel.selected_index = idx;
                    if let DisplayItem::Theme(t) = panel.display_items[idx] {
                        panel.preview_theme = t;
                    }
                    self.theme_panel = Some(panel);
                    let _ = self.close_theme_panel(true);
                } else {
                    // Click on header or out of bounds - just consume
                    self.theme_panel = Some(panel);
                }
                true
            }
            _ => true,
        }
    }

    /// Helper: apply theme preview after selection change.
    fn handle_theme_panel_preview_change(&mut self, panel: &ThemePanelState) {
        if panel.preview_theme != self.theme.palette().name {
            self.theme.set_mode(panel.preview_theme);
            self.clear_message_render_cache();
        }
        self.theme_panel = Some(panel.clone());
    }

    // ── Agents Panel ─────────────────────────────────────────────────────────

    fn handle_agents_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.agents_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.agents_panel.clone() else {
            return false;
        };

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.scroll_up(3);
                self.agents_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.scroll_down(3);
                self.agents_panel = Some(panel);
                true
            }
            _ => true,
        }
    }

    // ── Skills Panel ─────────────────────────────────────────────────────────

    fn handle_skills_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.skills_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.skills_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Determine left (35%) vs right (65%) pane
        let inner_w = inner.width as usize;
        let split_x = (inner_w * 35 / 100) as u16;
        let in_left = position.x < inner.x + split_x;
        // The list occupies left pane; the right pane is preview (scroll only)

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if in_left {
                    panel.move_up(10); // matches keyboard step
                } else {
                    panel.scroll_preview_up(3);
                }
                self.skills_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                if in_left {
                    panel.move_down(10);
                } else {
                    panel.scroll_preview_down(3);
                }
                self.skills_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_left {
                    // Left pane: list area. Layout: header rows then items.
                    // From render_skills_panel: header (1 line: "Skills") + filter bar (1) + divider (1) = 3 header rows
                    let header_rows = 3u16;
                    let list_area = Rect::new(inner.x, inner.y, split_x, inner.height);
                    if position.y >= list_area.y + header_rows {
                        let row = (position.y - list_area.y - header_rows) as usize;
                        let idx = panel.list_scroll + row;
                        if idx < panel.filtered_indices.len() {
                            panel.selected_index = idx;
                            panel.preview_scroll = 0;
                        }
                    }
                }
                self.skills_panel = Some(panel);
                true
            }
            _ => {
                self.skills_panel = Some(panel);
                true
            }
        }
    }

    // ── MCP Panel ────────────────────────────────────────────────────────────

    fn handle_mcp_panel_mouse(&mut self, mouse: MouseEvent, runtime: &Runtime) -> bool {
        let overlay = self.mcp_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.mcp_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // MCP panel layout (non-editor mode):
        // sections[0]: instruction (2 lines)
        // sections[1]: search input (3 lines)
        // sections[2]: list (Min 8)
        // sections[3]: footer (1 line)
        let header_rows = 5u16; // instruction (2) + search (3)

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let items = self.mcp_panel_items();
                panel.move_selection(&items, -1);
                self.mcp_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                let items = self.mcp_panel_items();
                panel.move_selection(&items, 1);
                self.mcp_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                if local_y >= header_rows {
                    let row = (local_y - header_rows) as usize;
                    let items = self.mcp_panel_items();
                    if row < items.len() {
                        panel.selected_index = row;
                        // Same as Enter: toggle connect/disconnect the clicked server
                        if let Some(selected) = panel.selected_item(&items) {
                            let name = selected.summary.name.clone();
                            let result = match selected.summary.status {
                                McpConnectionStatus::Connected
                                | McpConnectionStatus::Connecting => {
                                    runtime.block_on(self.tools.disconnect_mcp_server(&name))
                                }
                                _ => runtime.block_on(self.tools.toggle_mcp_server(&name)),
                            };
                            match result {
                                Ok(()) => {
                                    self.last_notice = Some(format!("Updated MCP server '{name}'"));
                                }
                                Err(error) => {
                                    self.last_notice = Some(error.to_string());
                                }
                            }
                        }
                    }
                }
                self.mcp_panel = Some(panel);
                true
            }
            _ => {
                self.mcp_panel = Some(panel);
                true
            }
        }
    }

    // ── Settings Panel ───────────────────────────────────────────────────────

    fn handle_settings_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.settings_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.settings_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.move_up();
                self.settings_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.move_down();
                self.settings_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < panel.items.len() as u16 {
                    panel.selected_index = local_y as usize;
                    // Same as Enter/Space: toggle the selected setting
                    panel.toggle_selected(self.config.read().unwrap().rtk.installed);
                }
                self.settings_panel = Some(panel);
                true
            }
            _ => {
                self.settings_panel = Some(panel);
                true
            }
        }
    }

    // ── Model Panel ──────────────────────────────────────────────────────────

    fn handle_model_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.model_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.model_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Layout sections:
        // [0] tab bar: 1 line
        // [1] instruction: 2 lines
        // [2] search box: 3 lines
        // [3] model list: Min 8
        let header_rows = 6u16; // tab bar (1) + instruction (2) + search (3)

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let items = self.model_panel_items(&panel);
                panel.move_selection(&items, -1);
                self.model_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                let items = self.model_panel_items(&panel);
                panel.move_selection(&items, 1);
                self.model_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                let local_x = position.x.saturating_sub(inner.x);

                // ── Tab bar click (row 0) ──
                if local_y == 0 {
                    let mut x_cursor = 0u16;
                    for (idx, tab) in panel.tabs.iter().enumerate() {
                        let label_w = tab.display_name.len() as u16 + 2;
                        if local_x >= x_cursor && local_x < x_cursor + label_w {
                            if idx != panel.selected_tab_index {
                                let mut next_panel = panel;
                                next_panel.select_tab(idx);
                                let items = self.model_panel_items(&next_panel);
                                if next_panel.is_general_tab() {
                                    // General tab: use self.active_model directly
                                    next_panel.reset_selection(
                                        &items,
                                        Some((
                                            &self.active_model.provider_id,
                                            &self.active_model.model_id,
                                        )),
                                    );
                                } else {
                                    let active = super::panels::agent_tab_active_model(
                                        &next_panel,
                                        &self.active_model,
                                    );
                                    if let Some((p, m)) = active {
                                        next_panel.reset_selection(&items, Some((&p, &m)));
                                    } else {
                                        next_panel.reset_selection(&items, None);
                                    }
                                }
                                self.model_panel = Some(next_panel);
                            } else {
                                self.model_panel = Some(panel);
                            }
                            return true;
                        }
                        // separator " │ " (3 chars), skip for last tab
                        let sep_w = if idx + 1 < panel.tabs.len() { 3 } else { 0 };
                        x_cursor += label_w + sep_w;
                    }
                    self.model_panel = Some(panel);
                    return true;
                }

                if local_y >= header_rows {
                    let row = (local_y - header_rows) as usize;
                    let items = self.model_panel_items(&panel);
                    let tab_index = panel.selected_tab_index;
                    let is_expanded = panel
                        .tabs
                        .get(tab_index)
                        .map(|t| t.thinking_level_expanded)
                        .unwrap_or(false);
                    let model_idx = panel
                        .tabs
                        .get(tab_index)
                        .map(|t| t.selected_index)
                        .unwrap_or(0);

                    if is_expanded {
                        // ── Expanded thinking level mode ──
                        // The render inserts extra rows for thinking options right after the model row.
                        // Layout:
                        //   [model_idx]         → items[model_idx] (the expanded model)
                        //   [model_idx + 1]     → thinking option 0
                        //   [model_idx + 2]     → thinking option 1
                        //   ...
                        //   [model_idx + tl_count] → thinking option N-1
                        //   [model_idx + tl_count + 1] → items[model_idx + 1] (next item, shifted)
                        let tl_options = thinking_options_for_model(&items, model_idx);
                        let tl_count = tl_options.len();

                        if row == model_idx {
                            // Click on the model row itself → confirm current thinking selection
                            self.confirm_after_thinking_click(&items, panel, tab_index, model_idx);
                            return true;
                        } else if row > model_idx && row <= model_idx + tl_count {
                            // Click on a thinking sub-option → set its index and confirm
                            let tl_click = row - model_idx - 1;
                            let mut next_panel = panel;
                            if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                                tab.selected_index = model_idx;
                                tab.thinking_level_index = tl_click;
                            }
                            self.confirm_after_thinking_click(
                                &items, next_panel, tab_index, model_idx,
                            );
                            return true;
                        } else if row < model_idx {
                            // Click above the expanded model: no offset adjustment needed
                            self.handle_model_click_normal(panel, &items, row, tab_index, false);
                        } else {
                            // Click below the expanded thinking area: adjust for extra rows
                            let real_row = row.saturating_sub(tl_count);
                            self.handle_model_click_normal(
                                panel, &items, real_row, tab_index, false,
                            );
                        }
                    } else {
                        // ── Normal (non-expanded) mode ──
                        // row directly maps to items[row] (no extra rows)
                        self.handle_model_click_normal(panel, &items, row, tab_index, false);
                    }
                } else {
                    self.model_panel = Some(panel);
                }
                true
            }
            _ => {
                self.model_panel = Some(panel);
                true
            }
        }
    }

    /// Handle a click on the model panel in non-expanded (normal) mode.
    /// `row` is the direct index into `items[]` (no thinking offset).
    /// When `skip_expand` is true, treat clicks on thinking-supporting models
    /// as immediate apply (used after confirming from expanded mode).
    fn handle_model_click_normal(
        &mut self,
        panel: ModelPanelState,
        items: &[ModelPanelItem],
        row: usize,
        tab_index: usize,
        skip_expand: bool,
    ) {
        // `row` is the direct index into items[]. Map it to a selectable model index.
        let model_idx = if row < items.len() && items[row].is_selectable() {
            row
        } else {
            let selectable = selectable_indices(items);
            if selectable.is_empty() {
                self.model_panel = Some(panel);
                return;
            }
            *selectable
                .iter()
                .min_by_key(|&&idx| (idx as isize - row as isize).abs())
                .unwrap()
        };

        let is_general = panel.is_general_tab();

        let selected_summary = {
            let mut p = panel.clone();
            if let Some(tab) = p.tabs.get_mut(tab_index) {
                tab.selected_index = model_idx;
            }
            p.selected_model(items).cloned()
        };

        let mut next_panel = panel;

        if let Some(summary) = selected_summary {
            let tl_options = thinking_options_for_model(items, model_idx);
            if tl_options.is_empty() || skip_expand {
                // No thinking, or skip_expand: apply immediately
                if is_general {
                    self.switch_model(Some(&summary.label())).ok();
                    if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                        tab.selected_index = model_idx;
                        tab.current_label = summary.label();
                        tab.thinking_level_expanded = false;
                    }
                } else {
                    let agent_type_str = next_panel
                        .tabs
                        .get(tab_index)
                        .map(|t| t.agent_type_str.clone())
                        .unwrap_or_default();
                    let model_str = summary.label();
                    self.config
                        .write()
                        .unwrap()
                        .set_agent_model(&self.paths, &agent_type_str, &model_str)
                        .ok();
                    if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                        tab.selected_index = model_idx;
                        tab.current_label = model_str.clone();
                        tab.thinking_level_expanded = false;
                    }
                    self.last_notice = Some(format!(
                        "Agent '{}' model set to {}",
                        agent_type_str, model_str
                    ));
                }
            } else {
                // Has thinking: expand the submenu
                if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                    tab.selected_index = model_idx;
                    tab.thinking_level_expanded = true;
                    let current_tl = self.thinking_level.to_string();
                    tab.thinking_level_index = tl_options
                        .iter()
                        .position(|opt| opt.to_ascii_lowercase() == current_tl)
                        .unwrap_or(0);
                }
            }
        } else {
            if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                tab.selected_index = model_idx;
            }
        }
        self.model_panel = Some(next_panel);
    }

    /// Confirm the currently selected thinking level and apply the model.
    /// Called when clicking in expanded thinking-level mode.
    fn confirm_after_thinking_click(
        &mut self,
        items: &[ModelPanelItem],
        mut panel: ModelPanelState,
        tab_index: usize,
        model_idx: usize,
    ) {
        let is_general = panel.is_general_tab();

        let summary = {
            let mut p = panel.clone();
            if let Some(tab) = p.tabs.get_mut(tab_index) {
                tab.selected_index = model_idx;
            }
            p.selected_model(items).cloned()
        };

        let Some(summary) = summary else {
            self.model_panel = Some(panel);
            return;
        };

        let tl_options = thinking_options_for_model(items, model_idx);
        let tl_index = panel
            .tabs
            .get(tab_index)
            .map(|t| t.thinking_level_index)
            .unwrap_or(0);
        let tl = if tl_options.is_empty() {
            String::new()
        } else {
            tl_options[tl_index % tl_options.len()].to_string()
        };

        if is_general {
            if !tl.is_empty() {
                let _ = self.store.save_model_thinking_level(
                    &summary.provider_id,
                    &summary.model_id,
                    &tl,
                );
            }
            self.switch_model(Some(&summary.label())).ok();
            if let Some(tab) = panel.tabs.get_mut(tab_index) {
                tab.current_label = summary.label();
                tab.thinking_level_expanded = false;
            }
        } else {
            let agent_type_str = panel
                .tabs
                .get(tab_index)
                .map(|t| t.agent_type_str.clone())
                .unwrap_or_default();
            let model_str = summary.label();
            self.config
                .write()
                .unwrap()
                .set_agent_model_and_thinking(&self.paths, &agent_type_str, &model_str, &tl)
                .ok();
            if let Some(tab) = panel.tabs.get_mut(tab_index) {
                tab.current_label = model_str.clone();
                tab.thinking_level_expanded = false;
            }
            self.last_notice = Some(format!(
                "Agent '{}' model set to {} ({})",
                agent_type_str,
                model_str,
                if tl.is_empty() { "auto" } else { &tl },
            ));
        }
        self.model_panel = Some(panel);
    }

    // ── Message Panel ────────────────────────────────────────────────────────

    fn handle_message_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.message_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.message_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Layout sections:
        // [0] instruction: 2 lines
        // [1] search input: 3 lines
        // [2] list: Min 8
        // [3] footer: 1 line
        let header_rows = 5u16;

        let query = self.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.move_selection(&query, -1);
                self.message_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.move_selection(&query, 1);
                self.message_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                if local_y >= header_rows {
                    let row = (local_y - header_rows) as usize;
                    if row < matches.len() {
                        panel.selected_index = row;
                        // Same as Enter: scroll to the selected message and close
                        if let Some(message) = panel.selected_message(&query) {
                            self.scroll_messages_to_message(message.message_id);
                            self.close_message_panel();
                            return true;
                        }
                    }
                }
                self.message_panel = Some(panel);
                true
            }
            _ => {
                self.message_panel = Some(panel);
                true
            }
        }
    }

    // ── Session Panel ────────────────────────────────────────────────────────

    fn handle_session_panel_mouse(&mut self, mouse: MouseEvent, runtime: &Runtime) -> bool {
        let overlay = self.session_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.session_panel.clone() else {
            return false;
        };
        let overlay = overlay.unwrap();
        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        // Layout sections:
        // [0] instruction: 2 lines
        // [1] search input: 3 lines
        // [2] table: Min 8
        // [3] footer: 1 line
        let header_rows = 5u16;

        let query = self.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if panel.selected_index > 0 {
                    panel.selected_index = panel.selected_index.saturating_sub(1);
                }
                self.session_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                if panel.selected_index + 1 < matches.len() {
                    panel.selected_index = panel.selected_index.saturating_add(1);
                }
                self.session_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                if local_y >= header_rows {
                    let row = (local_y - header_rows) as usize;
                    if row < matches.len() {
                        panel.selected_index = row;
                        // Same as Enter: load the selected session and close
                        if let Some(session) = panel.selected_session(&query).cloned() {
                            self.switch_session(session.session_id, runtime).ok();
                            self.close_session_panel();
                            return true;
                        }
                    }
                }
                self.session_panel = Some(panel);
                true
            }
            _ => {
                self.session_panel = Some(panel);
                true
            }
        }
    }

    // ── Balance Panel ────────────────────────────────────────────────────────

    fn handle_balance_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.balance_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }

        // Balance panel has no scrollable content currently; just consume events.
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => true,
            _ => true,
        }
    }

    // ── Stats Panel ──────────────────────────────────────────────────────────

    fn handle_stats_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let overlay = self.stats_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.scroll_offset = panel.scroll_offset.saturating_sub(3);
                }
                true
            }
            MouseEventKind::ScrollDown => {
                if let Some(panel) = &mut self.stats_panel {
                    panel.scroll_offset = panel.scroll_offset.saturating_add(3);
                }
                true
            }
            _ => true,
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
        true
    }

    /// Handle drag on scrollbar, returning true if consumed.
    fn handle_scrollbar_drag(&mut self, position: Position) -> bool {
        let Some(ref drag) = self.scrollbar_drag_state else {
            return false;
        };

        let max_scroll = drag.max_scroll;
        let track_height = self.message_scrollbar_area.map_or(1, |a| a.height as usize);

        if track_height == 0 {
            return false;
        }

        let delta_y = position.y as isize - drag.start_mouse_y as isize;
        let scroll_delta = (delta_y as f32 / track_height as f32) * max_scroll as f32;
        let new_scroll = (drag.start_scroll as isize + scroll_delta.round() as isize)
            .max(0)
            .min(max_scroll as isize) as usize;

        self.message_scroll_offset = new_scroll;
        self.message_follow_tail = self.message_scroll_offset >= max_scroll;
        true
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
    fn handle_memory_panel_mouse(&mut self, mouse: MouseEvent, _runtime: &Runtime) -> bool {
        let Some(overlay) = self.memory_panel_overlay.get() else {
            return false;
        };
        let position = Position::new(mouse.column, mouse.row);

        if !overlay.contains(position) {
            return false;
        }

        let inner = overlay.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let inner_x = inner.x;
        let inner_y = inner.y;
        let inner_w = inner.width;
        let inner_h = inner.height;

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
                        panel.preview_scroll = panel.preview_scroll.saturating_sub(3);
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
                        panel.preview_scroll = panel.preview_scroll.saturating_add(3);
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

    // ── Input area mouse handlers ────────────────────────────────────────────

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
        // Primary look-up: check the direct message_id → child_session_id map.
        // This is populated in record_tool_result for task tools.
        if let Some(&child_session_id) = self.subagent_result_message_map.get(&message_id) {
            self.switch_session(child_session_id, runtime).ok();
            return true;
        }

        // Fallback: look up via tool_call_id.
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
            self.dirty = true;
            let speed = self.config.read().unwrap().ui.scroll_speed as usize;
            self.scroll_messages_up_internal(speed);
        } else if pointer.y >= bottom_threshold {
            self.dirty = true;
            let speed = self.config.read().unwrap().ui.scroll_speed as usize;
            self.scroll_messages_down_internal(speed);
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

        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        let top_threshold = inner.y;
        let bottom_threshold = inner.y.saturating_add(inner.height.saturating_sub(1));

        // Auto-scroll up when cursor is above the input area
        if self
            .mouse_selection
            .pointer()
            .is_some_and(|p| p.y < top_threshold)
            && self.input_scroll_offset > 0
        {
            self.input_scroll_offset -= 1;
            return true;
        }

        // Auto-scroll down when cursor is below the input area
        let visible_lines = inner.height as usize;
        let total_lines = self.composer.display_line_count(inner.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);
        if self
            .mouse_selection
            .pointer()
            .is_some_and(|p| p.y > bottom_threshold)
            && self.input_scroll_offset < max_scroll
        {
            self.input_scroll_offset += 1;
            return true;
        }

        false
    }
}
