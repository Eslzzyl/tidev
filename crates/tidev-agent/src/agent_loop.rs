//! AgentLoop — the core LLM ↔ tool execution loop.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! Events carry NO `session_id` — the receiver already knows which session
//! the events belong to (Per-Session Event Bus).
//!
//! Permission approval, hooks, and tool execution are injected via fields
//! at construction time — no tight coupling to frontends.
//!
//! Subagent spawning is delegated to [`SessionManager::run_subagent`], which
//! handles model resolution, tool filtering, child session creation, and
//! frontend notification via [`FrontendEvent`].

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
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

use crate::session_manager::SessionManager;
use crate::types::{ApprovedTool, PendingToolApproval};

/// The per-session agent loop.
///
/// Architecture (Per-Session Event Bus):
/// - Owns its own event channel (`event_tx`)
/// - Receives permission approvals through `permission_tx`
/// - Runs hooks after tool execution via `hooks`
/// - Delegates subagent spawning to [`SessionManager::run_subagent`]
pub struct AgentLoop {
    pub session_id: Uuid,
    pub model: tidev_config::ActiveModel,
    pub conversation: Conversation,
    pub context: tidev_context::ContextManager,
    /// Pre-filtered tool definitions for this session's model.
    pub tools: Vec<tidev_tools::ToolDefinition>,
    pub store: Arc<tokio::sync::Mutex<SessionStore>>,
    pub llm: tidev_llm::LlmClient,
    pub event_tx: UnboundedSender<BackendEvent>,
    pub cancel_token: CancellationToken,
    pub mode: SessionMode,
    pub agent_type: tidev_types::agent::AgentType,
    /// Workspace root for this session.
    pub workspace_root: PathBuf,
    /// The composed static system prompt (frozen for session lifetime).
    pub system_prompt: String,
    /// Optional channel for interactive tool permission approval.
    pub permission_tx: Option<UnboundedSender<PendingToolApproval>>,
    /// Hook engine for PostToolUse hooks.
    pub hooks: tidev_hooks::HookEngine,
    /// Tool registry for executing tool calls.
    pub tool_registry: tidev_tools::ToolRegistry,
    /// SessionManager for subagent creation and lifecycle management.
    pub session_manager: SessionManager,
    /// Whether this loop can delegate to sub-agents.
    /// Child sessions set this to `false` to avoid async recursion.
    pub can_delegate: bool,
}

impl AgentLoop {
    /// Run the main agent loop.
    ///
    /// This is a thin wrapper around [`into_run_fut`] to present an `async fn`
    /// API while avoiding async recursion when sub-agents are spawned.
    pub async fn run(self, request_id: u64) -> Result<()> {
        self.into_run_fut(request_id).await
    }

