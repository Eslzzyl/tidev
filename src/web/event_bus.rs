use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

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

/// Event bus for broadcasting events to SSE clients
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl EventBus {
    /// Create a new event bus with the specified capacity
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    /// Publish an event to all subscribers
    pub fn publish(&self, event: AppEvent) {
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
