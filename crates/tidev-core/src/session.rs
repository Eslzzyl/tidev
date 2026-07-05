//! Session lifecycle management.
//!
//! Wraps [`SessionStore`] with the operations tidev-core needs: creating,
//! loading, listing sessions, and persisting compaction state.

use anyhow::Result;
use tidev_storage::SessionStore;
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
    pub fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &str,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
    ) -> Result<()> {
        self.store.create_session(
            session_id,
            workspace_root,
            provider_id,
            provider_display_name,
            model_id,
            model_display_name,
            title,
        )
    }

    /// Load a session record by ID.
    pub fn load_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<tidev_storage::SessionRecord>> {
        self.store.load_session(session_id)
    }

    /// Load all messages for a session, ordered by creation time.
    pub fn load_messages(&self, session_id: Uuid) -> Result<Vec<tidev_types::message::Message>> {
        self.store.load_messages(session_id)
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
        msg: &tidev_types::message::Message,
    ) -> Result<()> {
        self.store.append_message(session_id, msg)
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
        self.store
            .update_session(session_id, title, status, None, None, None, None, None, None, None)
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
    pub fn load_revert_state(
        &self,
        session_id: Uuid,
    ) -> Result<Option<(Uuid, Option<Vec<u8>>)>> {
        self.store.load_revert_state(session_id)
    }
}
