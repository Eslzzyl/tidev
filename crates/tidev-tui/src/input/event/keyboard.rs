use crate::App;

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

        // Image viewer overlay: any key closes it
        if self.image_viewer.is_some() {
            self.image_viewer = None;
            self.dirty = true;
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

        // Allow message scrolling even when security dialogs are shown.
        // This must come before the dialog dispatch so PageUp/PageDown
        // reach the scroll handler instead of being consumed by dialogs.
        if !self.command_palette.visible && self.handle_message_scroll_key(key) {
            return Ok(());
        }

        if self.permission_dialog.is_some() {
            return self.handle_permission_dialog_key(key, runtime);
        }

        if self.workspace_boundary_confirm_dialog.is_some() {
            return self.handle_workspace_boundary_confirm_dialog_key(key, runtime);
        }

        if self.workspace_boundary_dialog.is_some() {
            return self.handle_workspace_boundary_dialog_key(key, runtime);
        }

        if self.sensitive_file_confirm_dialog.is_some() {
            return self.handle_sensitive_file_confirm_dialog_key(key, runtime);
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

        if self.session_panel.is_some() {
            return self.handle_session_panel_key(key, runtime);
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

        if self.at_mention.visible {
            log::info!(
                "handle_key_event: at_mention visible, suggestions={}, key={:?}",
                self.at_mention.suggestions.len(),
                key.code,
            );
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
                    log::info!("handle_key_event: accepting @mention suggestion");
                    self.accept_at_mention();
                    return Ok(());
                }
                _ => {}
            }
        }

        // (shell completion removed — shell mode is not included in this rewrite)

        // In shell mode, Tab triggers command completion
        if !self.command_palette.visible && key.code == KeyCode::Tab {            if self.pending_mode.is_some() {
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

        if let Some(submission) = self.composer.handle_key_with_history(key, true) {
            log::info!(
                "handle_key_event: composer returned submission = {:?}, calling handle_submission",
                submission,
            );
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
            KeyCode::Down => {
                // Down always navigates to the last (most recently delegated) child.
                let children = self.store.load_child_sessions(parent_session_id)?;
                if children.is_empty() {
                    return Ok(false);
                }
                if let Some(target) = children.last() {
                    self.switch_session(target.session_id, runtime)?;
                    return Ok(true);
                }
            }
            KeyCode::Right | KeyCode::Left => {
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
        log::info!("handle_submission: ENTER submission={:?}", submission);

        let trimmed = submission.trim();
        if trimmed.starts_with('/') {
            log::info!("handle_submission: is command");
            self.execute_command_line(trimmed, runtime)?;
            self.at_mention.clear();
            self.draft_attachments.clear();
        } else {
            log::info!("handle_submission: is prompt, calling submit_prompt");
            self.submit_prompt(submission, runtime)?;
        }

        Ok(())
    }

    fn cleanup_shell_child(&mut self) {}

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
        log::debug!("handle_clipboard_paste: entering");
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(error) => {
                log::debug!("handle_clipboard_paste: clipboard unavailable: {error}");
                self.last_notice = Some(format!("Clipboard unavailable: {error}"));
                return Ok(());
            }
        };

        match clipboard.get_text() {
            Ok(text) if !text.is_empty() => {
                log::debug!(
                    "handle_clipboard_paste: got text ({} bytes), pasting as text",
                    text.len()
                );
                return self.handle_text_paste(&text);
            }
            Ok(text) => {
                log::debug!(
                    "handle_clipboard_paste: got empty text ({} bytes), trying image",
                    text.len()
                );
            }
            Err(e) => {
                log::debug!("handle_clipboard_paste: get_text failed: {e}, trying image");
            }
        }

        let image = match clipboard.get_image() {
            Ok(image) => {
                log::debug!(
                    "handle_clipboard_paste: got image {}x{}",
                    image.width,
                    image.height
                );
                image
            }
            Err(e) => {
                log::debug!("handle_clipboard_paste: get_image failed: {e}");
                // On WSL2, arboard cannot read image data from the Windows
                // clipboard.  Try the PowerShell fallback.
                match wsl_clipboard_image() {
                    Some(img) => {
                        log::debug!(
                            "handle_clipboard_paste: WSL fallback got image {}x{}",
                            img.width,
                            img.height
                        );
                        img
                    }
                    None => {
                        self.last_notice =
                            Some("Clipboard does not contain pasteable text or image".to_string());
                        return Ok(());
                    }
                }
            }
        };

        if !self.active_model.supports_images {
            log::debug!(
                "handle_clipboard_paste: model {} does not support images",
                self.active_model.model_id
            );
            self.last_notice = Some("This model does not support image attachments".to_string());
            return Ok(());
        }

        let data_url = match png_data_url_from_clipboard_image(image) {
            Ok(value) => value,
            Err(error) => {
                log::debug!("handle_clipboard_paste: image decode failed: {error}");
                self.last_notice = Some(format!("Failed to decode clipboard image: {error}"));
                return Ok(());
            }
        };

        let file_size = data_url
            .find("base64,")
            .map(|i| {
                let b64 = &data_url[i + 7..];
                let decoded_len = (b64.len() as u64).saturating_mul(3) / 4;
                let padding = b64.bytes().rev().take(2).filter(|&b| b == b'=').count() as u64;
                decoded_len.saturating_sub(padding)
            })
            .unwrap_or(0);

        let badge = format_image_badge("image/png", file_size);
        let data_url_for_span = data_url.clone();
        let span_start = self.composer.cursor();
        self.composer.insert_str(&badge);
        let span_end = self.composer.cursor();
        self.composer
            .register_span(span_start, span_end, badge, InlineSpanKind::Image, Some(data_url_for_span));
        self.draft_attachments.push(MessageAttachment::Image {
            filename: format!("pasted-image-{}.png", Uuid::new_v4()),
            mime: "image/png".to_string(),
            data_url,
            file_size,
        });
        log::debug!("handle_clipboard_paste: image pasted successfully ({file_size} bytes)");
        self.last_notice = Some("Image pasted into draft".to_string());
        Ok(())
    }

    pub(crate) fn open_external_editor(&mut self) -> Result<()> {
        let text = self.composer.text().to_string();
        if text.is_empty() {
            self.last_notice = Some("No text to edit".to_string());
            return Ok(());
        }

        let Some((cmd, mut args)) =
            crate::input::editor::resolve_editor(&self.config.read().unwrap().ui)
        else {
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

/// Guard that restores terminal settings on drop.
///
/// Saves the current termios for a given fd (typically stdin = fd 0) and
/// restores it when the guard goes out of scope. This ensures that even
/// if a `!` shell command corrupts terminal settings (e.g. via `/dev/tty`),
/// the TUI's raw mode is restored after the command completes or is cancelled.
#[cfg(unix)]
struct TermiosGuard {
    saved: libc::termios,
    fd: std::os::unix::io::RawFd,
}

#[cfg(unix)]
impl TermiosGuard {
    fn save(fd: std::os::unix::io::RawFd) -> Self {
        let mut saved: libc::termios = unsafe { std::mem::zeroed() };
        // Best-effort: if tcgetattr fails, saved is zeroed which will
        // likely produce a no-op or safe restore on tcsetattr.
        unsafe {
            libc::tcgetattr(fd, &mut saved);
        }
        Self { saved, fd }
    }
}

#[cfg(unix)]
impl Drop for TermiosGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
        }
    }
}

/// No-op guard on non-Unix platforms.
#[cfg(not(unix))]
struct TermiosGuard;

#[cfg(not(unix))]
impl TermiosGuard {
    fn save(_fd: i32) -> Self {
        Self
    }
}

/// Format an image badge string like `[100.0 KB PNG]` for display in the composer.
pub(crate) fn format_image_badge(mime: &str, file_size: u64) -> String {
    let type_label = mime
        .strip_prefix("image/")
        .unwrap_or(mime)
        .to_uppercase();
    let size_str = crate::render::chat_render::tool::format_file_size(file_size);
    format!("[{} {}]", size_str, type_label)
}

// ---------------------------------------------------------------------------
// WSL clipboard image fallback
// ---------------------------------------------------------------------------

/// On WSL2, `arboard::Clipboard::get_image()` often fails because the X11
/// clipboard bridge (WSLg / RDP) does not expose image data in `image/png`
/// format.  This function works around the limitation by asking the Windows
/// host to save the clipboard image via PowerShell, then reading the
/// resulting PNG file back into an `arboard::ImageData`.
#[cfg(target_os = "linux")]
pub(crate) fn wsl_clipboard_image() -> Option<arboard::ImageData<'static>> {
    use super::super::mouse_selection::is_probably_wsl;
    if !is_probably_wsl() {
        return None;
    }

    log::debug!("wsl_clipboard_image: attempting PowerShell fallback");

    // PowerShell script that saves the clipboard image to a temp PNG and
    // prints the Windows path.  UTF-8 output is forced to avoid encoding
    // mismatches between powershell.exe (UTF-16LE) and pwsh (UTF-8).
    let script = r#"[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
$img = Get-Clipboard -Format Image; \
if ($img -ne $null) { \
  $p=[System.IO.Path]::GetTempFileName(); \
  $p=[System.IO.Path]::ChangeExtension($p,'png'); \
  $img.Save($p,[System.Drawing.Imaging.ImageFormat]::Png); \
  Write-Output $p \
} else { exit 1 }"#;

    let win_path = try_powershell_command(script)?;
    log::debug!("wsl_clipboard_image: PowerShell saved to {win_path}");

    let wsl_path = windows_path_to_wsl(&win_path)?;
    log::debug!("wsl_clipboard_image: mapped to {}", wsl_path.display());

    let img = image::open(&wsl_path).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();

    log::debug!("wsl_clipboard_image: decoded {w}x{h} RGBA image");

    Some(arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: std::borrow::Cow::Owned(rgba.into_raw()),
    })
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn wsl_clipboard_image() -> Option<arboard::ImageData<'static>> {
    None
}

