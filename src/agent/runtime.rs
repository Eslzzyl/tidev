//! Shared agent runtime — orchestrates the LLM ↔ tool execution loop.
//!
//! Both the TUI and web frontends use this same runtime so that tool
//! definitions, system-prompt composition, message preprocessing, and the
//! core streaming loop are defined in a single place.
//!
//! Consumers provide an [`UnboundedSender<BackendEvent>`] to receive
//! real-time events (text deltas, tool calls, tool results, …) and call
//! [`AgentRuntime::run_agent_loop`] which drives the full turn loop:
//!
//! ```text
//!  load messages  →  compose system prompt  →  stream LLM
//!       ↑                                        |
//!       |                              tool calls? ──no──→ done
//!       |                                        |
//!       └──── persist results ←── execute tools ←┘
//! ```

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use chrono::Utc;
use tokio::sync::{Mutex, mpsc::UnboundedSender, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{AgentDefinition, AgentType},
    config::{
        reasoning::ThinkingLevelType,
        ActiveModel, ConfigPaths,
    },
    context::ContextManager,
    instructions,
    prompts::SessionMode,
    session::{
        AssistantTurn, BackendEvent, Message, MessageAttachment, MessageRole, ToolCall,
        ToolExecutionResult,
    },
    storage::SessionStore,
    system_info::SystemInfo,
    tooling::{ToolDefinition, ToolRegistry, canonical_tool_name},
};

/// A user message received while the agent loop was already processing a turn.
///
/// After the current turn completes, `run_agent_loop` picks up the next
/// queued message, persists it to the database, and continues the loop.
/// This is the shared mechanism for "type-ahead" across all frontends.
#[derive(Clone, Debug)]
pub struct QueuedUserMessage {
    pub content: String,
    pub attachments: Vec<MessageAttachment>,
    pub mode: Option<crate::prompts::SessionMode>,
    pub thinking_level: Option<ThinkingLevelType>,
}

/// A tool call with an optional rejection reason.
///
/// Sent by frontends through the permission channel to tell
/// `run_agent_loop` which tools are approved and which are rejected.
#[derive(Debug)]
pub struct ApprovedTool {
    pub tool_call: ToolCall,
    /// If `Some`, the tool is rejected; this [`ToolExecutionResult`] will
    /// be persisted as the tool's output.  If `None`, the tool is approved
    /// for execution.
    pub rejection: Option<ToolExecutionResult>,
}

/// Request sent by `run_agent_loop` to the frontend for tool call approval.
///
/// The frontend must respond via `response_tx` with a list of
/// [`ApprovedTool`] entries, one per tool call in `tool_calls`.
#[derive(Debug)]
pub struct PendingToolApproval {
    pub tool_calls: Vec<ToolCall>,
    pub mode: SessionMode,
    pub response_tx: oneshot::Sender<Vec<ApprovedTool>>,
}

/// Shared agent runtime that both TUI and web can use.
#[derive(Clone)]
pub struct AgentRuntime {
    pub workspace_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_paths: ConfigPaths,
    pub store: Arc<Mutex<SessionStore>>,
    pub llm_client: crate::llm::LlmClient,
    pub tools: ToolRegistry,
    /// Instruction file paths/URLs from config (e.g. `config.instructions`).
    pub instructions: Vec<String>,
    /// Cache for instruction file contents to avoid re-reading.
    pub instruction_content_cache: HashMap<String, String>,
    /// Queue of user messages received while the agent loop is running.
    /// After each turn completes, the loop processes the next message
    /// automatically.  Frontends push through [`queue_user_message`].
    pub queued_messages: Arc<StdMutex<VecDeque<QueuedUserMessage>>>,
    /// When `false` (default), tools that need user confirmation are
    /// rejected with an error instead of executed.  When `true`, all
    /// tools are executed without interactive confirmation.
    ///
    /// The TUI sets this to `true` because it handles interactive
    /// permission dialogs itself via the [`PendingToolApproval`] channel.
    /// Web and gateway frontends typically leave this `false` as a
    /// safe default: tools that require approval are simply rejected.
    pub auto_approve_permissions: bool,
}

impl AgentRuntime {
    /// Enqueue a user message for processing after the current turn ends.
    ///
    /// This is the shared "type-ahead" mechanism — when a frontend receives
    /// a user message while `run_agent_loop` is still processing, it can
    /// call this method and the loop will pick it up automatically.
    ///
    /// Returns `true` if the message was queued (the loop is running).
    /// Returns `false` if the queue is not being consumed (no loop active);
    /// the frontend should start a new loop manually.
    pub fn queue_user_message(&self, msg: QueuedUserMessage) -> bool {
        let mut queue = self.queued_messages.lock().unwrap();
        let was_empty = queue.is_empty();
        queue.push_back(msg);
        // If the queue already had items, the loop is definitely running.
        // If it was empty, the caller needs to verify a loop is active.
        !was_empty
    }

