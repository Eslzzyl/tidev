//! Agent loop — the core LLM ↔ tool execution loop.
//!
//! This module contains the methods that drive the main agent loop:
//! streaming LLM turns, executing tool calls, and iterating until the
//! model produces a final response.

use std::sync::atomic::AtomicBool;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};
use tokio_util::sync::CancellationToken;

use tidev_session::session::{AssistantTurn, BackendEvent, Message, ToolCall, ToolExecutionResult};
use tidev_types::prompts::SessionMode;

use tidev_types::{Goal, GoalStatus};

use crate::config::{ActiveModel, reasoning::ThinkingLevelType};
use crate::context::ContextManager;
use crate::tooling::{ToolDefinition, canonical_tool_name};

use super::AgentRuntime;

// ── Continuation prompt construction ────────────────────────────────────────

/// Build the goal continuation prompt that is injected as a User message.
/// The prompt is wrapped in `<goal_context>` markers so the UI can recognise
/// and hide it.
fn build_goal_prompt(goal: &Goal) -> String {
    format!(
        "<goal_context>\n\
         Continue working toward the active thread goal.\n\n\
         <objective>\n{objective}\n</objective>\n\n\
         Continuation behavior:\n\
         - This goal persists across turns. Ending this turn does not \
         require finishing everything now.\n\
         - Keep the full objective intact. If it cannot be finished now, \
         make concrete progress.\n\
         - Do not redefine success around a smaller or easier task than \
         what is requested.\n\n\
         Completion audit:\n\
         Before deciding the goal is achieved, treat completion as unproven \
         and verify against the actual current state:\n\
         - Derive concrete requirements from the objective.\n\
         - Preserve the original scope; do not redefine success around \
         work that already exists.\n\
         - For every requirement, identify authoritative evidence that \
         would prove it, then inspect the relevant current-state sources.\n\
         - If any requirement lacks proof, the goal is not complete — \
         continue working.\n\n\
         Task decomposition:\n\
         If this objective is large, consider using the task tool to spawn \
         sub-agents for independent sub-tasks. Each sub-agent works in its \
         own context, keeping the main context focused on orchestration.\n\n\
         Resource usage:\n\
         - Tokens used: {tokens_used}\n\
         - Time spent: {time_used_seconds} seconds\n\
         </goal_context>",
        objective = goal.objective,
        tokens_used = goal.tokens_used,
        time_used_seconds = goal.time_used_seconds,
    )
}

// ── Single-turn streaming ──────────────────────────────────────────────────

