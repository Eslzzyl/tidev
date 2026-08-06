//! Default runtime implementation for the generic agent loop.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use tidev_llm::message::{AssistantTurn, Message, ToolCall, ToolExecutionResult};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_llm::{LlmClient, LlmProviderConfig, ToolDefinition};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::context_manager::ContextManager;
use crate::event::{AgentEvent, AgentEventSender, llm_event_to_agent_event};
use crate::loop_::run_agent_loop;
use crate::message_buf::MessageBuffer;
use crate::registry::ToolRegistry;
use crate::tool::ToolContext;

/// Persistence boundary for a generic agent runtime.
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Load the protocol messages for one session.
    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>>;

    /// Persist protocol messages in the order supplied by the runtime.
    async fn save_messages(&self, session_id: uuid::Uuid, messages: &[Message]) -> Result<()>;
}

/// A ready-to-use [`AgentContext`] implementation.
///
/// The runtime owns protocol messages and context state, while persistence is
/// delegated to [`MessageStore`]. Product-specific approval, application data,
/// and subagent behavior remain host responsibilities.
pub struct AgentRuntime {
    session_id: uuid::Uuid,
    llm: LlmClient,
    model: LlmProviderConfig,
    tools: Arc<ToolRegistry>,
    context_manager: Arc<Mutex<ContextManager>>,
    messages: Arc<Mutex<MessageBuffer>>,
    store: Arc<dyn MessageStore>,
    event_tx: AgentEventSender,
    workspace_root: PathBuf,
    cancel: CancellationToken,
}

impl AgentRuntime {
    /// Construct a runtime from already loaded protocol messages.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: uuid::Uuid,
        llm: LlmClient,
        model: LlmProviderConfig,
        tools: Arc<ToolRegistry>,
        context_manager: ContextManager,
        messages: Vec<Message>,
        store: Arc<dyn MessageStore>,
        workspace_root: PathBuf,
        event_tx: impl Into<AgentEventSender>,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            session_id,
            llm,
            model,
            tools,
            context_manager: Arc::new(Mutex::new(context_manager)),
            messages: Arc::new(Mutex::new(MessageBuffer::new(messages))),
            store,
            event_tx: event_tx.into(),
            workspace_root,
            cancel,
        }
    }

    /// Load protocol messages from a store and construct a runtime.
    #[allow(clippy::too_many_arguments)]
    pub async fn from_store(
        llm: LlmClient,
        model: LlmProviderConfig,
        tools: Arc<ToolRegistry>,
        context_manager: ContextManager,
        store: Arc<dyn MessageStore>,
        session_id: uuid::Uuid,
        workspace_root: PathBuf,
        event_tx: impl Into<AgentEventSender>,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let messages = store
            .load_messages(session_id)
            .await
            .context("failed to load agent messages")?;
        Ok(Self::new(
            session_id,
            llm,
            model,
            tools,
            context_manager,
            messages,
            store,
            workspace_root,
            event_tx,
            cancel,
        ))
    }

    /// Run the generic loop using this runtime's event and cancellation channels.
    pub async fn run(
        &self,
        system_prompt: String,
        thinking_level: ThinkingLevelType,
        queued_messages: Arc<Mutex<VecDeque<tidev_llm::message::QueuedUserMessage>>>,
    ) -> Result<()> {
        run_agent_loop(
            self,
            AgentLoopConfig {
                session_id: self.session_id,
                system_prompt,
                thinking_level,
                event_tx: self.event_tx.clone(),
                cancel: self.cancel.clone(),
                queued_messages,
            },
        )
        .await
    }

    /// Return the current protocol messages, including messages not yet exposed
    /// in the context view because they were compacted.
    pub fn stored_messages(&self) -> Vec<Message> {
        self.messages
            .lock()
            .map(|messages| messages.load().to_vec())
            .unwrap_or_default()
    }

    /// Access the runtime context manager for host-controlled compaction.
    pub fn context_manager(&self) -> Arc<Mutex<ContextManager>> {
        self.context_manager.clone()
    }

    /// Return this runtime's event channel.
    pub fn event_sender(&self) -> AgentEventSender {
        self.event_tx.clone()
    }

    /// Return this runtime's cancellation token.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Return the session bound to this runtime's in-memory message buffer.
    pub fn session_id(&self) -> uuid::Uuid {
        self.session_id
    }

    /// Execute one registered tool without entering the agent loop.
    pub async fn execute_tool(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        self.tools.execute(call, self).await
    }

    fn emit(&self, event: AgentEvent) {
        let _ = self.event_tx.send(event);
    }

    fn ensure_session(&self, session_id: uuid::Uuid) -> Result<()> {
        if session_id != self.session_id {
            anyhow::bail!(
                "agent runtime is bound to session {}, got {}",
                self.session_id,
                session_id
            );
        }
        Ok(())
    }

    fn cancelled_result() -> ToolExecutionResult {
        ToolExecutionResult::new("User cancelled the request")
    }

    fn error_result(error: anyhow::Error) -> ToolExecutionResult {
        ToolExecutionResult::new(format!("Error: tool call failed: {error:#}"))
    }

    fn emit_tool_starting(&self, request_id: u64, tool_call: &ToolCall) {
        self.emit(AgentEvent::ToolStarting {
            request_id,
            tool_call: tool_call.clone(),
        });
    }

    fn emit_tool_completed(
        &self,
        request_id: u64,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
    ) {
        self.emit(AgentEvent::ToolCompleted {
            request_id,
            tool_call: tool_call.clone(),
            result: Box::new(result.clone()),
        });
    }
}

