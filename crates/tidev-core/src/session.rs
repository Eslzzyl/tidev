//! Session lifecycle management.
//!
//! Wraps [`SessionStore`] with the operations tidev-core needs: creating,
//! loading, listing sessions, and persisting compaction state.

use anyhow::Result;
use tidev_storage::SessionStore;
use std::collections::HashMap;
use uuid::Uuid;

/// Thin wrapper around [`SessionStore`] for session lifecycle operations.
#[derive(Clone)]
pub struct SessionManager {
    store: SessionStore,
}

impl SessionManager {
    pub fn new(store: SessionStore) -> Self {
        Self { store }
    }

    /// Create a new session in the database.
    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &str,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
        parent_session_id: Option<Uuid>,
        snapshot_start_hash: Option<&str>,
    ) -> Result<()> {
        self.store.create_session(
            session_id,
            workspace_root,
            provider_id,
            provider_display_name,
            model_id,
            model_display_name,
            title,
            parent_session_id,
            snapshot_start_hash,
        )
    }

    /// Load a session record by ID.
    pub fn load_session(&self, session_id: Uuid) -> Result<Option<tidev_storage::SessionRecord>> {
        self.store.load_session(session_id)
    }

    /// Load all messages for a session, ordered by creation time.
    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<tidev_llm::message::Message>> {
        self.store.load_messages(session_id)
    }

    /// Load application-owned message fields separately from protocol data.
    pub fn load_message_app_data(
        &self,
        session_id: Uuid,
    ) -> Result<HashMap<Uuid, tidev_storage::MessageAppData>> {
        self.store.load_message_app_data(session_id)
    }

    /// Load protocol messages and pair them with their application data.
    pub fn load_session_messages(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<crate::SessionMessage>> {
        let messages = self.load_messages(session_id)?;
        let app_data = self.load_message_app_data(session_id)?;
        Ok(messages
            .into_iter()
            .map(|message| {
                let data = app_data
                    .get(&message.id)
                    .cloned()
                    .unwrap_or_default();
                crate::SessionMessage::new(message, data)
            })
            .collect())
    }

    /// List sessions (newest first).
    pub fn list_sessions(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<tidev_storage::SessionRecord>> {
        self.store.list_sessions(limit, offset)
    }

    /// Append a single message to a session.
    pub fn append_message(
        &self,
        session_id: Uuid,
        msg: &tidev_llm::message::Message,
    ) -> Result<()> {
        self.store.append_message(session_id, msg)
    }

    /// Append multiple messages to a session in a single transaction.
    pub fn append_messages(
        &self,
        session_id: Uuid,
        messages: &[tidev_llm::message::Message],
    ) -> Result<()> {
        self.store.append_messages(session_id, messages)
    }

    /// Append protocol messages together with application-owned fields.
    pub fn append_messages_with_app_data(
        &self,
        session_id: Uuid,
        messages: &[tidev_llm::message::Message],
        app_data: &HashMap<Uuid, tidev_storage::MessageAppData>,
    ) -> Result<()> {
        self.store
            .append_messages_with_app_data(session_id, messages, app_data)
    }

    /// Persist compaction state (summary + retained_from).
    pub fn update_context_state(
        &self,
        session_id: Uuid,
        summary: Option<&str>,
        retained_from: usize,
    ) -> Result<()> {
        self.store.update_session(
            session_id,
            None,
            None,
            summary,
            Some(retained_from),
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// General session metadata update.
    pub fn update_session(
        &self,
        session_id: Uuid,
        title: Option<&str>,
        status: Option<&str>,
    ) -> Result<()> {
        self.store.update_session(
            session_id, title, status, None, None, None, None, None, None, None,
        )
    }

    /// Persist the system prompt for a session.
    pub fn update_system_prompt(&self, session_id: Uuid, system_prompt: &str) -> Result<()> {
        self.store.update_session(
            session_id,
            None,
            None,
            None,
            None,
            Some(system_prompt),
            None,
            None,
            None,
            None,
        )
    }

    /// Update the content of a single message in-place in the store.
    pub fn update_message_content(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        content: &str,
    ) -> Result<()> {
        self.store
            .update_message_content(session_id, message_id, content)
    }

    /// Update the metadata of a single message in-place in the store.
    pub fn update_message_metadata(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        metadata: &tidev_llm::message::ToolMetadata,
    ) -> Result<()> {
        self.store
            .update_message_metadata(session_id, message_id, metadata)
    }

    /// Delete specific messages from a session.
    pub fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        self.store.delete_messages(session_id, message_ids)
    }

    /// Delegate access to the underlying store.
    pub fn store(&self) -> &SessionStore {
        &self.store
    }

    /// Update the provider/model info for a session.
    pub fn update_session_model(
        &self,
        session_id: Uuid,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
    ) -> Result<()> {
        self.store.update_session(
            session_id,
            None,
            None,
            None,
            None,
            None,
            Some(provider_id),
            Some(provider_display_name),
            Some(model_id),
            Some(model_display_name),
        )
    }

    /// Persist revert state for undo/redo.
    pub fn save_revert_state(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        redo_snapshot: Option<&str>,
    ) -> Result<()> {
        self.store
            .save_revert_state(session_id, message_id, redo_snapshot.map(|s| s.as_bytes()))
    }

    /// Load revert state for undo/redo.
    pub fn load_revert_state(&self, session_id: Uuid) -> Result<Option<(Uuid, Option<Vec<u8>>)>> {
        self.store.load_revert_state(session_id)
    }

    /// Persist the snapshot start hash for a session.
    pub fn update_session_start_hash(
        &self,
        session_id: Uuid,
        snapshot_start_hash: &str,
    ) -> Result<()> {
        self.store
            .update_session_start_hash(session_id, snapshot_start_hash)
    }
}
