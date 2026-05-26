use super::*;

use crate::App;
use tidev_session::session::{BackendEvent, Message, MessageRole};

impl App {
    pub(crate) fn handle_key_event(&mut self, key: KeyEvent, runtime: &Runtime) -> Result<()> {
        log::debug!(
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
            self.last_notice = Some(
                "Up: parent session, Down/Left/Right: switch subagent, e: external editor"
                    .to_string(),
            );
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

        if self.sandbox_elevation.is_some() {
            return self.handle_sandbox_elevation_key(key);
        }

        if self.workspace_boundary_confirm_dialog.is_some() {
            return self.handle_workspace_boundary_confirm_dialog_key(key, runtime);
        }

        if self.workspace_boundary_dialog.is_some() {
            return self.handle_workspace_boundary_dialog_key(key, runtime);
        }

        if self.sensitive_file_dialog.is_some() {
            return self.handle_sensitive_file_dialog_key(key, runtime);
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

        // Panel launcher overlay takes priority over individual panels
        if self.panel_launcher.visible {
            return self.handle_panel_launcher_key(key, runtime);
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

        if self.sandbox_panel.is_some() {
            return self.handle_sandbox_panel_key(key);
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

        if self.search_panel.is_some() {
            return self.handle_search_panel_key(key);
        }

        if self.message_panel.is_some() {
            return self.handle_message_panel_key(key);
        }

        if self.sync_panel.is_some() {
            return self.handle_sync_panel_key(key, runtime);
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

        // Ctrl+P: 打开面板启动器（命令面板）
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.open_panel_launcher();
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
            } else if self.pending_request || !self.pending_prompt_queue.is_empty() {
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
            if let Err(e) = self.store.save_model_thinking_level(
                &self.active_model.provider_id,
                &self.active_model.model_id,
                &self.thinking_level.to_string(),
            ) {
                log::warn!("failed to save thinking level preference: {}", e);
            }
            return Ok(());
        }

        if matches!(key.code, KeyCode::Char('t')) && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.thinking_level = self.thinking_level.next();
            self.last_notice = Some(format!("Thinking: {}", self.thinking_level.display_name()));
            if let Err(e) = self.store.save_model_thinking_level(
                &self.active_model.provider_id,
                &self.active_model.model_id,
                &self.thinking_level.to_string(),
            ) {
                log::warn!("failed to save thinking level preference: {}", e);
            }
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
            KeyCode::Char('e') => {
                self.open_external_editor()?;
                return Ok(true);
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

    pub(crate) fn execute_shell_command(&mut self, command: &str, runtime: &Runtime) -> Result<()> {
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
                content = String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .to_string();
                if content.is_empty() {
                    content = String::from_utf8_lossy(&output.stderr)
                        .trim_end()
                        .to_string();
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

    pub(crate) fn open_external_editor(&mut self) -> Result<()> {
        let text = self.composer.text().to_string();
        if text.is_empty() {
            self.last_notice = Some("No text to edit".to_string());
            return Ok(());
        }

        let Some((cmd, mut args)) = crate::input::editor::resolve_editor(&self.config.ui) else {
            self.last_notice = Some(
                "No editor found. Set external_editor in config, $VISUAL, or $EDITOR.".to_string(),
            );
            return Ok(());
        };

        // Create a unique temp file (auto-cleaned on drop)
        let edit_file = match crate::input::editor::TempEditFile::create(&text) {
            Ok(f) => f,
            Err(e) => {
                self.last_notice = Some(format!("Failed to create temp file: {e}"));
                return Ok(());
            }
        };

        self.last_notice = Some(format!("Opening in {cmd}... Save and close to continue."));

        // Suspend the TUI so the editor can take over the terminal cleanly
        if let Some(session) = &self.terminal_session
            && let Err(e) = session.suspend()
        {
            self.last_notice = Some(format!("Failed to suspend TUI: {e}"));
            return Ok(());
        }

        // Spawn editor and wait for it to close
        args.push(edit_file.path().to_string_lossy().to_string());
        let status = std::process::Command::new(&cmd).args(&args).status();

        // Resume the TUI after editor exits
        if let Some(session) = &self.terminal_session
            && let Err(e) = session.resume()
        {
            self.last_notice = Some(format!("Failed to resume TUI: {e}"));
            return Ok(());
        }

        // Mark for full redraw — after alternate screen was left and
        // re-entered, ratatui's frame buffer is stale and won't redraw.
        self.force_full_redraw = true;

        // Read back content (even if editor exited with error — user may have saved)
        let edited = match edit_file.read() {
            Ok(c) => c,
            Err(e) => {
                self.last_notice = Some(format!("Failed to read edited file: {e}"));
                return Ok(());
            }
        };

        match status {
            Ok(s) if !s.success() => {
                self.last_notice = Some(format!("Editor exited with status {:?}", s.code()));
            }
            Err(e) => {
                self.last_notice = Some(format!("Failed to launch editor: {e}"));
                return Ok(());
            }
            _ => {}
        }

        // Most editors add a trailing newline when saving. Trim a single one
        // so the comparison against the original text is meaningful.
        let edited = edited.strip_suffix('\n').unwrap_or(&edited).to_string();

        if edited != text {
            self.composer.set_text(edited);
            self.last_notice = Some("Content updated from editor".to_string());
        }

        Ok(())
    }
}

/// Determine the shell command and argument for each platform.
fn shell_command() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("powershell", "-Command")
    } else {
        ("sh", "-c")
    }
}
