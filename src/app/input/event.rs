use super::*;
use crate::session::MessageRole;

impl App {
    pub(crate) fn handle_event(&mut self, event: Event, runtime: &Runtime) -> Result<()> {
        match event {
            Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.handle_key_event(key, runtime)?;
            }
            Event::Paste(text) => {
                self.handle_text_paste(&text)?;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
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

    pub(crate) fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);
                if self.handle_input_area_mouse_down(position) {
                    return;
                }
                if let Some(bounds) = self.selection_bounds_for_position(position) {
                    self.mouse_selection
                        .press_with_bounds(position, Some(bounds));
                } else {
                    self.clear_mouse_selection();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.mouse_selection
                    .drag(Position::new(mouse.column, mouse.row));
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let position = Position::new(mouse.column, mouse.row);

                if !self.mouse_selection.is_dragging() {
                    for (message_id, rect) in &self.tool_result_card_bounds {
                        if rect.contains(position) {
                            self.toggle_tool_result_expanded(*message_id);
                            return;
                        }
                    }
                }

                self.mouse_selection.release(position);
            }
            MouseEventKind::ScrollUp if self.can_scroll_conversation() => {
                self.clear_mouse_selection();
                self.scroll_messages_up(3);
            }
            MouseEventKind::ScrollDown if self.can_scroll_conversation() => {
                self.clear_mouse_selection();
                self.scroll_messages_down(3);
            }
            _ => {}
        }
    }

    fn handle_input_area_mouse_down(&mut self, position: Position) -> bool {
        let Some(inner) = self.input_area.get() else {
            return false;
        };

        if !inner.contains(position) || inner.width == 0 || inner.height == 0 {
            return false;
        }

        let visible_lines = inner.height.max(1) as usize;
        let total_lines = self.composer.display_line_count(inner.width as usize);
        let scroll = total_lines.saturating_sub(visible_lines) as u16;
        let local_line = position.y.saturating_sub(inner.y);
        let local_column = position.x.saturating_sub(inner.x);
        let target_line = scroll.saturating_add(local_line);

        self.composer
            .set_cursor_at_visual_position(inner.width, target_line, local_column);
        self.clear_mouse_selection();
        self.refresh_at_mention_state();
        self.command_palette
            .sync(self.composer.text(), &self.commands);
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

    pub(crate) fn update_mouse_selection_auto_scroll(&mut self) {
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
            self.scroll_messages_up_internal(3);
        } else if pointer.y >= bottom_threshold {
            self.scroll_messages_down_internal(3);
        }
    }

    pub(crate) fn clear_mouse_selection(&mut self) {
        self.mouse_selection.clear();
    }

    pub(crate) fn selection_bounds_for_position(&self, position: Position) -> Option<Rect> {
        if let Some(area) = self.message_content_area
            && area.contains(position)
        {
            return Some(area);
        }

        if let Some(area) = self.sidebar_area
            && area.contains(position)
        {
            return Some(area);
        }

        None
    }

    pub(crate) fn register_selection_region(&self, _area: Rect) {}

    pub(crate) fn can_scroll_conversation(&self) -> bool {
        self.screen == Screen::Chat
            && self.permission_dialog.is_none()
            && self.connect_dialog.is_none()
            && self.theme_panel.is_none()
            && self.model_panel.is_none()
            && self.mcp_panel.is_none()
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
        self.cancel_running_subagents();

        for running in self.running_tool_executions.drain(..) {
            running.cancel_requested.store(true, Ordering::SeqCst);
        }

        if let Some(message) = self.conversation.messages.last_mut()
            && message.streaming
            && matches!(message.role, MessageRole::Assistant)
        {
            message.role = MessageRole::Error;
            message.streaming = false;
            message.content = "Request cancelled".to_string();
            let persisted = message.clone();
            let _ = self
                .store
                .append_message(self.conversation.session_id, &persisted);
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
                    panel.toggle_selected();
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
        let Some(panel) = self.model_panel.clone() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Up => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let items = self.model_panel_items();
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.model_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.switch_model(Some(&summary.label()))?;
                    self.close_model_panel();
                }
            }
            KeyCode::Esc => {
                self.close_model_panel();
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let items = self.model_panel_items();
                if let Some(summary) = panel.selected_model(&items).cloned() {
                    self.close_model_panel();
                    self.begin_provider_edit_for_model(summary.provider_id, summary.model_id)?;
                }
            }
            KeyCode::Tab => {}
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    let items = self.model_panel_items();
                    let mut next_panel = panel;
                    next_panel.reset_selection(
                        &items,
                        Some((&self.active_model.provider_id, &self.active_model.model_id)),
                    );
                    self.model_panel = Some(next_panel);
                }
            }
        }

        Ok(())
    }

    pub(crate) fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        if self.leader_key_pending {
            self.leader_key_pending = false;
            let _ = self.handle_leader_key(key, runtime)?;
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('x')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.leader_key_pending = true;
            self.last_notice =
                Some("Leader key active: use arrows to navigate subagents".to_string());
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

        if self.question_dialog.is_some() {
            return self.handle_question_dialog_key(key, runtime);
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

        if self.session_panel.is_some() {
            return self.handle_session_panel_key(key, runtime);
        }

        if self.stats_panel.as_ref().is_some_and(|p| p.active) {
            return self.handle_stats_panel_key(key);
        }

        if self.handle_request_abort_key(key, runtime)? {
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('v'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT)
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            self.handle_clipboard_paste()?;
            return Ok(());
        }

        if !self.command_palette.visible && key.code == KeyCode::Tab {
            self.mode = self.mode.toggle();
            self.refresh_tools();
            self.last_notice = Some(format!("Mode switched to {}", self.mode.as_str()));
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

        if self.at_mention.visible {
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

        if matches!(key.code, KeyCode::Up | KeyCode::Down) {
            let Some(input_area) = self.input_area.get() else {
                return Ok(());
            };

            let input_width = input_area.width;
            match key.code {
                KeyCode::Up => self.composer.move_up(input_width),
                KeyCode::Down => self.composer.move_down(input_width),
                _ => {}
            }

            self.refresh_at_mention_state();
            self.command_palette
                .sync(self.composer.text(), &self.commands);
            return Ok(());
        }

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            self.handle_submission(submission, runtime)?;
            self.at_mention.clear();
        } else {
            if key.code == KeyCode::Enter && !self.draft_attachments.is_empty() {
                self.handle_submission(String::new(), runtime)?;
                self.at_mention.clear();
                self.command_palette
                    .sync(self.composer.text(), &self.commands);
                return Ok(());
            }
            self.refresh_at_mention_state();
        }

        self.command_palette
            .sync(self.composer.text(), &self.commands);
        Ok(())
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

    pub(crate) fn handle_text_paste(&mut self, text: &str) -> Result<()> {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        self.composer.insert_str(&normalized);
        self.refresh_at_mention_state();
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
        self.command_palette
            .sync(self.composer.text(), &self.commands);
    }

    pub(crate) fn execute_command_line(&mut self, line: &str, runtime: &Runtime) -> Result<()> {
        let Some((name, args)) = self.commands.parse_invocation(line) else {
            self.last_notice = Some("Invalid command".to_string());
            return Ok(());
        };

        let Some(spec) = self.commands.command(&name).cloned() else {
            self.last_notice = Some(format!("Unknown command '/{name}'"));
            return Ok(());
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
                | CommandAction::Clear => {}
                _ => {
                    self.last_notice = Some(
                        "A response is still streaming. Wait for it to finish before changing sessions.".to_string(),
                    );
                    return Ok(());
                }
            }
        }

        match action {
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
        self.theme_panel = Some(ThemePanelState::new(self.theme.palette().name));
    }

    pub(crate) fn open_settings_panel(&mut self) {
        self.mcp_panel = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.settings_panel = Some(SettingsPanelState::new(&self.config));
    }

    pub(crate) fn open_model_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.mcp_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search connected models by provider or model name");
        self.composer.set_text(initial_query);

        let mut panel = ModelPanelState::new();
        let items = self.model_panel_items();
        panel.reset_selection(
            &items,
            Some((&self.active_model.provider_id, &self.active_model.model_id)),
        );
        self.model_panel = Some(panel);
    }

    pub(crate) fn close_model_panel(&mut self) {
        self.model_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();
        self.composer.clear();
        self.composer
            .set_placeholder("Ask TiDev about your code, task, or question...");
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
}
