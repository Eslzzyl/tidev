use anyhow::Result;
use tokio::runtime::Runtime;
use uuid::Uuid;

use crate::{context::ContextManager, snapshot::Patch};

use super::{App, Screen};

impl App {
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

        let snapshot_hash = {
            let Some(msg) = self
                .conversation
                .messages
                .iter()
                .find(|m| m.id == last_user_message_id)
            else {
                crate::log_warn!("finalize_snapshot: message not found in messages list");
                return Ok(());
            };
            msg.snapshot_hash.clone()
        };

        let Some(snapshot_hash) = snapshot_hash else {
            crate::log_info!("finalize_snapshot: snapshot_hash is None");
            return Ok(());
        };

        let patch = runtime.block_on(self.snapshot.patch(&snapshot_hash))?;
        crate::log_info!("finalize_snapshot: patch.files.len()={}", patch.files.len());

        if !patch.files.is_empty() {
            let patch_files = serde_json::to_string(&patch.files)?;
            crate::log_info!(
                "finalize_snapshot: saving patch_files, len={}",
                patch_files.len()
            );
            self.store.update_message_patch(
                self.conversation.session_id,
                last_user_message_id,
                &patch_files,
            )?;

            if let Some(msg) = self
                .conversation
                .messages
                .iter_mut()
                .find(|m| m.id == last_user_message_id)
            {
                msg.patch_files = Some(patch_files);
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

        let Some(message) = self.conversation.last_visible_user_message().cloned() else {
            crate::log_info!("undo_last_user_message: no visible user message found");
            self.last_notice = Some("No earlier user message to undo".to_string());
            return Ok(());
        };

        crate::log_info!(
            "undo_last_user_message: found message id={}, content_len={}",
            message.id,
            message.content.len()
        );
        crate::log_info!(
            "undo_last_user_message: message snapshot_hash={:?}, patch_files={:?}",
            message.snapshot_hash,
            message.patch_files.as_ref().map(|s| s.len())
        );

        let patches = self.collect_patches_after_message(message.id)?;

        crate::log_info!(
            "undo_last_user_message: patches.len()={}, revert_message_id={:?}",
            patches.len(),
            self.conversation.revert_message_id
        );

        let mut notice = None;

        let redo_snapshot = if let Some(existing) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            crate::log_info!("undo_last_user_message: using existing redo_snapshot");
            existing
        } else {
            crate::log_info!("undo_last_user_message: capturing new redo_snapshot");
            match runtime.block_on(self.snapshot.track()) {
                Ok(Some(hash)) => {
                    crate::log_info!(
                        "undo_last_user_message: captured redo_snapshot hash={}",
                        hash
                    );
                    hash
                }
                Ok(None) => {
                    crate::log_info!("undo_last_user_message: track() returned None (no changes)");
                    String::new()
                }
                Err(error) => {
                    crate::log_warn!("undo_last_user_message: track() failed: {}", error);
                    notice = Some(format!("Failed to capture redo snapshot: {error}"));
                    String::new()
                }
            }
        };

        if let Some(existing_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            crate::log_info!("undo_last_user_message: restoring redo_snapshot");
            runtime.block_on(self.snapshot.restore(&existing_snapshot))?;
        }

        if !patches.is_empty() {
            crate::log_info!(
                "undo_last_user_message: reverting {} patches",
                patches.len()
            );
            if let Err(error) = runtime.block_on(self.snapshot.revert(&patches)) {
                crate::log_warn!("undo_last_user_message: revert failed: {}", error);
                notice = Some(format!("Undo partially failed: {error}"));
            }
        }

        crate::log_info!("undo_last_user_message: setting revert_message_id and updating UI");
        self.command_palette.clear();
        self.context_manager = ContextManager::new();
        self.set_revert_message_id(
            Some(message.id),
            if redo_snapshot.is_empty() {
                None
            } else {
                Some(&redo_snapshot)
            },
        )?;
        self.composer.set_text(message.content);
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        self.last_notice = notice.or_else(|| Some("Undid previous user message".to_string()));
        crate::log_info!("undo_last_user_message: completed successfully");
        Ok(())
    }

    pub(crate) fn redo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        if self.pending_request {
            self.abort_current_request();
        }

        let Some(_current_revert) = self.conversation.revert_message_id else {
            self.last_notice = Some("Nothing to redo".to_string());
            return Ok(());
        };

        self.command_palette.clear();

        let Some(redo_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        else {
            self.last_notice = Some("Redo state unavailable".to_string());
            self.clear_revert_state()?;
            return Ok(());
        };

        let mut notice = None;

        if let Err(error) = runtime.block_on(self.snapshot.restore(&redo_snapshot)) {
            notice = Some(format!("Redo failed: {error}"));
        }

        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        self.composer.clear();
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        self.last_notice = notice.or_else(|| Some("Redo complete".to_string()));
        Ok(())
    }

    pub(crate) fn capture_prompt_snapshot(
        &mut self,
        message_id: Uuid,
        runtime: &Runtime,
    ) -> Result<()> {
        crate::log_info!("capture_prompt_snapshot: message_id={}", message_id);

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
        Ok(())
    }

    fn collect_patches_after_message(&self, message_id: Uuid) -> Result<Vec<Patch>> {
        crate::log_info!("collect_patches: looking for message_id={}", message_id);

        let mut patches = Vec::new();
        let mut found = false;

        for message in &self.conversation.messages {
            if found {
                if let Some(patch_files_str) = &message.patch_files {
                    if let Some(snapshot_hash) = &message.snapshot_hash {
                        let files: Vec<String> = serde_json::from_str(patch_files_str)?;
                        crate::log_info!(
                            "collect_patches: found patch in subsequent message, hash={}, files={}",
                            snapshot_hash,
                            files.len()
                        );
                        patches.push(Patch {
                            hash: snapshot_hash.clone(),
                            files,
                        });
                    }
                }
                continue;
            }

            if message.id == message_id {
                found = true;
                crate::log_info!(
                    "collect_patches: found target message, snapshot_hash={:?}, patch_files={:?}",
                    message.snapshot_hash,
                    message.patch_files.as_ref().map(|s| s.len())
                );
                if let Some(patch_files_str) = &message.patch_files {
                    if let Some(snapshot_hash) = &message.snapshot_hash {
                        let files: Vec<String> = serde_json::from_str(patch_files_str)?;
                        crate::log_info!(
                            "collect_patches: target message has patch, hash={}, files={}",
                            snapshot_hash,
                            files.len()
                        );
                        patches.push(Patch {
                            hash: snapshot_hash.clone(),
                            files,
                        });
                    }
                }
            }
        }

        crate::log_info!("collect_patches: returning {} patches", patches.len());
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
}
