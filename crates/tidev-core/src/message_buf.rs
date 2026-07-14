//! In-memory message buffer — append-only, single source of truth.
//!
//! Initialized from the database on session start. All agent loop reads
//! go through this buffer; writes are synchronously dual-written
//! (cache + DB) by the caller.

use tidev_types::message::Message;

/// In-memory authoritative copy of a session's messages.
///
/// All message reads during an active session go through this struct,
/// never directly to the database. Write operations are the caller's
/// responsibility to keep the cache and DB in sync.
pub struct MessageBuffer {
    messages: Vec<Message>,
}

impl MessageBuffer {
    /// Create from an existing message list (typically loaded from DB).
    pub fn new(messages: Vec<Message>) -> Self {
        Self { messages }
    }

    /// Create an empty buffer (new session).
    pub fn empty() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    /// Read-only access to all current messages.
    pub fn load(&self) -> &[Message] {
        &self.messages
    }

    /// Append a single message.
    pub fn append(&mut self, msg: Message) {
        self.messages.push(msg);
    }

    /// Replace all messages (used after compaction).
    pub fn replace_all(&mut self, messages: Vec<Message>) {
        self.messages = messages;
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
