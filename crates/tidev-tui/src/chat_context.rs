//! Local chat context — provides the rendering code with the data it needs,
//! replacing the old `tidev_session::session::Conversation` struct.

use std::path::Path;
use uuid::Uuid;

use tidev_types::message::Message;

/// A lightweight replacement for the old `Conversation` type.
/// Holds the message list and session metadata needed by rendering code.
#[derive(Clone, Debug)]
pub struct ChatContext {
    pub session_id: Uuid,
    pub title: String,
    pub workspace_root: String,
    pub messages: Vec<Message>,
    /// Number of messages visible (after undo slicing).
    pub visible_count: usize,
    pub parent_session_id: Option<Uuid>,
    pub provider_id: String,
    pub model_id: String,
}

impl ChatContext {
    pub fn new(
        session_id: Uuid,
        title: String,
        workspace_root: String,
        messages: Vec<Message>,
        parent_session_id: Option<Uuid>,
        provider_id: String,
        model_id: String,
    ) -> Self {
        let visible_count = messages.len();
        Self {
            session_id,
            title,
            workspace_root,
            messages,
            visible_count,
            parent_session_id,
            provider_id,
            model_id,
        }
    }

    /// Return the visible subset of messages (respecting undo revert).
    pub fn visible_messages(&self) -> &[Message] {
        &self.messages[..self.visible_count.min(self.messages.len())]
    }

    /// Whether the chat has been reverted (undo active).
    pub fn is_reverted(&self) -> bool {
        self.visible_count < self.messages.len()
    }
}

impl ChatContext {
    /// Normalised workspace root for display.
    pub fn cwd(&self) -> &Path {
        Path::new(&self.workspace_root)
    }
}
