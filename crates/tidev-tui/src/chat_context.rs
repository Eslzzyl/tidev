//! Local chat context — provides the rendering code with the data it needs,
//! replacing the old `tidev_session::session::Conversation` struct.

use std::path::Path;
use uuid::Uuid;

use tidev_types::message::Message;

/// A lightweight replacement for the old `Conversation` type.
/// Holds the message list and session metadata needed by rendering code.
#[derive(Clone, Debug, Default)]
pub struct ChatContext {
    pub session_id: Uuid,
    pub title: String,
    pub workspace_root: String,
    pub messages: Vec<Message>,
    /// When set, only messages up to (not including) this one are visible
    /// (undo revert point).
    pub revert_message_id: Option<Uuid>,
    pub parent_session_id: Option<Uuid>,
    pub provider_id: String,
    pub model_id: String,
    pub model_display_name: String,
    pub provider_display_name: String,
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
        model_display_name: String,
        provider_display_name: String,
    ) -> Self {
        Self {
            session_id,
            title,
            workspace_root,
            messages,
            revert_message_id: None,
            parent_session_id,
            provider_id,
            model_id,
            model_display_name,
            provider_display_name,
        }
    }

    /// Return the visible subset of messages (respecting undo revert).
    pub fn visible_messages(&self) -> &[Message] {
        let end = self
            .revert_message_id
            .and_then(|id| self.messages.iter().position(|m| m.id == id))
            .unwrap_or(self.messages.len());
        &self.messages[..end]
    }

    /// Whether the chat has been reverted (undo active).
    pub fn is_reverted(&self) -> bool {
        self.revert_message_id.is_some()
    }
}

impl ChatContext {
    /// Normalised workspace root for display.
    pub fn cwd(&self) -> &Path {
        Path::new(&self.workspace_root)
    }

    /// Push a message to the end of the context.
    pub fn push(&mut self, message: Message) {
        self.messages.push(message);
    }
}