    /// Compose the system prompt for a turn.
    ///
    /// Returns `(prompt, instruction_sources)`.
    pub fn compose_system_prompt(
        &mut self,
        base_prompt: &str,
        mode: SessionMode,
    ) -> (String, Vec<String>) {
        let base_prompt = base_prompt.trim();
        let mode_reminder = mode.reminder();

        let (instruction_prompt, sources, new_cache) =
            instructions::system_prompt_and_sources_with_cache(
                &self.workspace_root,
                &self.config_dir,
                &self.instructions,
                &self.instruction_content_cache,
            )
            .unwrap_or_default();

        self.instruction_content_cache = new_cache;

        let mut prompt = String::new();
        if !base_prompt.is_empty() {
            prompt.push_str(base_prompt);
        }
        if !instruction_prompt.is_empty() {
            if !prompt.is_empty() {
                prompt.push_str("\n\n");
            }
            prompt.push_str(&instruction_prompt);
        }
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(mode_reminder);

        // Environment info (same format as TUI)
        let system_info = SystemInfo::detect();
        let working_dir = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let is_git = crate::system_info::is_git_repo(&self.workspace_root);
        prompt.push_str("\n\nHere is some useful information about the environment:\n<env>\n  ");
        prompt.push_str(&format!("Working directory: {}\n  ", working_dir));
        prompt.push_str(&format!(
            "Workspace root folder: {}\n  ",
            self.workspace_root.display()
        ));
        prompt.push_str(&format!(
            "Is directory a git repo: {}\n  ",
            if is_git { "yes" } else { "no" }
        ));
        prompt.push_str(&system_info.format_env());
        prompt.push_str("\n</env>");

        // Workspace memories
        let ws = self.workspace_root.display().to_string();
        let memory_store = self.tools.memory_store();
        if let Ok(memories) = memory_store.select_hot(&ws, 5, 800) {
            let memory_prompt = crate::memory::types::MemoryStore::format_for_prompt(&memories);
            if !memory_prompt.is_empty() {
                prompt.push_str(&memory_prompt);
            }
        }

        (prompt, sources)
    }

    /// Build request messages from stored session messages, preprocessed
    /// through a [`ContextManager`].
    pub fn build_request_messages(
        &self,
        messages: &[Message],
        context_manager: &ContextManager,
        mode: SessionMode,
    ) -> Vec<Message> {
        // We replicate the filtering logic of ContextManager::build_request_messages
        // but accept a plain slice instead of a Conversation.
        let mut result = Vec::new();
        let mut pending_tool_calls: HashMap<String, String> = HashMap::new();
        let mut was_plan_mode = mode == SessionMode::Plan;

        if let Some(summary) = &context_manager.summary {
            result.push(Message::new(
                MessageRole::System,
                format!("Context summary for continuation:\n{summary}"),
            ));
        }

        for message in messages.iter().skip(context_manager.retained_from) {
            if message.streaming {
                continue;
            }

            match message.role {
                MessageRole::System => {}
                MessageRole::User => {
                    // Inject synthetic failure results for any orphaned tool calls
                    // before the user message, so the provider doesn't see an
                    // assistant(tool_calls) without corresponding tool results.
                    for (tool_call_id, tool_name) in pending_tool_calls.drain() {
                        crate::log_warn!(
                            "build_request_messages: injecting synthetic failure for orphaned \
                             tool call id={} name={} before user message",
                            tool_call_id,
                            tool_name
                        );
                        result.push(Message::tool_result(
                            tool_call_id,
                            tool_name,
                            ToolExecutionResult::new(
                                "Tool call failed: execution was interrupted or did not complete",
                            ),
                        ));
                    }
                    result.push(message.clone());
                    if let Some(m) = message.mode {
                        was_plan_mode = m == SessionMode::Plan;
                    }
                }
                MessageRole::Assistant => {
                    if message.content.is_empty() && message.tool_calls.is_empty() {
                        continue;
                    }
                    if let Some(m) = message.mode {
                        was_plan_mode = m == SessionMode::Plan;
                    } else if message.content.contains("PLAN MODE")
                        || message.content.contains("read-only")
                    {
                        was_plan_mode = true;
                    }
                    pending_tool_calls = message
                        .tool_calls
                        .iter()
                        .map(|tc| (tc.id.clone(), tc.name.clone()))
                        .collect();
                    result.push(message.clone());
                }
                MessageRole::Tool => {
                    let Some(tool_call_id) = message.tool_call_id.as_ref() else {
                        continue;
                    };
                    if pending_tool_calls.remove(tool_call_id).is_some() {
                        result.push(message.clone());
                    }
                }
                MessageRole::Error | MessageRole::Shell => {}
            }
        }

        // Inject synthetic failures for orphaned tool calls
        for (tool_call_id, tool_name) in &pending_tool_calls {
            crate::log_warn!(
                "build_request_messages: orphaned tool call id={} name={}, injecting synthetic failure",
                tool_call_id,
                tool_name
            );
            result.push(Message::tool_result(
                tool_call_id.clone(),
                tool_name.clone(),
                ToolExecutionResult::new(
                    "Tool call failed: execution was interrupted or did not complete",
                ),
            ));
        }

        // Mode-switch reminder
        if mode == SessionMode::Plan && !was_plan_mode {
            let reminder = crate::prompts::plan_switch_reminder();
            if let Some(last_user) = result.iter_mut().rev().find(|m| m.role == MessageRole::User)
            {
                last_user.content = format!("{}\n\n{}", reminder, last_user.content);
            }
        } else if mode == SessionMode::Build && was_plan_mode {
            let reminder = crate::prompts::build_switch_reminder();
            if let Some(last_user) = result.iter_mut().rev().find(|m| m.role == MessageRole::User)
            {
                last_user.content = format!("{}\n\n{}", reminder, last_user.content);
            }
        }

        result
    }

