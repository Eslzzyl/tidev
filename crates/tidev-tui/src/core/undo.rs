use anyhow::Result;
use tidev_session::session::MessageRole;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::render::chat_render::strip_system_reminder_tags;
use tidev_snapshot::FileDiff;
use tidev_context::ContextManager;
use tidev_snapshot::{StepPatch, Patch};

use super::{App, BackendEvent, Screen};

impl App {
    /// Finalize snapshot for the last user message by using cached per-step patches
    /// and dispatching async diff_full for sidebar display.
    pub(crate) fn finalize_snapshot_for_last_user_message_sync(
        &mut self,
        runtime: &Runtime,
    ) -> Result<()> {
        log::info!("finalize_snapshot: starting");

        let last_user_message_id = {
            let Some(last_user_message) = self.conversation.last_visible_user_message() else {
                log::info!("finalize_snapshot: no visible user message");
                return Ok(());
            };

            let Some(hash) = last_user_message.snapshot_hash.clone() else {
                log::info!("finalize_snapshot: message has no snapshot_hash");
                return Ok(());
            };

            log::info!(
                "finalize_snapshot: message id={}, snapshot_hash={}",
                last_user_message.id,
                hash
            );
            last_user_message.id
        };

        let initial_hash = {
            let Some(msg) = self
                .conversation
                .messages
                .iter()
                .find(|m| m.id == last_user_message_id)
            else {
                log::warn!("finalize_snapshot: message not found in messages list");
                return Ok(());
            };
            match msg.snapshot_hash.clone() {
                Some(h) => h,
                None => {
                    log::info!("finalize_snapshot: snapshot_hash is None");
                    return Ok(());
                }
            }
        };

        // Collect all snapshot hashes and cached file lists
        let step_hashes: Vec<String> = self.step_snapshot_hashes.drain(..).collect();
        let cached_file_lists: Vec<Vec<String>> = self.step_cached_file_lists.drain(..).collect();

        log::info!(
            "finalize_snapshot: {} cached step patches, {} step hashes",
            cached_file_lists.len(),
            step_hashes.len()
        );

        // Build step patches from cached data — avoids re-running expensive patch() calls
        let mut step_patches: Vec<StepPatch> = Vec::new();
        if !cached_file_lists.is_empty() {
            for (i, files) in cached_file_lists.iter().enumerate() {
                if !files.is_empty() {
                    let hash = step_hashes.get(i).cloned().unwrap_or_default();
                    log::info!(
                        "finalize_snapshot: cached step {} hash={} files={}",
                        i + 1,
                        hash,
                        files.len()
                    );
                    step_patches.push(StepPatch {
                        hash,
                        files: files.clone(),
                        step: i + 1,
                    });
                }
            }
        } else {
            // Fallback: no cached patches. Compute patch for the initial hash
            // to capture changes made since the initial snapshot (e.g., direct file edits).
            log::info!("finalize_snapshot: no cached patches, computing from snapshot");
            match runtime.block_on(self.snapshot.patch(&initial_hash)) {
                Ok(patch) => {
                    if !patch.files.is_empty() {
                        log::info!(
                            "finalize_snapshot: fallback patch: {} files",
                            patch.files.len()
                        );
                        step_patches.push(StepPatch {
                            hash: initial_hash.clone(),
                            files: patch.files,
                            step: 0,
                        });
                    }
                }
                Err(e) => {
                    log::warn!("finalize_snapshot: fallback patch failed: {}", e);
                }
            }
        }

        if !step_patches.is_empty() {
            let patch_files_json = serde_json::to_string(&step_patches)?;
            log::info!(
                "finalize_snapshot: saving patch_files, steps={}",
                step_patches.len()
            );
            self.store.update_message_patch(
                self.conversation.session_id,
                last_user_message_id,
                &patch_files_json,
            )?;

            if let Some(msg) = self
                .conversation
                .messages
                .iter_mut()
                .find(|m| m.id == last_user_message_id)
            {
                msg.patch_files = Some(patch_files_json);
            }

            // Dispatch async diff_full for accurate sidebar display with full patches.
            // Skip if from and to are the same hash — this happens when
            // finalize_snapshot is called a second time (from finish_assistant_turn)
            // after step data was already consumed by ToolCompleted.  dispatching
            // diff_full(same, same) would return an empty list and overwrite the
            // previously stored file_diffs with nothing.
            let final_hash = step_hashes.last().cloned().unwrap_or(initial_hash.clone());
            if final_hash != initial_hash {
                self.dispatch_async_diff_full(
                    runtime,
                    initial_hash.clone(),
                    final_hash,
                    last_user_message_id,
                );
            }
        }

        // Clear per-step tracking state
        self.step_cached_file_diffs = None;
        self.step_prev_hash = None;

        log::info!("finalize_snapshot: completed (async diff_full dispatched)");
        Ok(())
    }

