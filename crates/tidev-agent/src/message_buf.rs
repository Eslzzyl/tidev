//! In-memory protocol message buffer used by the generic agent runtime.

use tidev_llm::message::Message;

/// Append-only in-memory storage for protocol messages.
pub struct MessageBuffer {
    messages: Vec<Message>,
}

impl MessageBuffer {
    /// Create a buffer from protocol messages.
    pub fn new(messages: Vec<Message>) -> Self {
        Self { messages }
    }

    /// Create an empty buffer.
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Read all protocol messages in insertion order.
    pub fn load(&self) -> &[Message] {
        &self.messages
    }

    /// Append a protocol message.
    pub fn append(&mut self, message: Message) {
        self.messages.push(message);
    }

    /// Replace all protocol messages.
    pub fn replace_all(&mut self, messages: Vec<Message>) {
        self.messages = messages;
    }

    /// Update the content of a message identified by its ID.
    pub fn update_content(&mut self, id: uuid::Uuid, new_content: String) -> Option<String> {
        for message in &mut self.messages {
            if message.id == id {
                return Some(std::mem::replace(&mut message.content, new_content));
            }
        }
        None
    }

    /// Replace one message in place while retaining its position.
    pub fn replace(&mut self, id: uuid::Uuid, message: Message) -> bool {
        let Some(index) = self.messages.iter().position(|current| current.id == id) else {
            return false;
        };
        self.messages[index] = message;
        true
    }

    /// Remove one message without disturbing the surrounding order.
    pub fn remove(&mut self, id: uuid::Uuid) -> Option<Message> {
        let index = self.messages.iter().position(|message| message.id == id)?;
        Some(self.messages.remove(index))
    }

    /// Remove all messages from `index` onward.
    pub fn truncate(&mut self, index: usize) {
        self.messages.truncate(index);
    }

    /// Return the number of buffered messages.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Return whether the buffer contains no messages.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_llm::message::MessageRole;

    #[test]
    fn protocol_messages_keep_order_and_content_updates() {
        let mut first = Message::new(MessageRole::User, "first");
        first.id = uuid::Uuid::from_u128(1);
        let mut second = Message::new(MessageRole::Assistant, "second");
        second.id = uuid::Uuid::from_u128(2);
        let mut buffer = MessageBuffer::new(vec![first.clone(), second]);

        assert_eq!(buffer.load().len(), 2);
        assert_eq!(buffer.load()[0].id, first.id);
        assert_eq!(
            buffer.update_content(first.id, "updated".to_string()),
            Some("first".to_string())
        );
        assert_eq!(buffer.load()[0].content, "updated");
    }

    #[test]
    fn truncate_only_removes_protocol_messages() {
        let messages = vec![
            Message::new(MessageRole::User, "first"),
            Message::new(MessageRole::User, "second"),
        ];
        let mut buffer = MessageBuffer::new(messages);

        buffer.truncate(1);

        assert_eq!(buffer.len(), 1);
        assert!(!buffer.is_empty());
    }
}