    /// Get all available tool definitions (built-in + MCP).
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.all_definitions()
    }

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
        tokio::spawn(async move {
            llm.stream_chat(session_id, request_id, model_for_task, msgs, tools, tx, tl)
                .await;
        });

        let mut turn = AssistantTurn::default();

        while let Some(event) = rx.recv().await {
            // Forward to consumer first
            let _ = event_tx.send(event.clone());

            match event {
                BackendEvent::Delta {
                    content, ..
                } => {
                    turn.content.push_str(&content);
                }
                BackendEvent::ReasoningDelta {
                    content, ..
                } => {
                    turn.reasoning.push_str(&content);
                }
                BackendEvent::ToolCallUpdated {
                    tool_call, ..
                } => {
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
                            Some(total_tokens as f32 / (ms as f32 / 1000.0))
                        } else {
                            None
                        }
                    });
                }
                BackendEvent::Finished {
                    turn: finished_turn,
                    ..
                } => {
                    turn = finished_turn;
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
    /// Returns a list of `(tool_call, result)` pairs for callers that need
    /// to inspect or forward the results.
    /// Execute tool calls with optional mode-based permission filtering.
    ///
    /// Before executing each tool, checks:
    /// 1. Whether the tool is allowed in the current mode (`can_execute`).
    ///    Disallowed tools are rejected with an error result.
    /// 2. Whether the tool needs user confirmation.  If
    ///    `auto_approve_permissions` is `false`, these tools are rejected
    ///    with an error.  If `true`, they execute without confirmation
    ///    (the TUI handles confirmation itself via the permission channel).
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_tool_calls(
        &mut self,
        session_id: uuid::Uuid,
        request_id: u64,
        tool_calls: &[ToolCall],
        mode: SessionMode,
        event_tx: &UnboundedSender<BackendEvent>,
        _parent_model: &crate::config::ActiveModel,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        let runtime = tokio::runtime::Handle::current();
        let mut results: Vec<(ToolCall, ToolExecutionResult)> =
            Vec::with_capacity(tool_calls.len());

        // ─── Phase 0: Mode-based + confirmation filtering ────────────
        // Reject tools that are not allowed in the current mode, or that
        // need confirmation when auto_approve is off.
        let mut filtered: Vec<&ToolCall> = Vec::with_capacity(tool_calls.len());
        for call in tool_calls {
            if !self.tools.can_execute(&call.name, mode) {
                crate::log_info!(
                    "execute_tool_calls: rejecting '{}' — not allowed in {:?} mode",
                    call.name, mode
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

            if !self.auto_approve_permissions {
                if let Some(def) = self.tools.definition_for(&call.name) {
                    if def.needs_confirmation() {
                        crate::log_info!(
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
                }
            }

            filtered.push(call);
        }

        // All tools filtered out — return early
        if filtered.is_empty() {
            return Ok(results);
        }

        // Separate tool calls by execution strategy.
        let mut read_only: Vec<&ToolCall> = Vec::new();
        let mut write: Vec<&ToolCall> = Vec::new();
        for call in &filtered {
            if self.tools.is_read_only_call(call) {
                read_only.push(call);
            } else {
                write.push(call);
            }
        }

        // ─── Phase 1: Read-only tools in parallel ───────────────────────
        if !read_only.is_empty() {
            let mut stores: Vec<SessionStore> = Vec::with_capacity(read_only.len());
            {
                let store = self.store.lock().await;
                for _ in 0..read_only.len() {
                    stores.push(store.clone());
                }
            }

            let mut handles: Vec<(
                ToolCall,
                tokio::task::JoinHandle<ToolExecutionResult>,
            )> = Vec::with_capacity(read_only.len());
            for (tool_call, store) in read_only.into_iter().zip(stores) {
                let handle = self.tools.execute_call_spawned(
                    runtime.clone(),
                    store,
                    session_id,
                    tool_call.clone(),
                    mode,
                    false,
                );
                handles.push((tool_call.clone(), handle));
            }

            for (tool_call, handle) in handles {
                let result = handle.await.unwrap_or_else(|join_err| {
                    ToolExecutionResult::new(format!(
                        "Tool task panicked/aborted: {join_err}"
                    ))
                });
                results.push((tool_call, result));
            }
        }

        // ─── Phase 2: Write tools serially ──────────────────────────────
        for tool_call in write {
            let store = {
                let s = self.store.lock().await;
                s.clone()
            };
            let handle = self.tools.execute_call_spawned(
                runtime.clone(),
                store,
                session_id,
                tool_call.clone(),
                mode,
                false,
            );
            let result = handle.await.unwrap_or_else(|join_err| {
                ToolExecutionResult::new(format!(
                    "Tool task panicked/aborted: {join_err}"
                ))
            });
            results.push((tool_call.clone(), result));
        }

        // ─── Phase 4: Persist results and emit events sequentially ──────
        for (tool_call, result) in &results {
            self.persist_tool_result(session_id, request_id, tool_call, result, event_tx)
                .await?;
        }

        Ok(results)
    }

    /// Persist a pre-built message to the database.
    ///
    /// Useful when the caller has already constructed the message with
    /// token usage, mode, and other fields set (e.g. TUI's flow).
    pub async fn persist_message(
        &self,
        session_id: uuid::Uuid,
        msg: &Message,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.append_message(session_id, msg)?;
        Ok(())
    }

    /// Persist an assistant message to the database.
    ///
    /// Captured token usage from [`AssistantTurn`] is automatically written
    /// to the stored message, so consumers of `run_agent_loop` do not need
    /// to manually set token fields.
    pub async fn persist_assistant_message(
        &self,
        session_id: uuid::Uuid,
        turn: &AssistantTurn,
    ) -> Result<()> {
        let mut msg = Message::new(MessageRole::Assistant, &turn.content);
        msg.reasoning = turn.reasoning.clone();
        msg.tool_calls = turn.tool_calls.clone();
        msg.streaming = false;
        msg.completed_at = Some(Utc::now());
        // Token usage captured from UsageStats during streaming
        msg.input_tokens = turn.input_tokens;
        msg.output_tokens = turn.output_tokens;
        msg.total_tokens = turn.total_tokens;
        msg.cache_read_tokens = turn.cache_read_tokens;
        msg.cache_write_tokens = turn.cache_write_tokens;
        msg.model_id = turn.model_id.clone();
        msg.tokens_per_second = turn.tokens_per_second;

        let store = self.store.lock().await;
        store.append_message(session_id, &msg)?;
        Ok(())
    }

    /// Persist a tool result to the database and emit a `ToolCompleted` event.
    ///
    /// This encapsulates the common pattern: create a `Message::tool_result`,
    /// append it to the DB, and send the `ToolCompleted` event. Both
    /// `execute_tool_calls` and the TUI's `record_tool_result` can delegate
    /// to this method.
    pub async fn persist_tool_result(
        &self,
        session_id: uuid::Uuid,
        request_id: u64,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
        event_tx: &UnboundedSender<BackendEvent>,
    ) -> Result<()> {
        let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
        {
            let store = self.store.lock().await;
            store.append_message(session_id, &tool_msg)?;
        }
        let _ = event_tx.send(BackendEvent::ToolCompleted {
            session_id,
            request_id,
            tool_call: tool_call.clone(),
            result: result.clone(),
        });
        Ok(())
    }

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
    pub async fn run_subagent(
        mut self,
        parent_session_id: uuid::Uuid,
        parent_request_id: u64,
        tool_call: ToolCall,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        parent_model: crate::config::ActiveModel,
    ) -> ToolExecutionResult {
        let result = self
            .run_subagent_inner(
                parent_session_id,
                parent_request_id,
                &tool_call,
                &event_tx,
                cancel_token,
                &parent_model,
            )
            .await;
        match result {
            Ok(output) => ToolExecutionResult::new(output),
            Err(e) => ToolExecutionResult::new(format!("Subagent failed: {e}")),
        }
    }

    async fn run_subagent_inner(
        &mut self,
        parent_session_id: uuid::Uuid,
        _parent_request_id: u64,
        tool_call: &ToolCall,
        event_tx: &UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        parent_model: &crate::config::ActiveModel,
    ) -> anyhow::Result<String> {
        use crate::tooling::TaskArgs;

        // 1. Parse tool call arguments
        let args = serde_json::from_str::<TaskArgs>(&tool_call.arguments)?;
        let description = args.description.trim().to_string();
        let prompt = args.prompt.trim().to_string();
        let subagent_type = args.subagent_type.unwrap_or_default();
        let agent_type = AgentType::parse(&subagent_type).unwrap_or(AgentType::General);
        let agent_def = AgentDefinition::new(agent_type);

        let child_session_id = uuid::Uuid::new_v4();
        let parent_record = {
            let store = self.store.lock().await;
            store
                .load_session_record(parent_session_id)?
                .ok_or_else(|| anyhow::anyhow!("parent session not found"))?
        };

        // Use agent's model override if set, else inherit parent model
        let child_model = agent_def
            .model_override
            .clone()
            .unwrap_or_else(|| parent_model.clone());

        // 2. Create child session
        {
            let store = self.store.lock().await;
            let agent_label = agent_type.display_name();
            let child_title = format!("Task ({agent_label}): {description}");

            store.create_session_with_parent(
                child_session_id,
                parent_session_id,
                &self.workspace_root,
                &parent_record.provider_id,
                &parent_record.provider_display_name,
                &child_model.model_id,
                &child_model.display_name,
                &child_title,
            )?;
            store.copy_tool_permissions(parent_session_id, child_session_id)?;

            let bootstrap = Message::new(MessageRole::System, agent_def.bootstrap_content());
            store.append_message(child_session_id, &bootstrap)?;
            let user_msg = Message::new(MessageRole::User, prompt);
            store.append_message(child_session_id, &user_msg)?;
        }

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

        // 4. Run an inline agent loop for the child session.
        //    We do NOT call run_agent_loop_with_tools here to avoid async
        //    recursive type detection (the methods would form a cycle).
        //    Instead we replicate the loop body using the same primitives
        //    (run_single_turn, persist_assistant_message, persist_tool_result).
        let mut child_context = ContextManager::new();
        let child_thinking = child_model.thinking_level.clone();
        let mut request_sequence: u64 = rand::random();
        let runtime = tokio::runtime::Handle::current();

        loop {
            // Check cancellation
            if let Some(ref ct) = cancel_token {
                if ct.is_cancelled() {
                    crate::log_info!("run_subagent: cancelled");
                    return Ok(String::new());
                }
            }

            // Load messages
            let db_messages = {
                let store = self.store.lock().await;
                store.load_messages(child_session_id)?
            };

            // Compose + build
            let (system_prompt, _sources) =
                self.compose_system_prompt(&child_model.system_prompt, SessionMode::Build);
            let mut model_for_turn = child_model.clone();
            model_for_turn.system_prompt = system_prompt;
            let request_messages =
                self.build_request_messages(&db_messages, &mut child_context, SessionMode::Build);

            // Stream LLM
            let turn = self
                .run_single_turn(
                    child_session_id,
                    request_sequence,
                    model_for_turn,
                    request_messages,
                    tools.clone(),
                    child_thinking.clone(),
                    event_tx,
                )
                .await?;

            // Persist assistant message
            self.persist_assistant_message(child_session_id, &turn)
                .await?;

            // If no tool calls, done
            if turn.tool_calls.is_empty() {
                break String::new();
            }

            // Execute tools (no sub-sub-agents: task tools treated as errors)
            let mut tool_results: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();
            for tc in &turn.tool_calls {
                if tc.name == "task" {
                    tool_results.push((
                        tc.clone(),
                        ToolExecutionResult::new(
                            "Sub-agent nesting too deep: task tool unavailable in nested sessions",
                        ),
                    ));
                    continue;
                }

                if self.tools.is_read_only_call(tc) {
                    // Read-only: parallel execution via spawn_blocking
                    let store = {
                        let s = self.store.lock().await;
                        s.clone()
                    };
                    let handle = self.tools.execute_call_spawned(
                        runtime.clone(),
                        store,
                        child_session_id,
                        tc.clone(),
                        SessionMode::Build,
                        false,
                    );
                    let result = handle.await.unwrap_or_else(|join_err| {
                        ToolExecutionResult::new(format!(
                            "Tool task panicked/aborted: {join_err}"
                        ))
                    });
                    tool_results.push((tc.clone(), result));
                } else {
                    // Write tool: serial execution
                    let store = {
                        let s = self.store.lock().await;
                        s.clone()
                    };
                    let handle = self.tools.execute_call_spawned(
                        runtime.clone(),
                        store,
                        child_session_id,
                        tc.clone(),
                        SessionMode::Build,
                        false,
                    );
                    let result = handle.await.unwrap_or_else(|join_err| {
                        ToolExecutionResult::new(format!(
                            "Tool task panicked/aborted: {join_err}"
                        ))
                    });
                    tool_results.push((tc.clone(), result));
                }
            }

            // Persist tool results
            for (tc, result) in &tool_results {
                self.persist_tool_result(
                    child_session_id,
                    request_sequence,
                    tc,
                    result,
                    event_tx,
                )
                .await?;
            }

            request_sequence = request_sequence.wrapping_add(1);
        };

        // 5. Read last assistant message from child session
        let store = self.store.lock().await;
        let messages = store.load_messages(child_session_id)?;
        let output = messages
            .into_iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant && !m.content.is_empty())
            .map(|m| m.content)
            .unwrap_or_default();

        Ok(output)
    }

    /// Run the full agent loop with the given tool definitions.
    ///
    /// Same as [`run_agent_loop`](Self::run_agent_loop) but uses the provided
    /// tool list instead of calling `self.tool_definitions()`.  This is used
    /// internally by [`run_subagent`](Self::run_subagent) to restrict tools
    /// based on the subagent's [`AgentDefinition`].
    ///
    /// If `permission_tx` is provided, tool calls are not executed directly.
    /// Instead, a [`PendingToolApproval`] is sent through the channel and
    /// the loop waits for the frontend to approve/reject each tool.  This
    /// is used by the TUI to implement interactive permission dialogs.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_agent_loop_with_tools(
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
        self.run_agent_loop_with_tools_inner(
            session_id, model, context_manager, mode, thinking_level,
            tools, event_tx, cancel_token, None,
        ).await
    }

    /// Internal implementation with optional permission channel.
    async fn run_agent_loop_with_tools_inner(
        &mut self,
        session_id: uuid::Uuid,
        mut model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        tools: Vec<ToolDefinition>,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        permission_tx: Option<tokio::sync::mpsc::UnboundedSender<PendingToolApproval>>,
    ) -> Result<()> {
        let mut request_id: u64 = rand::random();

        loop {
            // Check cancellation
            if let Some(ref ct) = cancel_token {
                if ct.is_cancelled() {
                    crate::log_info!("run_agent_loop: cancelled");
                    return Ok(());
                }
            }

            // 1. Load messages from DB
            let db_messages = {
                let store = self.store.lock().await;
                store.load_messages(session_id)?
            };

            // 2. Compose system prompt
            let (system_prompt, _sources) = self.compose_system_prompt(&model.system_prompt, mode);
            model.system_prompt = system_prompt;

            // 3. Build request messages
            let request_messages = self.build_request_messages(&db_messages, context_manager, mode);

            // 4. Stream LLM — `Finished` is already forwarded to event_tx
            //    by `run_single_turn`.
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

            // 5. Persist assistant message (Finished was already emitted)
            self.persist_assistant_message(session_id, &turn).await?;

            // 6. If no tool calls, check for queued user messages before
            //    compacting and returning.  This implements the shared
            //    "type-ahead" mechanism — frontends push messages through
            //    `queue_user_message()` while the loop is running.
            if turn.tool_calls.is_empty() {
                let next_msg = self.queued_messages.lock().unwrap().pop_front();
                if let Some(qmsg) = next_msg {
                    crate::log_info!(
                        "run_agent_loop: processing queued message ({} chars)",
                        qmsg.content.len()
                    );
                    let mut user_msg = Message::new(MessageRole::User, &qmsg.content);
                    user_msg.attachments = qmsg.attachments;
                    user_msg.mode = qmsg.mode;
                    user_msg.thinking_level = qmsg.thinking_level;
                    user_msg.completed_at = Some(Utc::now());
                    {
                        let store = self.store.lock().await;
                        store.append_message(session_id, &user_msg)?;
                    }
                    // Continue the loop — the next iteration picks up the
                    // newly persisted user message.
                    request_id = rand::random::<u64>();
                    continue;
                }
                self.maybe_compact(session_id, &model, context_manager, &event_tx).await;
                return Ok(());
            }

            // ─── 6a. Permission approval (frontend interception) ─────────
            //
            // If a permission channel is configured, send all tool calls to
            // the frontend for approval.  The frontend can approve, reject,
            // or partially approve tools.  Rejected tools are persisted as
            // error results; approved tools proceed to execution.
            //
            // Without a permission channel (Web/Gateway), all tool calls
            // proceed directly — non-interactive filtering (can_execute,
            // needs_confirmation) is handled inside execute_tool_calls below.

            let mut task_calls: Vec<ToolCall> = Vec::new();
            let mut other_calls: Vec<ToolCall> = Vec::new();

            if let Some(ref perm_tx) = permission_tx {
                let (resp_tx, resp_rx) = oneshot::channel();
                let _ = perm_tx.send(PendingToolApproval {
                    tool_calls: turn.tool_calls.clone(),
                    mode,
                    response_tx: resp_tx,
                });

                match resp_rx.await {
                    Ok(approvals) => {
                        for approved in approvals {
                            if let Some(rejection) = approved.rejection {
                                self.persist_tool_result(
                                    session_id, request_id,
                                    &approved.tool_call, &rejection, &event_tx,
                                ).await?;
                            } else if approved.tool_call.name == "task" {
                                task_calls.push(approved.tool_call);
                            } else {
                                other_calls.push(approved.tool_call);
                            }
                        }
                    }
                    Err(_) => {
                        crate::log_info!("run_agent_loop: permission channel closed, stopping loop");
                        return Ok(());
                    }
                }
            } else {
                // No permission channel — partition tool calls normally
                for tc in turn.tool_calls {
                    if tc.name == "task" {
                        task_calls.push(tc);
                    } else {
                        other_calls.push(tc);
                    }
                }
            }

            // Check cancellation again before executing tools
            if let Some(ref ct) = cancel_token {
                if ct.is_cancelled() {
                    crate::log_info!("run_agent_loop: cancelled before tool execution");
                    return Ok(());
                }
            }

            // 7a. Subagent (task) tools — read-only types (Explorer, Librarian,
            // Oracle) run in parallel; write-capable types (Designer, Fixer,
            // General) run serially.
            let mut task_handles: Vec<(
                ToolCall,
                tokio::task::JoinHandle<ToolExecutionResult>,
            )> = Vec::new();

            for tc in task_calls {
                // Determine if this subagent is read-only by parsing the
                // task arguments to extract the subagent_type field.
                let is_read_only = serde_json::from_str::<crate::tooling::TaskArgs>(
                    &tc.arguments,
                )
                .ok()
                .and_then(|args| {
                    args.subagent_type
                        .as_deref()
                        .and_then(AgentType::parse)
                })
                .map_or(false, |t| t.is_read_only());

                if is_read_only {
                    // Read-only subagent — spawn in parallel
                    let agent = self.clone();
                    let owned_tc = tc.clone();
                    let tx = event_tx.clone();
                    let sid = session_id;
                    let rid = request_id;
                    let pm = model.clone();

                    let handle = tokio::spawn(async move {
                        let fut: Pin<
                            Box<dyn Future<Output = ToolExecutionResult> + Send>,
                        > = Box::pin(
                            agent.run_subagent(sid, rid, owned_tc, tx, None, pm),
                        );
                        fut.await
                    });
                    task_handles.push((tc, handle));
                } else {
                    // Write-capable subagent — run serially to avoid ordering
                    // issues with filesystem and database mutations.
                    let agent = self.clone();
                    let owned_tc = tc.clone();
                    let tx = event_tx.clone();
                    let sid = session_id;
                    let rid = request_id;
                    let pm = model.clone();

                    let result: ToolExecutionResult = {
                        let fut: Pin<
                            Box<dyn Future<Output = ToolExecutionResult> + Send>,
                        > = Box::pin(
                            agent.run_subagent(sid, rid, owned_tc, tx, None, pm),
                        );
                        fut.await
                    };
                    self.persist_tool_result(session_id, request_id, &tc, &result, &event_tx)
                        .await?;
                }
            }

            // Collect parallel task results in order
            for (tc, handle) in task_handles {
                let result = handle.await.unwrap_or_else(|e| {
                    ToolExecutionResult::new(format!(
                        "Subagent task panicked/aborted: {e}"
                    ))
                });
                self.persist_tool_result(session_id, request_id, &tc, &result, &event_tx)
                    .await?;
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
                    )
                    .await?;
            }

            // 8. Generate new request ID for next iteration
            request_id = rand::random::<u64>();
        }
    }

    /// Run the full agent loop.
    ///
    /// 1. Load messages from DB
    /// 2. Compose system prompt
    /// 3. Build request messages
    /// 4. Stream LLM (events forwarded to `event_tx`; `Finished` is already
    ///    emitted by the LLM stream and forwarded via `run_single_turn`)
    /// 5. Persist assistant message
    /// 6. If tool calls: execute each, persist results, emit `ToolCompleted`
    /// 7. Loop back to step 1 until no more tool calls
    ///
    /// If `cancel_token` is provided and cancelled, the loop stops after the
    /// current turn/tool execution completes.
    pub async fn run_agent_loop(
        &mut self,
        session_id: uuid::Uuid,
        model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
    ) -> Result<()> {
        let tools = self.tool_definitions();
        self.run_agent_loop_with_tools(
            session_id,
            model,
            context_manager,
            mode,
            thinking_level,
            tools,
            event_tx,
            cancel_token,
        )
        .await
    }

    /// Run the full agent loop with a permission approval channel.
    ///
    /// When tool calls are generated, the loop sends a [`PendingToolApproval`]
    /// through `permission_tx` and waits for the frontend to approve/reject
    /// each tool.  This is used by the TUI to implement interactive permission
    /// dialogs.  Web and gateway frontends use [`run_agent_loop`] instead.
    pub async fn run_agent_loop_with_permission_channel(
        &mut self,
        session_id: uuid::Uuid,
        model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
        permission_tx: tokio::sync::mpsc::UnboundedSender<PendingToolApproval>,
    ) -> Result<()> {
        let tools = self.tool_definitions();
        self.run_agent_loop_with_tools_inner(
            session_id, model, context_manager, mode, thinking_level,
            tools, event_tx, cancel_token, Some(permission_tx),
        ).await
    }

    /// Optionally compact the session context after a completed turn.
    ///
    /// Loads the conversation from DB, checks whether compaction is needed,
    /// runs it if so, and persists the updated context state back to DB.
    /// Errors are logged but not propagated (compaction is best-effort).
    async fn maybe_compact(
        &self,
        session_id: uuid::Uuid,
        model: &ActiveModel,
        context_manager: &mut ContextManager,
        event_tx: &UnboundedSender<BackendEvent>,
    ) {
        let conversation = match self.load_conversation(session_id).await {
            Ok(Some(c)) => c,
            _ => return,
        };

        if !context_manager.needs_compaction(&conversation, model) {
            return;
        }

        match context_manager
            .compact(
                &self.llm_client,
                model,
                &conversation,
                false,
                None,
            )
            .await
        {
            Ok(true) => {
                // Persist updated context state
                if let Ok(ref store) = self.store.try_lock() {
                    let _ = store.update_session_context_state(
                        session_id,
                        context_manager.summary.as_deref(),
                        context_manager.retained_from,
                    );
                }
                crate::log_info!(
                    "run_agent_loop: context compacted for session {}",
                    session_id
                );
                let _ = event_tx.send(BackendEvent::ContextCompacted {
                    session_id,
                    compacted: true,
                    manual: false,
                    summary: context_manager.summary.clone(),
                    retained_from: context_manager.retained_from,
                    error: None,
                });
            }
            Ok(false) => {
                // Compaction skipped (not needed after all)
            }
            Err(e) => {
                crate::log_warn!("run_agent_loop: context compaction failed: {e}");
                let _ = event_tx.send(BackendEvent::ContextCompacted {
                    session_id,
                    compacted: false,
                    manual: false,
                    summary: None,
                    retained_from: 0,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    /// Load a full conversation (session record + messages) from the store.
    async fn load_conversation(
        &self,
        session_id: uuid::Uuid,
    ) -> Result<Option<crate::session::Conversation>> {
        let store = self.store.lock().await;
        store.load_conversation(session_id)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use crate::{
        config::ConfigPaths,
        context::ContextManager,
        prompts::SessionMode,
        session::{Message, MessageRole, ToolCall, ToolExecutionResult},
        storage::SessionStore,
    };

    use super::AgentRuntime;

    /// Create a minimal AgentRuntime backed by a tempfile database.
    fn agent_runtime() -> (AgentRuntime, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test.db");
        let store = SessionStore::open(&db_path).unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let config_dir = tmp.path().join("config");
        std::fs::create_dir_all(&config_dir).unwrap();

        let agent = AgentRuntime {
            workspace_root: ws,
            config_dir,
            config_paths: ConfigPaths {
                config_dir: tmp.path().join("config"),
                data_dir: tmp.path().join("data"),
                config_file: tmp.path().join("config").join("config.toml"),
                database_file: db_path.clone(),
                auth_file: tmp.path().join("auth.json"),
            },
            store: Arc::new(Mutex::new(store)),
            llm_client: crate::llm::LlmClient::new().unwrap(),
            tools: crate::tooling::ToolRegistry::new(
                tmp.path().join("workspace"),
                tmp.path().join("config"),
                vec![],
                crate::mcp::McpManager::new(tmp.path().join("workspace"), Default::default()),
                crate::config::PermissionConfig::default(),
                std::sync::Arc::new(crate::tooling::FileReadTracker::new()),
                std::sync::Arc::new(
                    crate::memory::types::MemoryStore::open(&db_path).unwrap(),
                ),
                false,
                None,
            ),
            instructions: vec![],
            instruction_content_cache: Default::default(),
            queued_messages: std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            auto_approve_permissions: false,
        };
        (agent, tmp)
    }

    #[test]
    fn build_request_messages_basic_filtering() {
        let msgs = vec![
            Message::new(MessageRole::User, "Hello"),
            Message::new(MessageRole::Assistant, "Hi there!"),
            Message::new(MessageRole::User, "What is the weather?"),
            Message::new(MessageRole::Assistant, "Let me check."),
        ];
        let (agent, _tmp) = agent_runtime();
        let cm = ContextManager {
            retained_from: 2,
            ..ContextManager::new()
        };
        let result = agent.build_request_messages(&msgs, &cm, SessionMode::Build);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "What is the weather?");
        assert_eq!(result[1].content, "Let me check.");
    }

    #[test]
    fn build_request_messages_empty_after_retained() {
        let msgs = vec![
            Message::new(MessageRole::User, "Hello"),
            Message::new(MessageRole::Assistant, "Hi"),
        ];
        let (agent, _tmp) = agent_runtime();
        let cm = ContextManager {
            retained_from: 2,
            ..ContextManager::new()
        };
        let result = agent.build_request_messages(&msgs, &cm, SessionMode::Build);
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn build_request_messages_skips_streaming_messages() {
        let (agent, _tmp) = agent_runtime();
        let msgs = vec![
            Message::new(MessageRole::User, "hello"),
            Message::streaming(MessageRole::Assistant, "still typing..."),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        assert!(!result.iter().any(|m| m.content == "still typing..."));
    }

    #[test]
    fn build_request_messages_keeps_valid_tool_results() {
        let (agent, _tmp) = agent_runtime();
        let mut assistant = Message::new(MessageRole::Assistant, "searching");
        assistant.tool_calls = vec![ToolCall {
            id: "tc-1".to_string(),
            name: "grep".to_string(),
            arguments: "{}".to_string(),
        }];
        let msgs = vec![
            Message::new(MessageRole::User, "find it"),
            assistant.clone(),
            Message::tool_result("tc-1", "grep", ToolExecutionResult::new("found!")),
            Message::new(MessageRole::Assistant, "result"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        let roles: Vec<_> = result.iter().map(|m| m.role.clone()).collect();
        assert_eq!(roles, vec![
            MessageRole::User,
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::Assistant,
        ]);
    }

    #[test]
    fn build_request_messages_injects_orphan_tool_failures() {
        let (agent, _tmp) = agent_runtime();
        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls = vec![ToolCall {
            id: "orphan".to_string(),
            name: "edit".to_string(),
            arguments: "{}".to_string(),
        }];
        let msgs = vec![
            assistant,
            Message::new(MessageRole::User, "what happened?"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        let roles: Vec<_> = result.iter().map(|m| m.role.clone()).collect();
        assert_eq!(roles, vec![
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::User,
        ]);
        let tool_msg = &result[1];
        assert_eq!(tool_msg.tool_call_id.as_deref(), Some("orphan"));
        assert!(tool_msg.content.contains("interrupted"));
    }

    #[test]
    fn build_request_messages_orphan_before_user_regression() {
        let (agent, _tmp) = agent_runtime();
        let mut orphan_tool_call = Message::new(MessageRole::Assistant, "");
        orphan_tool_call.tool_calls = vec![ToolCall {
            id: "orphan-call".to_string(),
            name: "edit".to_string(),
            arguments: "{}".to_string(),
        }];
        let msgs = vec![
            orphan_tool_call,
            Message::new(MessageRole::User, "the edit failed"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        let roles: Vec<_> = result.iter().map(|m| m.role.clone()).collect();
        assert_eq!(roles, vec![
            MessageRole::Assistant,
            MessageRole::Tool,
            MessageRole::User,
        ]);
        let synthetic = &result[1];
        assert_eq!(synthetic.role, MessageRole::Tool);
        assert_eq!(synthetic.tool_call_id.as_deref(), Some("orphan-call"));
    }

    #[test]
    fn build_request_messages_mode_switch_injection() {
        let (agent, _tmp) = agent_runtime();
        // Assistant was in Build mode → now Plan mode → inject plan switch reminder
        let mut assistant = Message::new(MessageRole::Assistant, "ok");
        assistant.mode = Some(SessionMode::Build);
        let msgs = vec![
            Message::new(MessageRole::User, "do something"),
            assistant,
            Message::new(MessageRole::User, "now plan it"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Plan);
        // The last user message should have the plan switch reminder prepended
        let last_user = result.iter().rev().find(|m| m.role == MessageRole::User).unwrap();
        assert!(
            last_user.content.contains("PLAN MODE") || last_user.content.contains("plan"),
            "Expected plan mode reminder in user message, got: {}",
            last_user.content
        );
    }

    #[test]
    fn build_request_messages_context_summary() {
        let (agent, _tmp) = agent_runtime();
        let cm = ContextManager {
            summary: Some("Previous context was about Rust".to_string()),
            ..ContextManager::new()
        };
        let msgs = vec![
            Message::new(MessageRole::User, "continue"),
        ];
        let result = agent.build_request_messages(&msgs, &cm, SessionMode::Build);
        assert!(result[0].content.contains("Previous context was about Rust"));
        assert_eq!(result[0].role, MessageRole::System);
    }

    #[test]
    fn build_request_messages_tool_result_cleared_by_new_user() {
        let (agent, _tmp) = agent_runtime();
        let msgs = vec![
            Message::new(MessageRole::User, "hello"),
            Message::tool_result("nonexistent", "grep", ToolExecutionResult::new("data")),
            Message::new(MessageRole::Assistant, "reply"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        let roles: Vec<_> = result.iter().map(|m| m.role.clone()).collect();
        assert_eq!(roles, vec![MessageRole::User, MessageRole::Assistant]);
    }

    #[test]
    fn build_request_messages_empty_assistant_skipped() {        let (agent, _tmp) = agent_runtime();
        let msgs = vec![
            Message::new(MessageRole::User, "hello"),
            Message::new(MessageRole::Assistant, ""),
            Message::new(MessageRole::Assistant, "real reply"),
        ];
        let result = agent.build_request_messages(&msgs, &ContextManager::new(), SessionMode::Build);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "hello");
        assert_eq!(result[1].content, "real reply");
    }
}