/// Try to execute a PowerShell script that saves the clipboard image and
/// returns the Windows path to the temp file.  Tries `powershell.exe`,
/// `pwsh`, and `powershell` in order.
#[cfg(target_os = "linux")]
fn try_powershell_command(script: &str) -> Option<String> {
    for cmd in ["powershell.exe", "pwsh", "powershell"] {
        match std::process::Command::new(cmd)
            .args(["-NoProfile", "-Command", script])
            .output()
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    log::debug!("wsl_clipboard_image: {cmd} succeeded");
                    return Some(path);
                }
                log::debug!("wsl_clipboard_image: {cmd} returned empty path");
            }
            Ok(output) => {
                log::debug!(
                    "wsl_clipboard_image: {cmd} failed with status {}",
                    output.status
                );
            }
            Err(e) => {
                log::debug!("wsl_clipboard_image: {cmd} not executable: {e}");
            }
        }
    }
    None
}

/// Convert a Windows path like `C:\Users\...\tmp.png` to a WSL path
/// like `/mnt/c/Users/.../tmp.png`.
#[cfg(target_os = "linux")]
fn windows_path_to_wsl(input: &str) -> Option<std::path::PathBuf> {
    let drive = input.chars().next()?.to_ascii_lowercase();
    if !drive.is_ascii_lowercase() || input.get(1..2) != Some(":") {
        return None;
    }
    let mut result = std::path::PathBuf::from(format!("/mnt/{drive}"));
    for component in input
        .get(2..)?
        .trim_start_matches(['\\', '/'])
        .split(['\\', '/'])
        .filter(|c| !c.is_empty())
    {
        result.push(component);
    }
    Some(result)
}
