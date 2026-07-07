use super::*;
use crate::ui::model_panel::{ModelPanelItem, selectable_indices, thinking_options_for_model};
use crate::ui::theme_panel::DisplayItem;
use ratatui::layout::Margin;

/// Helper: check if a position is within an overlay rect (including border).
fn in_overlay(position: Position, overlay: Option<Rect>) -> bool {
    overlay.is_some_and(|r| r.contains(position))
}

impl App {
    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        // Image viewer overlay: close on any click (Up), block everything else.
        if self.ui.image_viewer.is_some() {
            if matches!(mouse.kind, MouseEventKind::Up(_)) {
                if self.ui.image_viewer_consume_next_up {
                    // This Up belongs to the click that opened the viewer — skip.
                    self.ui.image_viewer_consume_next_up = false;
                } else {
                    self.ui.image_viewer = None;
                    self.ui.dirty = true;
                }
            }
            return;
        }

        // Route mouse events to active overlay panel first.
        // Panels are mutually exclusive — only one can be open at a time.
        if self.ui.theme_panel.is_some() {
            if self.handle_theme_panel_mouse(mouse) {
                return;
            }
            return; // Panel open but event not in its area; still consume.
        }
        if self.ui.agents_panel.is_some() {
            if self.handle_agents_panel_mouse(mouse) {
                return;
            }
            return;
        }
        if self.ui.skills_panel.is_some() {
            if self.handle_skills_panel_mouse(mouse) {
                return;
            }
            return;
        }
        if self.ui.settings_panel.is_some() {
            if self.handle_settings_panel_mouse(mouse) {
                return;
            }
            return;
        }
        if self.ui.model_panel.is_some() {
            if self.handle_model_panel_mouse(mouse) {
                return;
            }
            return;
        }
        if self.ui.message_panel.is_some() {
            if self.handle_message_panel_mouse(mouse) {
                return;
            }
            return;
        }
        if self.ui.session_panel.is_some() {
            if self.handle_session_panel_mouse(mouse) {
                return;
            }
            return;
        }

        // Fall through to chat-area mouse handling
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                self.ui.hovered_card = None;
                self.ui.scrollbar_hovered = false;

                // Check if clicking on scrollbar
                if self.handle_scrollbar_mouse_down(position) {
                    return;
                }

