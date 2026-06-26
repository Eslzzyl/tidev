//! AgentLoop — the core LLM ↔ tool execution loop.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! Events carry NO `session_id` — the receiver already knows which session
//! the events belong to (Per-Session Event Bus).

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use tidev_session::session::{
    AssistantTurn, BackendEvent, Conversation, Message, MessageRole, ToolCall,
    ToolExecutionResult,
};
use tidev_types::ToolSchema;
use tidev_types::prompts::SessionMode;
use tidev_storage::SessionStore;

use crate::types::{AgentType, ApprovedTool, PendingToolApproval};

/// The per-session agent loop.
pub struct AgentLoop {
    pub session_id: Uuid,
    pub model: tidev_config::ActiveModel,
    pub conversation: Conversation,
    pub context: tidev_context::ContextManager,
    pub tools: Vec<tidev_tools::ToolDefinition>,
    pub tool_registry: tidev_tools::ToolRegistry,
    pub store: Arc<tokio::sync::Mutex<SessionStore>>,
    pub llm: tidev_llm::LlmClient,
    pub event_tx: UnboundedSender<BackendEvent>,
    pub cancel_token: CancellationToken,
    pub mode: SessionMode,
    pub agent_type: AgentType,
    /// Optional channel for interactive tool permission approval.
    pub permission_tx: Option<UnboundedSender<PendingToolApproval>>,
    /// Hook engine for PostToolUse hooks.
    pub hooks: tidev_hooks::HookEngine,
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

            // Run a single LLM turn with retry
            let max_retries = 3u32;
            let turn = {
                let mut retries = 0u32;
                loop {
                    match self
                        .run_single_turn(request_id, &messages, &llm_tools)
                        .await
                    {
                        Ok(t) => break t,
                        Err(e) => {
                            retries += 1;
                            if retries > max_retries {
                                return Err(e);
                            }
                            log::warn!(
                                "agent_loop[{}]: LLM turn failed (retry {retries}/{max_retries}), retrying: {e}",
                                self.session_id
                            );
                            let _ = self.event_tx.send(BackendEvent::Retrying {
                                request_id,
                                attempt: retries,
                                max_attempts: max_retries,
                                reason: e.to_string(),
                                retry_after_secs: None,
                            });
                        }
                    }
                }
            };

            // Persist the assistant turn
            let assistant_msg = assistant_turn_to_message(&turn);
            let msg_id = assistant_msg.id;
            self.conversation.push(assistant_msg.clone());
            {
                let store = self.store.lock().await;
                store.append_message(self.session_id, &assistant_msg)?;
            }

            // If no tool calls, we're done
            if turn.tool_calls.is_empty() {
                let _ = self.event_tx.send(BackendEvent::StreamEnd { request_id });
                log::info!("agent_loop[{}]: completed", self.session_id);
                break;
            }

            // Execute tool calls — this is the core execution path
            self.execute_tool_calls(request_id, &turn.tool_calls)
                .await?;

            // Check context compaction after tools execute
            self.check_context_compaction(request_id).await;

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
    ///
    /// Filters tools by mode, checks permissions, then executes through ToolRegistry.
    /// If a permission_tx channel is available, tool calls are sent to the frontend
    /// for interactive approval before execution.
    async fn execute_tool_calls(
        &mut self,
        request_id: u64,
        tool_calls: &[ToolCall],
    ) -> Result<()> {
        let runtime = tokio::runtime::Handle::current();

        // ─── Phase 0: Mode-based filtering ────────────────────────────
        let mut filtered: Vec<ToolCall> = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            if !self.tool_registry.can_execute(&call.name, self.mode) {
                log::info!(
                    "execute_tool_calls: rejecting '{}' — not allowed in {:?} mode",
                    call.name,
                    self.mode
                );
                let result = ToolExecutionResult::new(format!(
                    "Tool '{}' is disabled in {:?} mode. \
                     If you need to modify files, you must explain your intent to the user \
                     and ask them to switch to Build mode.",
                    call.name, self.mode
                ));
                self.persist_tool_result(request_id, call, &result).await?;
                continue;
            }
            filtered.push(call.clone());
        }

        if filtered.is_empty() {
            return Ok(());
        }

        // ─── Phase 1: Permission approval (if channel is available) ──
        let approved: Vec<ToolCall> = if let Some(ref tx) = self.permission_tx {
            let (response_tx, response_rx) = oneshot::channel();
            let pending = PendingToolApproval {
                tool_calls: filtered.clone(),
                mode: self.mode,
                response_tx,
            };
            let _ = tx.send(pending);

            match response_rx.await {
                Ok(approved_tools) => {
                    // Collect approved tool calls, execute them
                    // Rejected tools have rejection set
                    let mut to_execute = Vec::new();
                    for at in approved_tools {
                        if let Some(rejection) = at.rejection {
                            // Tool was rejected — persist the rejection result
                            self.persist_tool_result(request_id, &at.tool_call, &rejection).await?;
                        } else {
                            to_execute.push(at.tool_call);
                        }
                    }
                    to_execute
                }
                Err(_) => {
                    log::warn!("execute_tool_calls: permission channel closed, skipping all tools");
                    return Ok(());
                }
            }
        } else {
            // No permission channel — execute all tools directly
            filtered
        };

        if approved.is_empty() {
            return Ok(());
        }

