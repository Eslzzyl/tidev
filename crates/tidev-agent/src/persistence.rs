//! Message persistence helpers for the agent runtime.
//!
//! Helper functions that write messages, tool results, and assistant
//! turns to the session database and forward [`BackendEvent`]s to frontends.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use tidev_session::session::{
    AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};
use tidev_storage::SessionStore;

/// Persist a pre-built message to the database.
pub async fn persist_message(
    store: &Arc<Mutex<SessionStore>>,
    session_id: Uuid,
    msg: &Message,
) -> Result<()> {
    let store_guard = store.lock().await;
    store_guard.append_message(session_id, msg)?;
    Ok(())
}

/// Persist a tool result to the database and emit a `ToolCompleted` event.
pub async fn persist_tool_result(
    store: &Arc<Mutex<SessionStore>>,
    session_id: Uuid,
    request_id: u64,
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
    event_tx: &UnboundedSender<BackendEvent>,
) -> Result<()> {
    let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
    {
        let store_guard = store.lock().await;
        store_guard.append_message(session_id, &tool_msg)?;
    }
    let _ = event_tx.send(BackendEvent::ToolCompleted {
        request_id,
        tool_call: tool_call.clone(),
        result: result.clone(),
    });
    Ok(())
}

/// Persist an [`AssistantTurn`] as a persisted assistant message.
///
/// Returns the persisted [`Message`] so callers can reuse its ID for
/// frontend events — avoiding duplicate messages when the TUI loads
/// the session from the database and then also receives the live event.
pub async fn persist_assistant_message(
    store: &Arc<Mutex<SessionStore>>,
    session_id: Uuid,
    turn: &AssistantTurn,
) -> Result<Message> {
    let created_at = turn.created_at.unwrap_or_else(chrono::Utc::now);
    let completed_at = turn.completed_at.unwrap_or_else(chrono::Utc::now);
    let mut msg = Message::persisted(
        Uuid::new_v4(),
        MessageRole::Assistant,
        &turn.content,
        created_at,
        false,
    );
    msg.completed_at = Some(completed_at);
    msg.reasoning = turn.reasoning.clone();
    msg.tool_calls = turn.tool_calls.clone();
    msg.input_tokens = turn.input_tokens;
    msg.output_tokens = turn.output_tokens;
    msg.total_tokens = turn.total_tokens;
    msg.cache_read_tokens = turn.cache_read_tokens;
    msg.cache_write_tokens = turn.cache_write_tokens;
    msg.model_id = turn.model_id.clone();
    msg.tokens_per_second = turn.tokens_per_second;

    let store_guard = store.lock().await;
    store_guard.append_message(session_id, &msg)?;
    Ok(msg)
}
