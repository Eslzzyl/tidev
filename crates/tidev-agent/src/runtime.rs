//! Default runtime implementation for the generic agent loop.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use tidev_llm::message::{AssistantTurn, Message, ToolCall, ToolExecutionResult};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_llm::{LlmClient, LlmProviderConfig, ToolDefinition};

use crate::context::{AgentContext, AgentLoopConfig};
use crate::context_manager::ContextManager;
#[cfg(test)]
use crate::event::AgentEvent;
use crate::event::AgentEventSender;
use crate::loop_::run_agent_loop;
use crate::message_buf::MessageBuffer;
use crate::registry::ToolRegistry;
use crate::scheduler::{ToolCallExecutor, execute_tool_calls};
use crate::tool::ToolContext;
use crate::turn::stream_turn;

/// Persistence boundary for a generic agent runtime.
#[async_trait]
pub trait MessageStore: Send + Sync {
    /// Load the protocol messages for one session.
    async fn load_messages(&self, session_id: uuid::Uuid) -> Result<Vec<Message>>;

    /// Persist protocol messages in the order supplied by the runtime.
    async fn save_messages(&self, session_id: uuid::Uuid, messages: &[Message]) -> Result<()>;

    /// Persist the generic context-compaction state for one session.
    ///
    /// Stores that do not persist context metadata may keep the default no-op
    /// implementation. Product hosts with undo or session reload support
    /// should override it.
    async fn save_context_state(
        &self,
        _session_id: uuid::Uuid,
        _summary: Option<&str>,
        _retained_from: usize,
    ) -> Result<()> {
        Ok(())
    }
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
        steer_signal: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<()> {
        run_agent_loop(
            self,
            AgentLoopConfig {
                session_id: self.session_id,
                system_prompt,
                thinking_level,
                event_tx: self.event_tx.clone(),
                cancel: self.cancel.clone(),
                steer_signal,
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

struct RuntimeToolExecutor {
    tools: Arc<ToolRegistry>,
    context: RuntimeToolContext,
}

#[async_trait]
impl ToolCallExecutor for RuntimeToolExecutor {
    fn is_read_only(&self, tool_call: &ToolCall) -> bool {
        self.tools.is_read_only(&tool_call.name).unwrap_or(false)
    }

    async fn execute(&self, tool_call: ToolCall) -> Result<ToolExecutionResult> {
        self.tools.execute(&tool_call, &self.context).await
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
        stream_turn(
            &self.llm,
            self.model.clone(),
            messages,
            &self.tools.definitions(),
            system_prompt,
            thinking_level,
            request_id,
            self,
            &self.cancel,
        )
        .await
    }

    async fn execute_tools(
        &self,
        tool_calls: &[ToolCall],
        session_id: uuid::Uuid,
        request_id: u64,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        self.ensure_session(session_id)?;
        let executor = Arc::new(RuntimeToolExecutor {
            tools: self.tools.clone(),
            context: RuntimeToolContext {
                workspace_root: self.workspace_root.clone(),
                event_tx: self.event_tx.clone(),
            },
        });
        execute_tool_calls(
            executor,
            tool_calls,
            &self.event_tx,
            &self.cancel,
            request_id,
        )
        .await
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
        let buffer = {
            let messages = self
                .messages
                .lock()
                .map_err(|_| anyhow::anyhow!("agent message buffer is poisoned"))?;
            MessageBuffer::new(messages.load().to_vec())
        };
        let mut context_manager = {
            let context_manager = self
                .context_manager
                .lock()
                .map_err(|_| anyhow::anyhow!("agent context manager is poisoned"))?;
            context_manager.clone()
        };
        let prepared = context_manager
            .prepare_request_messages(
                &self.llm,
                &self.model,
                &self.tools.definitions(),
                &buffer,
                None,
                None,
            )
            .await?;
        {
            let mut shared_context_manager = self
                .context_manager
                .lock()
                .map_err(|_| anyhow::anyhow!("agent context manager is poisoned"))?;
            *shared_context_manager = context_manager;
        }

        if let Some(compaction) = prepared.compaction.as_ref() {
            self.store
                .save_context_state(
                    session_id,
                    Some(&compaction.summary),
                    compaction.retained_from,
                )
                .await?;
            let marker = Message::compaction(&compaction.summary);
            self.store
                .save_messages(session_id, std::slice::from_ref(&marker))
                .await?;
            let mut messages = self
                .messages
                .lock()
                .map_err(|_| anyhow::anyhow!("agent message buffer is poisoned"))?;
            messages.append(marker);
        }
        Ok(prepared.messages)
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
            user_agent: None,
            model_id: "test".into(),
            request_model_id: None,
            system_prompt: None,
            thinking_level: ThinkingLevelType::None,
            extra_body: None,
            max_output_tokens: 128,
            context_window: 0,
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
