//! Subagent delegation — running child agents via the `task` tool.
//!
//! Subagents run in dedicated child sessions with their own tool sets,
//! system prompts, and message history.  Results are returned as
//! [`ToolExecutionResult`] so they integrate seamlessly with the
//! parent session's tool execution pipeline.

use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use tidev_session::session::{
    AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};

use crate::agent::AgentType;
use crate::config::reasoning::ThinkingLevelType;
use crate::context::ContextManager;
use crate::tooling::{ToolDefinition, canonical_tool_name};

use super::{AgentRuntime, SubagentConfig};

/// Helper: returns a future that resolves when the cancellation token fires.
/// If the token is `None`, returns a future that never resolves.
async fn cancel_token_cancelled(ct: &Option<CancellationToken>) {
    match ct {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

impl AgentRuntime {
    /// Run a sub-agent (`task` tool) in a dedicated child session.
    ///
    /// This method **owns `self`** (a clone created by `execute_tool_calls`)
    /// so it can call `run_agent_loop_with_tools` which needs `&mut self`.
    ///
    /// 1. Parse the [`TaskArgs`] from the tool call
    /// 2. Create a child session (with parent reference, copied permissions)
    /// 3. Filter tools per the subagent's [`AgentDefinition`]
    /// 4. Run `run_agent_loop_with_tools` for the child
    /// 5. Return the last assistant message content
    ///
    /// All panics/errors are caught and returned as a [`ToolExecutionResult`]
    /// so this never throws from the perspective of `execute_tool_calls`.
    pub async fn run_subagent(mut self, config: SubagentConfig) -> ToolExecutionResult {
        let result = self
            .run_subagent_inner(
                config.parent_session_id,
                config.parent_request_id,
                &config.tool_call,
                &config.event_tx,
                config.cancel_token,
                &config.parent_model,
                config.child_session_id,
            )
            .await;
        match result {
            Ok(output) => ToolExecutionResult::new(output),
            Err(e) => ToolExecutionResult::new(format!("Subagent failed: {e}")),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_subagent_inner(
        &mut self,
        parent_session_id: uuid::Uuid,
        parent_request_id: u64,
        tool_call: &ToolCall,
        event_tx: &UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        parent_model: &crate::config::ActiveModel,
        child_session_id: Option<uuid::Uuid>,
    ) -> anyhow::Result<String> {
        use crate::tooling::TaskArgs;

        // 1. Parse tool call arguments
        let args = serde_json::from_str::<TaskArgs>(&tool_call.arguments)?;
        let description = args.description.trim().to_string();
        let prompt = args.prompt.trim().to_string();
        let subagent_type = args.subagent_type.trim();
        if subagent_type.is_empty() {
            anyhow::bail!(
                "subagent_type is required: specify one of explorer, librarian, oracle, designer, fixer"
            );
        }
        let agent_type = AgentType::parse(subagent_type).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown subagent type '{subagent_type}': expected one of explorer, librarian, oracle, designer, fixer"
            )
        })?;
        let agent_def = crate::agent::AgentDefinition::new(agent_type);

        let child_session_id = child_session_id.unwrap_or_else(uuid::Uuid::new_v4);

        // Helper to emit SubagentStatus events to BOTH parent and child sessions.
        // The child-session event updates the subsession conversation in the TUI;
        // the parent-session event updates the subagent card overlay.
        //
        // The child session event uses `child_request_id` (the child's per-turn
        // `request_sequence`) so that the TUI's `is_active_request` check passes —
        // the child session's `active_request_id` is set to `request_sequence` by
        // `TurnStarting`.  The parent session event keeps `parent_request_id`.
        let send_status = |event_tx: &UnboundedSender<BackendEvent>,
                           child_request_id: u64,
                           status_text: String,
                           current_tool_call: Option<ToolCall>,
                           content_delta: Option<String>,
                           reasoning_delta: Option<String>| {
            // Send to child session (for subsession conversation view)
            let _ = event_tx.send(BackendEvent::SubagentStatus {
                session_id: child_session_id,
                request_id: child_request_id,
                child_session_id,
                status_text: status_text.clone(),
                current_tool_call: current_tool_call.clone(),
                assistant_message: None,
                content_delta: content_delta.clone(),
                reasoning_delta: reasoning_delta.clone(),
            });
            // Send to parent session (for subagent card in main conversation)
            let _ = event_tx.send(BackendEvent::SubagentStatus {
                session_id: parent_session_id,
                request_id: parent_request_id,
                child_session_id,
                status_text,
                current_tool_call,
                assistant_message: None,
                content_delta,
                reasoning_delta,
            });
        };

        // Use agent's model override if set, else inherit parent model.
        let child_model = {
            let agent_type_name = agent_type.display_name();
            match self
                .config
                .resolve_agent_active_model(&self.auth, agent_type_name)
            {
                Ok(Some(model)) => model,
                _ => {
                    let mut m = agent_def
                        .model_override
                        .clone()
                        .unwrap_or_else(|| parent_model.clone());
                    m.system_prompt = agent_def.system_prompt.clone();
                    m.thinking_level = ThinkingLevelType::default();
                    m
                }
            }
        };

        // 2. Create child session
        {
            let store = self.store.lock().await;
            let agent_label = agent_type.display_name();
            let child_title = format!("Task ({agent_label}): {description}");

            store.create_session_with_parent(
                child_session_id,
                parent_session_id,
                &self.workspace_root,
                &child_model.provider_id,
                &child_model.provider_display_name,
                &child_model.model_id,
                &child_model.display_name,
                &child_title,
            )?;
            store.copy_tool_permissions(parent_session_id, child_session_id)?;

            let bootstrap = Message::new(MessageRole::System, agent_def.bootstrap_content());
            store.append_message(child_session_id, &bootstrap)?;
            let user_msg = Message::new(MessageRole::User, &prompt);
            store.append_message(child_session_id, &user_msg)?;
        }

        // Sync tools to the child model so apply_patch vs write/edit filtering
        // (all_definitions → use_apply_patch) uses the correct model for the
        // subagent, not the parent's model.
        self.tools.set_active_model(child_model.clone());

        // 3. Filter tools based on agent definition
        let all_tools = self.tool_definitions();
        let tools: Vec<ToolDefinition> = if let Some(allowed) = &agent_def.allowed_tools {
            all_tools
                .into_iter()
                .filter(|def| {
                    allowed.contains(&def.name)
                        || matches!(canonical_tool_name(&def.name), Some("question"))
                })
                .collect()
        } else {
            all_tools
        };

        let child_context = ContextManager::new();
        let child_thinking = child_model.thinking_level.clone();
        let mut request_sequence: u64 = rand::random();
        let tools: Vec<ToolDefinition> = tools.into_iter().filter(|t| t.name != "task").collect();

        send_status(
            event_tx,
            request_sequence,
            format!("Thinking ({})", agent_type.display_name()),
            None,
            None,
            None,
        );

        // Compose the STATIC system prompt ONCE for the subagent
        let static_system_prompt = self.compose_static_system_prompt(&child_model.system_prompt);

        // ─── Turn loop ─────────────────────────────────────────────────
        loop {
            // Check cancellation
            if let Some(ref ct) = cancel_token
                && ct.is_cancelled()
            {
                log::info!("run_subagent: cancelled");
                return Ok(String::new());
            }

            // Build request messages
            let conversation = {
                let store = self.store.lock().await;
                store.load_conversation(child_session_id)?
            };

            let request_messages = if let Some(ref conv) = conversation {
                child_context.build_request_messages(conv, tidev_types::prompts::SessionMode::Build)
            } else {
                anyhow::bail!("Child session conversation not found");
            };

            if request_messages.is_empty() {
                break;
            }

            // Compose full system prompt
            let mut model_for_turn = child_model.clone();
            model_for_turn.system_prompt = static_system_prompt.clone();

            send_status(event_tx, request_sequence, "Thinking".to_string(), None, None, None);

            // Emit TurnStarting
            let _ = event_tx.send(BackendEvent::TurnStarting {
                session_id: child_session_id,
                request_id: request_sequence,
            });

            // ─── Custom streaming loop ─────────────────────────────────
            use tokio::sync::mpsc::unbounded_channel;
            let (stream_tx, mut stream_rx) = unbounded_channel();

            let llm = self.llm_client.clone();
            let model_for_task = model_for_turn.clone();
            let msgs = request_messages.clone();
            let tl = child_thinking.clone();
            let stream_req_id = request_sequence;
            let tools_for_spawn = tools.clone();
            let stream_tx_for_llm = stream_tx.clone();

            tokio::spawn(async move {
                let llm_config = tidev_llm::LlmProviderConfig::from(model_for_task);
                let llm_tools: Vec<tidev_llm::ToolDefinition> = tools_for_spawn
                    .iter()
                    .map(tidev_llm::ToolDefinition::from)
                    .collect();
                llm.stream_chat(
                    child_session_id,
                    stream_req_id,
                    llm_config,
                    msgs,
                    llm_tools,
                    stream_tx_for_llm,
                    tl,
                )
                .await;
            });

            let mut turn = AssistantTurn::default();
            let call_start = Utc::now();
            let mut turn_has_content = false;

            // Guard against the spawned LLM task hanging or completing
            // silently without sending any terminal event (Finished / Failed).
            // If no event arrives within the timeout, we bail out so the
            // main agent does not block forever on this subagent.
            const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
            let idle_timeout = tokio::time::sleep(STREAM_IDLE_TIMEOUT);
            tokio::pin!(idle_timeout);

            loop {
                tokio::select! {
                    biased;

                    event = stream_rx.recv() => {
                        let Some(event) = event else {
                            // Channel closed — all senders dropped.
                            break;
                        };

                        // Reset the idle timeout on every received event.
                        idle_timeout.as_mut().reset(
                            tokio::time::Instant::now() + STREAM_IDLE_TIMEOUT,
                        );

                        let _ = event_tx.send(event.clone());

                        match event {
                            BackendEvent::Delta { content, .. } => {
                                if turn.created_at.is_none() {
                                    turn.created_at = Some(Utc::now());
                                }
                                turn.content.push_str(&content);
                                if !turn_has_content {
                                    turn_has_content = true;
                                }
                                send_status(
                                    event_tx,
                                    request_sequence,
                                    "Writing output".to_string(),
                                    None,
                                    Some(content),
                                    None,
                                );
                            }
                            BackendEvent::ReasoningDelta { content, .. } => {
                                if turn.created_at.is_none() {
                                    turn.created_at = Some(Utc::now());
                                }
                                turn.reasoning.push_str(&content);
                                send_status(event_tx, request_sequence, "Thinking".to_string(), None, None, Some(content));
                            }
                            BackendEvent::ToolCallUpdated { tool_call, .. } => {
                                let tc = tool_call.clone();
                                turn.upsert_tool_call(tool_call);
                                send_status(event_tx, request_sequence, "Tool".to_string(), Some(tc), None, None);
                            }
                            BackendEvent::UsageStats {
                                input_tokens,
                                output_tokens,
                                total_tokens,
                                cache_read_tokens,
                                cache_write_tokens,
                                model_id,
                                duration_ms,
                                ..
                            } => {
                                turn.input_tokens = Some(input_tokens);
                                turn.output_tokens = Some(output_tokens);
                                turn.total_tokens = Some(total_tokens);
                                turn.cache_read_tokens = Some(cache_read_tokens);
                                turn.cache_write_tokens = Some(cache_write_tokens);
                                turn.model_id = Some(model_id.clone());
                                turn.tokens_per_second = duration_ms.and_then(|ms| {
                                    if ms > 0 {
                                        Some(output_tokens as f32 / (ms as f32 / 1000.0))
                                    } else {
                                        None
                                    }
                                });
                            }
                            BackendEvent::Finished {
                                turn: finished_turn,
                                ..
                            } => {
                                let saved_tokens = (
                                    turn.input_tokens,
                                    turn.output_tokens,
                                    turn.total_tokens,
                                    turn.cache_read_tokens,
                                    turn.cache_write_tokens,
                                    turn.model_id.clone(),
                                    turn.tokens_per_second,
                                );
                                let saved_created_at = turn.created_at;
                                turn = finished_turn;
                                if turn.input_tokens.is_none() {
                                    turn.input_tokens = saved_tokens.0;
                                    turn.output_tokens = saved_tokens.1;
                                    turn.total_tokens = saved_tokens.2;
                                    turn.cache_read_tokens = saved_tokens.3;
                                    turn.cache_write_tokens = saved_tokens.4;
                                    turn.model_id = saved_tokens.5;
                                    turn.tokens_per_second = saved_tokens.6;
                                }
                                turn.created_at = saved_created_at.or(Some(call_start));
                                turn.completed_at = Some(Utc::now());
                                break;
                            }
                            BackendEvent::Failed { error, .. } => {
                                return Err(anyhow::anyhow!("Subagent LLM Error: {}", error));
                            }
                            _ => {}
                        }
                    }
                    // Respond to cancellation immediately instead of waiting
                    // for the LLM stream to finish or the idle timeout to fire.
                    _ = cancel_token_cancelled(&cancel_token) => {
                        log::info!("run_subagent: cancelled during LLM stream");
                        anyhow::bail!("Subagent cancelled");
                    }
                    _ = &mut idle_timeout => {
                        return Err(anyhow::anyhow!(
                            "Subagent LLM stream idle timeout ({}s) — no event received",
                            STREAM_IDLE_TIMEOUT.as_secs(),
                        ));
                    }
                }
            }

            // Verify we got a complete turn
            if turn.completed_at.is_none() {
                anyhow::bail!("Subagent stream ended without a final turn");
            }

            // Persist assistant message and reuse its ID for the frontend
            // event so the TUI's SubagentStatus handler finds the existing
            // DB-loaded message (by ID) and updates it in place instead of
            // pushing a duplicate.
            let persisted_msg = self
                .persist_assistant_message(child_session_id, &turn)
                .await?;

            // Send assistant message to subsession conversation
            {
                let mut assistant_msg = Message::persisted(
                    persisted_msg.id,
                    MessageRole::Assistant,
                    &turn.content,
                    persisted_msg.created_at,
                    false,
                );
                assistant_msg.completed_at = persisted_msg.completed_at;
                assistant_msg.reasoning = turn.reasoning.clone();
                assistant_msg.tool_calls = turn.tool_calls.clone();
                assistant_msg.input_tokens = turn.input_tokens;
                assistant_msg.output_tokens = turn.output_tokens;
                assistant_msg.total_tokens = turn.total_tokens;
                assistant_msg.cache_read_tokens = turn.cache_read_tokens;
                assistant_msg.cache_write_tokens = turn.cache_write_tokens;
                assistant_msg.model_id = turn.model_id.clone();
                assistant_msg.tokens_per_second = turn.tokens_per_second;
                let _ = event_tx.send(BackendEvent::SubagentStatus {
                    session_id: child_session_id,
                    request_id: request_sequence,
                    child_session_id,
                    status_text: if turn.tool_calls.is_empty() {
                        "Completed".to_string()
                    } else {
                        "Tool".to_string()
                    },
                    current_tool_call: None,
                    assistant_message: Some(assistant_msg),
                    content_delta: None,
                    reasoning_delta: None,
                });
            }

            // If no tool calls, done
            if turn.tool_calls.is_empty() {
                send_status(event_tx, request_sequence, "Completed".to_string(), None, None, None);
                break;
            }

            // Execute tools
            'tool_loop: for tool_call in &turn.tool_calls {
                // Reject phantom "task" tool calls
                if tool_call.name == "task" || canonical_tool_name(&tool_call.name) == Some("task")
                {
                    log::info!(
                        "run_subagent: rejecting phantom '{}' call from subagent LLM",
                        tool_call.name
                    );
                    let result = ToolExecutionResult::new(format!(
                        "Tool '{}' is not available in subagent context. \
                         Subagents cannot delegate further tasks.",
                        tool_call.name
                    ));
                    self.persist_tool_result(
                        child_session_id,
                        request_sequence,
                        tool_call,
                        &result,
                        event_tx,
                    )
                    .await?;
                    // Forward the tool result to the child session view in TUI.
                    // Use request_sequence (not parent_request_id) so the TUI's
                    // is_active_request check passes — TurnStarting sets the
                    // child session's active_request_id to request_sequence.
                    let tool_msg =
                        Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
                    let _ = event_tx.send(BackendEvent::SubagentToolResult {
                        session_id: child_session_id,
                        request_id: request_sequence,
                        child_session_id,
                        message: tool_msg,
                    });
                    continue 'tool_loop;
                }

                let canonical = canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
                let summary = format!("Tool: {canonical}");
                send_status(event_tx, request_sequence, summary, Some(tool_call.clone()), None, None);

                let call_with_allow = [(tool_call.clone(), false, false)];
                let results = self
                    .execute_tool_calls(
                        child_session_id,
                        request_sequence,
                        &call_with_allow,
                        tidev_types::prompts::SessionMode::Build,
                        event_tx,
                        &child_model,
                        cancel_token.clone(),
                    )
                    .await?;

                for (_, result) in results {
                    // Forward the tool result to the child session view in TUI.
                    // execute_tool_calls already called persist_tool_result which
                    // emits ToolCompleted with request_sequence (ignored by TUI).
                    // Use request_sequence here so is_active_request passes.
                    let tool_msg =
                        Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
                    let _ = event_tx.send(BackendEvent::SubagentToolResult {
                        session_id: child_session_id,
                        request_id: request_sequence,
                        child_session_id,
                        message: tool_msg,
                    });
                }

                send_status(event_tx, request_sequence, "Working".to_string(), None, None, None);
            }

            // Next turn
            request_sequence = rand::random();
        }

        // Return the final assistant message content
        let store = self.store.lock().await;
        let messages = store.load_messages(child_session_id)?;
        drop(store);

        let last_assistant = messages
            .into_iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant && !m.streaming);

        Ok(last_assistant.map(|m| m.content).unwrap_or_default())
    }
}