    /// Core agent loop implementation as a boxed future.
    pub(crate) fn into_run_fut(
        mut self,
        mut request_id: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send>> {
        Box::pin(async move {
        log::info!("agent_loop[{}]: started", self.session_id);

        loop {
            if self.cancel_token.is_cancelled() {
                log::info!("agent_loop[{}]: cancelled", self.session_id);
                break;
            }

            // 1. Build request messages from the conversation (with context compaction)
            let mut messages = self.context.build_request_messages(
                &self.conversation,
                self.mode,
            );

            // Prepend system prompt as the first message
            messages.insert(0, Message::new(MessageRole::System, &self.system_prompt));

            // 2. Convert tools to LLM-facing ToolSchema
            let llm_tools: Vec<ToolSchema> = self
                .tools
                .iter()
                .map(|t| ToolSchema {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect();

            // 3. Run a single LLM turn with retry
            let max_retries = 3u32;
            let turn = self
                .run_single_turn_with_retry(
                    &messages,
                    &llm_tools,
                    request_id,
                    max_retries,
                )
                .await?;

            // 4. Persist the assistant turn to DB (with full token metadata)
            let persisted_msg = crate::persistence::persist_assistant_message(
                &self.store, self.session_id, &turn,
            ).await?;
            self.conversation.push(persisted_msg);

            // 5. Send Finished event
            let _ = self.event_tx.send(BackendEvent::Finished {
                request_id,
                turn: turn.clone(),
            });

            // 6. If there are tool calls, execute them
            if !turn.tool_calls.is_empty() {
                // Separate internal (task/todo) and external tools
                let (internal, external): (Vec<&ToolCall>, Vec<&ToolCall>) = turn.tool_calls
                    .iter()
                    .partition(|tc| tc.name == "task" || tc.name == "todo");

                // Handle internal tools
                let mut task_handles: Vec<(
                    ToolCall,
                    tokio::task::JoinHandle<ToolExecutionResult>,
                )> = Vec::new();

                // === Serial subagents (write-capable) first ===
                for tc in &internal {
                    if tc.name == "task" && self.can_delegate {
                        let is_read_only = serde_json::from_str::<tidev_tools::TaskArgs>(&tc.arguments)
                            .ok()
                            .and_then(|args| tidev_types::agent::AgentType::parse(&args.subagent_type))
                            .is_some_and(|t| t.is_read_only());
                        if !is_read_only {
                            // Write-capable subagent: run serially via SessionManager
                            let result = self.session_manager.run_subagent(
                                self.session_id,
                                &self.model,
                                &self.workspace_root,
                                &self.system_prompt,
                                tc,
                            ).await;
                            crate::persistence::persist_tool_result(
                                &self.store, self.session_id, request_id,
                                tc, &result, &self.event_tx,
                            ).await?;
                            self.conversation.push(Message::tool_result(
                                &tc.id, &tc.name, result,
                            ));
                        }
                    }
                }

                // === Parallel subagents (read-only) second ===
                for tc in &internal {
                    if tc.name == "task" && self.can_delegate
                        && let Ok(args) = serde_json::from_str::<tidev_tools::TaskArgs>(&tc.arguments)
                            && let Some(agent_type) = tidev_types::agent::AgentType::parse(&args.subagent_type)
                                && agent_type.is_read_only() {
                                    let session_manager = self.session_manager.clone();
                                    let parent_session_id = self.session_id;
                                    let parent_model = self.model.clone();
                                    let workspace_root = self.workspace_root.clone();
                                    let system_prompt = self.system_prompt.clone();
                                    let tc_clone = (*tc).clone();
                                    let handle = tokio::spawn(async move {
                                        session_manager.run_subagent(
                                            parent_session_id,
                                            &parent_model,
                                            &workspace_root,
                                            &system_prompt,
                                            &tc_clone,
                                        ).await
                                    });
                                    task_handles.push(((*tc).clone(), handle));
                                }
                }

                // Collect parallel results in original tool_use order
                for (tc, handle) in task_handles {
                    let result = handle.await.unwrap_or_else(|e| {
                        ToolExecutionResult::new(format!("Subagent task panicked: {e}"))
                    });
                    crate::persistence::persist_tool_result(
                        &self.store, self.session_id, request_id,
                        &tc, &result, &self.event_tx,
                    ).await?;
                    self.conversation.push(Message::tool_result(
                        &tc.id, &tc.name, result,
                    ));
                }

                // Handle todo tool
                for tc in &internal {
                    if tc.name == "todo" {
                        let result = self.execute_external_tool(tc, request_id, false).await?;
                        self.conversation.push(Message::tool_result(
                            &result.tool_call_id, &result.tool_name, result.result,
                        ));
                    }
                }

                // Handle non-delegable task (child sessions: task tool is not available)
                for tc in &internal {
                    if tc.name == "task" && !self.can_delegate {
                        let result = ToolExecutionResult::new(
                            "Subagent delegation is not available in this context.".to_string(),
                        );
                        crate::persistence::persist_tool_result(
                            &self.store, self.session_id, request_id,
                            tc, &result, &self.event_tx,
                        ).await?;
                        self.conversation.push(Message::tool_result(
                            &tc.id, &tc.name, result,
                        ));
                    }
                }

                // Execute external tools through the tool registry
                if !external.is_empty() {
                    let results = self.execute_external_tools(&external, request_id).await?;
                    for result in &results {
                        self.conversation.push(Message::tool_result(
                            &result.tool_call_id, &result.tool_name, result.result.clone(),
                        ));
                    }

                    // Run post-tool-use hooks
                    for (tc, exec_result) in external.iter().zip(results.iter()) {
                        self.hooks
                            .on_post_tool_use(tc, &exec_result.result, Some(self.session_id))
                            .await;
                    }
                }
            }

            // 7. Check for context compaction
            if self.context.needs_compaction(&self.conversation, &self.model) {
                self.compact_context().await;
            }

            // 8. If no tool calls, this was a final response — exit the loop.
            if turn.tool_calls.is_empty() {
                break;
            }

            // 9. Generate new request ID and continue loop
            request_id = request_id.wrapping_add(1);

            // 10. Notify frontend about the new turn
            let _ = self.event_tx.send(BackendEvent::TurnStarting {
                request_id,
            });
        }

        Ok(())
        }) // end of Box::pin(async move { })
    }

    /// Run a single LLM turn with retry logic.
    async fn run_single_turn_with_retry(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        request_id: u64,
        max_retries: u32,
    ) -> Result<AssistantTurn> {
        let mut last_error = None;

        for attempt in 1..=max_retries {
            if self.cancel_token.is_cancelled() {
                anyhow::bail!("cancelled");
            }

            match self
                .run_single_turn(messages, tools, request_id)
                .await
            {
                Ok(turn) => return Ok(turn),
                Err(e) => {
                    log::warn!(
                        "agent_loop[{}]: LLM turn attempt {}/{} failed: {}",
                        self.session_id,
                        attempt,
                        max_retries,
                        e
                    );
                    last_error = Some(e);

                    // Notify frontend about the retry
                    let _ = self.event_tx.send(BackendEvent::Retrying {
                        request_id,
                        attempt,
                        max_attempts: max_retries,
                        reason: format!("{}", last_error.as_ref().unwrap()),
                        retry_after_secs: Some(attempt),
                    });

                    // Brief backoff before retry
                    tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM turn failed after {} retries", max_retries)))
    }

    /// Run a single LLM streaming turn.
    async fn run_single_turn(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        request_id: u64,
    ) -> Result<AssistantTurn> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let llm = self.llm.clone();
        let model_for_task = self.model.clone();
        let msgs = messages.to_vec();
        let tl = self.model.thinking_level.clone();
        let sid = self.session_id;
        let llm_tools = tools.to_vec();

        tokio::spawn(async move {
            let llm_config = tidev_llm::LlmProviderConfig::from(model_for_task);
            llm.stream_chat(sid, request_id, llm_config, msgs, llm_tools, tx, tl)
                .await;
        });

        let mut turn = AssistantTurn::default();

        while let Some(event) = rx.recv().await {
            match event {
                BackendEvent::Delta { content, .. } => {
                    turn.content.push_str(&content);
                    let _ = self.event_tx.send(BackendEvent::Delta {
                        request_id,
                        content,
                    });
                }
                BackendEvent::ReasoningDelta { content, .. } => {
                    turn.reasoning.push_str(&content);
                    let _ = self.event_tx.send(BackendEvent::ReasoningDelta {
                        request_id,
                        content,
                    });
                }
                BackendEvent::ToolCallUpdated { tool_call: tc_update, .. } => {
                    // Update or add the tool call
                    if let Some(existing) = turn
                        .tool_calls
                        .iter_mut()
                        .find(|tc| tc.id == tc_update.id)
                    {
                        existing.name = tc_update.name.clone();
                        existing.arguments = tc_update.arguments.clone();
                    } else {
                        turn.tool_calls.push(tc_update.clone());
                    }
                    let _ = self.event_tx.send(BackendEvent::ToolCallUpdated {
                        request_id,
                        tool_call: tc_update,
                    });
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
                    let _ = self.event_tx.send(BackendEvent::UsageStats {
                        request_id,
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        model_id,
                        duration_ms,
                    });
                }
                BackendEvent::Finished { turn: finished_turn, .. } => {
                    // Extract finish_reason from the turn if available
                    if let Some(reason) = finished_turn.finish_reason {
                        turn.finish_reason = Some(reason);
                    }
                    break;
                }
                BackendEvent::Failed { error, .. } => {
                    anyhow::bail!("LLM streaming error: {error}");
                }
                _ => {
                    // Forward any other events (Retrying, StreamEnd, etc.)
                    let _ = self.event_tx.send(event);
                }
            }
        }

        Ok(turn)
    }

    /// Execute a batch of external tool calls.
    async fn execute_external_tools(
        &self,
        tool_calls: &[&ToolCall],
        request_id: u64,
    ) -> Result<Vec<ToolExecResult>> {
        // If there is a permission channel, request approval first
        if let Some(ref permission_tx) = self.permission_tx {
            let (response_tx, response_rx) = tokio::sync::oneshot::channel();

            permission_tx
                .send(PendingToolApproval {
                    tool_calls: tool_calls.iter().map(|tc| (*tc).clone()).collect(),
                    mode: self.mode,
                    response_tx,
                })
                .map_err(|_| anyhow::anyhow!("permission channel closed"))?;

            let approved = response_rx
                .await
                .map_err(|_| anyhow::anyhow!("permission response cancelled"))?;

            let mut results = Vec::new();

            for approved_tool in &approved {
                if let Some(rejection) = &approved_tool.rejection {
                    results.push(ToolExecResult {
                        tool_call_id: approved_tool.tool_call.id.clone(),
                        tool_name: approved_tool.tool_call.name.clone(),
                        result: rejection.clone(),
                    });
                    continue;
                }

                match self
                    .execute_external_tool(&approved_tool.tool_call, request_id, approved_tool.allow_outside)
                    .await
                {
                    Ok(mut result) => {
                        result.tool_call_id = approved_tool.tool_call.id.clone();
                        results.push(result);
                    }
                    Err(e) => {
                        let error_result = ToolExecResult {
                            tool_call_id: approved_tool.tool_call.id.clone(),
                            tool_name: approved_tool.tool_call.name.clone(),
                            result: ToolExecutionResult::new(format!("Error: {}", e)),
                        };
                        results.push(error_result);
                    }
                }
            }

            Ok(results)
        } else {
            let mut results = Vec::new();
            for tc in tool_calls {
                match self.execute_external_tool(tc, request_id, false).await {
                    Ok(mut result) => {
                        result.tool_call_id = tc.id.clone();
                        results.push(result);
                    }
                    Err(e) => {
                        results.push(ToolExecResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            result: ToolExecutionResult::new(format!("Error: {e}")),
                        });
                    }
                }
            }
            Ok(results)
        }
    }

    /// Request approval for tool calls from the permission channel.
    #[allow(dead_code)]
    async fn request_tool_approval(
        &self,
        tool_calls: &[ToolCall],
    ) -> Result<Vec<ApprovedTool>> {
        let (response_tx, response_rx) = tokio::sync::oneshot::channel();

        if let Some(ref permission_tx) = self.permission_tx {
            permission_tx
                .send(PendingToolApproval {
                    tool_calls: tool_calls.to_vec(),
                    mode: self.mode,
                    response_tx,
                })
                .map_err(|_| anyhow::anyhow!("permission channel closed"))?;

            let approved = response_rx
                .await
                .map_err(|_| anyhow::anyhow!("permission response cancelled"))?;
            Ok(approved)
        } else {
            // No permission channel — auto-approve all tools
            Ok(tool_calls
                .iter()
                .map(|tc| ApprovedTool {
                    tool_call: tc.clone(),
                    rejection: None,
                    child_session_id: None,
                    allow_outside: false,
                    sensitive_file_approved: false,
                })
                .collect())
        }
    }

    /// Execute a single external tool call.
    async fn execute_external_tool(
        &self,
        tool_call: &ToolCall,
        _request_id: u64,
        allow_outside: bool,
    ) -> Result<ToolExecResult> {
        let store = self.store.lock().await;

        let result = self.tool_registry.execute_call(
            &tokio::runtime::Handle::current(),
            &store,
            self.session_id,
            tool_call,
            self.mode,
            allow_outside,
            false,
        )?;
        drop(store);

        Ok(ToolExecResult {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            result,
        })
    }

    /// Compact conversation context.
    async fn compact_context(&mut self) {
        log::info!("agent_loop[{}]: context compaction triggered", self.session_id);

        let event_tx = self.event_tx.clone();
        let context = &mut self.context;

        if let Err(e) = context
            .compact(tidev_context::CompactionConfig {
                llm: &self.llm,
                model: &self.model,
                conversation: &self.conversation,
                manual: false,
                stream_ctx: None,
                tools: &self.tools,
                mode: self.mode,
            })
            .await
        {
            log::warn!(
                "agent_loop[{}]: context compaction failed: {}",
                self.session_id,
                e
            );
        }

        let _ = event_tx.send(BackendEvent::ContextCompacted {
            compacted: context.summary.is_some(),
            manual: false,
            summary: context.summary.clone(),
            retained_from: context.retained_from,
            error: None,
        });
    }
}

/// Result of executing a single tool call.
#[derive(Clone, Debug)]
pub struct ToolExecResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: ToolExecutionResult,
}
