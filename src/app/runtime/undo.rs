use anyhow::Result;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::{context::ContextManager, snapshot::Patch};

use super::{App, Screen};

/// A single step-level patch stored within a round.
/// Multiple step patches are serialized as a JSON array in `patch_files`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct StepPatch {
    hash: String,
    files: Vec<String>,
    step: usize,
}

impl App {
    /// Finalize snapshot for the last user message by computing per-step patches
    /// and file diffs for sidebar display.
    pub(crate) fn finalize_snapshot_for_last_user_message_sync(
        &mut self,
        runtime: &Runtime,
    ) -> Result<()> {
        crate::log_info!("finalize_snapshot: starting");

        let last_user_message_id = {
            let Some(last_user_message) = self.conversation.last_visible_user_message() else {
                crate::log_info!("finalize_snapshot: no visible user message");
                return Ok(());
            };

            let Some(hash) = last_user_message.snapshot_hash.clone() else {
                crate::log_info!("finalize_snapshot: message has no snapshot_hash");
                return Ok(());
            };

            crate::log_info!(
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
                crate::log_warn!("finalize_snapshot: message not found in messages list");
                return Ok(());
            };
            match msg.snapshot_hash.clone() {
                Some(h) => h,
                None => {
                    crate::log_info!("finalize_snapshot: snapshot_hash is None");
                    return Ok(());
                }
            }
        };

        // Collect all snapshot hashes: initial + step snapshots
        let step_hashes: Vec<String> = self.step_snapshot_hashes.drain(..).collect();
        let mut all_hashes = vec![initial_hash.clone()];
        all_hashes.extend(step_hashes);

        crate::log_info!(
            "finalize_snapshot: computing patches for {} snapshots (initial + {} steps)",
            all_hashes.len(),
            all_hashes.len().saturating_sub(1)
        );

        // Compute cumulative patch for each snapshot
        let mut step_patches: Vec<StepPatch> = Vec::new();
        for (i, hash) in all_hashes.iter().enumerate() {
            match runtime.block_on(self.snapshot.patch(hash)) {
                Ok(patch) => {
                    if !patch.files.is_empty() {
                        crate::log_info!(
                            "finalize_snapshot: step {} hash={} files={}",
                            i,
                            hash,
                            patch.files.len()
                        );
                        step_patches.push(StepPatch {
                            hash: hash.clone(),
                            files: patch.files,
                            step: i,
                        });
                    }
                }
                Err(e) => {
                    crate::log_warn!("finalize_snapshot: patch for step {} failed: {}", i, e);
                }
            }
        }

        if !step_patches.is_empty() {
            let patch_files_json = serde_json::to_string(&step_patches)?;
            crate::log_info!(
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

            // Compute full file diffs for sidebar display
            match runtime.block_on(self.snapshot.diff_full(&initial_hash, &all_hashes.last().cloned().unwrap_or(initial_hash.clone()))) {
                Ok(file_diffs) => {
                    let diffs_json = serde_json::to_string(&file_diffs)?;
                    self.store.update_message_file_diffs(
                        self.conversation.session_id,
                        last_user_message_id,
                        &diffs_json,
                    )?;
                    if let Some(msg) = self
                        .conversation
                        .messages
                        .iter_mut()
                        .find(|m| m.id == last_user_message_id)
                    {
                        msg.file_diffs = Some(diffs_json);
                    }
                }
                Err(e) => {
                    crate::log_warn!("finalize_snapshot: diff_full failed: {}", e);
                }
            }
        }

        crate::log_info!("finalize_snapshot: completed");
        Ok(())
    }

    pub(crate) fn undo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!("undo_last_user_message: starting");

        if self.pending_request {
            self.abort_current_request();
        }

        let message = if let Some(current_revert) = self.conversation.revert_message_id {
            crate::log_info!(
                "undo_last_user_message: already in revert state, looking for prev user message before {}",
                current_revert
            );
            self.conversation.prev_user_message_before(current_revert)
        } else {
            crate::log_info!(
                "undo_last_user_message: not in revert state, looking for last visible user message"
            );
            self.conversation.last_visible_user_message()
        };

        let Some(message) = message else {
            crate::log_info!("undo_last_user_message: no user message found");
            self.last_notice = Some("No earlier user message to undo".to_string());
            return Ok(());
        };

        let message = message.clone();
        crate::log_info!(
            "undo_last_user_message: found message id={}, content_len={}",
            message.id,
            message.content.len()
        );

        self.revert_to_message(message.id, message.content.clone(), runtime)?;
        self.last_notice = Some("Undid previous user message".to_string());
        crate::log_info!("undo_last_user_message: completed successfully");
        Ok(())
    }