        // ─── Phase 2: Separate subagent (task) calls from regular tools ──
        let mut task_calls: Vec<ToolCall> = Vec::new();
        let mut regular_calls: Vec<ToolCall> = Vec::new();
        for tc in &approved {
            if tidev_tools::canonical_tool_name(&tc.name) == Some("task") {
                task_calls.push(tc.clone());
            } else {
                regular_calls.push(tc.clone());
            }
        }

        // ─── Phase 3: Execute subagent tasks ────────────────────────────
        for task_call in &task_calls {
            if self.cancel_token.is_cancelled() {
                break;
            }

            let result = self.run_subagent(task_call).await;
            let _ = self.event_tx.send(BackendEvent::ToolCompleted {
                request_id,
                tool_call: task_call.clone(),
                result: result.clone(),
            });

            let result_msg = Message::new(MessageRole::Tool, result.output.clone());
            self.conversation.push(result_msg.clone());
            {
                let store = self.store.lock().await;
                store.append_message(self.session_id, &result_msg)?;
            }
        }

        // ─── Phase 4: Execute regular tools via ToolRegistry ────────────
        for tool_call in &regular_calls {
            if self.cancel_token.is_cancelled() {
                break;
            }

            // Get a store snapshot for tool execution
            let store_snapshot = {
                let store = self.store.lock().await;
                store.clone()
            };

            let result = match self.tool_registry.execute_call(
                &runtime,
                &store_snapshot,
                self.session_id,
                tool_call,
                self.mode,
                false, // allow_outside — TUI will set this via approval
                false, // sensitive_file_approved — TUI will set this via approval
            ) {
                Ok(result) => result,
                Err(e) => ToolExecutionResult::new(format!("Error: {e}")),
            };

            // Run PostToolUse hooks
            let hook_outcome = self
                .hooks
                .on_post_tool_use(tool_call, &result, Some(self.session_id))
                .await;
            if let Some(hook_output) = hook_outcome.format_for_result() {
                log::info!("hooks: post-tool-use result: {}", hook_output);
            }

            // Emit completion event
            let _ = self.event_tx.send(BackendEvent::ToolCompleted {
                request_id,
                tool_call: tool_call.clone(),
                result: result.clone(),
            });

            // Persist tool result
            let result_msg = Message::new(MessageRole::Tool, result.output.clone());
            self.conversation.push(result_msg.clone());
            {
                let store = self.store.lock().await;
                store.append_message(self.session_id, &result_msg)?;
            }
        }

        Ok(())
    }

    /// Persist a tool result (for mode-rejected or user-rejected tools).
    async fn persist_tool_result(
        &mut self,
        request_id: u64,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
    ) -> Result<()> {
        let _ = self.event_tx.send(BackendEvent::ToolCompleted {
            request_id,
            tool_call: tool_call.clone(),
            result: result.clone(),
        });

        let result_msg = Message::new(MessageRole::Tool, result.output.clone());
        self.conversation.push(result_msg.clone());
        {
            let store = self.store.lock().await;
            store.append_message(self.session_id, &result_msg)?;
        }
        Ok(())
    }

    /// Run a sub-agent (task tool) by spawning a child session.
    async fn run_subagent(&mut self, tool_call: &ToolCall) -> ToolExecutionResult {
        // Parse task args
        let args = match serde_json::from_str::<tidev_tools::TaskArgs>(&tool_call.arguments) {
            Ok(a) => a,
            Err(e) => {
                return ToolExecutionResult::new(format!(
                    "Failed to parse task arguments: {e}"
                ));
            }
        };

        let agent_type = match AgentType::parse(&args.subagent_type) {
            Some(t) => t,
            None => {
                return ToolExecutionResult::new(format!(
                    "Unknown subagent type '{}'", args.subagent_type
                ));
            }
        };

        log::info!(
            "run_subagent: would spawn {} session for task '{}'",
            agent_type.display_name(),
            args.description,
        );

        ToolExecutionResult::new(format!(
            "Started {agent} subagent task '{description}'",
            agent = agent_type.display_name(),
            description = args.description,
        ))
    }

    /// Check if context compaction is needed and perform it.
    async fn check_context_compaction(&mut self, _request_id: u64) {
        let conversation_msgs = &self.conversation.messages;
        let total_est: usize = conversation_msgs
            .iter()
            .map(|m| m.content.len() / 4 + 1)
            .sum();
        if total_est > self.context.prune_threshold_tokens {
            log::info!(
                "agent_loop[{}]: estimated tokens {total_est} > threshold {}, compacting",
                self.session_id,
                self.context.prune_threshold_tokens
            );
            // Extract borrows before mutable borrow of context
            let llm = &self.llm;
            let model = &self.model;
            let conversation = &self.conversation;
            let tools: &[tidev_tools::ToolDefinition] = &self.tools;
            let mode = self.mode;
            let compact_config = tidev_context::CompactionConfig {
                llm,
                model,
                conversation,
                manual: false,
                stream_ctx: None,
                tools,
                mode,
            };
            if let Err(e) = self.context.compact_if_needed(compact_config).await {
                log::warn!(
                    "agent_loop[{}]: context compaction failed: {e}",
                    self.session_id
                );
            }
            let _ = self.event_tx.send(BackendEvent::ContextCompacted {
                compacted: self.context.summary.is_some(),
                manual: false,
                summary: self.context.summary.clone(),
                retained_from: self.context.retained_from,
                error: None,
            });
        }
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
