use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock, broadcast};
use tokio_util::sync::CancellationToken;

use crate::{
    agent::runtime::AgentRuntime,
    config::{AppConfig, AuthStore},
    llm::LlmClient,
    shared::file_search::FileSearchIndex,
    snapshot::SnapshotService,
    storage::SessionStore,
};

use crate::web::terminal::{TerminalManager, TerminalOutput};

// Re-export ConfigPaths for use in routes
pub use crate::config::ConfigPaths;

use super::event_bus::EventBus;

/// Shared application state for web handlers
#[derive(Clone)]
pub struct AppState {
    /// Database store for sessions
    pub store: Arc<Mutex<SessionStore>>,
    /// Event bus for SSE
    pub event_bus: EventBus,
    /// LLM client
    pub llm_client: LlmClient,
    /// Application configuration
    pub config: Arc<RwLock<AppConfig>>,
    /// Auth store (API keys)
    pub auth: Arc<RwLock<AuthStore>>,
    /// Active request tracking (session_id -> request_id)
    pub active_requests: Arc<RwLock<std::collections::HashMap<uuid::Uuid, u64>>>,
    /// Current workspace root path
    pub workspace_root: PathBuf,
    /// Config directory path (for SkillCatalog discovery)
    pub config_dir: PathBuf,
    /// Config paths for saving config/auth files
    pub config_paths: crate::config::ConfigPaths,
    /// Cancellation token for graceful shutdown
    pub cancel_token: CancellationToken,
    /// File search index for @-mention completion
    pub file_search_index: Arc<FileSearchIndex>,
    /// Snapshot service for undo/revert operations
    pub snapshot: Arc<SnapshotService>,
    /// Shared agent runtime (tools, system prompt, agent loop)
    pub agent: AgentRuntime,
    /// Terminal session manager
    pub terminal_manager: Arc<TerminalManager>,
    /// Broadcast channel for terminal output
    pub terminal_tx: broadcast::Sender<TerminalOutput>,
}

impl AppState {
    /// Create a new application state
    pub fn new(
        store: SessionStore,
        event_bus: EventBus,
        llm_client: LlmClient,
        config: AppConfig,
        auth: AuthStore,
        workspace_root: PathBuf,
        paths: &ConfigPaths,
        agent: AgentRuntime,
    ) -> anyhow::Result<Self> {
        crate::log_debug!("Creating new AppState");

        // Create snapshot service for undo operations
        let snapshot = SnapshotService::new(&workspace_root, paths)?;

        // Create terminal manager
        let terminal_manager = Arc::new(TerminalManager::new());
        let (terminal_tx, _) = broadcast::channel::<TerminalOutput>(1024);

        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            event_bus,
            llm_client,
            config: Arc::new(RwLock::new(config)),
            auth: Arc::new(RwLock::new(auth)),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
            workspace_root,
            config_dir: paths.config_dir.clone(),
            config_paths: paths.clone(),
            cancel_token: CancellationToken::new(),
            file_search_index: Arc::new(FileSearchIndex::new()),
            snapshot: Arc::new(snapshot),
            agent,
            terminal_manager,
            terminal_tx,
        })
    }

    /// Track an active request
    pub async fn track_request(&self, session_id: uuid::Uuid, request_id: u64) {
        crate::log_debug!("Tracking request {} for session {}", request_id, session_id);
        let mut requests = self.active_requests.write().await;
        requests.insert(session_id, request_id);
    }

    /// Get the active request for a session
    pub async fn get_active_request(&self, session_id: uuid::Uuid) -> Option<u64> {
        let requests = self.active_requests.read().await;
        let result = requests.get(&session_id).copied();
        crate::log_debug!(
            "Getting active request for session {}: {:?}",
            session_id,
            result
        );
        result
    }

    /// Remove an active request
    pub async fn remove_request(&self, session_id: uuid::Uuid) {
        crate::log_debug!("Removing active request for session {}", session_id);
        let mut requests = self.active_requests.write().await;
        requests.remove(&session_id);
    }
}
