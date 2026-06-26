//! AgentLoop — the core LLM ↔ tool execution loop.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! Events carry NO `session_id` — the receiver already knows which session
//! the events belong to (Per-Session Event Bus).
//!
//! Permission approval, hooks, and tool execution are injected via fields
//! at construction time — no tight coupling to frontends.
//!
//! Subagent spawning is handled by creating a child AgentLoop with its own
//! session and running it synchronously within the parent's execution flow.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
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

use crate::session_manager::SessionManager;
use crate::types::{ApprovedTool, ControlEvent, PendingToolApproval, compose_static_system_prompt};
use crate::AgentDefinition;

/// The per-session agent loop.
///
/// Architecture (Per-Session Event Bus):
/// - Owns its own event channel (`event_tx`)
/// - Receives permission approvals through `permission_tx`
/// - Runs hooks after tool execution via `hooks`
/// - Delegates subagent spawning through `session_manager`
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
    pub agent_type: tidev_types::agent::AgentType,
    /// Workspace root for this session.
    pub workspace_root: PathBuf,
    /// The composed static system prompt (frozen for session lifetime).
    pub system_prompt: String,
    /// Optional channel for interactive tool permission approval.
    pub permission_tx: Option<UnboundedSender<PendingToolApproval>>,
    /// Hook engine for PostToolUse hooks.
    pub hooks: tidev_hooks::HookEngine,
    /// SessionManager for spawning subagent sessions.
    pub session_manager: SessionManager,
    /// Whether this loop can delegate to sub-agents.
    /// Child sessions set this to `false` to avoid async recursion.
    pub can_delegate: bool,
    /// Control event channel for parent-child coordination.
    /// Used to notify SessionManager about subagent lifecycle events.
    pub control_tx: tokio::sync::mpsc::UnboundedSender<ControlEvent>,
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
    ///
    /// Returns a `Pin<Box<dyn Future>>` instead of being `async` so that
    /// child sessions can call this method without creating async recursion
    /// in the call graph (`run_subagent_inner` → `into_run_fut` → `run_subagent_inner`
    /// is not a cycle because the compiler sees `into_run_fut` as a non-async
    /// function that returns a future).
    fn into_run_fut(
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

            // 4. Persist the assistant turn
            let assistant_msg = {
                let mut msg = Message::new(MessageRole::Assistant, &turn.content);
                if !turn.tool_calls.is_empty() {
                    msg.tool_calls = turn.tool_calls.clone();
                }
                if !turn.reasoning.is_empty() {
                    msg.reasoning = turn.reasoning.clone();
                }
                msg
            };
            self.conversation.push(assistant_msg);

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
                for tc in &internal {
                    if tc.name == "todo" {
                        let result = self.execute_external_tool(tc, request_id, false).await?;
                        let r = ToolExecResult {
                            tool_call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            result: result.result,
                        };
                        self.conversation.push(Message::tool_result(
                            &r.tool_call_id, &r.tool_name, r.result.clone(),
                        ));
                    }
                    if tc.name == "task" {
                        if self.can_delegate {
                            // Full subagent execution
                            let result = self.run_subagent(tc).await;
                            let _ = self.event_tx.send(BackendEvent::ToolCompleted {
                                request_id,
                                tool_call: (*tc).clone(),
                                result: result.clone(),
                            });
                            self.conversation.push(Message::tool_result(
                                &tc.id, &tc.name, result.clone(),
                            ));
                        } else {
                            // Child sessions: task tool is not available
                            let result = ToolExecutionResult::new(
                                "Subagent delegation is not available in this context.".to_string(),
                            );
                            let _ = self.event_tx.send(BackendEvent::ToolCompleted {
                                request_id,
                                tool_call: (*tc).clone(),
                                result: result.clone(),
                            });
                            self.conversation.push(Message::tool_result(
                                &tc.id, &tc.name, result.clone(),
                            ));
                        }
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

            // 8. Generate new request ID and continue loop
            request_id = request_id.wrapping_add(1);

            // 9. Notify frontend about the new turn
            let _ = self.event_tx.send(BackendEvent::TurnStarting {
                request_id,
            });
        }

        Ok(())
        }) // end of Box::pin(async move { })
    }

    /// Run the agent loop for a sub-agent session.
    ///
    /// Similar to `run()` but does not own `self` — instead creates and
    /// runs a new AgentLoop for the child, then returns the result.
    /// Used by the `task` tool to delegate to specialist sub-agents.
    pub async fn run_subagent(&self, tool_call: &ToolCall) -> ToolExecutionResult {
        let result = self.run_subagent_inner(tool_call).await;
        match result {
            Ok(output) => ToolExecutionResult::new(output),
            Err(e) => ToolExecutionResult::new(format!("Subagent failed: {e}")),
        }
    }

    async fn run_subagent_inner(&self, tool_call: &ToolCall) -> Result<String> {
        // 1. Parse task arguments
        let args: TaskArgs = serde_json::from_str(&tool_call.arguments)
            .map_err(|e| anyhow::anyhow!("failed to parse task arguments: {e}"))?;

        let agent_type = tidev_types::agent::AgentType::parse(&args.subagent_type)
            .ok_or_else(|| anyhow::anyhow!(
                "unknown subagent type '{}': expected one of explorer, librarian, oracle, designer, fixer",
                args.subagent_type
            ))?;

        let description = args.description.trim();
        let prompt = args.prompt.trim();
        anyhow::ensure!(!description.is_empty(), "task description cannot be empty");
        anyhow::ensure!(!prompt.is_empty(), "task prompt cannot be empty");

        log::info!(
            "run_subagent: starting {} subagent for '{}' (parent={})",
            agent_type.display_name(),
            description,
            self.session_id
        );

        // 2. Create child session
        let child_session_id = Uuid::new_v4();
        let (child_event_tx, _child_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let child_cancel_token = self.cancel_token.child_token();

        // Determine child model — use parent model or agent-specific override
        let child_model = self.model.clone();

        // Create session in store
        {
            let store = self.store.lock().await;
            store.create_session(
                child_session_id,
                &self.workspace_root,
                &child_model.provider_id,
                &child_model.provider_display_name,
                &child_model.model_id,
                &child_model.display_name,
                agent_type.display_name(),
            )?;
        }

        // 3. Compose child system prompt
        let agent_def = AgentDefinition::new(agent_type);
        let child_prompt = compose_static_system_prompt(&agent_def.system_prompt, &self.workspace_root);

        // Store system prompt
        {
            let store = self.store.lock().await;
            store.update_session_system_prompt(child_session_id, &child_prompt)?;
        }

        // 4. Create bootstrap message with the task prompt
        let bootstrap_msg = Message::new(MessageRole::User, prompt);
        {
            let store = self.store.lock().await;
            store.append_message(child_session_id, &bootstrap_msg)?;
        }

        // Load bootstrap messages for child conversation
        let child_messages = {
            let store = self.store.lock().await;
            store.load_messages(child_session_id).unwrap_or_default()
        };

        // 5. Filter tools for child agent type
        let (child_tools, _) = self.restrict_tools_for_agent(agent_type);

        // 6. Create and run child AgentLoop
        let mut child_conv = Conversation::new(
            child_session_id,
            self.workspace_root.display().to_string(),
            &child_model.provider_id,
            &child_model.provider_display_name,
            &child_model.model_id,
            &child_model.display_name,
            description,
        );
        child_conv.messages = child_messages;

        let child = AgentLoop {
            session_id: child_session_id,
            model: child_model,
            conversation: child_conv,
            context: tidev_context::ContextManager::new(),
            tools: child_tools,
            tool_registry: self.tool_registry.clone(),
            store: self.store.clone(),
            llm: self.llm.clone(),
            event_tx: child_event_tx,
            cancel_token: child_cancel_token,
            mode: self.mode,
            agent_type,
            workspace_root: self.workspace_root.clone(),
            system_prompt: child_prompt,
            permission_tx: None, // auto-approve for sub-agents
            hooks: tidev_hooks::HookEngine::new(
                Default::default(),
                self.workspace_root.clone(),
            ),
            session_manager: self.session_manager.clone(),
            can_delegate: false,
            control_tx: self.control_tx.clone(),
        };

        // Notify SessionManager about the child session via ControlEvent
        let (ack_tx, _ack_rx) = tokio::sync::oneshot::channel();
        let _ = self.control_tx.send(ControlEvent::SubtaskRequested {
            parent_session_id: self.session_id,
            child_session_id,
            agent_type,
            description: description.to_string(),
            ack_tx,
        });

        // Notify parent TUI about child session
        let _ = self.event_tx.send(BackendEvent::InstructionsLoaded {
            sources: vec![format!("Subagent {} started: {}", agent_type.display_name(), description)],
        });

        // Run child agent inline. The child has can_delegate=false so it
        // will not attempt to spawn sub-agents (avoiding async recursion).
        // Run child agent via into_run_fut to break the async recursion detection.
        if let Err(e) = child.into_run_fut(1).await {
            log::warn!("run_subagent: child session failed: {e}");
        }

        // Notify SessionManager that child completed
        let _ = self.control_tx.send(ControlEvent::SubtaskCompleted {
            child_session_id,
            success: true,
        });
        // 7. Get last assistant message from child session
        let last_content = {
            let store = self.store.lock().await;
            let msgs = store.load_messages(child_session_id).unwrap_or_default();
            msgs.iter()
                .rev()
                .find(|m| m.role == MessageRole::Assistant && !m.streaming)
                .map(|m| m.content.clone())
                .unwrap_or_default()
        };

        log::info!(
            "run_subagent: {} subagent '{}' completed (child={})",
            agent_type.display_name(),
            description,
            child_session_id
        );

        Ok(last_content)
    }

    /// Filter tool definitions for a specific agent type based on its
    /// default_tool_restrictions.
    fn restrict_tools_for_agent(
        &self,
        agent_type: tidev_types::agent::AgentType,
    ) -> (Vec<tidev_tools::ToolDefinition>, bool) {
        let is_read_only = agent_type.is_read_only();
        match agent_type.default_tool_restrictions() {
            Some(allowed) => {
                let filtered: Vec<tidev_tools::ToolDefinition> = self
                    .tools
                    .iter()
                    .filter(|t| allowed.contains(&t.name.as_str()))
                    .cloned()
                    .collect();
                (filtered, is_read_only)
            }
            None => (self.tools.clone(), is_read_only),
        }
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
            // Forward event to frontend
            let _ = self.event_tx.send(event.clone());

            match event {
                BackendEvent::Delta { content, .. } => {
                    turn.content.push_str(&content);
                }
                BackendEvent::ToolCallUpdated { tool_call, .. } => {
                    turn.upsert_tool_call(tool_call);
                }
                BackendEvent::ReasoningDelta { content, .. } => {
                    turn.reasoning.push_str(&content);
                }
                BackendEvent::Finished { turn: finished_turn, .. } => {
                    if !finished_turn.content.is_empty() {
                        turn.content = finished_turn.content;
                    }
                    if !finished_turn.tool_calls.is_empty() {
                        turn.tool_calls = finished_turn.tool_calls;
                    }
                    if !finished_turn.reasoning.is_empty() {
                        turn.reasoning = finished_turn.reasoning;
                    }
                    turn.input_tokens = finished_turn.input_tokens;
                    turn.output_tokens = finished_turn.output_tokens;
                    turn.total_tokens = finished_turn.total_tokens;
                    turn.cache_read_tokens = finished_turn.cache_read_tokens;
                    turn.cache_write_tokens = finished_turn.cache_write_tokens;
                    turn.model_id = finished_turn.model_id;
                    turn.tokens_per_second = finished_turn.tokens_per_second;
                    break;
                }
                BackendEvent::Failed { error, .. } => {
                    anyhow::bail!("LLM turn error: {}", error);
                }
                _ => {}
            }
        }

        Ok(turn)
    }

    /// Execute external tool calls through the ToolRegistry.
    async fn execute_external_tools(
        &self,
        tool_calls: &[&ToolCall],
        request_id: u64,
    ) -> Result<Vec<ToolExecResult>> {
        let mut results = Vec::new();

        // Check if any tools need permission approval
        let needs_approval = self.needs_tool_approval(tool_calls);
        let approved_tools = if needs_approval {
            self.request_tool_approval(tool_calls).await?
        } else {
            tool_calls
                .iter()
                .map(|tc| ApprovedTool {
                    tool_call: (*tc).clone(),
                    rejection: None,
                    child_session_id: None,
                    allow_outside: false,
                    sensitive_file_approved: false,
                })
                .collect()
        };

        // Execute approved tools
        for approved in &approved_tools {
            if let Some(ref rejection) = approved.rejection {
                results.push(ToolExecResult {
                    tool_call_id: approved.tool_call.id.clone(),
                    tool_name: approved.tool_call.name.clone(),
                    result: rejection.clone(),
                });
                continue;
            }

            match self
                .execute_external_tool(&approved.tool_call, request_id, approved.allow_outside)
                .await
            {
                Ok(mut result) => {
                    result.tool_call_id = approved.tool_call.id.clone();
                    results.push(result);
                }
                Err(e) => {
                    let error_result = ToolExecResult {
                        tool_call_id: approved.tool_call.id.clone(),
                        tool_name: approved.tool_call.name.clone(),
                        result: ToolExecutionResult::new(format!("Error: {}", e)),
                    };
                    results.push(error_result);
                }
            }
        }

        Ok(results)
    }

    /// Check if any tools need user permission approval.
    fn needs_tool_approval(&self, tool_calls: &[&ToolCall]) -> bool {
        self.permission_tx.is_some() && !tool_calls.is_empty()
    }

    /// Request tool approval from the frontend via the permission channel.
    async fn request_tool_approval(
        &self,
        tool_calls: &[&ToolCall],
    ) -> Result<Vec<ApprovedTool>> {
        let permission_tx = self
            .permission_tx
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no permission channel available"))?;

        let (response_tx, response_rx) = oneshot::channel();

        let pending = PendingToolApproval {
            tool_calls: tool_calls.iter().map(|tc| (*tc).clone()).collect(),
            mode: self.mode,
            response_tx,
        };

        let _ = permission_tx.send(pending);

        let approved = response_rx
            .await
            .map_err(|_| anyhow::anyhow!("permission channel closed"))?;

        Ok(approved)
    }

    /// Execute a single external tool via the ToolRegistry.
    async fn execute_external_tool(
        &self,
        tool_call: &ToolCall,
        request_id: u64,
        allow_outside: bool,
    ) -> Result<ToolExecResult> {
        let runtime_handle = tokio::runtime::Handle::current();
        let store = self.store.lock().await;

        let result = self
            .tool_registry
            .execute_call(
                &runtime_handle,
                &store,
                self.session_id,
                tool_call,
                self.mode,
                allow_outside,
                false,
            )?;

        drop(store);

        let _ = self.event_tx.send(BackendEvent::ToolCompleted {
            request_id,
            tool_call: tool_call.clone(),
            result: result.clone(),
        });

        Ok(ToolExecResult {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            result,
        })
    }

    /// Compact the context when it grows too large.
    async fn compact_context(&mut self) {
        let config = tidev_context::CompactionConfig {
            llm: &self.llm,
            model: &self.model,
            conversation: &self.conversation,
            manual: false,
            stream_ctx: None,
            tools: &self.tools,
            mode: self.mode,
        };

        match self.context.compact_if_needed(config).await {
            Ok(_) => {
                log::info!(
                    "agent_loop[{}]: context compaction succeeded",
                    self.session_id
                );
            }
            Err(e) => {
                log::warn!(
                    "agent_loop[{}]: context compaction failed: {}",
                    self.session_id,
                    e
                );
            }
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

/// Arguments for the `task` tool.
#[derive(serde::Deserialize)]
struct TaskArgs {
    description: String,
    prompt: String,
    subagent_type: String,
}

/// Result of executing a single tool call.
#[derive(Clone, Debug)]
pub struct ToolExecResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub result: ToolExecutionResult,
}