                if self.handle_input_area_mouse_down(position) {
                    return;
                }
                if let Some(bounds) = self.selection_bounds_for_position(position) {
                    self.ui.mouse_selection.press_with_bounds(
                        position,
                        Some(bounds),
                        self.ui.message_scroll_offset,
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
                self.ui.mouse_selection.drag(position);
                self.handle_input_area_drag(position);
            }
            MouseEventKind::Moved => {
                let position = Position::new(mouse.column, mouse.row);
                let hit_id = self.ui.tool_result_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .map(|(id, _)| *id)
                    // If not on a tool card, check user message cards
                    .or_else(|| {
                        self.ui.user_card_bounds
                            .iter()
                            .find(|(_, rect)| rect.contains(position))
                            .map(|(id, _)| *id)
                    });
                if self.ui.hovered_card != hit_id {
                    self.ui.hovered_card = hit_id;
                }

                // Check inline running subagent card hover
                let hit_inline = self.ui.inline_subagent_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .and_then(|(idx, _)| self.ui.running_subagent_executions.get(*idx))
                    .map(|exec| exec.child_session_id);
                if self.ui.hovered_inline_subagent != hit_inline {
                    self.ui.hovered_inline_subagent = hit_inline;
                }

                // Check queued prompt hover
                let hit_queued = self.ui.queued_card_bounds
                    .iter()
                    .find(|(_, rect)| rect.contains(position))
                    .map(|(idx, _)| *idx);
                if self.ui.hovered_queued_index != hit_queued {
                    self.ui.hovered_queued_index = hit_queued;
                }

                // Check scrollbar hover
                let scrollbar_hovered = self.ui.message_scrollbar_area
                    .is_some_and(|area| area.contains(position));
                if self.ui.scrollbar_hovered != scrollbar_hovered {
                    self.ui.scrollbar_hovered = scrollbar_hovered;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);

                // Clear scrollbar drag state
                self.ui.scrollbar_drag = None;

                // Image badge click: open viewer if this was a click (not drag)
                // on an Image span in the input area.
                if !self.ui.mouse_selection.is_dragging() {
                    if let Some(picker) = &self.ui.image_picker {
                        // Check composer image badges first
                        if let Some(inner) = self.ui.input_area.get()
                            && inner.contains(position)
                        {
                            // Compute the raw text position (before span snapping)
                            // to check if the click landed inside an Image span.
                            let scroll = self.ui.input_scroll_offset as u16;
                            let local_line = position.y.saturating_sub(inner.y);
                            let local_column = position.x.saturating_sub(inner.x);
                            let target_line = scroll.saturating_add(local_line);
                            let raw_pos = self.ui.composer.raw_text_position_at_visual(
                                inner.width,
                                target_line,
                                local_column,
                            );
                            if let Some(span) = self.ui.composer.span_at(raw_pos)
                                && span.kind == crate::input::composer::InlineSpanKind::Image
                                && let Some(data_url) = &span.data_url
                            {
                                let data_url = data_url.clone();
                                let filename = span.display.clone();
                                if let Some(viewer) = crate::ui::image_viewer::ImageViewerState::new(
                                    picker, &data_url, &filename,
                                ) {
                                    self.ui.image_viewer = Some(viewer);
                                    self.ui.image_viewer_consume_next_up = true;
                                    self.ui.dirty = true;
                                    self.ui
                                        .mouse_selection
                                        .release(position, self.ui.message_scroll_offset);
                                    return;
                                }
                            }
                        }

                        // Check user message card image badges
                        if let Some((_, _, data_url)) = self.ui.user_image_badge_bounds
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
                                    let ext = mime.strip_prefix("image/").unwrap_or(mime);
                                    Some(format!("image.{ext}"))
                                })
                                .unwrap_or_else(|| "image".to_string());
                            if let Some(viewer) = crate::ui::image_viewer::ImageViewerState::new(
                                picker, &data_url, &filename,
                            ) {
                                self.ui.image_viewer = Some(viewer);
                                self.ui.image_viewer_consume_next_up = true;
                                self.ui.dirty = true;
                                self.ui
                                    .mouse_selection
                                    .release(position, self.ui.message_scroll_offset);
                                return;
                            }
                        }
                    }
                }

                // Handle input area mouse up for selection
                if self.handle_input_area_mouse_up(position) {
                    return;
                }

                if !self.ui.mouse_selection.is_dragging() {
                    // Click on an inline running subagent card → enter subsession directly.
                    // If the execution was already removed (e.g. ToolCompleted fired but render
                    // hasn't caught up), fall through to tool_result_card_bounds below.
                    let hit_running = self.ui.inline_subagent_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(idx, _)| *idx);

                    if let Some(exec_index) = hit_running
                        && let Some(execution) = self.ui.running_subagent_executions.get(exec_index)
                    {
                        self.switch_session(execution.child_session_id).ok();
                        return;
                    }
                    // Execution already gone (completed) — fall through to the
                    // completed tool result card check below.

                    // Click on a tool result card
                    let hit_message_id = self.ui.tool_result_card_bounds
                        .iter()
                        .find(|(_, rect)| rect.contains(position))
                        .map(|(id, _)| *id);

                    if let Some(message_id) = hit_message_id {
                        // For task/subagent results: click enters subsession directly
                        if self.try_navigate_to_subagent_subsession(message_id) {
                            return;
                        }
                        // For other tools: click toggles expand/collapse
                        self.toggle_tool_result_expanded(message_id);
                        return;
                    }
                }

                self.ui
                    .mouse_selection
                    .release(position, self.ui.message_scroll_offset);
            }
            MouseEventKind::ScrollUp => {
                let position = Position::new(mouse.column, mouse.row);
                self.ui.hovered_card = None;
                self.ui.scrollbar_hovered = false;
                if self.handle_input_area_scroll_up(position) {
                    return;
                }
                if self.handle_sidebar_scroll_up(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    let speed = self.runtime.config().ui.scroll_speed as usize;
                    self.scroll_messages_up(speed);
                }
            }
            MouseEventKind::ScrollDown => {
                let position = Position::new(mouse.column, mouse.row);
                self.ui.hovered_card = None;
                self.ui.scrollbar_hovered = false;
                if self.handle_input_area_scroll_down(position) {
                    return;
                }
                if self.handle_sidebar_scroll_down(position) {
                    return;
                }
                if self.can_scroll_conversation() {
                    self.clear_mouse_selection();
                    let speed = self.runtime.config().ui.scroll_speed as usize;
                    self.scroll_messages_down(speed);
                }
            }
            _ => {}
        }
    }

    // ── Theme Panel ──────────────────────────────────────────────────────────

    fn handle_theme_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.theme_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.theme_panel.clone() else {
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
                    self.ui.theme_panel = Some(panel);
                    let _ = self.close_theme_panel(true);
                } else {
                    // Click on header or out of bounds - just consume
                    self.ui.theme_panel = Some(panel);
                }
                true
            }
            _ => true,
        }
    }

    /// Helper: apply theme preview after selection change.
    fn handle_theme_panel_preview_change(&mut self, panel: &ThemePanelState) {
        if panel.preview_theme != self.ui.theme.palette().name {
            self.ui.theme.set_mode(panel.preview_theme);
            self.clear_message_render_cache();
        }
        self.ui.theme_panel = Some(panel.clone());
    }

    // ── Agents Panel ─────────────────────────────────────────────────────────

    fn handle_agents_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.agents_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.agents_panel.clone() else {
            return false;
        };

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.scroll_up(3);
                self.ui.agents_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.scroll_down(3);
                self.ui.agents_panel = Some(panel);
                true
            }
            _ => true,
        }
    }

    // ── Skills Panel ─────────────────────────────────────────────────────────

    fn handle_skills_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.skills_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.skills_panel.clone() else {
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
                self.ui.skills_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                if in_left {
                    panel.move_down(10);
                } else {
                    panel.scroll_preview_down(3);
                }
                self.ui.skills_panel = Some(panel);
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
                self.ui.skills_panel = Some(panel);
                true
            }
            _ => {
                self.ui.skills_panel = Some(panel);
                true
            }
        }
    }

    // ── Settings Panel ───────────────────────────────────────────────────────

    fn handle_settings_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.settings_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.settings_panel.clone() else {
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
                self.ui.settings_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.move_down();
                self.ui.settings_panel = Some(panel);
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let local_y = position.y.saturating_sub(inner.y);
                if local_y < panel.items.len() as u16 {
                    panel.selected_index = local_y as usize;
                    // Same as Enter/Space: toggle the selected setting
                    panel.toggle_selected();
                }
                self.ui.settings_panel = Some(panel);
                true
            }
            _ => {
                self.ui.settings_panel = Some(panel);
                true
            }
        }
    }

    // ── Model Panel ──────────────────────────────────────────────────────────

    fn handle_model_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.model_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.model_panel.clone() else {
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
                self.ui.model_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                let items = self.model_panel_items(&panel);
                panel.move_selection(&items, 1);
                self.ui.model_panel = Some(panel);
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
                                    // General tab: use self.runtime.active_model() directly
                                    next_panel.reset_selection(
                                        &items,
                                        Some((
                                            &self.runtime.active_model().provider_id,
                                            &self.runtime.active_model().model_id,
                                        )),
                                    );
                                } else {
                                    let active = super::panels::agent_tab_active_model(
                                        &next_panel,
                                        &self.runtime.active_model(),
                                    );
                                    if let Some((p, m)) = active {
                                        next_panel.reset_selection(&items, Some((&p, &m)));
                                    } else {
                                        next_panel.reset_selection(&items, None);
                                    }
                                }
                                self.ui.model_panel = Some(next_panel);
                            } else {
                                self.ui.model_panel = Some(panel);
                            }
                            return true;
                        }
                        // separator " │ " (3 chars), skip for last tab
                        let sep_w = if idx + 1 < panel.tabs.len() { 3 } else { 0 };
                        x_cursor += label_w + sep_w;
                    }
                    self.ui.model_panel = Some(panel);
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
                    self.ui.model_panel = Some(panel);
                }
                true
            }
            _ => {
                self.ui.model_panel = Some(panel);
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
                self.ui.model_panel = Some(panel);
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
                    let at = agent_type_str.clone();
                    let ms = model_str.clone();
                    self.runtime.update_config(|c| {
                        if ms.is_empty() {
                            c.agent.models.remove(&at);
                        } else {
                            c.agent.models.insert(at, ms);
                        }
                    });
                    let _ = self.runtime.save_config();
                    if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                        tab.selected_index = model_idx;
                        tab.current_label = model_str.clone();
                        tab.thinking_level_expanded = false;
                    }
                    self.ui.last_notice = Some(format!(
                        "Agent '{}' model set to {}",
                        agent_type_str, model_str
                    ));
                }
            } else {
                // Has thinking: expand the submenu
                if let Some(tab) = next_panel.tabs.get_mut(tab_index) {
                    tab.selected_index = model_idx;
                    tab.thinking_level_expanded = true;
                    let current_tl = self.ui.thinking_level.to_string();
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
        self.ui.model_panel = Some(next_panel);
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
            self.ui.model_panel = Some(panel);
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
                let _ = self
                    .runtime
                    .set_model_thinking_level(&summary.provider_id, &summary.model_id, &tl);
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
            let at = agent_type_str.clone();
                    let ms = model_str.clone();
                    let tl_str = tl.clone();
                    self.runtime.update_config(|c| {
                        if ms.is_empty() {
                            c.agent.models.remove(&at);
                        } else {
                            c.agent.models.insert(at.clone(), ms);
                        }
                        if tl_str.is_empty() {
                            c.agent.thinking_levels.remove(&at);
                        } else {
                            c.agent.thinking_levels.insert(at, tl_str);
                        }
                    });
                    let _ = self.runtime.save_config();
            if let Some(tab) = panel.tabs.get_mut(tab_index) {
                tab.current_label = model_str.clone();
                tab.thinking_level_expanded = false;
            }
            self.ui.last_notice = Some(format!(
                "Agent '{}' model set to {} ({})",
                agent_type_str,
                model_str,
                if tl.is_empty() { "auto" } else { &tl },
            ));
        }
        self.ui.model_panel = Some(panel);
    }

    // ── Message Panel ────────────────────────────────────────────────────────

    fn handle_message_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.message_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.message_panel.clone() else {
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

        let query = self.ui.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                panel.move_selection(&query, -1);
                self.ui.message_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                panel.move_selection(&query, 1);
                self.ui.message_panel = Some(panel);
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
                self.ui.message_panel = Some(panel);
                true
            }
            _ => {
                self.ui.message_panel = Some(panel);
                true
            }
        }
    }

    // ── Session Panel ────────────────────────────────────────────────────────

    fn handle_session_panel_mouse(&mut self, mouse: MouseEvent) -> bool {
        let overlay = self.ui.session_panel_overlay.get();
        let position = Position::new(mouse.column, mouse.row);
        if !in_overlay(position, overlay) {
            return false;
        }
        let Some(mut panel) = self.ui.session_panel.clone() else {
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

        let query = self.ui.composer.text().to_string();
        let matches = panel.matching_indices(&query);

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if panel.selected_index > 0 {
                    panel.selected_index = panel.selected_index.saturating_sub(1);
                }
                self.ui.session_panel = Some(panel);
                true
            }
            MouseEventKind::ScrollDown => {
                if panel.selected_index + 1 < matches.len() {
                    panel.selected_index = panel.selected_index.saturating_add(1);
                }
                self.ui.session_panel = Some(panel);
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
                            self.switch_session(session.session_id).ok();
                            self.close_session_panel();
                            return true;
                        }
                    }
                }
                self.ui.session_panel = Some(panel);
                true
            }
            _ => {
                self.ui.session_panel = Some(panel);
                true
            }
        }
    }

    /// Handle mouse down on scrollbar area.
    /// Returns true if the event was handled by the scrollbar.
    fn handle_scrollbar_mouse_down(&mut self, position: Position) -> bool {
        let Some(scrollbar_area) = self.ui.message_scrollbar_area else {
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

        self.ui.message_scroll_offset = target_scroll.min(max_scroll);
        self.ui.message_follow_tail = self.ui.message_scroll_offset >= max_scroll;

        // Initialize drag state for continuous dragging after click
        self.ui.scrollbar_drag = Some(state::ScrollbarDragState {
            start_scroll: self.ui.message_scroll_offset,
            start_mouse_y: position.y,
            max_scroll,
        });
        true
    }

    /// Handle drag on scrollbar, returning true if consumed.
    fn handle_scrollbar_drag(&mut self, position: Position) -> bool {
        let Some(ref drag) = self.ui.scrollbar_drag else {
            return false;
        };

        let max_scroll = drag.max_scroll;
        let track_height = self
            .ui
            .message_scrollbar_area
            .map_or(1, |a| a.height as usize);

        if track_height == 0 {
            return false;
        }

        let delta_y = position.y as isize - drag.start_mouse_y as isize;
        let scroll_delta = (delta_y as f32 / track_height as f32) * max_scroll as f32;
        let new_scroll = (drag.start_scroll as isize + scroll_delta.round() as isize)
            .max(0)
            .min(max_scroll as isize) as usize;

        self.ui.message_scroll_offset = new_scroll;
        self.ui.message_follow_tail = self.ui.message_scroll_offset >= max_scroll;
        true
    }

    pub(crate) fn clear_mouse_selection(&mut self) {
        self.ui.mouse_selection.clear();
    }

    pub(crate) fn selection_bounds_for_position(&self, position: Position) -> Option<Rect> {
        if let Some(area) = self.ui.message_content_area
            && area.contains(position)
        {
            for r in &self.ui.selectable_regions {
                if r.contains(position) {
                    return Some(Rect {
                        x: r.x,
                        y: area.y,
                        width: r.width,
                        height: area.height,
                    });
                }
            }
            return Some(area);
        }

        if let Some(area) = self.ui.sidebar_area
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

    // ── Input area mouse handlers ────────────────────────────────────────────

    fn handle_input_area_mouse_down(&mut self, position: Position) -> bool {
        let Some(inner) = self.ui.input_area.get() else {
            return false;
        };

        if !inner.contains(position) || inner.width == 0 || inner.height == 0 {
            return false;
        }

        let scroll = self.ui.input_scroll_offset as u16;
        let local_line = position.y.saturating_sub(inner.y);
        let local_column = position.x.saturating_sub(inner.x);
        let target_line = scroll.saturating_add(local_line);

        self.ui
            .composer
            .set_cursor_at_visual_position(inner.width, target_line, local_column);
        // Start a new selection at the current cursor position
        self.ui.composer.start_selection();
        self.ui.input_dragging = true;
        self.clear_mouse_selection();
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.ui
            .command_palette
            .sync(self.ui.composer.text(), &self.ui.commands);
        true
    }

    /// Handle mouse drag in input area for text selection.
    fn handle_input_area_drag(&mut self, position: Position) -> bool {
        if !self.ui.input_dragging {
            return false;
        }

        let Some(inner) = self.ui.input_area.get() else {
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

        let scroll = self.ui.input_scroll_offset as u16;
        let local_line = clamped_y.saturating_sub(inner.y);
        let local_column = clamped_x.saturating_sub(inner.x);
        let target_line = scroll.saturating_add(local_line);

        self.ui
            .composer
            .set_cursor_at_visual_position(inner.width, target_line, local_column);
        // Selection anchor is already set, cursor movement extends selection
        self.refresh_at_mention_state();
        self.refresh_snippet_state();
        self.ui
            .command_palette
            .sync(self.ui.composer.text(), &self.ui.commands);
        true
    }

    /// Handle mouse up in input area - finalize selection and auto-copy.
    fn handle_input_area_mouse_up(&mut self, _position: Position) -> bool {
        if !self.ui.input_dragging {
            return false;
        }

        self.ui.input_dragging = false;

        // If there's a selection, auto-copy it to clipboard
        let selected_text = self.ui.composer.selected_text().map(|s| s.to_string());
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
                self.ui.selection_clipboard_lease = lease;
                self.ui.toast = Some((
                    "Selection copied to clipboard".to_string(),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
            Err(error) => {
                self.ui.toast = Some((
                    format!("Failed to copy selection: {error}"),
                    std::time::Instant::now() + std::time::Duration::from_secs(3),
                ));
            }
        }
    }

    /// Handle mouse scroll up in input area.
    fn handle_input_area_scroll_up(&mut self, position: Position) -> bool {
        let Some(inner) = self.ui.input_area.get() else {
            return false;
        };

        if !inner.contains(position) {
            return false;
        }

        if self.ui.input_scroll_offset > 0 {
            self.ui.input_scroll_offset -= 1;
        }
        true
    }

    /// Handle mouse scroll down in input area.
    fn handle_input_area_scroll_down(&mut self, position: Position) -> bool {
        let Some(inner) = self.ui.input_area.get() else {
            return false;
        };

        if !inner.contains(position) {
            return false;
        }

        let visible_lines = inner.height as usize;
        let total_lines = self.ui.composer.display_line_count(inner.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);

        if self.ui.input_scroll_offset < max_scroll {
            self.ui.input_scroll_offset += 1;
        }
        true
    }

    pub(crate) fn toggle_tool_result_expanded(&mut self, message_id: Uuid) {
        if self.ui.expanded_tool_results.contains(&message_id) {
            self.ui.expanded_tool_results.remove(&message_id);
        } else {
            self.ui.expanded_tool_results.insert(message_id);
        }
        self.clear_message_render_cache();
    }

    /// Attempts to navigate to a subagent's child session from a tool result message.
    /// Returns true if navigation was performed.
    fn try_navigate_to_subagent_subsession(&mut self, message_id: Uuid) -> bool {
        // Primary look-up: check the direct message_id → child_session_id map.
        // This is populated in record_tool_result for task tools.
        if let Some(&child_session_id) = self.ui.subagent_result_message_map.get(&message_id) {
            self.switch_session(child_session_id).ok();
            return true;
        }

        // Fallback: look up via tool_call_id.
        let tool_call_id = self.ui.chat_context
            .messages
            .iter()
            .find(|m| m.id == message_id)
            .and_then(|m| m.tool_call_id.as_deref())
            .map(|id| id.to_string());

        let Some(tool_call_id) = tool_call_id else {
            return false;
        };

        let Ok(tc_id) = uuid::Uuid::parse_str(&tool_call_id) else {
            return false;
        };

        if let Some(&child_session_id) = self.ui.subagent_task_map.get(&tc_id) {
            self.switch_session(child_session_id).ok();
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

        if !self.ui.mouse_selection.is_dragging() || !self.can_scroll_conversation() {
            return;
        }

        let Some(area) = self.ui.message_content_area else {
            return;
        };

        let Some(pointer) = self.ui.mouse_selection.pointer() else {
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
            self.ui.dirty = true;
            let speed = self.runtime.config().ui.scroll_speed as usize;
            self.scroll_messages_up_internal(speed);
        } else if pointer.y >= bottom_threshold {
            self.ui.dirty = true;
            let speed = self.runtime.config().ui.scroll_speed as usize;
            self.scroll_messages_down_internal(speed);
        }
    }

    /// Update input area scroll based on mouse drag position.
    /// Returns true if auto-scroll was performed.
    fn update_input_area_auto_scroll(&mut self) -> bool {
        if !self.ui.input_dragging {
            return false;
        }

        let Some(inner) = self.ui.input_area.get() else {
            return false;
        };

        if inner.width == 0 || inner.height == 0 {
            return false;
        }

        let top_threshold = inner.y;
        let bottom_threshold = inner.y.saturating_add(inner.height.saturating_sub(1));

        // Auto-scroll up when cursor is above the input area
        if self.ui.mouse_selection
            .pointer()
            .is_some_and(|p| p.y < top_threshold)
            && self.ui.input_scroll_offset > 0
        {
            self.ui.input_scroll_offset -= 1;
            return true;
        }

        // Auto-scroll down when cursor is below the input area
        let visible_lines = inner.height as usize;
        let total_lines = self.ui.composer.display_line_count(inner.width as usize);
        let max_scroll = total_lines.saturating_sub(visible_lines);
        if self.ui.mouse_selection
            .pointer()
            .is_some_and(|p| p.y > bottom_threshold)
            && self.ui.input_scroll_offset < max_scroll
        {
            self.ui.input_scroll_offset += 1;
            return true;
        }

        false
    }
}
