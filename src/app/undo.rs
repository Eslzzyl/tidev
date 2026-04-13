use anyhow::{Context, Result};
use std::{fs, path::PathBuf};
use uuid::Uuid;

use crate::{context::ContextManager, session::MessageRole, workspace_snapshot::WorkspaceSnapshot};

use super::{App, Screen};

impl App {
    pub(crate) fn undo_last_user_message(&mut self) -> Result<()> {
        if self.pending_request {
            self.abort_current_request();
        }

        let Some(message) = self.conversation.last_visible_user_message().cloned() else {
            self.last_notice = Some("No earlier user message to undo".to_string());
            return Ok(());
        };

        let mut notice = None;

        if let Err(error) = self.capture_redo_snapshot() {
            notice = Some(format!(
                "Saved undo state without workspace snapshot: {error}"
            ));
        }

        if let Err(error) = self.restore_workspace_snapshot_for_message(message.id) {
            notice = Some(format!(
                "Undid message, but workspace rollback failed: {error}"
            ));
        }

        self.command_palette.clear();
        self.context_manager = ContextManager::new();
        self.set_revert_message_id(Some(message.id))?;
        self.composer.set_text(message.content);
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        self.last_notice = notice.or_else(|| Some("Undid previous user message".to_string()));
        Ok(())
    }

    pub(crate) fn redo_last_user_message(&mut self) -> Result<()> {
        if self.pending_request {
            self.abort_current_request();
        }

        let Some(current_revert) = self.conversation.revert_message_id else {
            self.last_notice = Some("Nothing to redo".to_string());
            return Ok(());
        };

        self.command_palette.clear();
        let mut notice = None;

        if let Some(next_message) = self
            .conversation
            .next_user_message_after(current_revert)
            .cloned()
        {
            if let Err(error) = self.restore_workspace_snapshot_for_message(next_message.id) {
                notice = Some(format!(
                    "Redid message, but workspace rollback failed: {error}"
                ));
            }

            self.set_revert_message_id(Some(next_message.id))?;
            self.context_manager = ContextManager::new();
            self.screen = Screen::Chat;
            self.scroll_messages_to_bottom();
            self.last_notice = notice.or_else(|| Some("Redid previous undo".to_string()));
            return Ok(());
        }

        if let Err(error) = self.restore_redo_snapshot() {
            notice = Some(format!("Redo state unavailable: {error}"));
        }

        self.clear_revert_state()?;
        self.clear_redo_snapshot();
        self.context_manager = ContextManager::new();
        self.composer.clear();
        self.screen = Screen::Chat;
        self.scroll_messages_to_bottom();
        self.last_notice = notice.or_else(|| Some("Redo complete".to_string()));
        Ok(())
    }

    pub(crate) fn capture_prompt_snapshot(&self, message_id: Uuid) -> Result<()> {
        let snapshot = WorkspaceSnapshot::capture(self.workspace_root.as_path())?;
        self.write_message_snapshot(message_id, &snapshot)
    }

    pub(crate) fn discard_reverted_branch(&mut self) -> Result<()> {
        if !self.conversation.is_reverted() {
            return Ok(());
        }

        let visible_count = self.conversation.visible_message_count();
        let hidden_messages = self.conversation.messages[visible_count..].to_vec();
        let hidden_user_message_ids: Vec<Uuid> = hidden_messages
            .iter()
            .filter(|message| matches!(message.role, MessageRole::User))
            .map(|message| message.id)
            .collect();

        self.store.delete_messages(
            self.conversation.session_id,
            &hidden_messages
                .iter()
                .map(|message| message.id)
                .collect::<Vec<_>>(),
        )?;

        for message_id in hidden_user_message_ids {
            let _ = self.delete_message_snapshot(message_id);
        }

        self.clear_redo_snapshot();
        let _ = self.conversation.take_hidden_messages();
        self.clear_revert_state()?;
        self.context_manager = ContextManager::new();
        Ok(())
    }

    fn set_revert_message_id(&mut self, message_id: Option<Uuid>) -> Result<()> {
        self.conversation.revert_message_id = message_id;
        if let Some(message_id) = message_id {
            self.store
                .set_revert_message_id(self.conversation.session_id, Some(message_id))?;
        } else {
            self.store
                .clear_revert_message_id(self.conversation.session_id)?;
        }
        Ok(())
    }

    pub(crate) fn clear_revert_state(&mut self) -> Result<()> {
        self.set_revert_message_id(None)
    }

    fn undo_state_root(&self) -> PathBuf {
        self.paths
            .data_dir
            .join("undo")
            .join(self.conversation.session_id.to_string())
    }

    fn redo_snapshot_path(&self) -> PathBuf {
        self.undo_state_root().join("redo.json")
    }

    fn message_snapshot_path(&self, message_id: Uuid) -> PathBuf {
        self.undo_state_root()
            .join("messages")
            .join(format!("{message_id}.json"))
    }

    fn write_message_snapshot(&self, message_id: Uuid, snapshot: &WorkspaceSnapshot) -> Result<()> {
        let path = self.message_snapshot_path(message_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create snapshot directory {}", parent.display())
            })?;
        }

        let contents = serde_json::to_vec_pretty(snapshot)
            .context("failed to serialize workspace snapshot")?;
        fs::write(&path, contents)
            .with_context(|| format!("failed to write snapshot {}", path.display()))?;
        Ok(())
    }

    fn capture_redo_snapshot(&self) -> Result<()> {
        let snapshot = WorkspaceSnapshot::capture(self.workspace_root.as_path())?;
        let path = self.redo_snapshot_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create snapshot directory {}", parent.display())
            })?;
        }

        let contents =
            serde_json::to_vec_pretty(&snapshot).context("failed to serialize redo snapshot")?;
        fs::write(&path, contents)
            .with_context(|| format!("failed to write snapshot {}", path.display()))?;
        Ok(())
    }

    fn restore_workspace_snapshot_for_message(&self, message_id: Uuid) -> Result<()> {
        let path = self.message_snapshot_path(message_id);
        let contents = fs::read(&path)
            .with_context(|| format!("failed to read snapshot {}", path.display()))?;
        let snapshot: WorkspaceSnapshot =
            serde_json::from_slice(&contents).context("failed to parse workspace snapshot")?;
        snapshot.restore(self.workspace_root.as_path())
    }

    fn restore_redo_snapshot(&self) -> Result<()> {
        let path = self.redo_snapshot_path();
        let contents = fs::read(&path)
            .with_context(|| format!("failed to read snapshot {}", path.display()))?;
        let snapshot: WorkspaceSnapshot =
            serde_json::from_slice(&contents).context("failed to parse redo snapshot")?;
        snapshot.restore(self.workspace_root.as_path())
    }

    fn clear_redo_snapshot(&self) {
        let _ = fs::remove_file(self.redo_snapshot_path());
    }

    fn delete_message_snapshot(&self, message_id: Uuid) -> Result<()> {
        let path = self.message_snapshot_path(message_id);
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("failed to remove snapshot {}", path.display()))
            }
        }
    }
}
