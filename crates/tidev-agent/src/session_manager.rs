//! SessionManager — manages session lifecycle with Per-Session Event Bus.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::BackendEvent;
use tidev_storage::SessionStore;

use crate::agent_loop::AgentLoop;
use crate::types::{
    AgentType, AgentLoopConfig, PendingToolApproval, QueuedUserMessage,
    SessionConfig, SessionHandle, SessionInfo,
};

/// Manages all active sessions, each with its own event bus.
#[derive(Clone)]
pub struct SessionManager {
    // Core runtime fields (used by SessionManager internally)
    pub store: Arc<tokio::sync::Mutex<SessionStore>>,
    pub llm_client: tidev_llm::LlmClient,
    pub active: Arc<AsyncMutex<HashMap<Uuid, ActiveSession>>>,

    // TUI-facing fields (held for frontend access, not used internally by SessionManager)
    pub workspace_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_paths: tidev_config::ConfigPaths,
    pub config: tidev_config::AppConfig,
    pub auth: tidev_config::AuthStore,
    pub tools: tidev_tools::ToolRegistry,
    pub instructions: Vec<String>,
    pub instruction_content_cache: HashMap<String, String>,
    pub queued_messages: Arc<Mutex<VecDeque<QueuedUserMessage>>>,
    pub auto_approve_permissions: bool,
    pub hooks: tidev_hooks::HookEngine,
}

pub struct ActiveSession {
    agent_type: AgentType,
    parent_session_id: Option<Uuid>,
    cancel_token: CancellationToken,
    event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    started_at: chrono::DateTime<chrono::Utc>,
}

impl SessionManager {
    /// Queue a user message for the next agent loop turn.
    pub fn queue_user_message(&self, msg: QueuedUserMessage) {
        if let Ok(mut queue) = self.queued_messages.lock() {
            queue.push_back(msg);
        }
    }

    /// Compose the static system prompt — called exactly once per session lifetime.
    ///
    /// Content: base prompt + environment info.
    /// Result is persisted to the session DB record and never changes.
    pub fn compose_static_system_prompt(&self, base_prompt: &str) -> String {
        let base_prompt = base_prompt.trim();
        let system_info = tidev_session::system_info::SystemInfo::detect();
        let working_dir = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let is_git = tidev_session::system_info::is_git_repo(&self.workspace_root);

        let mut prompt = String::new();
        if !base_prompt.is_empty() {
            prompt.push_str(base_prompt);
        }
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
        prompt
    }

    /// Run the agent loop with a permission approval channel.
    ///
    /// Creates an AgentLoop from the config and runs it inline.
    /// Tool execution goes through ToolRegistry for real tool calls.
    /// Permission approvals flow through the provided channel.
    pub fn run_agent_loop_with_permission_channel(
        &mut self,
        config: AgentLoopConfig,
        request_id: u64,
        permission_tx: tokio::sync::mpsc::UnboundedSender<PendingToolApproval>,
    ) -> anyhow::Result<()> {
        // This runs synchronously (blocking) to match the TUI's call pattern.
        // The actual AgentLoop implementation is in tokio::runtime::Handle.
        log::info!(
            "run_agent_loop_with_permission_channel: starting for session {}",
            config.session_id
        );

        // Build tools list from registry
        let tools = self.tools.definitions_for_model(&config.model);
        let tool_registry = self.tools.clone();
        let store = self.store.clone();
        let llm = self.llm_client.clone();
        let event_tx = config.event_tx.clone();

        // Build main conversation from store
        let runtime_handle = tokio::runtime::Handle::current();

        // We need to run the AgentLoop as a blocking task since this method is synchronous
        runtime_handle.block_on(async move {
            // Load conversation from store
            let conversation = {
                let store = store.lock().await;
                let msgs = store.load_messages(config.session_id).unwrap_or_default();
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

            let loop_ = AgentLoop {
                session_id: config.session_id,
                model: config.model,
                conversation,
                context: tidev_context::ContextManager::new(),
                tools,
                tool_registry,
                store,
                llm,
                event_tx,
                cancel_token: config.cancel_token.unwrap_or_else(CancellationToken::new),
                mode: config.mode,
                agent_type: AgentType::General,
                permission_tx: Some(permission_tx),
            };

            loop_.run().await
        })
    }

    pub async fn spawn(&self, config: SessionConfig) -> SessionHandle {
        let session_id = Uuid::new_v4();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let parent_token = CancellationToken::new();
        let child_token = parent_token.child_token();

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
                "New session",
            );
        }

        // Build conversation & agent loop
        let conversation = tidev_session::session::Conversation::new(
            session_id,
            workspace_root.display().to_string(),
            &config.model.provider_id,
            &config.model.provider_display_name,
            &config.model.model_id,
            &config.model.display_name,
            "New session",
        );

        let tools_list = self.tools.definitions_for_model(&config.model);
        let loop_ = AgentLoop {
            session_id,
            model: config.model,
            conversation,
            context: tidev_context::ContextManager::new(),
            tools: tools_list,
            tool_registry: self.tools.clone(),
            store: self.store.clone(),
            llm: self.llm_client.clone(),
            event_tx: event_tx.clone(),
            cancel_token: child_token,
            mode: tidev_types::prompts::SessionMode::Build,
            agent_type: AgentType::General,
            permission_tx: None,
        };

        // Register as active
        {
            let mut active = self.active.lock().await;
            active.insert(
                session_id,
                ActiveSession {
                    agent_type: AgentType::General,
                    parent_session_id: config.parent_session_id,
                    cancel_token: parent_token.clone(),
                    event_tx: event_tx.clone(),
                    started_at: Utc::now(),
                },
            );
        }

        // Spawn the agent loop
        let active_clone = self.active.clone();
        tokio::spawn(async move {
            let result = loop_.run().await;
            if let Err(e) = result {
                log::error!("session[{}] failed: {:#}", session_id, e);
            }
            let mut active = active_clone.lock().await;
            active.remove(&session_id);
        });

        SessionHandle {
            session_id,
            event_rx,
            cancel_token: parent_token.clone(),
        }
    }

    pub async fn cancel(&self, session_id: Uuid) {
        let mut active = self.active.lock().await;
        if let Some(session) = active.get(&session_id) {
            session.cancel_token.cancel();
            active.remove(&session_id);
        }
    }

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

    pub async fn is_active(&self, session_id: Uuid) -> bool {
        let active = self.active.lock().await;
        active.contains_key(&session_id)
    }

    pub async fn active_count(&self) -> usize {
        let active = self.active.lock().await;
        active.len()
    }
}