    pub(crate) fn redo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!("redo_last_user_message: starting");

        if self.pending_request {
            self.abort_current_request();
        }

        let Some(current_revert) = self.conversation.revert_message_id else {
            crate::log_info!("redo_last_user_message: not in revert state");
            self.last_notice = Some("Nothing to redo".to_string());
            return Ok(());
        };

        crate::log_info!(
            "redo_last_user_message: looking for next user message after {}",
            current_revert
        );

        if let Some(next_message) = self.conversation.next_user_message_after(current_revert) {
            crate::log_info!(
                "redo_last_user_message: found next user message id={}",
                next_message.id
            );
            let message_id = next_message.id;
            let content = next_message.content.clone();
            self.revert_to_message(message_id, content, runtime)?;
            self.last_notice = Some("Redo complete".to_string());
        } else {
            crate::log_info!("redo_last_user_message: no next user message, unreverting");
            self.unrevert(runtime)?;
            self.last_notice = Some("Redo complete".to_string());
        }

        crate::log_info!("redo_last_user_message: completed successfully");
        Ok(())
    }

    fn revert_to_message(
        &mut self,
        message_id: Uuid,
        message_content: String,
        runtime: &Runtime,
    ) -> Result<()> {
        crate::log_info!("revert_to_message: message_id={}", message_id);

        let patches = self.collect_patches_after_message(message_id)?;

        crate::log_info!(
            "revert_to_message: patches.len()={}, revert_message_id={:?}",
            patches.len(),
            self.conversation.revert_message_id
        );

        let mut notice = None;

        let redo_snapshot = if let Some(existing) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            crate::log_info!("revert_to_message: using existing redo_snapshot");
            existing
        } else {
            crate::log_info!("revert_to_message: capturing new redo_snapshot");
            match runtime.block_on(self.snapshot.track()) {
                Ok(Some(hash)) => {
                    crate::log_info!("revert_to_message: captured redo_snapshot hash={}", hash);
                    hash
                }
                Ok(None) => {
                    crate::log_info!("revert_to_message: track() returned None (no changes)");
                    String::new()
                }
                Err(error) => {
                    crate::log_warn!("revert_to_message: track() failed: {}", error);
                    notice = Some(format!("Failed to capture redo snapshot: {error}"));
                    String::new()
                }
            }
        };

        if let Some(existing_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            crate::log_info!("revert_to_message: restoring redo_snapshot");
            runtime.block_on(self.snapshot.restore(&existing_snapshot))?;
        }

        if !patches.is_empty() {
            crate::log_info!("revert_to_message: reverting {} patches", patches.len());
            if let Err(error) = runtime.block_on(self.snapshot.revert(&patches)) {
                crate::log_warn!("revert_to_message: revert failed: {}", error);
                notice = Some(format!("Revert partially failed: {error}"));
            }
        }

        crate::log_info!("revert_to_message: setting revert_message_id and updating UI");
        self.command_palette.clear();
        self.context_manager = ContextManager::new();
        self.conversation.clear_context_state();
        self.set_revert_message_id(
            Some(message_id),
            if redo_snapshot.is_empty() {
                None
            } else {
                Some(&redo_snapshot)
            },
        )?;
        self.composer.set_text(message_content);
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        if let Some(n) = notice {
            self.last_notice = Some(n);
        }
        Ok(())
    }

    fn unrevert(&mut self, runtime: &Runtime) -> Result<()> {
        crate::log_info!("unrevert: starting");

        let Some(redo_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        else {
            crate::log_info!("unrevert: no redo_snapshot found");
            self.clear_revert_state()?;
            return Ok(());
        };

        crate::log_info!("unrevert: restoring redo_snapshot");
        if let Err(error) = runtime.block_on(self.snapshot.restore(&redo_snapshot)) {
            crate::log_warn!("unrevert: restore failed: {}", error);
            self.last_notice = Some(format!("Redo failed: {error}"));
        }

        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        self.conversation.clear_context_state();
        self.composer.clear();
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        crate::log_info!("unrevert: completed");
        Ok(())
    }

    pub(crate) fn capture_prompt_snapshot(
        &mut self,
        message_id: Uuid,
        runtime: &Runtime,
    ) -> Result<()> {
        crate::log_info!("capture_prompt_snapshot: message_id={}", message_id);

        // Clear any intermediate step snapshots from previous rounds
        self.step_snapshot_hashes.clear();

        match runtime.block_on(self.snapshot.track()) {
            Ok(Some(hash)) => {
                crate::log_info!("capture_prompt_snapshot: captured hash={}", hash);
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
                    msg.snapshot_hash = Some(hash);
                }
            }
            Ok(None) => {
                crate::log_info!(
                    "capture_prompt_snapshot: track() returned None (not a git repo or no changes)"
                );
            }
            Err(error) => {
                crate::log_warn!("capture_prompt_snapshot: track() failed: {}", error);
            }
        }

        Ok(())
    }

    /// Capture an intermediate snapshot between tool execution steps within a round.
    /// These are used at round end to compute per-step patches.
    pub(crate) fn capture_step_snapshot(&mut self, runtime: &Runtime) {
        crate::log_info!("capture_step_snapshot: starting");
        match runtime.block_on(self.snapshot.track()) {
            Ok(Some(hash)) => {
                crate::log_info!("capture_step_snapshot: captured hash={}", hash);
                self.step_snapshot_hashes.push(hash);
            }
            Ok(None) => {
                crate::log_info!("capture_step_snapshot: track() returned None (no changes)");
            }
            Err(error) => {
                crate::log_warn!("capture_step_snapshot: track() failed: {}", error);
            }
        }
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

        let _ = self.conversation.take_hidden_messages();
        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        self.conversation.clear_context_state();
        Ok(())
    }

    fn collect_patches_after_message(&self, message_id: Uuid) -> Result<Vec<Patch>> {
        crate::log_info!("collect_patches: looking for message_id={}", message_id);

        let mut patches = Vec::new();
        let mut found = false;

        // We iterate through the messages in forward order to find the target.
        // For REVERTING, we collect patches in order such that the OLDEST
        // snapshot hash for each file takes priority (to restore pre-round state).
        // The patches are inserted at position 0, so they end up in reverse order
        // (newest message first). Within each message, step patches are also reversed
        // so the initial snapshot hash takes priority.
        for message in &self.conversation.messages {
            if found {
                patches = self.collect_patches_from_message(patches, message);
                continue;
            }

            if message.id == message_id {
                found = true;
                crate::log_info!(
                    "collect_patches: found target message, snapshot_hash={:?}, patch_files={:?}",
                    message.snapshot_hash,
                    message.patch_files.as_ref().map(|s| s.len())
                );
                // Also include the target message's own patches
                patches = self.collect_patches_from_message(patches, message);
            }
        }

        // Reverse so the OLDEST patches (closest to target message) are processed first.
        // This ensures the initial snapshot hash takes priority in revert dedup.
        patches.reverse();

        crate::log_info!("collect_patches: returning {} patches", patches.len());
        Ok(patches)
    }

    /// Extract patches from a single message's patch_files.
    /// Handles both new nested format `[{"hash":"...","files":[...],"step":N}]`
    /// and old flat format `["file1","file2"]`.
    fn extract_patches_from_message(message: &crate::session::Message) -> Vec<Patch> {
        let Some(patch_files_str) = &message.patch_files else {
            return Vec::new();
        };

        // Try nested format first
        if let Ok(step_patches) = serde_json::from_str::<Vec<StepPatch>>(patch_files_str) {
            return step_patches
                .into_iter()
                .map(|sp| Patch {
                    hash: sp.hash,
                    files: sp.files,
                })
                .collect();
        }

        // Fallback: old flat format `["file1","file2"]` — use message's snapshot_hash
        if let Ok(files) = serde_json::from_str::<Vec<String>>(patch_files_str) {
            if !files.is_empty() {
                if let Some(hash) = &message.snapshot_hash {
                    return vec![Patch {
                        hash: hash.clone(),
                        files,
                    }];
                }
            }
        }

        Vec::new()
    }

    /// Collect patches from a message, inserting them at the beginning of the list
    /// so that newer messages appear first (will be reversed later for correct ordering).
    fn collect_patches_from_message(
        &self,
        mut patches: Vec<Patch>,
        message: &crate::session::Message,
    ) -> Vec<Patch> {
        let msg_patches = Self::extract_patches_from_message(message);
        if msg_patches.is_empty() {
            return patches;
        }
        crate::log_info!(
            "collect_patches: message {} has {} step patches",
            message.id,
            msg_patches.len()
        );
        for msg_patch in msg_patches.into_iter().rev() {
            patches.insert(0, msg_patch);
        }
        patches
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::ConfigPaths,
        session::{Message, MessageRole},
    };
    use std::{fs, path::PathBuf, process::Command};

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
        workspace_root: &PathBuf,
        config_root: &PathBuf,
        data_root: &PathBuf,
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
            config_dir: config_root.clone(),
            data_dir: data_root.clone(),
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
