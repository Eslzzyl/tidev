use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

/// Max buffered events per session (for late SSE subscribers).
const MAX_SESSION_EVENTS: usize = 256;

/// Events broadcasted to connected clients via SSE
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEvent {
    /// LLM message chunk (streaming)
    MessageChunk {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    /// LLM reasoning/thinking chunk (streaming)
    ReasoningChunk {
        session_id: Uuid,
        request_id: u64,
        content: String,
    },
    /// LLM message complete
    MessageComplete { session_id: Uuid, request_id: u64 },
    /// Token usage stats for a completed assistant message
    UsageStats {
        session_id: Uuid,
        request_id: u64,
        input_tokens: u32,
        output_tokens: u32,
        total_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
        tokens_per_second: Option<f32>,
    },
    /// Tool call requested
    ToolCall {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
    },
    /// Tool execution result
    ToolResult {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        output: String,
        /// Unified diff patch for write/edit tools
        #[serde(skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        /// File path affected by the tool
        #[serde(skip_serializing_if = "Option::is_none")]
        filepath: Option<String>,
        /// Whether the command was rewritten by RTK
        #[serde(skip_serializing_if = "is_false")]
        rtk_rewritten: bool,
    },
    /// Request user permission for tool
    PermissionRequest {
        session_id: Uuid,
        request_id: u64,
        tool_call_id: String,
        tool_name: String,
        arguments: String,
    },
    /// Request was aborted
    Aborted { session_id: Uuid, request_id: u64 },
    /// Shell command output (streaming)
    ShellOutput {
        session_id: Uuid,
        content: String,
        finished: bool,
        exit_code: Option<i32>,
    },
    /// Messages were updated (e.g., after revert)
    MessagesUpdated { session_id: Uuid },
    /// Error occurred
    Error {
        session_id: Uuid,
        request_id: u64,
        message: String,
    },
    /// Heartbeat to keep connection alive
    Heartbeat,
}

impl AppEvent {
    /// Return the session_id for this event, if any.
    /// Heartbeat is global (no session).
    pub fn session_id(&self) -> Option<Uuid> {
        match self {
            AppEvent::Heartbeat => None,
            AppEvent::MessageChunk { session_id, .. } => Some(*session_id),
            AppEvent::ReasoningChunk { session_id, .. } => Some(*session_id),
            AppEvent::MessageComplete { session_id, .. } => Some(*session_id),
            AppEvent::UsageStats { session_id, .. } => Some(*session_id),
            AppEvent::ToolCall { session_id, .. } => Some(*session_id),
            AppEvent::ToolResult { session_id, .. } => Some(*session_id),
            AppEvent::PermissionRequest { session_id, .. } => Some(*session_id),
            AppEvent::Aborted { session_id, .. } => Some(*session_id),
            AppEvent::ShellOutput { session_id, .. } => Some(*session_id),
            AppEvent::Error { session_id, .. } => Some(*session_id),
            AppEvent::MessagesUpdated { session_id } => Some(*session_id),
        }
    }
}

/// Event bus for broadcasting events to SSE clients.
///
/// Maintains a per-session ring buffer so that late SSE subscribers
/// (who connect after events were published) can catch up.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
    /// Per-session ring buffer of recent events.
    /// Locked with std::sync::Mutex because critical sections are short
    /// (push/pop on a VecDeque) and we need to atomically drain +
    /// subscribe without async await points in between.
    session_events: std::sync::Arc<Mutex<HashMap<Uuid, VecDeque<AppEvent>>>>,
}

impl EventBus {
    /// Create a new event bus with the specified broadcast capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self {
            sender,
            session_events: std::sync::Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Subscribe to live events.
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    /// Atomically subscribe AND drain the per-session buffer.
    ///
    /// Acquires the session buffer lock before subscribing so that
    /// any `publish()` call that fires during this window blocks on the
    /// lock, ensuring the event either:
    ///  - was in the buffer (drained before subscribe -> replayed), OR
    ///  - arrives via broadcast (subscribed before publish -> live).
    ///
    /// Returns (receiver, buffered_events).
    pub fn subscribe_and_drain(&self, session_id: Uuid) -> (broadcast::Receiver<AppEvent>, Vec<AppEvent>) {
        let mut buffers = self.session_events.lock().unwrap();
        let rx = self.sender.subscribe();
        let buffered = buffers.remove(&session_id).unwrap_or_default().into();
        (rx, buffered)
    }

    /// Publish an event to all subscribers and buffer it per-session.
    pub fn publish(&self, event: AppEvent) {
        // Buffer for late SSE subscribers
        if let (Some(sid), Ok(mut buffers)) = (event.session_id(), self.session_events.lock()) {
            let buf = buffers.entry(sid).or_default();
            buf.push_back(event.clone());
            if buf.len() > MAX_SESSION_EVENTS {
                buf.pop_front();
            }
        }
        // Ignore send errors (no subscribers is OK)
        let _ = self.sender.send(event);
    }

    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

/// Helper for serde skip_serializing_if on bool fields
fn is_false(v: &bool) -> bool {
    !v
}
