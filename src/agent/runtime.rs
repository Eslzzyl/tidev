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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{
        reasoning::ThinkingLevelType,
        ActiveModel, ConfigPaths,
    },
    context::ContextManager,
    instructions,
    prompts::SessionMode,
    session::{
        AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
    },
    storage::SessionStore,
    system_info::SystemInfo,
    tooling::{ToolDefinition, ToolRegistry},
};

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
}

impl AgentRuntime {
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
    /// Returns a list of `(tool_call, result)` pairs for callers that need
    /// to inspect or forward the results (e.g. gateway channels that send
    /// results to users).
    pub async fn execute_tool_calls(
        &self,
        session_id: uuid::Uuid,
        request_id: u64,
        tool_calls: &[ToolCall],
        mode: SessionMode,
        event_tx: &UnboundedSender<BackendEvent>,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        let runtime = tokio::runtime::Handle::current();
        let mut results = Vec::with_capacity(tool_calls.len());

        for tool_call in tool_calls {
            let result = {
                let store = self.store.lock().await;
                match self.tools.execute_call(
                    &runtime,
                    &store,
                    session_id,
                    tool_call,
                    mode,
                    false,
                ) {
                    Ok(r) => r,
                    Err(e) => ToolExecutionResult::new(format!("Error: {e}")),
                }
            };

            // Persist tool result message
            let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
            {
                let store = self.store.lock().await;
                store.append_message(session_id, &tool_msg)?;
            }

            // Emit event (clone result so we can return it)
            let _ = event_tx.send(BackendEvent::ToolCompleted {
                session_id,
                request_id,
                tool_call: tool_call.clone(),
                result: result.clone(),
            });

            results.push((tool_call.clone(), result));
        }

        Ok(results)
    }

    /// Persist an assistant message to the database.
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

        let store = self.store.lock().await;
        store.append_message(session_id, &msg)?;
        Ok(())
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
        mut model: ActiveModel,
        context_manager: &mut ContextManager,
        mode: SessionMode,
        thinking_level: ThinkingLevelType,
        event_tx: UnboundedSender<BackendEvent>,
        cancel_token: Option<CancellationToken>,
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

            // 4. Get tool definitions
            let tools = self.tool_definitions();

            // 5. Stream LLM — `Finished` is already forwarded to event_tx
            //    by `run_single_turn`.
            let turn = self
                .run_single_turn(
                    session_id,
                    request_id,
                    model.clone(),
                    request_messages,
                    tools,
                    thinking_level.clone(),
                    &event_tx,
                )
                .await?;

            // 6. Persist assistant message (Finished was already emitted)
            self.persist_assistant_message(session_id, &turn).await?;

            // 7. If no tool calls, we're done
            if turn.tool_calls.is_empty() {
                return Ok(());
            }

            // Check cancellation again before executing tools
            if let Some(ref ct) = cancel_token {
                if ct.is_cancelled() {
                    crate::log_info!("run_agent_loop: cancelled before tool execution");
                    return Ok(());
                }
            }

            // 8. Execute tools and persist results
            let _ = self.execute_tool_calls(
                session_id,
                request_id,
                &turn.tool_calls,
                mode,
                &event_tx,
            )
            .await?;

            // 9. Generate new request ID for next iteration
            request_id = rand::random::<u64>();
        }
    }
}
