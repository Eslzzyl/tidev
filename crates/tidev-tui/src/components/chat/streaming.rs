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
            && idx < messages.len()
            && messages[idx].id == mid
        {
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
        if let Some(msg) = messages
            .iter_mut()
            .rev()
            .find(|m| m.streaming && m.role == MessageRole::Assistant)
        {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_idle_state() {
        let sb = StreamingBuffer::new();
        assert!(sb.current_message_id.is_none());
        assert!(!sb.is_streaming);
    }

    #[test]
    fn begin_streaming_creates_placeholder() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        let id = sb.begin_streaming(&mut msgs);
        assert_eq!(msgs.len(), 1);
        assert!(msgs[0].streaming);
        assert_eq!(msgs[0].role, MessageRole::Assistant);
        assert_eq!(msgs[0].id, id);
        assert!(sb.is_streaming);
        assert_eq!(sb.current_message_id, Some(id));
    }

    #[test]
    fn push_delta_appends_to_correct_message() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        sb.begin_streaming(&mut msgs);
        sb.push_delta("Hello ", &mut msgs);
        sb.push_delta("world", &mut msgs);
        assert_eq!(msgs[0].content, "Hello world");
    }

    #[test]
    fn push_reasoning_delta_appends_to_correct_message() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        sb.begin_streaming(&mut msgs);
        sb.push_reasoning_delta("think", &mut msgs);
        sb.push_reasoning_delta(" harder", &mut msgs);
        assert_eq!(msgs[0].reasoning, "think harder");
    }

    #[test]
    fn push_delta_without_active_stream_is_noop() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = vec![Message::new(MessageRole::Assistant, "existing")];
        sb.push_delta("delta", &mut msgs);
        assert_eq!(msgs[0].content, "existing"); // unchanged
    }

    #[test]
    fn resolve_idx_fast_path_uses_cache() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        // Start streaming — creates message at index 0, caches idx = Some(0).
        let streaming_id = sb.begin_streaming(&mut msgs);
        assert_eq!(sb.current_message_idx, Some(0));
        assert_eq!(msgs[0].id, streaming_id);

        // Insert a message before the streaming one to make the cached idx stale.
        let before = Message::new(MessageRole::User, "before");
        msgs.insert(0, before);

        // Cached idx (0) is stale — should fall back to linear scan and find
        // the streaming message at index 1.
        assert_eq!(sb.resolve_idx(&msgs), Some(1));
    }

    #[test]
    fn resolve_idx_returns_none_when_not_streaming() {
        let sb = StreamingBuffer::new();
        assert_eq!(sb.resolve_idx(&[]), None);
    }

    #[test]
    fn finalise_message_clears_state() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        sb.begin_streaming(&mut msgs);
        let idx = sb.finalise_message(&mut msgs);
        assert_eq!(idx, Some(0));
        assert!(!msgs[0].streaming);
        assert!(sb.current_message_id.is_none());
        assert!(!sb.is_streaming);
    }

    #[test]
    fn finalise_message_when_not_streaming_returns_none() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        assert_eq!(sb.finalise_message(&mut msgs), None);
    }

    #[test]
    fn finalise_message_when_message_removed_returns_none() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        sb.begin_streaming(&mut msgs);
        msgs.clear(); // message was removed
        assert_eq!(sb.finalise_message(&mut msgs), None);
    }

    #[test]
    fn recover_or_begin_streaming_when_already_streaming_returns_current_id() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        let id = sb.begin_streaming(&mut msgs);
        let recovered = sb.recover_or_begin_streaming(&mut msgs);
        assert_eq!(recovered, id);
        assert_eq!(msgs.len(), 1); // no new message
    }

    #[test]
    fn recover_or_begin_streaming_finds_existing_streaming_message() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = vec![
            Message::new(MessageRole::User, "hello"),
            Message::streaming(MessageRole::Assistant, "partial"),
        ];
        let streaming_id = msgs[1].id;
        let recovered = sb.recover_or_begin_streaming(&mut msgs);
        assert_eq!(recovered, streaming_id);
        assert!(sb.is_streaming);
        assert_eq!(msgs.len(), 2); // no new message
    }

    #[test]
    fn recover_or_begin_streaming_creates_new_when_none_exists() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = vec![Message::new(MessageRole::User, "hello")];
        let id = sb.recover_or_begin_streaming(&mut msgs);
        assert!(sb.is_streaming);
        assert_eq!(msgs.len(), 2);
        assert!(msgs[1].streaming);
        assert_eq!(msgs[1].id, id);
    }

    #[test]
    fn stream_full_lifecycle() {
        let mut sb = StreamingBuffer::new();
        let mut msgs = Vec::new();
        sb.begin_streaming(&mut msgs);
        sb.push_delta("Hello", &mut msgs);
        sb.push_reasoning_delta("thinking...", &mut msgs);
        sb.push_delta(" world", &mut msgs);
        assert_eq!(msgs[0].content, "Hello world");
        assert_eq!(msgs[0].reasoning, "thinking...");
        let idx = sb.finalise_message(&mut msgs);
        assert_eq!(idx, Some(0));
        assert!(!msgs[0].streaming);
        assert!(!sb.is_streaming);
    }
}
