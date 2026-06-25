//! SessionManager — manages session lifecycle with Per-Session Event Bus.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use tokio::sync::Mutex as AsyncMutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_session::session::BackendEvent;
use tidev_storage::SessionStore;

use crate::agent_loop::AgentLoop;
use crate::types::{AgentType, SessionConfig, SessionHandle, SessionInfo};

/// Manages all active sessions, each with its own event bus.
#[derive(Clone)]
pub struct SessionManager {
    store: Arc<Mutex<SessionStore>>,
    llm: tidev_llm::LlmClient,
    active: Arc<AsyncMutex<HashMap<Uuid, ActiveSession>>>,
}

struct ActiveSession {
    agent_type: AgentType,
    parent_session_id: Option<Uuid>,
    cancel_token: CancellationToken,
    event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    started_at: chrono::DateTime<chrono::Utc>,
}

impl SessionManager {
    pub fn new(store: Arc<Mutex<SessionStore>>, llm: tidev_llm::LlmClient) -> Self {
        Self {
            store,
            llm,
            active: Arc::new(AsyncMutex::new(HashMap::new())),
        }
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
            let store = self.store.lock().unwrap();
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

        let loop_ = AgentLoop {
            session_id,
            model: config.model,
            conversation,
            context: tidev_context::ContextManager::new(),
            tools: config.tools,
            store: self.store.clone(),
            llm: self.llm.clone(),
            event_tx: event_tx.clone(),
            cancel_token: child_token,
            mode: tidev_types::prompts::SessionMode::Build,
            agent_type: AgentType::General,
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
