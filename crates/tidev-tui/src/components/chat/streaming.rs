//! Message buffer for streaming — accumulates delta content as the agent
//! generates a response, then flushes to ChatContext when complete.

use tidev_types::message::{Message, MessageRole};
use uuid::Uuid;

/// A mutable extension over ChatContext that supports streaming deltas.
///
/// When the agent generates a response, deltas arrive one at a time via
/// `BackendEvent::Delta`. This buffer accumulates them into a single
/// `Message`, inserting it at the end of the visible message list.
/// Once streaming ends, the completed message is appended to the
/// ChatContext's underlying message vector.
pub(crate) struct StreamingBuffer {
    /// ID of the message currently being streamed (if any).
    pub current_message_id: Option<Uuid>,
    /// Content being accumulated for the current streaming message.
    pending_content: String,
    /// Reasoning content being accumulated.
    pending_reasoning: String,
    /// Whether a delta is currently expected.
    pub is_streaming: bool,
}

impl StreamingBuffer {
    pub fn new() -> Self {
        Self {
            current_message_id: None,
            pending_content: String::new(),
            pending_reasoning: String::new(),
            is_streaming: false,
        }
    }

    /// Start a new streaming turn. Creates a placeholder Assistant message
    /// in the messages list and returns the message ID.
    pub fn begin_streaming(&mut self, messages: &mut Vec<Message>) -> Uuid {
        let message_id = Uuid::new_v4();
        let mut msg = Message::streaming(MessageRole::Assistant, String::new());
        msg.id = message_id;
        messages.push(msg);
        self.current_message_id = Some(message_id);
        self.pending_content = String::new();
        self.pending_reasoning = String::new();
        self.is_streaming = true;
        message_id
    }

    /// Append a content delta to the pending message.
    pub fn push_delta(&mut self, delta: &str) {
        self.pending_content.push_str(delta);
    }

    /// Append a reasoning delta.
    pub fn push_reasoning_delta(&mut self, delta: &str) {
        self.pending_reasoning.push_str(delta);
    }

    /// Finalise the streaming message: copy accumulated content into the
    /// message in `messages`. Returns the message's index.
    pub fn finalise_message(&mut self, messages: &mut Vec<Message>) -> Option<usize> {
        let message_id = self.current_message_id.take()?;
        let idx = messages.iter().position(|m| m.id == message_id)?;

        let msg = &mut messages[idx];
        msg.content = std::mem::take(&mut self.pending_content);
        msg.reasoning = std::mem::take(&mut self.pending_reasoning);
        msg.streaming = false;

        self.is_streaming = false;
        Some(idx)
    }

    /// Update a streaming message's content in the messages list.
    /// Called on each frame during streaming to sync the pending content.
    pub fn sync_pending(&mut self, messages: &mut Vec<Message>) {
        let Some(message_id) = self.current_message_id else { return };
        let Some(idx) = messages.iter().position(|m| m.id == message_id) else { return };

        let msg = &mut messages[idx];
        msg.content = self.pending_content.clone();
        msg.reasoning = self.pending_reasoning.clone();
        msg.streaming = true;
    }
}
