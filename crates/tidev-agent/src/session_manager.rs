//! SessionManager — manages session lifecycle with Per-Session Event Bus.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! The SessionManager is responsible for spawning, cancelling, and listing
//! active sessions. It also resolves subagent model overrides and creates
//! child sessions with correctly filtered tool lists.
//!
//! Frontend-specific state (workspace_root, config, tools, hooks, etc.)
//! is NOT stored here — it lives in the frontend and is passed to
//! `run_agent_loop_with_permission_channel` as needed.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use chrono::Utc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::{
    BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};
use tidev_storage::SessionStore;

use crate::agent_loop::AgentLoop;
use crate::types::{
    AgentLoopConfig, FrontendEvent, PendingToolApproval, SessionConfig, SessionHandle, SessionInfo,
};
use crate::{AgentDefinition, compose_static_system_prompt};

/// Manages all active sessions, each with its own event bus.
///
/// Architecture (Per-Session Event Bus):
/// - `store`: shared persistence layer
/// - `llm_client`: shared LLM client
/// - `active`: map of session ID -> active session state
/// - `config`: shared app config for resolving agent model overrides
/// - `auth_store`: auth store for model resolution
/// - `tool_registry`: tool registry for model-aware tool filtering
/// - `frontend_tx`: channel to notify frontend of subagent lifecycle events
#[derive(Clone)]
pub struct SessionManager {
    pub store: Arc<AsyncMutex<SessionStore>>,
    pub llm_client: tidev_llm::LlmClient,
    pub active: Arc<AsyncMutex<HashMap<Uuid, ActiveSession>>>,
    /// Shared app config for resolving agent model overrides.
    pub config: Arc<tokio::sync::RwLock<tidev_config::AppConfig>>,
    /// Auth store for model resolution.
    pub auth_store: Arc<tidev_config::AuthStore>,
    /// Tool registry for model-aware tool filtering.
    pub tool_registry: tidev_tools::ToolRegistry,
    /// Channel to notify the frontend of subagent lifecycle events.
    pub frontend_tx: UnboundedSender<FrontendEvent>,
}

pub struct ActiveSession {
    pub agent_type: tidev_types::agent::AgentType,
    pub parent_session_id: Option<Uuid>,
    pub cancel_token: CancellationToken,
    pub event_tx: UnboundedSender<BackendEvent>,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

impl SessionManager {
    /// Create a new SessionManager with shared resources.
    pub fn new(
        store: Arc<AsyncMutex<SessionStore>>,
        llm_client: tidev_llm::LlmClient,
        config: Arc<tokio::sync::RwLock<tidev_config::AppConfig>>,
        auth_store: Arc<tidev_config::AuthStore>,
        tool_registry: tidev_tools::ToolRegistry,
        frontend_tx: UnboundedSender<FrontendEvent>,
    ) -> Self {
        Self {
            store,
            llm_client,
            active: Arc::new(AsyncMutex::new(HashMap::new())),
            config,
            auth_store,
            tool_registry,
            frontend_tx,
        }
    }

    /// Spawn a new session and return a handle to receive its events.
    ///
    /// This creates the session in the database, sets up an independent
    /// event channel, and registers the session in the active map.
    /// The caller is responsible for starting the AgentLoop.
    pub async fn spawn(&self, config: SessionConfig) -> SessionHandle {
        let session_id = Uuid::new_v4();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel_token = CancellationToken::new();

        // Create session in store
        let workspace_root = config
            .workspace_root
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("/"));
        {
            let store = self.store.lock().await;
            let _ = store.create_session(
                session_id,
                workspace_root,
                &config.model.provider_id,
                &config.model.provider_display_name,
                &config.model.model_id,
                &config.model.display_name,
                "agent",
            );
        }

        // Register active session
        {
            let mut active = self.active.lock().await;
            active.insert(
                session_id,
                ActiveSession {
                    agent_type: tidev_types::agent::AgentType::General,
                    parent_session_id: config.parent_session_id,
                    cancel_token: cancel_token.clone(),
                    event_tx: event_tx.clone(),
                    started_at: Utc::now(),
                },
            );
        }

        log::info!(
            "SessionManager::spawn: created session {} (parent: {:?})",
            session_id,
            config.parent_session_id
        );

        SessionHandle {
            session_id,
            event_rx,
            cancel_token,
        }
    }

