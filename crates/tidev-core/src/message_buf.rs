//! In-memory message buffer — append-only, single source of truth.
//!
//! Initialized from the database on session start. All agent loop reads
//! go through this buffer; writes are synchronously dual-written
//! (cache + DB) by the caller.

use std::collections::HashMap;

use tidev_llm::message::Message;
use tidev_storage::MessageAppData;

use crate::SessionMessage;

/// In-memory authoritative copy of a session's messages.
///
/// All message reads during an active session go through this struct,
/// never directly to the database. Write operations are the caller's
/// responsibility to keep the cache and DB in sync.
pub struct MessageBuffer {
    messages: Vec<Message>,
    app_data: HashMap<uuid::Uuid, MessageAppData>,
}

impl MessageBuffer {
    /// Create from an existing message list (typically loaded from DB).
    pub fn new(messages: Vec<Message>) -> Self {
        let app_data = messages
            .iter()
            .map(|message| (message.id, MessageAppData::default()))
            .collect();
        Self { messages, app_data }
    }

    /// Create from protocol messages and their persisted application data.
    pub fn from_session_messages(session_messages: Vec<SessionMessage>) -> Self {
        let mut messages = Vec::with_capacity(session_messages.len());
        let mut app_data = HashMap::with_capacity(session_messages.len());
        for session_message in session_messages {
            let id = session_message.message.id;
            messages.push(session_message.message);
            app_data.insert(id, session_message.app_data);
        }
        Self { messages, app_data }
    }

    /// Create an empty buffer (new session).
    pub fn empty() -> Self {
        Self {
            messages: Vec::new(),
            app_data: HashMap::new(),
        }
    }

    /// Read-only access to all current messages.
    pub fn load(&self) -> &[Message] {
        &self.messages
    }

    /// Append a single message.
    pub fn append(&mut self, msg: Message) {
        self.append_with_app_data(msg, MessageAppData::default());
    }

    /// Append a protocol message with its application-owned fields.
    pub fn append_with_app_data(&mut self, msg: Message, app_data: MessageAppData) {
        let id = msg.id;
        self.messages.push(msg);
        self.app_data.insert(id, app_data);
    }

    /// Return the application-owned fields for a message.
    pub fn app_data(&self, id: uuid::Uuid) -> Option<&MessageAppData> {
        self.app_data.get(&id)
    }

    /// Return protocol messages paired with their application-owned fields.
    pub fn session_messages(&self) -> Vec<SessionMessage> {
        self.messages
            .iter()
            .map(|message| {
                let app_data = self
                    .app_data
                    .get(&message.id)
                    .cloned()
                    .unwrap_or_default();
                SessionMessage::new(message.clone(), app_data)
            })
            .collect()
    }

    /// Replace all messages (used after compaction).
    pub fn replace_all(&mut self, messages: Vec<Message>) {
        *self = Self::new(messages);
    }

    /// Replace all messages with protocol data and persisted application data.
    pub fn replace_all_with_session_messages(&mut self, session_messages: Vec<SessionMessage>) {
        *self = Self::from_session_messages(session_messages);
    }

    /// Update the content of a message identified by its ID.
    /// Returns the old content if the message was found, `None` otherwise.
    pub fn update_content(&mut self, id: uuid::Uuid, new_content: String) -> Option<String> {
        for msg in &mut self.messages {
            if msg.id == id {
                let old = std::mem::replace(&mut msg.content, new_content);
                return Some(old);
            }
        }
        None
    }

    /// Remove all messages from `index` onward.
    pub fn truncate(&mut self, index: usize) {
        self.messages.truncate(index);
        let retained: std::collections::HashSet<_> = self.messages.iter().map(|m| m.id).collect();
        self.app_data.retain(|id, _| retained.contains(id));
    }

    /// Number of messages currently buffered.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_llm::message::MessageRole;
    use uuid::Uuid;

    #[test]
    fn session_messages_preserve_protocol_and_app_data_pairing() {
        let first_id = Uuid::from_u128(1);
        let second_id = Uuid::from_u128(2);
        let mut first = Message::new(MessageRole::User, "first");
        first.id = first_id;
        let mut second = Message::new(MessageRole::Assistant, "second");
        second.id = second_id;
        let first_data = MessageAppData {
            mode: Some("plan".into()),
            ..Default::default()
        };
        let second_data = MessageAppData {
            file_diffs: Some("diff".into()),
            ..Default::default()
        };

        let buffer = MessageBuffer::from_session_messages(vec![
            SessionMessage::new(first.clone(), first_data.clone()),
            SessionMessage::new(second.clone(), second_data.clone()),
        ]);

        assert_eq!(buffer.load().len(), 2);
        assert_eq!(buffer.load()[0].id, first_id);
        assert_eq!(buffer.load()[0].content, "first");
        assert_eq!(buffer.load()[1].id, second_id);
        assert_eq!(buffer.load()[1].content, "second");
        let paired = buffer.session_messages();
        assert_eq!(paired[0].message.content, "first");
        assert_eq!(paired[0].app_data, first_data);
        assert_eq!(paired[1].message.content, "second");
        assert_eq!(paired[1].app_data, second_data);
    }

    #[test]
    fn truncate_removes_app_data_for_removed_messages() {
        let mut first = Message::new(MessageRole::User, "first");
        first.id = Uuid::from_u128(1);
        let mut second = Message::new(MessageRole::User, "second");
        second.id = Uuid::from_u128(2);
        let second_data = MessageAppData {
            snapshot_hash: Some("hash".into()),
            ..Default::default()
        };
        let mut buffer = MessageBuffer::from_session_messages(vec![
            SessionMessage::new(first.clone(), MessageAppData::default()),
            SessionMessage::new(second.clone(), second_data),
        ]);

        buffer.truncate(1);

        assert_eq!(buffer.load().len(), 1);
        assert_eq!(buffer.load()[0].id, first.id);
        assert!(buffer.app_data(second.id).is_none());
    }
}