    /// Dispatch diff_full computation to a background async task.
    /// Result is delivered via BackendEvent::SidebarSnapshotReady.
    fn dispatch_async_diff_full(
        &self,
        runtime: &Runtime,
        from: String,
        to: String,
        message_id: Uuid,
    ) {
        let snapshot = self.snapshot.clone();
        let tx = self.backend_tx.clone();
        let _session_id = self.conversation.session_id;
        let request_id = self.active_request_id;

        runtime.spawn(async move {
            log::info!("async diff_full: starting from={} to={}", from, to);
            match snapshot.diff_full(&from, &to).await {
                Ok(file_diffs) => match serde_json::to_string(&file_diffs) {
                    Ok(diffs_json) => {
                        log::info!("async diff_full: completed, {} files", file_diffs.len());
                        let _ = tx.send(BackendEvent::SidebarSnapshotReady {
                            request_id,
                            message_id,
                            file_diffs_json: diffs_json,
                        });
                    }
                    Err(e) => {
                        log::warn!("async diff_full: serialization failed: {}", e);
                    }
                },
                Err(e) => {
                    log::warn!("async diff_full: failed: {}", e);
                }
            }
        });
    }

    pub(crate) fn undo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        log::info!("undo_last_user_message: starting");

        if self.pending_request {
            self.abort_current_request();
        }

        let message = if let Some(current_revert) = self.conversation.revert_message_id {
            log::info!(
                "undo_last_user_message: already in revert state, looking for prev user message before {}",
                current_revert
            );
            self.conversation.prev_user_message_before(current_revert)
        } else {
            log::info!(
                "undo_last_user_message: not in revert state, looking for last visible user message"
            );
            self.conversation.last_visible_user_message()
        };

        let Some(message) = message else {
            log::info!("undo_last_user_message: no user message found");
            self.last_notice = Some("No earlier user message to undo".to_string());
            return Ok(());
        };

        let message = message.clone();
        log::info!(
            "undo_last_user_message: found message id={}, content_len={}",
            message.id,
            message.content.len()
        );

