//! Message persistence helpers for the agent runtime.
//!
//! Small helper methods that write messages, tool results, and assistant
//! turns to the session database and forward [`BackendEvent`]s to frontends.

use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc::UnboundedSender;

use tidev_session::session::{
    AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};

use super::AgentRuntime;

impl AgentRuntime {
    /// Persist a pre-built message to the database.
    ///
    /// Useful when the caller has already constructed the message with
    /// the correct timestamps and IDs (e.g. tool results).
    pub async fn persist_message(&self, session_id: uuid::Uuid, msg: &Message) -> Result<()> {
        let store = self.store.lock().await;
        store.append_message(session_id, msg)?;
        Ok(())
    }

    /// Persist a tool result to the database and emit a `ToolCompleted` event.
    pub async fn persist_tool_result(
        &self,
        session_id: uuid::Uuid,
        request_id: u64,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
        event_tx: &UnboundedSender<BackendEvent>,
    ) -> Result<()> {
        let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
        let _t_start = std::time::Instant::now();
        {
            let store = self.store.lock().await;
            store.append_message(session_id, &tool_msg)?;
        }
        let _t_elapsed = _t_start.elapsed();
        log::debug!(
            "persist_tool_result: store.lock + append_message took {:?}",
            _t_elapsed
        );
        if _t_elapsed > std::time::Duration::from_millis(200) {
            log::warn!(
                "persist_tool_result: store.lock + append_message took {:?} (slow)",
                _t_elapsed
            );
        }
        let _ = event_tx.send(BackendEvent::ToolCompleted {
            session_id,
            request_id,
            tool_call: tool_call.clone(),
            result: result.clone(),
        });
        Ok(())
    }

    /// Persist an [`AssistantTurn`] as a persisted assistant message.
    pub async fn persist_assistant_message(
        &self,
        session_id: uuid::Uuid,
        turn: &AssistantTurn,
    ) -> Result<()> {
        let created_at = turn.created_at.unwrap_or_else(Utc::now);
        let completed_at = turn.completed_at.unwrap_or_else(Utc::now);
        let mut msg = Message::persisted(
            uuid::Uuid::new_v4(),
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

        let _t_start = std::time::Instant::now();
        let store = self.store.lock().await;
        store.append_message(session_id, &msg)?;
        let _t_elapsed = _t_start.elapsed();
        log::debug!(
            "persist_assistant_message: store.lock + append_message took {:?}",
            _t_elapsed
        );
        if _t_elapsed > std::time::Duration::from_millis(200) {
            log::warn!(
                "persist_assistant_message: store.lock + append_message took {:?} (slow)",
                _t_elapsed
            );
        }
        Ok(())
    }
}
