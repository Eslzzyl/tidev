//! Message buffer for streaming — accumulates delta content as the agent
//! generates a response, then flushes to ChatContext when complete.

use tidev_types::message::{Message, MessageRole};
use uuid::Uuid;

/// A mutable extension over ChatContext that supports streaming deltas.
///
/// When the agent generates a response, deltas arrive one at a time via
/// `BackendEvent::Delta`. This buffer tracks the streaming message and
/// applies deltas directly to it in the ChatContext's message list.
/// Once streaming ends, the message is finalised.
pub(crate) struct StreamingBuffer {
    /// ID of the message currently being streamed (if any).
    pub current_message_id: Option<Uuid>,
    /// Cached index into messages[] for O(1) push_delta lookup.
    current_message_idx: Option<usize>,
    /// Whether a delta is currently expected.
    pub is_streaming: bool,
}

impl StreamingBuffer {
    pub fn new() -> Self {
        Self {
            current_message_id: None,
            current_message_idx: None,
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
        self.current_message_idx = Some(messages.len() - 1);
        self.is_streaming = true;
        message_id
    }

    /// Append a content delta directly to the streaming message in `messages`.
    pub fn push_delta(&mut self, delta: &str, messages: &mut [Message]) {
        let idx = self.resolve_idx(messages);
        if let Some(idx) = idx {
            messages[idx].content.push_str(delta);
        }
    }

    /// Append a reasoning delta directly to the streaming message in `messages`.
    pub fn push_reasoning_delta(&mut self, delta: &str, messages: &mut [Message]) {
        let idx = self.resolve_idx(messages);
        if let Some(idx) = idx {
            messages[idx].reasoning.push_str(delta);
        }
    }

    /// Resolve the message index, using the cached index if still valid.
    fn resolve_idx(&self, messages: &[Message]) -> Option<usize> {
        let mid = self.current_message_id?;
        // Fast path: cached index still points to the right message.
        if let Some(idx) = self.current_message_idx
            && idx < messages.len() && messages[idx].id == mid {
                return Some(idx);
            }
        // Slow path: find by id.
        messages.iter().position(|m| m.id == mid)
    }

    /// Finalise the streaming message: set streaming=false.
    /// Returns the message's index.
    pub fn finalise_message(&mut self, messages: &mut [Message]) -> Option<usize> {
        let message_id = self.current_message_id.take()?;
        let idx = messages.iter().position(|m| m.id == message_id)?;

        messages[idx].streaming = false;
        self.current_message_idx = None;
        self.is_streaming = false;
        Some(idx)
    }

    /// Recover from a missed TurnStarting event (e.g. after a session switch).
    ///
    /// If there's already a streaming Assistant message (from a prior
    /// `begin_streaming` that survived the switch), pick it up. Otherwise
    /// create a new placeholder. Returns the message ID.
    pub fn recover_or_begin_streaming(&mut self, messages: &mut Vec<Message>) -> Uuid {
        if self.is_streaming {
            // Already streaming — nothing to do.
            return self.current_message_id.unwrap_or_else(Uuid::new_v4);
        }

        // Look for an existing streaming Assistant message.
        if let Some(msg) = messages.iter_mut().rev().find(|m| {
            m.streaming && m.role == MessageRole::Assistant
        }) {
            let id = msg.id;
            self.current_message_id = Some(id);
            self.current_message_idx = messages.iter().position(|m| m.id == id);
            self.is_streaming = true;
            return id;
        }

        // No existing streaming message — begin a new stream.
        self.begin_streaming(messages)
    }
}