    /// Run the agent loop with a permission approval channel.
    ///
    /// Convenience method that builds an AgentLoop from the provided config
    /// and additional runtime resources, then runs it asynchronously.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_agent_loop_with_permission_channel<'a>(
        &self,
        config: AgentLoopConfig<'a>,
        request_id: u64,
        permission_tx: UnboundedSender<PendingToolApproval>,
        tool_registry: tidev_tools::ToolRegistry,
        hooks: tidev_hooks::HookEngine,
    ) -> anyhow::Result<()> {
        log::info!(
            "run_agent_loop_with_permission_channel: starting for session {}",
            config.session_id
        );

        let tools = tool_registry.definitions_for_model(&config.model);
        let store = self.store.clone();
        let llm = self.llm_client.clone();
        let event_tx = config.event_tx.clone();
        let cancel_token = config.cancel_token.unwrap_or_default();

        let conversation = {
            let store_guard = store.lock().await;
            let msgs = store_guard.load_messages(config.session_id).unwrap_or_default();
            let mut conv = tidev_session::session::Conversation::new(
                config.session_id,
                "".to_string(),
                &config.model.provider_id,
                &config.model.provider_display_name,
                &config.model.model_id,
                &config.model.display_name,
                "Session",
            );
            conv.messages = msgs;
            conv
        };

        let agent_loop = AgentLoop {
            session_id: config.session_id,
            model: config.model,
            conversation,
            context: config.context_manager.clone(),
            tools,
            store,
            llm,
            event_tx,
            cancel_token,
            mode: config.mode,
            agent_type: tidev_types::agent::AgentType::General,
            workspace_root: config.workspace_root,
            system_prompt: config.system_prompt,
            permission_tx: Some(permission_tx),
            hooks,
            tool_registry,
            session_manager: self.clone(),
            can_delegate: true,
        };

        agent_loop.run(request_id).await
    }

    /// Create and run a subagent session, returning the result.
    ///
    /// This is called by the parent AgentLoop when it encounters a `task` tool call.
    /// Handles: model resolution → tool filtering → child session creation →
    /// child execution → result collection. Notifies the frontend via `FrontendEvent`
    /// so it can subscribe to the child's event channel for inline rendering.
    pub async fn run_subagent(
        &self,
        parent_session_id: Uuid,
        parent_model: &tidev_config::ActiveModel,
        workspace_root: &Path,
        parent_system_prompt_for_thinking: &str,
        tool_call: &ToolCall,
    ) -> ToolExecutionResult {
        let result = self
            .run_subagent_inner(
                parent_session_id,
                parent_model,
                workspace_root,
                parent_system_prompt_for_thinking,
                tool_call,
            )
            .await;
        match result {
            Ok(output) => ToolExecutionResult::new(output),
            Err(e) => ToolExecutionResult::new(format!("Subagent failed: {e}")),
        }
    }

    async fn run_subagent_inner(
        &self,
        parent_session_id: Uuid,
        parent_model: &tidev_config::ActiveModel,
        workspace_root: &Path,
        _parent_system_prompt: &str,
        tool_call: &ToolCall,
    ) -> Result<String> {
        use tidev_tools::TaskArgs;

        // 1. Parse task arguments
        let args: TaskArgs = serde_json::from_str(&tool_call.arguments)
            .map_err(|e| anyhow!("failed to parse task arguments: {e}"))?;

        let agent_type = tidev_types::agent::AgentType::parse(&args.subagent_type)
            .ok_or_else(|| anyhow!(
                "unknown subagent type '{}': expected one of explorer, librarian, oracle, designer, fixer",
                args.subagent_type
            ))?;

        let description = args.description.trim().to_string();
        let prompt = args.prompt.trim().to_string();
        ensure!(!description.is_empty(), "task description cannot be empty");
        ensure!(!prompt.is_empty(), "task prompt cannot be empty");

        // 2. Resolve child model
        let child_model = self.resolve_child_model(agent_type, parent_model).await?;

        // 3. Filter tools for child model
        let mut child_tools = self.tool_registry.definitions_for_model(&child_model);

        // 4. Apply agent type restrictions + remove task tool
        if let Some(allowed) = agent_type.default_tool_restrictions() {
            child_tools.retain(|t| allowed.contains(&t.name.as_str()));
        }
        child_tools.retain(|t| t.name != "task");

        // 5. Create child event channel
        let child_session_id = Uuid::new_v4();
        let (child_event_tx, child_event_rx) = tokio::sync::mpsc::unbounded_channel();
        let child_cancel_token = CancellationToken::new();

        // 6. Persist child session (with parent reference)
        {
            let store = self.store.lock().await;
            store.create_session_with_parent(
                child_session_id,
                parent_session_id,
                workspace_root,
                &child_model.provider_id,
                &child_model.provider_display_name,
                &child_model.model_id,
                &child_model.display_name,
                &format!("Task ({}): {}", agent_type.display_name(), description),
            )?;
        }

        // 7. Build and persist child system prompt
        let agent_def = AgentDefinition::new(agent_type);
        let child_system_prompt = compose_static_system_prompt(
            &agent_def.system_prompt,
            workspace_root,
        );
        {
            let store = self.store.lock().await;
            store.update_session_system_prompt(child_session_id, &child_system_prompt)?;
        }

        // 8. Store bootstrap message (user prompt)
        let bootstrap_msg = Message::new(MessageRole::User, &prompt);
        {
            let store = self.store.lock().await;
            store.append_message(child_session_id, &bootstrap_msg)?;
        }

        // 9. Load messages for child conversation
        let child_messages = {
            let store = self.store.lock().await;
            store.load_messages(child_session_id).unwrap_or_default()
        };

        // 10. Notify frontend: new subagent session available
        let _ = self.frontend_tx.send(FrontendEvent::SubagentSpawned {
            child_session_id,
            parent_session_id,
            agent_type,
            description: description.clone(),
            event_rx: child_event_rx,
        });

        // 11. Build child conversation
        let mut child_conv = tidev_session::session::Conversation::new(
            child_session_id,
            workspace_root.display().to_string(),
            &child_model.provider_id,
            &child_model.provider_display_name,
            &child_model.model_id,
            &child_model.display_name,
            &description,
        );
        child_conv.messages = child_messages;

        // 12. Build child AgentLoop
        let child = AgentLoop {
            session_id: child_session_id,
            model: child_model,
            conversation: child_conv,
            context: tidev_context::ContextManager::new(),
            tools: child_tools,
            store: self.store.clone(),
            llm: self.llm_client.clone(),
            event_tx: child_event_tx,
            cancel_token: child_cancel_token,
            mode: tidev_types::prompts::SessionMode::Build,
            agent_type,
            workspace_root: workspace_root.to_path_buf(),
            system_prompt: child_system_prompt,
            permission_tx: None, // auto-approve for sub-agents
            hooks: tidev_hooks::HookEngine::new(
                Default::default(),
                workspace_root.to_path_buf(),
            ),
            session_manager: self.clone(),
            tool_registry: self.tool_registry.clone(),
            can_delegate: false,
        };

        // 13. Run child agent
        if let Err(e) = child.into_run_fut(1).await {
            log::warn!("run_subagent: child session failed: {e}");
        }

        // 14. Notify frontend: subagent completed
        let _ = self.frontend_tx.send(FrontendEvent::SubagentFinished {
            child_session_id,
            parent_session_id,
        });

        // 15. Read result from child session
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

    /// Resolve the child model for a subagent, checking:
    /// 1. `[agent.models]` config override
    /// 2. `AgentDefinition.model_override`
    /// 3. Fallback to parent model
    async fn resolve_child_model(
        &self,
        agent_type: tidev_types::agent::AgentType,
        parent_model: &tidev_config::ActiveModel,
    ) -> Result<tidev_config::ActiveModel> {
        // Try config-level agent model override
        let config = self.config.read().await;
        if let Ok(Some(model)) = config.resolve_agent_active_model(
            &self.auth_store,
            agent_type.display_name(),
        ) {
            return Ok(model);
        }
        drop(config);

        // Try AgentDefinition.model_override (may be set by create_agent())
        let agent_def = AgentDefinition::new(agent_type);
        if let Some(model) = agent_def.model_override {
            return Ok(model);
        }

        // Fallback: clone parent model
        let mut model = parent_model.clone();
        // Use the agent's default thinking level for the child
        model.thinking_level = tidev_config::reasoning::ThinkingLevelType::default();
        Ok(model)
    }

    /// Cancel a running session and remove it from the active map.
    pub async fn cancel(&self, session_id: Uuid) {
        let mut active = self.active.lock().await;
        if let Some(session) = active.get(&session_id) {
            session.cancel_token.cancel();
            active.remove(&session_id);
        }
    }

    /// List all active sessions.
    pub async fn list_active(&self) -> Vec<SessionInfo> {
        let active = self.active.lock().await;
        active
            .iter()
            .map(|(id, s)| SessionInfo {
                session_id: *id,
                parent_session_id: s.parent_session_id,
                agent_type: s.agent_type,
                started_at: s.started_at,
            })
            .collect()
    }

    /// Check if a session is active.
    pub async fn is_active(&self, session_id: Uuid) -> bool {
        let active = self.active.lock().await;
        active.contains_key(&session_id)
    }

    /// Return the number of active sessions.
    pub async fn active_count(&self) -> usize {
        let active = self.active.lock().await;
        active.len()
    }
}