struct RuntimeToolContext {
    workspace_root: PathBuf,
    event_tx: AgentEventSender,
}

impl ToolContext for AgentRuntime {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn event_tx(&self) -> AgentEventSender {
        self.event_tx.clone()
    }
}

impl ToolContext for RuntimeToolContext {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn event_tx(&self) -> AgentEventSender {
        self.event_tx.clone()
    }
}

#[async_trait]
impl AgentContext for AgentRuntime {
    fn tools(&self) -> Vec<ToolDefinition> {
        self.tools.definitions()
    }

    fn event_tx(&self) -> AgentEventSender {
        self.event_tx.clone()
    }

    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    async fn stream_turn(
        &self,
        messages: &[Message],
        system_prompt: &str,
        thinking_level: &ThinkingLevelType,
        request_id: u64,
    ) -> Result<AssistantTurn> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let llm = self.llm.clone();
        let mut model = self.model.clone();
        model.system_prompt = Some(system_prompt.to_string());
        let tools = self.tools.definitions();
        let messages = messages.to_vec();
        let thinking_level = thinking_level.clone();

        let handle = tokio::spawn(async move {
            llm.stream_chat(model, messages, tools, tx, thinking_level)
                .await;
        });

        let mut turn = AssistantTurn {
            created_at: Some(Utc::now()),
            ..Default::default()
        };

        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    handle.abort();
                    return Err(anyhow::anyhow!("Stream cancelled by user"));
                }
                event = rx.recv() => {
                    let Some(event) = event else { break };
                    let event = llm_event_to_agent_event(event, request_id);
                    self.emit(event.clone());
                    match event {
                        AgentEvent::Delta { content, .. } => {
                            turn.content.push_str(&content);
                            if turn.reasoning_started_at.is_some()
                                && turn.reasoning_completed_at.is_none()
                            {
                                turn.reasoning_completed_at = Some(Utc::now());
                            }
                        }
                        AgentEvent::ReasoningDelta { content, .. } => {
                            if turn.reasoning_started_at.is_none() {
                                turn.reasoning_started_at = Some(Utc::now());
                            }
                            turn.reasoning.push_str(&content);
                        }
                        AgentEvent::ToolCallUpdated { tool_call, .. } => {
                            turn.upsert_tool_call(tool_call);
                            if turn.reasoning_started_at.is_some()
                                && turn.reasoning_completed_at.is_none()
                            {
                                turn.reasoning_completed_at = Some(Utc::now());
                            }
                        }
                        AgentEvent::UsageStats {
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
                            turn.model_id = Some(model_id);
                            if let Some(ms) = duration_ms.filter(|ms| *ms > 0) {
                                turn.tokens_per_second = Some(output_tokens as f32 / (ms as f32 / 1000.0));
                            }
                        }
                        AgentEvent::Finished { turn: finished_turn, .. } => {
                            turn.responses_output_items = finished_turn.responses_output_items.clone();
                            break;
                        }
                        AgentEvent::Failed { error, .. } => {
                            return Err(anyhow::anyhow!("LLM error: {error}"));
                        }
                        _ => {}
                    }
                }
            }
        }

        turn.completed_at = Some(Utc::now());
        Ok(turn)
    }

    async fn execute_tools(
        &self,
        tool_calls: &[ToolCall],
        session_id: uuid::Uuid,
        request_id: u64,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        self.ensure_session(session_id)?;
        let mut results: Vec<Option<(ToolCall, ToolExecutionResult)>> =
            (0..tool_calls.len()).map(|_| None).collect();
        let mut read_only = Vec::new();
        let mut write = Vec::new();

        for (index, tool_call) in tool_calls.iter().cloned().enumerate() {
            if self.tools.is_read_only(&tool_call.name).unwrap_or(false) {
                read_only.push((index, tool_call));
            } else {
                write.push((index, tool_call));
            }
        }

        if !read_only.is_empty() {
            if self.cancel.is_cancelled() {
                for (index, tool_call) in read_only {
                    let result = Self::cancelled_result();
                    self.emit_tool_completed(request_id, &tool_call, &result);
                    results[index] = Some((tool_call, result));
                }
            } else {
                let mut pending = vec![false; tool_calls.len()];
                let mut tasks = JoinSet::new();
                for (index, tool_call) in read_only {
                    pending[index] = true;
                    self.emit_tool_starting(request_id, &tool_call);
                    let registry = self.tools.clone();
                    let context = RuntimeToolContext {
                        workspace_root: self.workspace_root.clone(),
                        event_tx: self.event_tx.clone(),
                    };
                    tasks.spawn(async move {
                        let result = registry
                            .execute(&tool_call, &context)
                            .await
                            .unwrap_or_else(Self::error_result);
                        (index, tool_call, result)
                    });
                }

                loop {
                    tokio::select! {
                        _ = self.cancel.cancelled() => {
                            tasks.abort_all();
                            for (index, tool_call) in tool_calls.iter().cloned().enumerate() {
                                if pending[index] {
                                    let result = Self::cancelled_result();
                                    self.emit_tool_completed(request_id, &tool_call, &result);
                                    results[index] = Some((tool_call, result));
                                }
                            }
                            break;
                        }
                        joined = tasks.join_next() => {
                            match joined {
                                Some(Ok((index, tool_call, result))) => {
                                    pending[index] = false;
                                    self.emit_tool_completed(request_id, &tool_call, &result);
                                    results[index] = Some((tool_call, result));
                                }
                                Some(Err(error)) => {
                                    if let Some(index) = pending.iter().position(|is_pending| *is_pending) {
                                        pending[index] = false;
                                        let tool_call = tool_calls[index].clone();
                                        let result = ToolExecutionResult::new(format!("Error: tool task failed: {error}"));
                                        self.emit_tool_completed(request_id, &tool_call, &result);
                                        results[index] = Some((tool_call, result));
                                    }
                                }
                                None => break,
                            }
                        }
                    }
                }
            }
        }

        for (index, tool_call) in write {
            if self.cancel.is_cancelled() {
                let result = Self::cancelled_result();
                self.emit_tool_completed(request_id, &tool_call, &result);
                results[index] = Some((tool_call, result));
                continue;
            }

            self.emit_tool_starting(request_id, &tool_call);
            let context = RuntimeToolContext {
                workspace_root: self.workspace_root.clone(),
                event_tx: self.event_tx.clone(),
            };
            let result = self
                .tools
                .execute(&tool_call, &context)
                .await
                .unwrap_or_else(Self::error_result);
            self.emit_tool_completed(request_id, &tool_call, &result);
            results[index] = Some((tool_call, result));
        }

        Ok(results.into_iter().flatten().collect())
    }

    async fn save_messages(&self, session_id: uuid::Uuid, messages: &[Message]) -> Result<()> {
        self.ensure_session(session_id)?;
        self.store.save_messages(session_id, messages).await?;
        let mut buffer = self
            .messages
            .lock()
            .map_err(|_| anyhow::anyhow!("agent message buffer is poisoned"))?;
        for message in messages {
            buffer.append(message.clone());
        }
        Ok(())
    }

    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>> {
        self.ensure_session(session_id)?;
        let buffer = self
            .messages
            .lock()
            .map_err(|_| anyhow::anyhow!("agent message buffer is poisoned"))?;
        let context_manager = self
            .context_manager
            .lock()
            .map_err(|_| anyhow::anyhow!("agent context manager is poisoned"))?;
        Ok(context_manager.build_request_messages(&buffer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use async_trait::async_trait;
    use tidev_llm::message::MessageRole;

    struct MemoryStore {
        messages: Mutex<Vec<Message>>,
        saved: Mutex<Vec<Vec<Message>>>,
    }

    #[async_trait]
    impl MessageStore for MemoryStore {
        async fn load_messages(&self, _session_id: uuid::Uuid) -> Result<Vec<Message>> {
            Ok(self.messages.lock().unwrap().clone())
        }

        async fn save_messages(&self, _session_id: uuid::Uuid, messages: &[Message]) -> Result<()> {
            self.saved.lock().unwrap().push(messages.to_vec());
            Ok(())
        }
    }

    struct TestTool {
        name: &'static str,
        read_only: bool,
        delay_ms: u64,
    }

    #[async_trait]
    impl crate::tool::Tool for TestTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                display_name: self.name.to_string(),
                description: "test tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }
        }

        fn read_only(&self) -> bool {
            self.read_only
        }

        async fn execute(
            &self,
            _args: serde_json::Value,
            _context: &dyn ToolContext,
        ) -> Result<ToolExecutionResult> {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            Ok(ToolExecutionResult::new(self.name))
        }
    }

    fn model() -> LlmProviderConfig {
        LlmProviderConfig {
            provider_id: "test".into(),
            api_type: tidev_llm::ApiType::OpenAiChatCompletions,
            api_key: None,
            base_url: "http://127.0.0.1:1".into(),
            model_id: "test".into(),
            request_model_id: None,
            system_prompt: None,
            thinking_level: ThinkingLevelType::None,
            extra_body: None,
            max_output_tokens: 128,
            context_window: 1024,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
        }
    }

    fn runtime(
        tools: Vec<TestTool>,
        cancel: CancellationToken,
    ) -> (
        AgentRuntime,
        tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
        Arc<MemoryStore>,
    ) {
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut registry = ToolRegistry::new(0);
        for tool in tools {
            registry.register(tool);
        }
        let store = Arc::new(MemoryStore {
            messages: Mutex::new(vec![Message::new(MessageRole::User, "hello")]),
            saved: Mutex::new(Vec::new()),
        });
        let llm = LlmClient::new(false, 0, false, 0).unwrap();
        let runtime = AgentRuntime::new(
            uuid::Uuid::from_u128(1),
            llm,
            model(),
            Arc::new(registry),
            ContextManager::new(),
            store.messages.lock().unwrap().clone(),
            store.clone(),
            PathBuf::from("/workspace"),
            event_tx,
            cancel,
        );
        (runtime, event_rx, store)
    }

    fn call(name: &str) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.to_string(),
            arguments: "{}".into(),
            thought_signature: None,
        }
    }

    #[tokio::test]
    async fn loads_context_view_and_persists_in_order() {
        let (runtime, _events, store) = runtime(Vec::new(), CancellationToken::new());
        let session_id = uuid::Uuid::from_u128(1);
        let visible = runtime.load_messages(session_id).await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].content, "hello");

        let first = Message::new(MessageRole::Assistant, "first");
        let second = Message::new(MessageRole::Tool, "second");
        runtime
            .save_messages(session_id, &[first.clone(), second.clone()])
            .await
            .unwrap();

        let stored = runtime.stored_messages();
        assert_eq!(stored.len(), 3);
        assert_eq!(stored[1].id, first.id);
        assert_eq!(stored[1].role, MessageRole::Assistant);
        assert_eq!(stored[1].content, "first");
        assert_eq!(stored[2].id, second.id);
        assert_eq!(stored[2].role, MessageRole::Tool);
        assert_eq!(stored[2].content, "second");
        assert_eq!(store.saved.lock().unwrap().len(), 1);
        assert_eq!(store.saved.lock().unwrap()[0].len(), 2);
    }

    #[tokio::test]
    async fn executes_reads_in_parallel_but_returns_call_order() {
        let (runtime, mut events, _store) = runtime(
            vec![
                TestTool {
                    name: "slow-read",
                    read_only: true,
                    delay_ms: 30,
                },
                TestTool {
                    name: "fast-read",
                    read_only: true,
                    delay_ms: 1,
                },
                TestTool {
                    name: "write",
                    read_only: false,
                    delay_ms: 1,
                },
            ],
            CancellationToken::new(),
        );
        let calls = vec![call("slow-read"), call("fast-read"), call("write")];
        let results = runtime
            .execute_tools(&calls, uuid::Uuid::from_u128(1), 7)
            .await
            .unwrap();
        assert_eq!(
            results
                .iter()
                .map(|(_, result)| result.output.as_str())
                .collect::<Vec<_>>(),
            vec!["slow-read", "fast-read", "write"]
        );

        let mut completed = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let AgentEvent::ToolCompleted { tool_call, .. } = event {
                completed.push(tool_call.name);
            }
        }
        assert_eq!(completed, vec!["fast-read", "slow-read", "write"]);
    }

    #[tokio::test]
    async fn cancellation_produces_results_for_pending_calls() {
        let cancel = CancellationToken::new();
        let (runtime, mut events, _store) = runtime(
            vec![TestTool {
                name: "slow-read",
                read_only: true,
                delay_ms: 100,
            }],
            cancel.clone(),
        );
        let task = tokio::spawn(async move {
            runtime
                .execute_tools(&[call("slow-read")], uuid::Uuid::from_u128(1), 1)
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        cancel.cancel();
        let results = task.await.unwrap();
        assert_eq!(results[0].1.output, "User cancelled the request");
        let mut saw_completed = false;
        while let Ok(event) = events.try_recv() {
            if matches!(event, AgentEvent::ToolCompleted { .. }) {
                saw_completed = true;
            }
        }
        assert!(saw_completed);
    }
}