impl AgentRuntime {
    /// Run a single LLM streaming turn.
    ///
    /// Spawns the LLM streaming task, forwards all [`BackendEvent`]s to
    /// `event_tx` in real time, and returns the final [`AssistantTurn`]
    /// when the LLM finishes.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_single_turn(
        &self,
        session_id: uuid::Uuid,
        request_id: u64,
        model: ActiveModel,
        messages: Vec<Message>,
        tools: Vec<ToolDefinition>,
        thinking_level: ThinkingLevelType,
        event_tx: &UnboundedSender<BackendEvent>,
    ) -> Result<AssistantTurn> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let llm = self.llm_client.clone();
        let model_for_task = model.clone();
        let msgs = messages.clone();
        let tl = thinking_level.clone();
        let _t_spawn = std::time::Instant::now();
        tokio::spawn(async move {
            let llm_config = tidev_llm::LlmProviderConfig::from(model_for_task);
            let llm_tools: Vec<tidev_llm::ToolDefinition> =
                tools.iter().map(tidev_llm::ToolDefinition::from).collect();
            llm.stream_chat(session_id, request_id, llm_config, msgs, llm_tools, tx, tl)
                .await;
        });

        let mut turn = AssistantTurn::default();
        let call_start = Utc::now();
        let mut first_event = true;

        while let Some(event) = rx.recv().await {
            if first_event {
                log::info!(
                    "run_single_turn: first event received after {:?} from spawn",
                    _t_spawn.elapsed()
                );
                first_event = false;
            }
            // Forward to consumer first
            let _ = event_tx.send(event.clone());

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
                    // Preserve token data accumulated from UsageStats event.
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
                    // Restore timestamps (finished_turn has None from Default::default())
                    turn.created_at = saved_created_at.or(Some(call_start));
                    turn.completed_at = Some(Utc::now());
                    break;
                }
                BackendEvent::Failed { error, .. } => {
                    return Err(anyhow::anyhow!("LLM Error: {}", error));
                }
                _ => {}
            }
        }

        Ok(turn)
    }

    /// Execute tool calls from an [`AssistantTurn`].
    ///
    /// Persists tool-result messages to the database and emits
    /// [`BackendEvent::ToolCompleted`] for each executed tool.
    ///
    /// Execution order:
    /// 1. **Read-only** tools in parallel (catch_unwind protected)
    /// 2. **Subagent** (`task`) tools in parallel — each runs a nested
    ///    `run_agent_loop` with its own cloned [`AgentRuntime`]
    /// 3. **Write** tools serially (catch_unwind protected)
    /// 4. All results persisted sequentially
    ///
    /// Each entry is `(tool_call, allow_outside, sensitive_file_approved)`.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_tool_calls(
        &mut self,
        session_id: uuid::Uuid,
        request_id: u64,
        tool_calls: &[(ToolCall, bool, bool)],
        mode: SessionMode,
        event_tx: &UnboundedSender<BackendEvent>,
        _parent_model: &crate::config::ActiveModel,
        cancel_token: Option<CancellationToken>,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        let runtime = tokio::runtime::Handle::current();
        let mut results: Vec<(ToolCall, ToolExecutionResult)> =
            Vec::with_capacity(tool_calls.len());

        // ─── Phase 0: Mode-based + confirmation filtering ────────────
        let mut filtered: Vec<(&ToolCall, bool, bool)> = Vec::with_capacity(tool_calls.len());
        for (call, allow_outside, sensitive_file_approved) in tool_calls {
            if !self.tools.can_execute(&call.name, mode) {
                log::info!(
                    "execute_tool_calls: rejecting '{}' — not allowed in {:?} mode",
                    call.name,
                    mode
                );
                let result = ToolExecutionResult::new(format!(
                    "Tool '{}' is disabled in {:?} mode",
                    call.name, mode
                ));
                self.persist_tool_result(session_id, request_id, call, &result, event_tx)
                    .await?;
                results.push((call.clone(), result));
                continue;
            }

            // Reject phantom "task" tool calls — the task tool is only
            // handled by the main agent loop's special path.
            if call.name == "task" || canonical_tool_name(&call.name) == Some("task") {
                log::info!(
                    "execute_tool_calls: rejecting '{}' — not allowed through execute_tool_calls",
                    call.name
                );
                let result = ToolExecutionResult::new(format!(
                    "Tool '{}' is not available in this context",
                    call.name
                ));
                self.persist_tool_result(session_id, request_id, call, &result, event_tx)
                    .await?;
                results.push((call.clone(), result));
                continue;
            }

            if !self.auto_approve_permissions
                && let Some(def) = self.tools.definition_for(&call.name)
                && def.needs_confirmation()
            {
                log::info!(
                    "execute_tool_calls: rejecting '{}' — needs confirmation and auto_approve is off",
                    call.name
                );
                let result = ToolExecutionResult::new(format!(
                    "Tool '{}' requires user approval in this mode",
                    call.name
                ));
                self.persist_tool_result(session_id, request_id, call, &result, event_tx)
                    .await?;
                results.push((call.clone(), result));
                continue;
            }

            filtered.push((call, *allow_outside, *sensitive_file_approved));
        }

        // All tools filtered out — return early
        if filtered.is_empty() {
            return Ok(results);
        }

        // Separate tool calls by execution strategy.
        let mut read_only: Vec<(&ToolCall, bool, bool)> = Vec::new();
        let mut write: Vec<(&ToolCall, bool, bool)> = Vec::new();
        for (call, allow, sensitive_approved) in &filtered {
            if self.tools.is_read_only_call(call) {
                read_only.push((call, *allow, *sensitive_approved));
            } else {
                write.push((call, *allow, *sensitive_approved));
            }
        }

        // ─── Phase 1: Read-only tools in parallel ───────────────────────
        if !read_only.is_empty() {
            let mut stores: Vec<tidev_storage::SessionStore> = Vec::with_capacity(read_only.len());
            {
                let store = self.store.lock().await;
                for _ in 0..read_only.len() {
                    stores.push(store.clone());
                }
            }

            let mut handles: Vec<(ToolCall, tokio::task::JoinHandle<ToolExecutionResult>)> =
                Vec::with_capacity(read_only.len());
            for ((tool_call, allow_outside, sensitive_file_approved), store) in
                read_only.into_iter().zip(stores)
            {
                let handle = self.tools.execute_call_spawned(
                    runtime.clone(),
                    store,
                    session_id,
                    tool_call.clone(),
                    mode,
                    allow_outside,
                    sensitive_file_approved,
                );
                handles.push((tool_call.clone(), handle));
            }

            for (tool_call, handle) in handles {
                let mut result = handle.await.unwrap_or_else(|join_err| {
                    ToolExecutionResult::new(format!("Tool task panicked/aborted: {join_err}"))
                });

                // Pre-tool enrich: search and inject memory relevant to the
                // file being operated on (agentmemory's mem::enrich equivalent).
                if crate::agent::runtime::is_file_operation(&tool_call.name)
                    && self.config.memory.enabled
                    && self.config.memory.enrich_tools
                    && let Some(ctx) = self
                        .hooks
                        .on_pre_tool_use_enrich(&tool_call, Some(session_id))
                        .await
                {
                    result.output.push_str(&format!(
                        "\n\n<system-reminder>\n{}\n</system-reminder>",
                        ctx
                    ));
                }

                // PostToolFailure observation
                if result.sandbox_denied
                    || result.output.starts_with("Error:")
                    || result.output.starts_with("Tool task panicked")
                    || result
                        .output
                        .starts_with("Tool execution returned no result")
                {
                    self.hooks
                        .on_post_tool_failure(&tool_call, &result.output, Some(session_id));
                } else {
                    self.hooks
                        .on_post_tool_use(&tool_call, &result, Some(session_id))
                        .await;
                }

                // Persist read-only results immediately
                self.persist_tool_result(session_id, request_id, &tool_call, &result, event_tx)
                    .await?;
                results.push((tool_call, result));
            }
        }

        // ─── Phase 2: Write tools serially ──────────────────────────────
        for (tool_call, allow_outside, sensitive_file_approved) in write {
            let store = {
                let s = self.store.lock().await;
                s.clone()
            };

            // Save original sandbox policy before any elevation
            let original_policy = self.tools.sandbox_policy().cloned();

            // Bash tool calls get streaming: output is sent chunk-by-chunk
            // via ShellOutput events while the command runs.
            let is_bash =
                tool_call.name == "bash" || canonical_tool_name(&tool_call.name) == Some("bash");
            let cancelled_flag: Option<std::sync::Arc<AtomicBool>> =
                if is_bash && cancel_token.is_some() {
                    let flag = std::sync::Arc::new(AtomicBool::new(false));
                    if let Some(ref ct) = cancel_token {
                        let flag_clone = flag.clone();
                        let ct = ct.clone();
                        tokio::spawn(async move {
                            ct.cancelled().await;
                            flag_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                        });
                    }
                    Some(flag)
                } else {
                    None
                };
            let handle = if is_bash {
                self.tools.execute_call_spawned_streaming(
                    runtime.clone(),
                    store,
                    session_id,
                    tool_call.clone(),
                    mode,
                    allow_outside,
                    sensitive_file_approved,
                    event_tx.clone(),
                    cancelled_flag.clone(),
                )
            } else {
                self.tools.execute_call_spawned(
                    runtime.clone(),
                    store,
                    session_id,
                    tool_call.clone(),
                    mode,
                    allow_outside,
                    sensitive_file_approved,
                )
            };

            // Record pre-tool-use observation (file operations only)
            if crate::agent::runtime::is_file_operation(&tool_call.name) {
                self.hooks.on_pre_tool_use(tool_call, Some(session_id));
            }

            let mut result = handle.await.unwrap_or_else(|join_err| {
                ToolExecutionResult::new(format!("Tool task panicked/aborted: {join_err}"))
            });

            // Pre-tool enrich: search and inject memory relevant to the
            // file being operated on (agentmemory's mem::enrich equivalent).
            if crate::agent::runtime::is_file_operation(&tool_call.name)
                && self.config.memory.enabled
                && self.config.memory.enrich_tools
                && let Some(ctx) = self
                    .hooks
                    .on_pre_tool_use_enrich(tool_call, Some(session_id))
                    .await
            {
                result.output.push_str(&format!(
                    "\n\n<system-reminder>\n{}\n</system-reminder>",
                    ctx
                ));
            }

            // ─── Sandbox elevation  ────────────────────────────────────
            // If the tool was denied by the OS sandbox, ask the user
            // whether to retry with full filesystem access.
            if result.sandbox_denied && is_bash {
                let (tx, rx) = oneshot::channel();
                let tx_wrapper = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
                let _ = event_tx.send(BackendEvent::SandboxElevationRequest {
                    session_id,
                    request_id,
                    tool_name: tool_call.name.clone(),
                    tool_arguments: tool_call.arguments.clone(),
                    response_tx: tx_wrapper,
                });

                if rx.await.unwrap_or(false) {
                    self.tools
                        .set_sandbox_policy(Some(crate::sandbox::SandboxPolicy::DangerFullAccess));
                    let retry_store = {
                        let s = self.store.lock().await;
                        s.clone()
                    };
                    let retry = self.tools.execute_call_spawned(
                        runtime.clone(),
                        retry_store,
                        session_id,
                        tool_call.clone(),
                        mode,
                        allow_outside,
                        sensitive_file_approved,
                    );
                    result = match retry.await {
                        Ok(r) => r,
                        Err(e) => ToolExecutionResult::new(format!("Tool panicked: {e}")),
                    };
                    if let Some(policy) = original_policy {
                        self.tools.set_sandbox_policy(Some(policy));
                    } else {
                        self.tools.set_sandbox_policy(None);
                    }
                }
            }

            // ─── PostToolUse Hooks ──────────────────────────────────────
            let hook_outcome = self
                .hooks
                .on_post_tool_use(tool_call, &result, Some(session_id))
                .await;

            if let Some(formatted_msg) = hook_outcome.format_for_result() {
                result.output.push_str(&format!("\n\n{}", formatted_msg));
            }

            // Persist write result immediately so diffs render one at a time
            self.persist_tool_result(session_id, request_id, tool_call, &result, event_tx)
                .await?;
            results.push((tool_call.clone(), result));
        }

        Ok(results)
    }

    /// Run the full agent loop.
    pub async fn run_agent_loop(&mut self, config: super::AgentLoopConfig<'_>) -> Result<()> {
        let tools = self.tool_definitions();
        self.run_agent_loop_with_tools(
            config.session_id,
            config.model,
            config.context_manager,
            config.mode,
            config.thinking_level,
            tools,
            config.event_tx,
            config.cancel_token,
        )
        .await
    }

    /// Run the full agent loop with a permission approval channel.
    pub async fn run_agent_loop_with_permission_channel(
        &mut self,
        config: super::AgentLoopConfig<'_>,
        request_id: u64,
        permission_tx: tokio::sync::mpsc::UnboundedSender<super::PendingToolApproval>,
    ) -> Result<()> {
        let tools = self.tool_definitions();
        self.run_agent_loop_with_tools_inner(
            request_id,
            config.session_id,
            config.model,
            config.context_manager,
            config.mode,
            config.thinking_level,
            tools,
            config.event_tx,
            config.cancel_token,
            Some(permission_tx),
        )
        .await
    }

    /// Run the agent loop with an explicit tool list.
    #[allow(clippy::too_many_arguments)]
    async fn run_agent_loop_with_tools(
        &mut self,
        session_id: uuid::Uuid,
        model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        tools: Vec<ToolDefinition>,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
    ) -> Result<()> {
        let request_id: u64 = rand::random();
        self.run_agent_loop_with_tools_inner(
            request_id,
            session_id,
            model,
            context_manager,
            mode,
            thinking_level,
            tools,
            event_tx,
            cancel_token,
            None,
        )
        .await
    }

    /// Internal implementation with optional permission channel.
    #[allow(clippy::too_many_arguments)]
    async fn run_agent_loop_with_tools_inner(
        &mut self,
        request_id: u64,
        session_id: uuid::Uuid,
        model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        tools: Vec<ToolDefinition>,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        permission_tx: Option<tokio::sync::mpsc::UnboundedSender<super::PendingToolApproval>>,
    ) -> Result<()> {
        let mut request_id = request_id;

        // Use the static system prompt that was composed at session creation
        let static_system_prompt = model.system_prompt.clone();
        log::info!(
            "run_agent_loop: using static system prompt ({} chars)",
            static_system_prompt.len()
        );

        loop {
            // Check cancellation
            if let Some(ref ct) = cancel_token
                && ct.is_cancelled()
            {
                log::info!("run_agent_loop: cancelled");
                return Ok(());
            }

            // 1. Load messages from DB
            let _t_load = std::time::Instant::now();
            let mut db_messages = {
                let store = self.store.lock().await;
                store.load_messages(session_id)?
            };
            log::info!(
                "agent_loop: loaded {} messages in {:?}",
                db_messages.len(),
                _t_load.elapsed()
            );

            // 2. Pick up queued user message, if any
            let next_user_msg = {
                let mut queue = self.queued_messages.lock().unwrap();
                queue.pop_front()
            };

            if let Some(msg) = next_user_msg {
                let new_msg = Message::new(tidev_session::session::MessageRole::User, &msg.content);
                let store = self.store.lock().await;
                store.append_message(session_id, &new_msg)?;
                drop(store);
                // Re-load messages to include the new user message
                db_messages = {
                    let store = self.store.lock().await;
                    store.load_messages(session_id)?
                };
            }

            // 3. Inject instructions into the last user message if needed
            let has_assistant = db_messages
                .iter()
                .any(|m| m.role == tidev_session::session::MessageRole::Assistant && !m.streaming);
            let last_user_idx = db_messages
                .iter()
                .rposition(|m| m.role == tidev_session::session::MessageRole::User && !m.streaming);

            if let Some(idx) = last_user_idx {
                let last_user_ptr: *mut tidev_session::session::Message = &mut db_messages[idx];
                // SAFETY: we hold a mutable reference to db_messages and are
                //  only calling async methods that do not re-borrow db_messages.
                let last_user_msg = unsafe { &mut *last_user_ptr };
                self.inject_new_instructions(session_id, last_user_msg)
                    .await?;
                self.inject_first_turn_memory(session_id, last_user_msg, has_assistant)
                    .await?;
            }

            // 4. Build request messages via ContextManager
            let _t_build = std::time::Instant::now();
            let conversation = {
                let store = self.store.lock().await;
                store.load_conversation(session_id)?
            };

            let request_messages = if let Some(ref conv) = conversation {
                context_manager.build_request_messages(conv, mode)
            } else {
                log::warn!(
                    "agent_loop: conversation not found for session {}",
                    session_id
                );
                break Ok(());
            };

            log::info!(
                "agent_loop: built {} request messages in {:?}",
                request_messages.len(),
                _t_build.elapsed()
            );

            if request_messages.is_empty() {
                log::info!("agent_loop: no messages to send, breaking");
                break Ok(());
            }

            // 5. Stream LLM turn
            let _t_turn = std::time::Instant::now();
            let turn = self
                .run_single_turn(
                    session_id,
                    request_id,
                    model.clone(),
                    request_messages,
                    tools.clone(),
                    thinking_level.clone(),
                    &event_tx,
                )
                .await?;
            log::info!("agent_loop: turn completed in {:?}", _t_turn.elapsed());

            // No tool calls — check if we should continue for an active goal
            if turn.tool_calls.is_empty() {
                log::info!("agent_loop: no tool calls, persisting assistant message");
                self.persist_assistant_message(session_id, &turn).await?;

                // Account goal usage for this turn
                if let Some(total) = turn.total_tokens {
                    let elapsed = turn
                        .completed_at
                        .zip(turn.created_at)
                        .map(|(end, start)| (end - start).num_seconds().max(0))
                        .unwrap_or(0);
                    let store = self.store.lock().await;
                    let _ = store.account_goal_usage(session_id, total as i64, elapsed);
                    drop(store);
                }

                // If an Active goal exists, persist a continuation prompt and keep going
                if self.continue_goal_if_active(session_id).await? {
                    log::info!("agent_loop: goal active, injecting continuation and continuing");
                    request_id = rand::random::<u64>();
                    continue;
                }

                break Ok(());
            }

            // 5b. Persist assistant message (with tool calls)
            self.persist_assistant_message(session_id, &turn).await?;

            // 6. Permission approval (frontend interception)
            let mut task_calls: Vec<(ToolCall, Option<uuid::Uuid>)> = Vec::new();
            let mut other_calls: Vec<(ToolCall, bool, bool)> = Vec::new();

            if let Some(ref perm_tx) = permission_tx {
                let (resp_tx, resp_rx) = oneshot::channel();
                let _ = perm_tx.send(super::PendingToolApproval {
                    tool_calls: turn.tool_calls.clone(),
                    mode,
                    response_tx: resp_tx,
                });

                match resp_rx.await {
                    Ok(approvals) => {
                        for approved in approvals {
                            if let Some(rejection) = approved.rejection {
                                self.persist_tool_result(
                                    session_id,
                                    request_id,
                                    &approved.tool_call,
                                    &rejection,
                                    &event_tx,
                                )
                                .await?;
                            } else if approved.tool_call.name == "task" {
                                task_calls.push((approved.tool_call, approved.child_session_id));
                            } else {
                                other_calls.push((
                                    approved.tool_call,
                                    approved.allow_outside,
                                    approved.sensitive_file_approved,
                                ));
                            }
                        }
                    }
                    Err(_) => {
                        log::info!("run_agent_loop: permission channel closed, stopping");
                        break Ok(());
                    }
                }
            } else {
                // Without permission channel, route task calls directly
                for tc in &turn.tool_calls {
                    if tc.name == "task" || canonical_tool_name(&tc.name) == Some("task") {
                        task_calls.push((tc.clone(), None));
                    } else {
                        other_calls.push((tc.clone(), false, false));
                    }
                }
            }

            // Check cancellation again before executing tools
            if let Some(ref ct) = cancel_token
                && ct.is_cancelled()
            {
                log::info!("run_agent_loop: cancelled before tool execution");
                return Ok(());
            }

            // 7a. Subagent (task) tools — read-only types (Explorer, Librarian,
            // Oracle) run in parallel; write-capable types (Designer, Fixer,
            // General) run serially.
            //
            // Serial subagents run FIRST so that if one fails, no parallel
            // subagents have been spawned yet (avoids orphaned tasks).
            let mut task_handles: Vec<(
                ToolCall,
                Option<uuid::Uuid>,
                tokio::task::JoinHandle<ToolExecutionResult>,
            )> = Vec::new();

            // Serial subagents first
            for (tc, child_sid) in &task_calls {
                let is_read_only = serde_json::from_str::<crate::tooling::TaskArgs>(&tc.arguments)
                    .ok()
                    .and_then(|args| crate::agent::AgentType::parse(args.subagent_type.trim()))
                    .is_some_and(|t| t.is_read_only());

                if !is_read_only {
                    let subagent_type =
                        serde_json::from_str::<crate::tooling::TaskArgs>(&tc.arguments)
                            .ok()
                            .and_then(|args| {
                                crate::agent::AgentType::parse(args.subagent_type.trim())
                            })
                            .map(|t| format!("{t:?}"))
                            .unwrap_or_else(|| "unknown".to_string());

                    let agent = self.clone();
                    let owned_tc = tc.clone();
                    let owned_child_sid = *child_sid;
                    let tx = event_tx.clone();
                    let sid = session_id;
                    let rid = request_id;
                    let pm = model.clone();

                    log::info!(
                        "agent_loop: serial subagent [{subagent_type}] starting (task: {})",
                        tc.name,
                    );
                    let _t_sub = std::time::Instant::now();
                    let result: ToolExecutionResult = {
                        let fut: std::pin::Pin<
                            Box<dyn std::future::Future<Output = ToolExecutionResult> + Send>,
                        > = std::boxed::Box::pin(agent.run_subagent(super::SubagentConfig {
                            parent_session_id: sid,
                            parent_request_id: rid,
                            tool_call: owned_tc,
                            event_tx: tx,
                            cancel_token: cancel_token.clone(),
                            parent_model: pm,
                            child_session_id: owned_child_sid,
                        }));
                        fut.await
                    };
                    log::info!(
                        "agent_loop: serial subagent [{subagent_type}] completed in {:?}",
                        _t_sub.elapsed(),
                    );
                    self.persist_tool_result(session_id, request_id, tc, &result, &event_tx)
                        .await?;
                    let _ = event_tx.send(BackendEvent::SubagentCompleted {
                        session_id,
                        request_id,
                        tool_call: tc.clone(),
                        child_session_id: child_sid.unwrap_or_else(uuid::Uuid::new_v4),
                        result: result.clone(),
                    });
                }
            }

            // Parallel subagents second — all spawned here, all collected below
            for (tc, child_sid) in &task_calls {
                let is_read_only = serde_json::from_str::<crate::tooling::TaskArgs>(&tc.arguments)
                    .ok()
                    .and_then(|args| crate::agent::AgentType::parse(args.subagent_type.trim()))
                    .is_some_and(|t| t.is_read_only());

                if is_read_only {
                    let subagent_type =
                        serde_json::from_str::<crate::tooling::TaskArgs>(&tc.arguments)
                            .ok()
                            .and_then(|args| {
                                crate::agent::AgentType::parse(args.subagent_type.trim())
                            })
                            .map(|t| format!("{t:?}"))
                            .unwrap_or_else(|| "unknown".to_string());

                    log::info!(
                        "agent_loop: parallel subagent [{subagent_type}] spawning (task: {})",
                        tc.name,
                    );

                    let agent = self.clone();
                    let owned_tc = tc.clone();
                    let owned_child_sid = *child_sid;
                    let tx = event_tx.clone();
                    let sid = session_id;
                    let rid = request_id;
                    let pm = model.clone();
                    let ct = cancel_token.clone();

                    let handle = tokio::spawn(async move {
                        let fut: std::pin::Pin<
                            Box<dyn std::future::Future<Output = ToolExecutionResult> + Send>,
                        > = std::boxed::Box::pin(agent.run_subagent(super::SubagentConfig {
                            parent_session_id: sid,
                            parent_request_id: rid,
                            tool_call: owned_tc,
                            event_tx: tx,
                            cancel_token: ct,
                            parent_model: pm,
                            child_session_id: owned_child_sid,
                        }));
                        fut.await
                    });
                    task_handles.push((tc.clone(), *child_sid, handle));
                }
            }

            // Collect parallel task results in order
            for (tc, child_sid, handle) in task_handles {
                let subagent_type = serde_json::from_str::<crate::tooling::TaskArgs>(&tc.arguments)
                    .ok()
                    .and_then(|args| crate::agent::AgentType::parse(args.subagent_type.trim()))
                    .map(|t| format!("{t:?}"))
                    .unwrap_or_else(|| "unknown".to_string());

                log::info!("agent_loop: collecting parallel subagent [{subagent_type}] result",);
                let _t_collect = std::time::Instant::now();
                let result = handle.await.unwrap_or_else(|e| {
                    ToolExecutionResult::new(format!("Subagent task panicked/aborted: {e}"))
                });
                log::info!(
                    "agent_loop: parallel subagent [{subagent_type}] collected in {:?}",
                    _t_collect.elapsed(),
                );
                self.persist_tool_result(session_id, request_id, &tc, &result, &event_tx)
                    .await?;
                let _ = event_tx.send(BackendEvent::SubagentCompleted {
                    session_id,
                    request_id,
                    tool_call: tc.clone(),
                    child_session_id: child_sid.unwrap_or_else(uuid::Uuid::new_v4),
                    result: result.clone(),
                });
            }

            // 7b. Regular tools through execute_tool_calls
            if !other_calls.is_empty() {
                let _ = self
                    .execute_tool_calls(
                        session_id,
                        request_id,
                        &other_calls,
                        mode,
                        &event_tx,
                        &model,
                        cancel_token.clone(),
                    )
                    .await?;
            }

            // 7c. Account goal usage for this turn (if goal is Active)
            if let Some(total) = turn.total_tokens {
                let elapsed = turn
                    .completed_at
                    .zip(turn.created_at)
                    .map(|(end, start)| (end - start).num_seconds().max(0))
                    .unwrap_or(0);
                let store = self.store.lock().await;
                let _ = store.account_goal_usage(session_id, total as i64, elapsed);
                drop(store);
            }

            // 8. Continue loop with new request ID for next turn
            let _t_post_tools = std::time::Instant::now();
            request_id = rand::random::<u64>();

            // Check cancellation before notifying the frontend
            if let Some(ref ct) = cancel_token
                && ct.is_cancelled()
            {
                log::info!("run_agent_loop: cancelled before TurnStarting");
                return Ok(());
            }

            // Notify frontend about the new turn
            let _ = event_tx.send(BackendEvent::TurnStarting {
                session_id,
                request_id,
            });
            log::info!(
                "agent_loop: post-tools to TurnStarting took {:?}",
                _t_post_tools.elapsed()
            );
        }
    }

    /// After an assistant turn completes, check if an Active goal exists.
    /// If so, persist a continuation-prompt User message and return `true`
    /// so the loop continues.  Returns `false` (no-op) when there is no
    /// Active goal.
    async fn continue_goal_if_active(&self, session_id: uuid::Uuid) -> anyhow::Result<bool> {
        let goal = {
            let store = self.store.lock().await;
            store.get_goal(session_id)?
        };
        let Some(goal) = goal else {
            return Ok(false);
        };
        if goal.status != GoalStatus::Active {
            return Ok(false);
        }

        let prompt = build_goal_prompt(&goal);
        let msg = tidev_session::session::Message::new(
            tidev_session::session::MessageRole::User,
            &prompt,
        );
        let store = self.store.lock().await;
        store.append_message(session_id, &msg)?;
        Ok(true)
    }
}
