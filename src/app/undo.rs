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
        let last_user_message_id = {
            let Some(last_user_message) = self.conversation.last_visible_user_message() else {
                return Ok(());
            };

            let Some(_) = last_user_message.snapshot_hash.clone() else {
                return Ok(());
            };

            last_user_message.id
        };

        let snapshot_hash = {
            let Some(msg) = self
                .conversation
                .messages
                .iter()
                .find(|m| m.id == last_user_message_id)
            else {
                return Ok(());
            };
            msg.snapshot_hash.clone()
        };

        let Some(snapshot_hash) = snapshot_hash else {
            return Ok(());
        };

        let patch = runtime.block_on(self.snapshot.patch(&snapshot_hash))?;

        if !patch.files.is_empty() {
            let patch_files = serde_json::to_string(&patch.files)?;
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

        Ok(())
    }

    pub(crate) fn undo_last_user_message(&mut self, runtime: &Runtime) -> Result<()> {
        if self.pending_request {
            self.abort_current_request();
        }

        let Some(message) = self.conversation.last_visible_user_message().cloned() else {
            self.last_notice = Some("No earlier user message to undo".to_string());
            return Ok(());
        };

        let patches = self.collect_patches_after_message(message.id)?;

        if patches.is_empty() && self.conversation.revert_message_id.is_none() {
            self.last_notice = Some("No changes to undo".to_string());
            return Ok(());
        }

        let mut notice = None;

        let redo_snapshot = if let Some(existing) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            existing
        } else {
            match runtime.block_on(self.snapshot.track()) {
                Ok(Some(hash)) => hash,
                Ok(None) => String::new(),
                Err(error) => {
                    notice = Some(format!("Failed to capture redo snapshot: {error}"));
                    String::new()
                }
            }
        };

        if let Some(existing_snapshot) = self
            .store
            .load_redo_snapshot(self.conversation.session_id)?
        {
            runtime.block_on(self.snapshot.restore(&existing_snapshot))?;
        }

        if !patches.is_empty() {
            if let Err(error) = runtime.block_on(self.snapshot.revert(&patches)) {
                notice = Some(format!("Undo partially failed: {error}"));
            }
        }

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
        match runtime.block_on(self.snapshot.track()) {
            Ok(Some(hash)) => {
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
            Ok(None) => {}
            Err(error) => {
                crate::log_warn!("failed to capture snapshot: {}", error);
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
        let mut patches = Vec::new();
        let mut found = false;

        for message in &self.conversation.messages {
            if found {
                if let Some(patch_files_str) = &message.patch_files {
                    if let Some(snapshot_hash) = &message.snapshot_hash {
                        let files: Vec<String> = serde_json::from_str(patch_files_str)?;
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
                if let Some(patch_files_str) = &message.patch_files {
                    if let Some(snapshot_hash) = &message.snapshot_hash {
                        let files: Vec<String> = serde_json::from_str(patch_files_str)?;
                        patches.push(Patch {
                            hash: snapshot_hash.clone(),
                            files,
                        });
                    }
                }
            }
        }

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
