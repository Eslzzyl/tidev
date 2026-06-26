//! AgentLoop — the core LLM ↔ tool execution loop.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! Events carry NO `session_id` — the receiver already knows which session
//! the events belong to (Per-Session Event Bus).

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use tidev_session::session::{
    AssistantTurn, BackendEvent, Conversation, Message, MessageRole, ToolCall,
    ToolExecutionResult,
};
use tidev_types::ToolSchema;
use tidev_types::prompts::SessionMode;
use tidev_storage::SessionStore;

use crate::types::AgentType;

/// The per-session agent loop.
pub struct AgentLoop {
    pub session_id: Uuid,
    pub model: tidev_config::ActiveModel,
    pub conversation: Conversation,
    pub context: tidev_context::ContextManager,
    pub tools: Vec<tidev_tools::ToolDefinition>,
    pub store: Arc<tokio::sync::Mutex<SessionStore>>,
    pub llm: tidev_llm::LlmClient,
    pub event_tx: UnboundedSender<BackendEvent>,
    pub cancel_token: CancellationToken,
    pub mode: SessionMode,
    pub agent_type: AgentType,
}

impl AgentLoop {
    /// Run the main agent loop.
    pub async fn run(mut self) -> Result<()> {
        log::info!("agent_loop[{}]: started", self.session_id);

        let mut request_id: u64 = 1;
        loop {
            if self.cancel_token.is_cancelled() {
                log::info!("agent_loop[{}]: cancelled", self.session_id);
                break;
            }

            // Build request messages from the conversation
            let messages = self.context.build_request_messages(
                &self.conversation,
                self.mode,
            );

            // Convert tools to LLM-facing ToolSchema
            let llm_tools: Vec<ToolSchema> = self
                .tools
                .iter()
                .map(|t| ToolSchema {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect();

            // Run a single LLM turn
            let turn = self
                .run_single_turn(request_id, &messages, &llm_tools)
                .await?;

            // Persist the assistant turn
            let assistant_msg = assistant_turn_to_message(&turn);
            self.conversation.push(assistant_msg.clone());
            {
            let store = self.store.lock().await;
            store.append_message(self.session_id, &assistant_msg)?;            }

            // If no tool calls, we're done
            if turn.tool_calls.is_empty() {
                let _ = self.event_tx.send(BackendEvent::StreamEnd { request_id });
                log::info!("agent_loop[{}]: completed", self.session_id);
                break;
            }

            // Execute tool calls
            self.execute_tool_calls(request_id, &turn.tool_calls)
                .await?;

            request_id += 1;
        }

        Ok(())
    }

    /// Run a single LLM streaming turn.
    async fn run_single_turn(
        &mut self,
        request_id: u64,
        messages: &[Message],
        llm_tools: &[ToolSchema],
    ) -> Result<AssistantTurn> {
        let _ = self
            .event_tx
            .send(BackendEvent::TurnStarting { request_id });

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        // Spawn LLM streaming task
        let llm = self.llm.clone();
        let model = tidev_llm::LlmProviderConfig::from(self.model.clone());
        let session_id = self.session_id;
        let tl = self.model.thinking_level.clone();
        let msgs = messages.to_vec();
        let tools = llm_tools.to_vec();

        tokio::spawn(async move {
            llm.stream_chat(session_id, request_id, model, msgs, tools, tx, tl)
                .await;
        });

        // Collect the assistant turn from streamed events
        let mut turn = AssistantTurn::default();
        while let Some(event) = rx.recv().await {
            let _ = self.event_tx.send(event.clone());

            match event {
                BackendEvent::Delta { content, .. } => {
                    if turn.created_at.is_none() {
                        turn.created_at = Some(Utc::now());
                    }
                    turn.content.push_str(&content);
                }
                BackendEvent::ReasoningDelta { content, .. } => {
                    if turn.created_at.is_none() {
                        turn.created_at = Some(Utc::now());
                    }
                    turn.reasoning.push_str(&content);
                }
                BackendEvent::ToolCallUpdated { tool_call, .. } => {
                    turn.upsert_tool_call(tool_call);
                }
                BackendEvent::UsageStats {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    model_id,
                    ..
                } => {
                    turn.input_tokens = Some(input_tokens);
                    turn.output_tokens = Some(output_tokens);
                    turn.total_tokens = Some(total_tokens);
                    turn.cache_read_tokens = Some(cache_read_tokens);
                    turn.cache_write_tokens = Some(cache_write_tokens);
                    turn.model_id = Some(model_id);
                }
                BackendEvent::Finished {
                    turn: finished_turn, ..
                } => {
                    turn = finished_turn;
                }
                BackendEvent::Failed { error, .. } => {
                    anyhow::bail!("LLM request failed: {}", error);
                }
                _ => {}
            }
        }

        Ok(turn)
    }

    /// Execute tool calls from an LLM turn.
    async fn execute_tool_calls(
        &mut self,
        request_id: u64,
        tool_calls: &[ToolCall],
    ) -> Result<()> {
        for tool_call in tool_calls {
            if self.cancel_token.is_cancelled() {
                break;
            }

            let result = ToolExecutionResult::new(format!(
                "Executed tool '{}' (standalone mode)",
                tool_call.name
            ));

            let _ = self.event_tx.send(BackendEvent::ToolCompleted {
                request_id,
                tool_call: tool_call.clone(),
                result: result.clone(),
            });

            // Persist tool result
            let result_msg = Message::new(MessageRole::Tool, result.output.clone());
            self.conversation.push(result_msg.clone());
            let store = self.store.lock().await;
            store.append_message(self.session_id, &result_msg)?;
        }

        Ok(())
    }
}

/// Convert an AssistantTurn into a Message for persistence.
fn assistant_turn_to_message(turn: &AssistantTurn) -> Message {
    let mut msg = Message::new(MessageRole::Assistant, &turn.content);
    if !turn.tool_calls.is_empty() {
        msg.tool_calls = turn.tool_calls.clone();
    }
    if !turn.reasoning.is_empty() {
        msg.reasoning = turn.reasoning.clone();
    }
    msg
}
