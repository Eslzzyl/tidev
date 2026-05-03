use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{AppConfig, AuthStore},
    llm::LlmClient,
    shared::file_search::FileSearchIndex,
    storage::SessionStore,
};

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
    /// Cancellation token for graceful shutdown
    pub cancel_token: CancellationToken,
    /// File search index for @-mention completion
    pub file_search_index: Arc<FileSearchIndex>,
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
    ) -> anyhow::Result<Self> {
        crate::log_debug!("Creating new AppState");
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            event_bus,
            llm_client,
            config: Arc::new(RwLock::new(config)),
            auth: Arc::new(RwLock::new(auth)),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
            workspace_root,
            cancel_token: CancellationToken::new(),
            file_search_index: Arc::new(FileSearchIndex::new()),
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
        crate::log_debug!("Getting active request for session {}: {:?}", session_id, result);
        result
    }

    /// Remove an active request
    pub async fn remove_request(&self, session_id: uuid::Uuid) {
        crate::log_debug!("Removing active request for session {}", session_id);
        let mut requests = self.active_requests.write().await;
        requests.remove(&session_id);
    }
}
