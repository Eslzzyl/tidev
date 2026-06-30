//! SessionManager — manages session lifecycle with Per-Session Event Bus.
//!
//! Each session runs its own AgentLoop with an independent event channel.
//! The SessionManager is responsible for spawning, cancelling, and listing
//! active sessions. It also resolves subagent model overrides and creates
//! child sessions with correctly filtered tool lists.
//!
//! In the three-channel architecture:
//! - Receives [`FrontendMessage`]s from the frontend
//! - Sends [`DisplayEvent`]s to the frontend
//! - Is the **sole** component authorized to write to [`SessionStore`]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow, ensure};
use chrono::Utc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::sync::mpsc::{UnboundedSender};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use tidev_session::session::{
    BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult,
};
use tidev_storage::SessionStore;
use tidev_config::LogConfig;

use crate::agent_loop::AgentLoop;
use crate::types::{
    AgentLoopConfig, DisplayEvent, FrontendEvent, FrontendMessage,
    PendingToolApproval, SessionConfig, SessionHandle, SessionInfo,
    SharedAgentState,
};
use crate::{AgentDefinition, compose_static_system_prompt};

/// Manages all active sessions, each with its own event bus.
///
/// Architecture (Per-Session Event Bus):
/// - `store`: shared persistence layer (SOLE writer)
/// - `llm_client`: shared LLM client
/// - `active`: map of session ID -> active session state
/// - `config`: shared app config for resolving agent model overrides
/// - `auth_store`: auth store for model resolution
/// - `tool_registry`: tool registry for model-aware tool filtering
/// - `frontend_tx`: channel to notify frontend of subagent lifecycle events
/// - `display_tx`: channel to send [`DisplayEvent`]s to the frontend
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
    /// Channel to send display events to the frontend (TUI).
    pub display_tx: UnboundedSender<DisplayEvent>,
    /// Shared mutable state for frontend <-> agent loop communication.
    pub shared_state: Arc<SharedAgentState>,
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
    /// Opens the database, creates the session store, and initializes
    /// the LLM client internally.
    pub fn new(
        db_path: &Path,
        config: Arc<tokio::sync::RwLock<tidev_config::AppConfig>>,
        auth_store: Arc<tidev_config::AuthStore>,
        tool_registry: tidev_tools::ToolRegistry,
        frontend_tx: UnboundedSender<FrontendEvent>,
        display_tx: UnboundedSender<DisplayEvent>,
        log_config: &LogConfig,
    ) -> Result<Self> {
        let db = tidev_storage::database::Database::open(db_path)?;
        let store = Arc::new(AsyncMutex::new(db.create_session_store()?));
        let llm_client = tidev_llm::LlmClient::new(
            log_config.save_request_body,
            log_config.max_request_files,
            log_config.save_response_body,
            log_config.max_response_files,
        )?;
        Ok(Self {
            store,
            llm_client,
            active: Arc::new(AsyncMutex::new(HashMap::new())),
            config,
            auth_store,
            tool_registry,
            frontend_tx,
            display_tx,
            shared_state: Arc::new(SharedAgentState::new()),
        })
    }

    /// Get a clone of the session store for synchronous frontend use.
    /// Each clone opens a new read connection but shares the write connection.
    pub fn clone_store(&self) -> SessionStore {
        self.store.blocking_lock().clone()
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
    pub async fn run_agent_loop_with_permission_channel(
        &self,
        config: AgentLoopConfig,
        request_id: u64,
        permission_tx: UnboundedSender<PendingToolApproval>,
        tool_registry: tidev_tools::ToolRegistry,
        hooks: crate::hooks::HookEngine,
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
            context: crate::context::ContextManager::from_state(
                config.context_summary,
                config.context_retained_from,
            ),
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
            shared_state: config.shared_state,
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
            tool_call_id: tool_call.id.clone(),
            tool_call_name: tool_call.name.clone(),
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
            context: crate::context::ContextManager::new(),
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
            hooks: crate::hooks::HookEngine::new(
                Default::default(),
                workspace_root.to_path_buf(),
            ),
            session_manager: self.clone(),
            shared_state: std::sync::Arc::new(crate::types::SharedAgentState::new()),
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

    // ========================================================================
    // DB Write Operations (SessionManager is the SOLE DB writer)
    // ========================================================================

    /// Append a message to a session in the database.
    pub async fn append_message(&self, session_id: Uuid, msg: &Message) -> Result<()> {
        let store = self.store.lock().await;
        store.append_message(session_id, msg)?;
        Ok(())
    }

    /// Create a new session in the database.
    pub async fn create_session(
        &self,
        session_id: Uuid,
        workspace_root: &Path,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
        title: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.create_session(
            session_id,
            workspace_root,
            provider_id,
            provider_display_name,
            model_id,
            model_display_name,
            title,
        )?;
        Ok(())
    }

    /// Delete sessions from the database.
    pub async fn delete_sessions(&self, session_ids: &[Uuid]) -> Result<()> {
        let store = self.store.lock().await;
        store.delete_sessions(session_ids)?;
        Ok(())
    }

    /// Update session context state (compaction result).
    pub async fn update_session_context_state(
        &self,
        session_id: Uuid,
        summary: Option<&str>,
        retained_from: usize,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_session_context_state(session_id, summary, retained_from)?;
        Ok(())
    }

    /// Request context compaction for a session.
    ///
    /// If the agent loop is running, queues a request via `SharedAgentState`.
    /// If idle, runs compaction directly.
    pub async fn compact(&self, session_id: Uuid) {
        if self.is_active(session_id).await {
            self.shared_state.request_compact();
        } else {
            self.compact_session_idle(session_id).await;
        }
    }

    /// Save full tool output to the database.
    pub async fn save_tool_output(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        tool_name: &str,
        output: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.save_tool_output(session_id, message_id, tool_name, output)?;
        Ok(())
    }

    /// Record LLM token usage.
    pub async fn record_usage(
        &self,
        provider_id: &str,
        model_id: &str,
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.record_usage(
            provider_id,
            model_id,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        )?;
        Ok(())
    }

    /// Update file diffs for a message.
    pub async fn update_message_file_diffs(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        file_diffs_json: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_message_file_diffs(session_id, message_id, file_diffs_json)?;
        Ok(())
    }

    /// Remember a tool permission decision.
    pub async fn remember_tool_permission(
        &self,
        session_id: Uuid,
        permission_key: &str,
        allow: bool,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.remember_tool_permission(session_id, permission_key, allow)?;
        Ok(())
    }

    /// Save model thinking level preference.
    pub async fn save_model_thinking_level(
        &self,
        provider_id: &str,
        model_id: &str,
        level: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.save_model_thinking_level(provider_id, model_id, level)?;
        Ok(())
    }

    /// Update the model associated with a session.
    pub async fn update_session_model(
        &self,
        session_id: Uuid,
        provider_id: &str,
        provider_display_name: &str,
        model_id: &str,
        model_display_name: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_session_model(
            session_id,
            provider_id,
            provider_display_name,
            model_id,
            model_display_name,
        )?;
        Ok(())
    }

    /// Update message patch (for undo/redo).
    pub async fn update_message_patch(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        patch_json: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_message_patch(session_id, message_id, patch_json)?;
        Ok(())
    }

    /// Update message snapshot hash.
    pub async fn update_message_snapshot(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        hash: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_message_snapshot(session_id, message_id, hash)?;
        Ok(())
    }

    /// Delete messages from a session.
    pub async fn delete_messages(&self, session_id: Uuid, message_ids: &[Uuid]) -> Result<()> {
        let store = self.store.lock().await;
        store.delete_messages(session_id, message_ids)?;
        Ok(())
    }

    /// Set revert message ID (for undo/redo state).
    pub async fn set_revert_message_id(
        &self,
        session_id: Uuid,
        message_id: Option<Uuid>,
        redo_snapshot: Option<String>,
    ) -> Result<()> {
        let store = self.store.lock().await;
        if let Some(message_id) = message_id {
            store.set_revert_message_id(session_id, Some(message_id), redo_snapshot.as_deref())?;
        } else {
            store.clear_revert_message_id(session_id)?;
        }
        Ok(())
    }

    /// Update session title.
    pub async fn update_session_title(&self, session_id: Uuid, title: &str) -> Result<()> {
        let store = self.store.lock().await;
        store.update_session_title(session_id, title)?;
        Ok(())
    }

    /// Update session system prompt.
    pub async fn update_session_system_prompt(
        &self,
        session_id: Uuid,
        system_prompt: &str,
    ) -> Result<()> {
        let store = self.store.lock().await;
        store.update_session_system_prompt(session_id, system_prompt)?;
        Ok(())
    }

    /// Clear instruction sources for a session.
    pub async fn clear_instruction_sources(&self, session_id: Uuid) -> Result<()> {
        let store = self.store.lock().await;
        store.clear_instruction_sources(session_id)?;
        Ok(())
    }

    // ========================================================================
    // Three-Channel Protocol — FrontendMessage Processing
    // ========================================================================

    /// Process a single [`FrontendMessage`] from the frontend.
    ///
    /// Each variant is handled by updating authoritative state, writing to DB,
    /// and sending [`DisplayEvent`]s back through `display_tx`.
    pub async fn handle_frontend_message(&self, msg: FrontendMessage) -> Result<()> {
        match msg {
            FrontendMessage::SubmitPrompt { .. } => {
                // SubmitPrompt is handled by the TUI directly for now
                // as it requires complex conversation setup flow.
                log::warn!("handle_frontend_message: SubmitPrompt not yet implemented via channel");
            }
            FrontendMessage::Command(cmd) => {
                self.handle_frontend_command(cmd).await?;
            }
            FrontendMessage::ToolApproval { session_id, approved } => {
                // Tool approval is handled via permission channel directly
                log::debug!(
                    "handle_frontend_message: ToolApproval for session {} ({} tools)",
                    session_id,
                    approved.len()
                );
            }
            FrontendMessage::SwitchSession { session_id } => {
                log::info!("handle_frontend_message: SwitchSession to {}", session_id);
                let _ = self.display_tx.send(DisplayEvent::StatusChanged {
                    session_id,
                    status: crate::types::SessionStatus::Idle,
                });
            }
            FrontendMessage::CreateSession {
                workspace_root,
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
                title,
            } => {
                let session_id = Uuid::new_v4();
                self.create_session(
                    session_id,
                    &workspace_root,
                    &provider_id,
                    &provider_display_name,
                    &model_id,
                    &model_display_name,
                    &title,
                )
                .await?;
                let _ = self.display_tx.send(DisplayEvent::SessionCreated {
                    session_id,
                    title,
                });
            }
            FrontendMessage::DeleteSessions { session_ids } => {
                let count = session_ids.len();
                self.delete_sessions(&session_ids).await?;
                let _ = self.display_tx.send(DisplayEvent::SessionsDeleted {
                    count,
                });
            }
            FrontendMessage::UpdateSessionModel {
                session_id,
                provider_id,
                provider_display_name,
                model_id,
                model_display_name,
            } => {
                self.update_session_model(
                    session_id,
                    &provider_id,
                    &provider_display_name,
                    &model_id,
                    &model_display_name,
                )
                .await?;
            }
            FrontendMessage::SaveThinkingLevel {
                provider_id,
                model_id,
                level,
            } => {
                self.save_model_thinking_level(&provider_id, &model_id, &level)
                    .await?;
            }
            FrontendMessage::RememberToolPermission {
                session_id,
                permission_key,
                allow,
            } => {
                self.remember_tool_permission(session_id, &permission_key, allow)
                    .await?;
            }
        }
        Ok(())
    }

    /// Handle a [`FrontendCommand`] — compact, undo, redo, cancel.
    async fn handle_frontend_command(&self, cmd: crate::types::FrontendCommand) -> Result<()> {
        match cmd {
            crate::types::FrontendCommand::Compact { session_id } => {
                log::info!("handle_frontend_command: Compact requested for session {}", session_id);
                if self.is_active(session_id).await {
                    // Agent loop is running -- queue a compact request.
                    self.shared_state.request_compact();
                } else {
                    // Agent loop is idle -- run compaction directly.
                    self.compact_session_idle(session_id).await;
                }
            }
            crate::types::FrontendCommand::Undo { session_id, target_message_id: _ } => {
                log::info!("handle_frontend_command: Undo requested for session {}", session_id);
                // Check if there's an active agent loop — reject if so.
                if self.is_active(session_id).await {
                    log::warn!("handle_frontend_command: Undo rejected — agent loop active for session {}", session_id);
                }
            }
            crate::types::FrontendCommand::Redo { session_id } => {
                log::info!("handle_frontend_command: Redo requested for session {}", session_id);
            }
            crate::types::FrontendCommand::Cancel { session_id } => {
                log::info!("handle_frontend_command: Cancel requested for session {}", session_id);
                self.cancel(session_id).await;
            }
        }
        Ok(())
    }

    /// Run compaction for a session when the agent loop is idle.
    ///
    /// Creates a temporary [`ContextManager`](crate::context::ContextManager) from
    /// stored state, runs the LLM summarisation, persists the result, and
    /// notifies the TUI via [`DisplayEvent::ContextCompacted`].
    async fn compact_session_idle(&self, session_id: Uuid) {
        // 1. Load session record and messages from the database.
        let (session_record, messages) = {
            let store = self.store.lock().await;
            let record = match store.load_session_record(session_id) {
                Ok(Some(r)) => r,
                Ok(None) => {
                    log::warn!("compact_session_idle: session {} not found", session_id);
                    return;
                }
                Err(e) => {
                    log::warn!("compact_session_idle: failed to load session {}: {}", session_id, e);
                    return;
                }
            };
            let msgs = store.load_messages(session_id).unwrap_or_default();
            (record, msgs)
        };

        // 2. Build Conversation and resolve model (with API key from auth store).
        let mut conv = tidev_session::session::Conversation::new(
            session_id,
            "".to_string(),
            &session_record.provider_id,
            &session_record.provider_display_name,
            &session_record.model_id,
            &session_record.model_display_name,
            &session_record.title,
        );
        conv.messages = messages;

        let model = {
            let config_guard = self.config.read().await;
            match config_guard.resolve_model_by_ids(
                &self.auth_store,
                &session_record.provider_id,
                &session_record.model_id,
            ) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("compact_session_idle: model resolve failed: {}", e);
                    return;
                }
            }
        };
        let tools = self.tool_registry.definitions_for_model(&model);

        // 3. Create ContextManager and capture prior state.
        let mut ctx = crate::context::ContextManager::from_state(
            session_record.context_summary.clone(),
            session_record.context_retained_from,
        );
        let prior_summary = ctx.summary.clone();
        let prior_retained_from = ctx.retained_from;

        // 4. Run compaction.
        if let Err(e) = ctx
            .compact(crate::context::CompactionConfig {
                llm: &self.llm_client,
                model: &model,
                conversation: &conv,
                manual: true,
                stream_ctx: None,
                tools: &tools,
                mode: tidev_types::prompts::SessionMode::Build,
            })
            .await
        {
            log::warn!("compact_session_idle: compaction failed for {}: {}", session_id, e);
            return;
        }

        // 5. Persist context state.
        {
            let store = self.store.lock().await;
            if let Err(e) = store.update_session_context_state(
                session_id,
                ctx.summary.as_deref(),
                ctx.retained_from,
            ) {
                log::warn!("compact_session_idle: persist context state failed: {}", e);
            }

            // 6. Create and persist compaction message.
            if let Some(ref summary) = ctx.summary {
                let mut compaction_msg = tidev_session::session::Message::compaction(summary.clone());
                compaction_msg.metadata.prior_summary = prior_summary.clone();
                compaction_msg.metadata.prior_retained_from = Some(prior_retained_from);
                compaction_msg.completed_at = Some(Utc::now());
                if let Err(e) = store.append_message(session_id, &compaction_msg) {
                    log::warn!("compact_session_idle: persist compaction msg failed: {}", e);
                }
            }
        }

        // 7. Notify TUI.
        let _ = self.display_tx.send(DisplayEvent::ContextCompacted {
            session_id,
            compacted: ctx.summary.is_some(),
            manual: true,
            summary: ctx.summary,
            retained_from: ctx.retained_from,
            prior_summary,
            prior_retained_from,
            error: None,
        });

        // Consume any stale compact request that may have been set between the
        // is_active() check and the idle compaction completing.  Without this,
        // the next agent loop would redundantly re-compact the already-summarised
        // context ("summary of a summary"), degrading information quality.
        self.shared_state.take_compact_request();
    }

    // ========================================================================
    // Three-Channel Protocol — BackendEvent Processing (AgentLoop → SessionManager)
    // ========================================================================

    /// Run the agent loop with BackendEvent routing through SessionManager.
    ///
    /// Creates an internal event channel so that all BackendEvents from the
    /// AgentLoop flow through SessionManager. Each event is:
    /// 1. Written to the database (for persistence events)
    /// 2. Converted to a [`DisplayEvent`] and forwarded to the frontend
    ///
    /// Returns when the agent loop completes.
    pub async fn run_agent_loop(
        &self,
        config: AgentLoopConfig,
        request_id: u64,
        permission_tx: UnboundedSender<PendingToolApproval>,
        tool_registry: tidev_tools::ToolRegistry,
        hooks: crate::hooks::HookEngine,
    ) -> Result<()> {
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
        let store = self.store.clone();
        let processor_display_tx = self.display_tx.clone();
        let session_id = config.session_id;

        // Spawn the event processor task
        let processor = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                Self::process_agent_event(
                    &store,
                    &processor_display_tx,
                    request_id,
                    &session_id,
                    event,
                )
                .await;
            }
        });

        // Run the agent loop with our event_tx
        let mut routed_config = config;
        routed_config.event_tx = event_tx;

        let result = self
            .run_agent_loop_with_permission_channel(
                routed_config,
                request_id,
                permission_tx,
                tool_registry,
                hooks,
            )
            .await;

        // Signal that the agent loop has ended
        let _ = self.display_tx.send(DisplayEvent::StatusChanged {
            session_id,
            status: crate::types::SessionStatus::Idle,
        });

        // Wait for the processor to finish processing remaining events
        drop(processor);

        result
    }

    /// Process a single BackendEvent from the AgentLoop.
    ///
    /// This is the core of the three-channel protocol:
    /// - Writes persistent data to the database
    /// - Forwards display-relevant events as [`DisplayEvent`]s
    async fn process_agent_event(
        store: &Arc<AsyncMutex<SessionStore>>,
        display_tx: &UnboundedSender<DisplayEvent>,
        request_id: u64,
        session_id: &Uuid,
        event: BackendEvent,
    ) {
        match &event {
            BackendEvent::Finished { turn, .. } => {
                // Persist the assistant turn
                let msg = tidev_session::session::Message::new(
                    tidev_session::session::MessageRole::Assistant,
                    &turn.content,
                );
                {
                    let store_guard = store.lock().await;
                    let _ = store_guard.append_message(*session_id, &msg);
                }
                // Record usage data
                if let (Some(_), Some(_)) = (turn.input_tokens, turn.output_tokens) {
                    let _ = display_tx.send(DisplayEvent::MessageFinalized {
                        message: msg,
                    });
                    let _ = display_tx.send(DisplayEvent::StatusChanged {
                        session_id: *session_id,
                        status: crate::types::SessionStatus::Idle,
                    });
                }
            }
            BackendEvent::Delta { content, .. } if content.is_empty() => {
                // Skip empty deltas
            }
            BackendEvent::Delta { .. } => {
                let _ = display_tx.send(DisplayEvent::MessageDelta {
                    request_id,
                    content: match &event {
                        BackendEvent::Delta { content, .. } => content.clone(),
                        _ => String::new(),
                    },
                });
            }
            BackendEvent::ReasoningDelta { .. } => {
                let _ = display_tx.send(DisplayEvent::ReasoningDelta {
                    request_id,
                    content: match &event {
                        BackendEvent::ReasoningDelta { content, .. } => content.clone(),
                        _ => String::new(),
                    },
                });
            }
            BackendEvent::ContextCompacted {
                compacted,
                manual,
                summary,
                retained_from,
                prior_summary,
                prior_retained_from,
                ..
            } if *compacted => {
                let _ = display_tx.send(DisplayEvent::ContextCompacted {
                    session_id: *session_id,
                    compacted: *compacted,
                    manual: *manual,
                    summary: summary.clone(),
                    retained_from: *retained_from,
                    prior_summary: prior_summary.clone(),
                    prior_retained_from: *prior_retained_from,
                    error: None,
                });
            }
            BackendEvent::ContextCompacted { .. } => {}
            _ => {
                // Other events (ToolCompleted, StreamEnd, etc.) are handled
                // by the frontend's existing event processing for now.
            }
        }
    }
}