        self.restored_attachments = message.attachments.clone();
        self.revert_to_message(message.id, message.content.clone(), runtime)?;
        self.last_notice = Some("Undid previous user message".to_string());
        log::info!("undo_last_user_message: completed successfully");
        Ok(())
    }

    pub(crate) fn redo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        log::info!("redo_last_user_message: starting");

        if self.pending_request {
            self.abort_current_request();
        }

        let Some(current_revert) = self.conversation.revert_message_id else {
            log::info!("redo_last_user_message: not in revert state");
            self.last_notice = Some("Nothing to redo".to_string());
            return Ok(());
        };

        log::info!(
            "redo_last_user_message: looking for next user message after {}",
            current_revert
        );

        if let Some(next_message) = self.conversation.next_user_message_after(current_revert) {
            log::info!(
                "redo_last_user_message: found next user message id={}",
                next_message.id
            );
            let message_id = next_message.id;
            let content = next_message.content.clone();
            self.revert_to_message(message_id, content, runtime)?;
            self.last_notice = Some("Redo complete".to_string());
        } else {
            log::info!("redo_last_user_message: no next user message, unreverting");
            self.unrevert(runtime)?;
            self.last_notice = Some("Redo complete".to_string());
        }

        log::info!("redo_last_user_message: completed successfully");
        Ok(())
    }

    pub(crate) fn revert_to_message(
        &mut self,
        message_id: Uuid,
        message_content: String,
        runtime: &Runtime,
    ) -> Result<()> {
        log::info!("revert_to_message: message_id={}", message_id);

        let patches = self.collect_patches_after_message(message_id)?;

        log::info!(
            "revert_to_message: patches.len()={}, revert_message_id={:?}",
            patches.len(),
            self.conversation.revert_message_id
        );

        let mut notice = None;

        let redo_snapshot = if let Some(existing) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            log::info!("revert_to_message: using existing redo_snapshot");
            existing
        } else {
            log::info!("revert_to_message: capturing new redo_snapshot");
            match self.snapshot.track() {
                Ok(Some(hash)) => {
                    log::info!("revert_to_message: captured redo_snapshot hash={}", hash);
                    hash
                }
                Ok(None) => {
                    log::info!("revert_to_message: track() returned None (no changes)");
                    String::new()
                }
                Err(error) => {
                    log::warn!("revert_to_message: track() failed: {}", error);
                    notice = Some(format!("Failed to capture redo snapshot: {error}"));
                    String::new()
                }
            }
        };

        if let Some(existing_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            log::info!("revert_to_message: restoring redo_snapshot");
            runtime.block_on(self.snapshot.restore(&existing_snapshot))?;
        }

        if !patches.is_empty() {
            log::info!("revert_to_message: reverting {} patches", patches.len());
            if let Err(error) = runtime.block_on(self.snapshot.revert(&patches)) {
                log::warn!("revert_to_message: revert failed: {}", error);
                notice = Some(format!("Revert partially failed: {error}"));
            }
        }

        log::info!("revert_to_message: setting revert_message_id and updating UI");
        self.command_palette.clear();

        // If the hidden range includes a compaction message, restore the
        // context state from before that compaction instead of resetting.
        let restored = self.restore_context_from_undo_compaction(message_id);

        if !restored {
            self.context_manager = ContextManager::new();
            self.conversation.clear_context_state();
        }

        // Persist context state to the database so it survives a session reload.
        if restored {
            if let Err(e) = self.store.update_session_context_state(
                self.conversation.session_id,
                self.conversation.context_summary.as_deref(),
                self.conversation.context_retained_from,
            ) {
                log::warn!("revert_to_message: failed to persist context state: {e}");
            }
        } else {
            if let Err(e) =
                self.store
                    .update_session_context_state(self.conversation.session_id, None, 0)
            {
                log::warn!("revert_to_message: failed to clear context state: {e}");
            }
        }

        self.set_revert_message_id(
            Some(message_id),
            if redo_snapshot.is_empty() {
                None
            } else {
                Some(&redo_snapshot)
            },
        )?;
        self.composer
            .set_text(strip_system_reminder_tags(&message_content));
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        if let Some(n) = notice {
            self.last_notice = Some(n);
        }
        Ok(())
    }

    fn unrevert(&mut self, runtime: &Runtime) -> Result<()> {
        log::info!("unrevert: starting");

        let Some(redo_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        else {
            log::info!("unrevert: no redo_snapshot found");
            self.clear_revert_state()?;
            return Ok(());
        };

        log::info!("unrevert: restoring redo_snapshot");
        if let Err(error) = runtime.block_on(self.snapshot.restore(&redo_snapshot)) {
            log::warn!("unrevert: restore failed: {}", error);
            self.last_notice = Some(format!("Redo failed: {error}"));
        }

        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        self.conversation.clear_context_state();
        if let Err(e) =
            self.store
                .update_session_context_state(self.conversation.session_id, None, 0)
        {
            log::warn!("unrevert: failed to clear context state: {e}");
        }
        self.composer.clear();
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        log::info!("unrevert: completed");
        Ok(())
    }

    /// Capture an intermediate snapshot between tool execution steps within a round.
    /// These are used at round end to compute per-step patches for undo/redo,
    /// and to provide lightweight sidebar diffs showing changed files in real-time.
    pub(crate) fn capture_step_snapshot(&mut self, runtime: &Runtime) {
        log::info!("capture_step_snapshot: starting");
        match self.snapshot.track() {
            Ok(Some(hash)) => {
                log::info!("capture_step_snapshot: captured hash={}", hash);
                self.step_snapshot_hashes.push(hash.clone());

                let prev_hash = self.step_prev_hash.clone().or_else(|| {
                    self.conversation
                        .last_visible_user_message()
                        .and_then(|m| m.snapshot_hash.clone())
                });

                if let Some(prev) = prev_hash {
                    match runtime.block_on(self.snapshot.diff_lightweight(&prev, &hash)) {
                        Ok(diffs) => {
                            let files: Vec<String> = diffs
                                .iter()
                                .map(|d| {
                                    self.workspace_root
                                        .join(&d.file)
                                        .to_string_lossy()
                                        .replace('\\', "/")
                                })
                                .collect();
                            self.step_cached_file_lists.push(files);
                            self.merge_step_diffs(diffs);
                        }
                        Err(e) => {
                            log::warn!("capture_step_snapshot: diff_lightweight failed: {}", e);
                            self.step_cached_file_lists.push(Vec::new());
                        }
                    }
                } else {
                    log::info!("capture_step_snapshot: no previous hash available");
                    self.step_cached_file_lists.push(Vec::new());
                }

                self.step_prev_hash = Some(hash);
            }
            Ok(None) => {
                log::info!("capture_step_snapshot: track() returned None (no changes)");
            }
            Err(error) => {
                log::warn!("capture_step_snapshot: track() failed: {}", error);
            }
        }
    }

    /// Merge lightweight per-step diffs into the running cumulative sidebar data.
    /// Updates step_cached_file_diffs and writes to the current user message's file_diffs.
    fn merge_step_diffs(&mut self, step_diffs: Vec<FileDiff>) {
        use std::collections::HashMap;

        let mut cumulative: HashMap<String, FileDiff> = HashMap::new();
        if let Some(existing) = self.step_cached_file_diffs.take() {
            for d in existing {
                cumulative.insert(d.file.clone(), d);
            }
        }

        for d in step_diffs {
            match cumulative.get_mut(&d.file) {
                Some(existing) => {
                    existing.additions = d.additions;
                    existing.deletions = d.deletions;
                    if existing.status.as_deref() != Some("added") {
                        existing.status = d.status;
                    }
                }
                None => {
                    cumulative.insert(d.file.clone(), d);
                }
            }
        }

        let merged: Vec<FileDiff> = cumulative.into_values().collect();
        self.step_cached_file_diffs = Some(merged.clone());

        let last_user_id = self
            .conversation
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, tidev_session::session::MessageRole::User))
            .map(|m| m.id);

        if let Some(msg_id) = last_user_id {
            let is_visible = self
                .conversation
                .message_index(msg_id)
                .map(|idx| {
                    self.conversation.revert_message_id.is_none_or(|revert_id| {
                        self.conversation
                            .message_index(revert_id)
                            .is_none_or(|revert_idx| idx < revert_idx)
                    })
                })
                .unwrap_or(false);

            if is_visible
                && let Some(msg) = self
                    .conversation
                    .messages
                    .iter_mut()
                    .find(|m| m.id == msg_id)
                && let Ok(json) = serde_json::to_string(&merged)
            {
                log::info!(
                    "merge_step_diffs: updating msg.file_diffs with {} files",
                    merged.len()
                );
                msg.file_diffs = Some(json.clone());
                // Persist immediately so changed files survive app restart
                if let Err(e) = self.store.update_message_file_diffs(
                    self.conversation.session_id,
                    msg_id,
                    &json,
                ) {
                    log::warn!("merge_step_diffs: failed to persist file_diffs: {}", e);
                }
                self.invalidate_active_message_render_cache_for(msg_id);
            }
        }
    }

    pub(crate) fn capture_prompt_snapshot(
        &mut self,
        message_id: Uuid,
        _runtime: &Runtime,
    ) -> Result<()> {
        log::info!("capture_prompt_snapshot: message_id={}", message_id);

        // Clear intermediate step tracking from any previous rounds
        self.step_snapshot_hashes.clear();
        self.step_cached_file_lists.clear();
        self.step_cached_file_diffs = None;

        // Seed step_prev_hash from the previous user message's snapshot
        // hash.  Load from the database (not the in-memory conversation)
        // because the previous round's background track may have updated
        // only the DB.  On the very first message of a brand-new session
        // there is no previous hash, so fall back to EMPTY_TREE_HASH.
        let prev_hash = self
            .store
            .load_messages(self.conversation.session_id)
            .ok()
            .and_then(|msgs| {
                msgs.iter()
                    .rev()
                    .find(|m| {
                        matches!(m.role, MessageRole::User) && m.id != message_id
                    })
                    .and_then(|m| m.snapshot_hash.clone())
            })
            .unwrap_or_else(|| tidev_snapshot::EMPTY_TREE_HASH.to_string());
        self.step_prev_hash = Some(prev_hash);

        // Synchronous track — ensures snapshot_hash is available in
        // memory before the agent loop or finalize_snapshot needs it.
        match self.snapshot.track() {
            Ok(Some(hash)) => {
                log::info!("capture_prompt_snapshot: captured hash={}", hash);
                self.store.update_message_snapshot(
                    self.conversation.session_id,
                    message_id,
                    &hash,
                )?;

                if let Some(msg) = self
                    .conversation
                    .messages
                    .iter_mut()
                    .find(|m| m.id == message_id)
                {
                    msg.snapshot_hash = Some(hash.clone());
                }

                // Use the freshly captured hash as the step-tracking
                // baseline so subsequent capture_step_snapshot calls diff
                // against the real pre-round state.
                self.step_prev_hash = Some(hash);
            }
            Ok(None) => {
                log::info!(
                    "capture_prompt_snapshot: track() returned None (not a git repo or no changes)"
                );
            }
            Err(error) => {
                log::warn!("capture_prompt_snapshot: track() failed: {}", error);
            }
        }

        Ok(())
    }

    pub(crate) fn discard_reverted_branch(&mut self) -> Result<()> {
        if !self.conversation.is_reverted() {
            return Ok(());
        }

        let visible_count = self.conversation.visible_message_count();
        let hidden_messages = self.conversation.messages[visible_count..].to_vec();

        self.store.delete_messages(
            self.conversation.session_id,
            &hidden_messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
        )?;

        // Clear instruction-source tracking so the next user message gets
        // a fresh injection.  The deleted messages (which may have been the
        // ones that received the instruction injection) are gone, so the
        // tracking table would otherwise prevent re-injection.
        match self
            .store
            .clear_instruction_sources(self.conversation.session_id)
        {
            Ok(_) => {}
            Err(e) => {
                log::warn!("discard_reverted_branch: failed to clear instruction sources: {e}");
            }
        }

        let _ = self.conversation.take_hidden_messages();
        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        self.conversation.clear_context_state();
        if let Err(e) =
            self.store
                .update_session_context_state(self.conversation.session_id, None, 0)
        {
            log::warn!("discard_reverted_branch: failed to clear context state: {e}");
        }
        Ok(())
    }

    fn collect_patches_after_message(&self, message_id: Uuid) -> Result<Vec<Patch>> {
        log::info!("collect_patches: looking for message_id={}", message_id);
        let patches = tidev_snapshot::collect_patches_after_message(
            &self.conversation.messages,
            message_id,
        )?;
        log::info!("collect_patches: returning {} patches", patches.len());
        Ok(patches)
    }

    fn set_revert_message_id(
        &mut self,
        message_id: Option<Uuid>,
        redo_snapshot: Option<&str>,
    ) -> Result<()> {
        self.conversation.revert_message_id = message_id;
        if let Some(message_id) = message_id {
            self.store.set_revert_message_id(
                self.conversation.session_id,
                Some(message_id),
                redo_snapshot,
            )?;
        } else {
            self.store
                .clear_revert_message_id(self.conversation.session_id)?;
        }
        Ok(())
    }

    pub(crate) fn clear_revert_state(&mut self) -> Result<()> {
        self.set_revert_message_id(None, None)
    }

    /// If undoing past any compaction messages, restore the context state
    /// from the earliest hidden compaction's prior state.
    /// Returns `true` if state was restored from a compaction message.
    fn restore_context_from_undo_compaction(&mut self, revert_to_id: Uuid) -> bool {
        if let Some((prior_summary, prior_retained_from)) =
            tidev_session::session::find_compaction_prior_state(
                &self.conversation.messages,
                revert_to_id,
            )
        {
            log::info!(
                "restore_context_from_undo_compaction: found compaction msg, \
                 restoring retained_from={} summary={}",
                prior_retained_from,
                prior_summary.is_some(),
            );
            self.context_manager.summary = prior_summary.clone();
            self.context_manager.retained_from = prior_retained_from;
            self.conversation
                .set_context_state(prior_summary, prior_retained_from);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, path::PathBuf, process::Command};
    use tidev_config::ConfigPaths;
    use tidev_session::session::{Message, MessageRole};

    struct CwdGuard(PathBuf);

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    fn temp_workspace(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{}-{}", prefix, Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("workspace should be created");
        dir
    }

    fn build_app(
        workspace_root: &Path,
        config_root: &Path,
        data_root: &Path,
        init_git: bool,
    ) -> (App, Runtime, CwdGuard) {
        let original_cwd = std::env::current_dir().expect("cwd should be readable");
        std::env::set_current_dir(workspace_root).expect("cwd should switch to workspace");

        if init_git {
            let status = Command::new("git")
                .current_dir(workspace_root)
                .args(["init"])
                .status()
                .expect("git init should run");
            assert!(status.success(), "git init should succeed");
        }

        let paths = ConfigPaths {
            config_dir: config_root.to_path_buf(),
            data_dir: data_root.to_path_buf(),
            config_file: config_root.join("config.toml"),
            auth_file: data_root.join("auth.json"),
            database_file: data_root.join("sessions.sqlite3"),
        };

        let app = App::new_with_paths(paths).expect("app should initialize");
        let runtime = Runtime::new().expect("runtime should initialize");

        (app, runtime, CwdGuard(original_cwd))
    }

    fn create_session(app: &mut App) {
        app.store
            .create_session(
                app.conversation.session_id,
                app.workspace_root.as_path(),
                &app.active_model.provider_id,
                &app.active_model.provider_display_name,
                &app.active_model.model_id,
                &app.active_model.display_name,
                "Untitled session",
            )
            .expect("session should be created");
    }

    fn add_user_message(app: &mut App, content: &str) -> Message {
        let message = Message::new(MessageRole::User, content);
        app.conversation.push(message.clone());
        app.store
            .append_message(app.conversation.session_id, &message)
            .expect("message should be stored");
        message
    }

    fn run_scenario(init_git: bool, prefix: &str) {
        let workspace_root = temp_workspace(prefix);
        let config_root = temp_workspace(&format!("{}-config", prefix));
        let data_root = temp_workspace(&format!("{}-data", prefix));
        let file_path = workspace_root.join("note.txt");
        fs::write(&file_path, "before\n").expect("file should be written");

        let (mut app, runtime, _cwd_guard) =
            build_app(&workspace_root, &config_root, &data_root, init_git);
        create_session(&mut app);

        let message = add_user_message(&mut app, "prompt");
        app.capture_prompt_snapshot(message.id, &runtime)
            .expect("prompt snapshot should capture");

        fs::write(&file_path, "after\n").expect("file should be modified");
        app.finalize_snapshot_for_last_user_message_sync(&runtime)
            .expect("finalize snapshot should succeed");

        app.undo_last_user_message(&runtime)
            .expect("undo should succeed");
        assert_eq!(
            fs::read_to_string(&file_path).expect("file should be readable"),
            "before\n"
        );

        let saved_redo_snapshot = app
            .store
            .load_redo_snapshot(app.conversation.session_id)
            .expect("redo snapshot should load");
        assert!(
            saved_redo_snapshot.is_some(),
            "redo snapshot should be stored"
        );

        app.redo_last_user_message(&runtime)
            .expect("redo should succeed");
        assert_eq!(
            fs::read_to_string(&file_path).expect("file should be readable"),
            "after\n"
        );

        let _ = fs::remove_dir_all(&workspace_root);
        let _ = fs::remove_dir_all(&config_root);
        let _ = fs::remove_dir_all(&data_root);
    }

    #[test]
    fn undo_and_redo_restore_files_in_both_workspace_types() {
        run_scenario(false, "tidev-undo-non-git");
        run_scenario(true, "tidev-undo-git");
    }
}
