use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};

use crate::{config::AppConfig, llm::LlmClient, storage::SessionStore};

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
    /// Active request tracking (session_id -> request_id)
    pub active_requests: Arc<RwLock<std::collections::HashMap<uuid::Uuid, u64>>>,
}

impl AppState {
    /// Create a new application state
    pub fn new(
        store: SessionStore,
        event_bus: EventBus,
        llm_client: LlmClient,
        config: AppConfig,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            event_bus,
            llm_client,
            config: Arc::new(RwLock::new(config)),
            active_requests: Arc::new(RwLock::new(std::collections::HashMap::new())),
        })
    }

    /// Track an active request
    pub async fn track_request(&self, session_id: uuid::Uuid, request_id: u64) {
        let mut requests = self.active_requests.write().await;
        requests.insert(session_id, request_id);
    }

    /// Get the active request for a session
    pub async fn get_active_request(&self, session_id: uuid::Uuid) -> Option<u64> {
        let requests = self.active_requests.read().await;
        requests.get(&session_id).copied()
    }

    /// Remove an active request
    pub async fn remove_request(&self, session_id: uuid::Uuid) {
        let mut requests = self.active_requests.write().await;
        requests.remove(&session_id);
    }
}
