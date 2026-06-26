//! SessionManager — manages session lifecycle with Per-Session Event Bus.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! The SessionManager is responsible for spawning, cancelling, and listing
//! active sessions. It holds only the shared resources needed to create new
//! sessions: the session store, LLM client, and control event channel.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::BackendEvent;
use tidev_storage::SessionStore;

use crate::agent_loop::AgentLoop;
use crate::types::{
    AgentLoopConfig, ControlEvent, PendingToolApproval, SessionConfig, SessionHandle, SessionInfo,
};

/// Manages all active sessions, each with its own event bus.
///
/// Architecture (Per-Session Event Bus):
/// - `store`: shared persistence layer
/// - `llm_client`: shared LLM client
/// - `active`: map of session ID -> active session state
/// - `control_tx` / `control_rx`: control event channel for parent-child coordination
///
/// Frontend-specific state (workspace_root, config, tools, hooks, etc.)
/// is NOT stored here — it lives in the frontend (e.g. TUI App struct)
/// and is passed to `run_agent_loop` or `run_agent_loop_with_permission_channel`
/// as needed.
#[derive(Clone)]
pub struct SessionManager {
    pub store: Arc<AsyncMutex<SessionStore>>,
    pub llm_client: tidev_llm::LlmClient,
    pub active: Arc<AsyncMutex<HashMap<Uuid, ActiveSession>>>,
    /// Sender side of the control event channel.
    /// Cloned to each AgentLoop for parent-child coordination.
    pub control_tx: UnboundedSender<ControlEvent>,
    /// Receiver side of the control event channel.
    pub control_rx: Arc<AsyncMutex<UnboundedReceiver<ControlEvent>>>,
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
    ) -> Self {
        let (control_tx, control_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            store,
            llm_client,
            active: Arc::new(AsyncMutex::new(HashMap::new())),
            control_tx,
            control_rx: Arc::new(AsyncMutex::new(control_rx)),
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
    /// and additional runtime resources, then runs it synchronously via
    /// `tokio::runtime::Handle::block_on`.
    #[allow(clippy::too_many_arguments)]
    pub fn run_agent_loop_with_permission_channel(
        &self,
        config: AgentLoopConfig,
        request_id: u64,
        permission_tx: UnboundedSender<PendingToolApproval>,
        tool_registry: tidev_tools::ToolRegistry,
        hooks: tidev_hooks::HookEngine,
        session_manager: SessionManager,
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
        let control_tx = self.control_tx.clone();

        let runtime_handle = tokio::runtime::Handle::current();
        runtime_handle.block_on(async move {
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
                context: tidev_context::ContextManager::new(),
                tools,
                tool_registry,
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
                session_manager,
                can_delegate: true,
                control_tx,
            };

            agent_loop.run(request_id).await
        })
    }

    /// Process pending control events for tracking purposes.
    ///
    /// In the current execution model, subagents are created and run inline
    /// by the parent AgentLoop (which has access to tools, workspace, etc.).
    /// ControlEvents are used for SessionManager tracking/logging.
    /// Returns the number of events processed.
    pub async fn process_control_events(&self) -> usize {
        let mut rx = self.control_rx.lock().await;
        let mut count = 0;
        loop {
            match rx.try_recv() {
                Ok(ControlEvent::SubtaskRequested {
                    parent_session_id,
                    child_session_id,
                    agent_type,
                    description,
                    ..
                }) => {
                    log::info!(
                        "SessionManager: {} subagent '{}' spawned (parent={}, child={})",
                        agent_type.display_name(),
                        description,
                        parent_session_id,
                        child_session_id,
                    );
                    count += 1;
                }
                Ok(ControlEvent::SubtaskCompleted {
                    child_session_id,
                    success,
                }) => {
                    log::info!(
                        "SessionManager: child session {} completed (success={})",
                        child_session_id,
                        success,
                    );
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
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
