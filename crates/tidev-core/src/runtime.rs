//! Runtime — top-level orchestration for tidev.
//!
//! [`Runtime`] is the single entry point that owns all the resources the
//! agent needs. It's created by [`RuntimeBuilder`] which wires up all the
//! components (config, storage, LLM, tools, snapshot, etc.).
//!
//! ```ignore
//! let rt = Runtime::builder()
//!     .workspace_root("/path/to/project")
//!     .build()
//!     .await?;
//!
//! // Submit a user prompt
//! rt.submit_prompt(session_id, "Refactor this function".into()).await;
//!
//! // Receive events
//! while let Some(event) = rt.event_rx().recv().await { ... }
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_config::{paths::ConfigPaths, AppConfig, AuthStore};
use tidev_storage::SessionStore;
use tidev_types::message::BackendEvent;
use tidev_types::message::Message;
use tidev_types::prompts::SessionMode;
use tidev_types::tools::TodoItem;

use tidev_agent::{AgentDefinition, PendingToolApproval};

use crate::context::ContextManager;
use crate::message_buf::MessageBuffer;
use crate::registry::ToolRegistry;
use crate::session::SessionManager;

// ---------------------------------------------------------------------------
// TodoPersistence impl — bridges tidev-tools to tidev-storage.
// ---------------------------------------------------------------------------

/// Implements [`tidev_tools::TodoPersistence`] by delegating to
/// [`SessionStore`] (which is now `Sync`).
struct TodoStore {
    store: SessionStore,
}

impl tidev_tools::TodoPersistence for TodoStore {
    fn load_todos(&self, session_id: Uuid) -> anyhow::Result<Vec<TodoItem>> {
        self.store.load_todos(session_id).map_err(Into::into)
    }

    fn replace_todos(
        &self,
        session_id: Uuid,
        todos: &[TodoItem],
    ) -> anyhow::Result<()> {
        self.store.save_todos(session_id, todos).map_err(Into::into)
    }
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// The top-level runtime context.
///
/// Owns all resources and exposes them for agent loops. Cloning is cheap
/// (Arc-based) so the runtime can be shared across tasks.
#[derive(Clone)]
pub struct Runtime {
    /// Application configuration.
    pub config: AppConfig,
    /// Authentication store.
    pub auth: AuthStore,
    /// Session manager (SQLite).
    pub session_manager: SessionManager,
    /// LLM client.
    pub llm: tidev_llm::LlmClient,
    /// Tool registry.
    pub tool_registry: Arc<ToolRegistry>,
    /// Skills catalog.
    pub skills: tidev_tools::SkillCatalog,
    /// Resolved model (for loop construction).
    active_model: Arc<tidev_config::auth::ActiveModel>,
    /// Co-operative cancellation token for the currently active agent loop.
    /// Replaced on each `submit_prompt` so cancellation is one-shot per loop.
    active_loop_cancel: Arc<Mutex<Option<CancellationToken>>>,
    /// Snapshot service for undo/redo (optional).
    snapshot: Option<tidev_snapshot::SnapshotService>,

    /// Per-session message buffers, keyed by session ID.
    buffers: Arc<Mutex<HashMap<Uuid, Arc<RwLock<MessageBuffer>>>>>,
    /// Per-session context managers, keyed by session ID.
    context_managers: Arc<Mutex<HashMap<Uuid, Arc<Mutex<ContextManager>>>>>,

    /// Event channel (sender → UI, receiver → TUI).
    event_tx: UnboundedSender<BackendEvent>,
    _event_rx: Arc<Mutex<Option<UnboundedReceiver<BackendEvent>>>>,
    /// Permission approval channel.
    perm_tx: UnboundedSender<PendingToolApproval>,
    _perm_rx: Arc<Mutex<Option<UnboundedReceiver<PendingToolApproval>>>>,

    /// Currently running agent loop handle.
    run_loop_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,

    /// Workspace root.
    workspace_root: PathBuf,
    /// Config directory.
    config_dir: PathBuf,
}

impl Runtime {
    /// Create a new builder.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Get the event receiver (consumed by the TUI).
    pub async fn event_rx(&self) -> Option<UnboundedReceiver<BackendEvent>> {
        self._event_rx.lock().await.take()
    }

    /// Get the permission receiver (consumed by the TUI).
    pub async fn perm_rx(&self) -> Option<UnboundedReceiver<PendingToolApproval>> {
        self._perm_rx.lock().await.take()
    }

    /// Get (or create) the message buffer for a session.
    pub async fn message_buffer(&self, session_id: Uuid) -> Arc<RwLock<MessageBuffer>> {
        let mut bufs = self.buffers.lock().await;
        if let Some(buf) = bufs.get(&session_id) {
            return buf.clone();
        }
        // Load from DB and create.
        let messages = self
            .session_manager
            .load_messages(session_id)
            .unwrap_or_default();
        let buf = Arc::new(RwLock::new(MessageBuffer::new(messages)));
        bufs.insert(session_id, buf.clone());
        buf
    }

    /// Get (or create) the context manager for a session.
    pub async fn context_manager(&self, session_id: Uuid) -> Arc<Mutex<ContextManager>> {
        let mut mgrs = self.context_managers.lock().await;
        if let Some(mgr) = mgrs.get(&session_id) {
            return mgr.clone();
        }
        // Restore from DB if available.
        let session = self.session_manager.load_session(session_id).ok().flatten();
        let cm = match session {
            Some(s) => ContextManager::from_state(s.context_summary, s.context_retained_from),
            None => ContextManager::new(),
        };
        let cm = Arc::new(Mutex::new(cm));
        mgrs.insert(session_id, cm.clone());
        cm
    }

    /// Get the tool registry.
    pub fn tool_registry(&self) -> &ToolRegistry {
        &self.tool_registry
    }

    /// Get the LLM client.
    pub fn llm(&self) -> &tidev_llm::LlmClient {
        &self.llm
    }

    /// Get the session manager.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Get the workspace root.
    pub fn workspace_root(&self) -> &PathBuf {
        &self.workspace_root
    }

    /// Get the config directory.
    pub fn config_dir(&self) -> &PathBuf {
        &self.config_dir
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    /// Submit a user prompt for a session.
    pub async fn submit_prompt(&self, session_id: Uuid, content: String) -> Result<()> {
        use tidev_types::message::{Message, MessageRole};

        let user_msg = Message::new(MessageRole::User, content);

        // 1. Persist the user message.
        {
            let buf = self.message_buffer(session_id).await;
            buf.write().await.append(user_msg.clone());
        }
        self.session_manager.append_message(session_id, &user_msg)?;

        // 2. Check if a loop is already running.
        {
            let handle = self.run_loop_handle.lock().await;
            if handle.is_some() {
                // Loop running — new message will be picked up on next turn.
                return Ok(());
            }
        }

        // 3. Build CoreContext + AgentLoopConfig and spawn the loop.
        // Create a fresh cancellation token for this loop — retired on cancel().
        let cancel = CancellationToken::new();
        *self.active_loop_cancel.lock().await = Some(cancel.clone());
        let buffer = self.message_buffer(session_id).await;
        let context_manager = self.context_manager(session_id).await;

        // Compose or load the system prompt.
        let system_prompt = {
            let session = self.session_manager.load_session(session_id)?;
            match session {
                Some(s) if !s.system_prompt.is_empty() => s.system_prompt,
                _ => {
                    // New session — compose and persist.
                    let sp = crate::agent_ctx::compose_system_prompt(
                        tidev_types::agent_type::AgentType::General,
                        &self.config.instructions,
                        &self.workspace_root,
                        &self.config_dir,
                        SessionMode::Build,
                    );
                    // Persist system prompt to session.
                    self.session_manager
                        .update_session(session_id, None, None)?;
                    sp
                }
            }
        };

        let llm_config = crate::agent_ctx::to_llm_provider_config(&self.active_model);

        let agent_def = AgentDefinition {
            agent_type: tidev_types::agent_type::AgentType::General,
            display_name: "tidev".into(),
            description: "A terminal-based AI coding agent".into(),
            system_prompt: system_prompt.clone(),
            allowed_tools: None,
            temperature: None,
            read_only: false,
        };

        let ctx = crate::agent_ctx::CoreContext::new(
            self.llm.clone(),
            self.session_manager.clone(),
            self.tool_registry.clone(),
            context_manager,
            buffer,
            self.event_tx.clone(),
            self.perm_tx.clone(),
            session_id,
            SessionMode::Build,
            self.active_model.thinking_level.clone(),
            system_prompt,
            llm_config,
            cancel.clone(),
            self.tool_registry.definitions(),
            self.workspace_root.clone(),
            self.active_model.as_ref().clone(),
            self.snapshot.clone(),
            self.config.clone(),
            self.auth.clone(),
        );

        let loop_config = tidev_agent::AgentLoopConfig {
            session_id,
            definition: agent_def,
            mode: SessionMode::Build,
            thinking_level: self.active_model.thinking_level.clone(),
            event_tx: self.event_tx.clone(),
            cancel,
        };

        let join = tokio::spawn(async move {
            if let Err(e) = tidev_agent::run_agent_loop(&ctx, loop_config).await {
                log::error!("agent loop for session {session_id} exited with error: {e}");
            }
        });

        {
            let mut handle = self.run_loop_handle.lock().await;
            *handle = Some(join);
        }

        Ok(())
    }

    /// Cancel the current operation.
    pub async fn cancel(&self) {
        // Cancel only the active loop's token — subsequent submit_prompt calls
        // will create a fresh token and work normally.
        if let Some(token) = self.active_loop_cancel.lock().await.take() {
            token.cancel();
        }

        // Give cooperative exit a brief window, then force-abort.
        let handle = self.run_loop_handle.lock().await.take();
        if let Some(h) = handle {
            tokio::select! {
                _ = h => {},
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    // Already aborted via Drop.
                }
            }
        }

        // Force-kill any lingering child processes.
        tidev_tools::kill_all_children();
    }

    /// Undo — revert to the previous user message's state.
    pub async fn undo(&self, session_id: Uuid) -> Result<()> {
        let buf = self.message_buffer(session_id).await;
        let messages = buf.read().await.load().to_vec();
        drop(buf);

        // Determine target: previous user message (or one more back if in revert state).
        let target_id = match self.session_manager.load_revert_state(session_id)? {
            Some((current, _)) if current != Uuid::nil() => {
                crate::undo::prev_user_message_before(&messages, current)
            }
            _ => crate::undo::last_visible_user_message(&messages),
        };
        let Some(target_id) = target_id else { return Ok(()) };

        self.revert_to_message(session_id, &messages, target_id).await?;
        log::info!("undo completed for session {session_id}, target {target_id}");
        Ok(())
    }

    /// Redo — move forward past the last undo, or restore pre-undo state.
    pub async fn redo(&self, session_id: Uuid) -> Result<()> {
        let Some((current_id, redo_snapshot)) =
            self.session_manager.load_revert_state(session_id)?
        else {
            return Ok(());
        };

        let buf = self.message_buffer(session_id).await;
        let messages = buf.read().await.load().to_vec();
        drop(buf);

        // Is there a next user message to move forward to?
        if let Some(next_id) = crate::undo::next_user_message_after(&messages, current_id) {
            // Move the undo point FORWARD to the next message.
            self.revert_to_message(session_id, &messages, next_id).await?;
        } else {
            // At the end of history — restore the original pre-undo state.
            self.unrevert(session_id, redo_snapshot.as_deref().unwrap_or_default()).await?;
        }

        log::info!("redo completed for session {session_id}");
        Ok(())
    }

    /// Core revert logic: roll workspace and context state to `target_id`.
    async fn revert_to_message(
        &self,
        session_id: Uuid,
        messages: &[Message],
        target_id: Uuid,
    ) -> Result<()> {
        // 1. Reuse existing redo_snapshot if one exists (maintains undo chain),
        //    otherwise capture current workspace as the redo point.
        let redo_hash: Option<Vec<u8>> =
            match self.session_manager.load_revert_state(session_id)? {
                Some((_, Some(existing))) => {
                    // Restore workspace to the pre-undo state first.
                    let s = String::from_utf8_lossy(&existing).to_string();
                    if let Some(ref snap) = self.snapshot {
                        snap.restore(&s).await?;
                    }
                    Some(existing)
                }
                _ => self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.track().ok())
                    .flatten()
                    .map(|h| h.into_bytes()),
            };

        // 2. Collect patches after target, then revert to roll files back.
        let patches = crate::undo::collect_patches_after_message(messages, target_id);
        if !patches.is_empty() {
            if let Some(ref snap) = self.snapshot {
                snap.revert(&patches).await?;
            }
        }

        // 3. Adjust context compaction state.
        let cm = self.context_manager(session_id).await;
        let mut cm_lock = cm.lock().await;
        let mut summary = cm_lock.summary.clone();
        let mut retained_from = cm_lock.retained_from;
        if !crate::undo::restore_context_from_compaction(
            messages,
            target_id,
            &mut summary,
            &mut retained_from,
        ) {
            summary = None;
            retained_from = 0;
        }
        cm_lock.summary = summary;
        cm_lock.retained_from = retained_from;
        drop(cm_lock);

        // 4. Persist revert state.
        if let Some(ref hash) = redo_hash {
            let s = std::str::from_utf8(hash)?;
            self.session_manager
                .save_revert_state(session_id, target_id, Some(s))?;
        } else {
            self.session_manager
                .save_revert_state(session_id, target_id, None)?;
        }

        // 5. Notify TUI.
        let _ = self.event_tx.send(BackendEvent::ContextCompacted {
            session_id,
            compacted: true,
            manual: false,
            summary: None,
            retained_from: 0,
            error: None,
        });

        Ok(())
    }

    /// Full unrevert: restore the pre-undo workspace snapshot and clear state.
    async fn unrevert(
        &self,
        session_id: Uuid,
        redo_snapshot: &[u8],
    ) -> Result<()> {
        let hash_str = String::from_utf8_lossy(redo_snapshot);
        if let Some(ref snap) = self.snapshot {
            snap.restore(&hash_str).await?;
        }

        // Clear revert state.
        self.session_manager
            .save_revert_state(session_id, Uuid::nil(), None)?;

        // Reset context.
        let cm = self.context_manager(session_id).await;
        let mut cm_lock = cm.lock().await;
        cm_lock.summary = None;
        cm_lock.retained_from = 0;
        drop(cm_lock);

        let _ = self.event_tx.send(BackendEvent::ContextCompacted {
            session_id,
            compacted: true,
            manual: false,
            summary: None,
            retained_from: 0,
            error: None,
        });

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// RuntimeBuilder
// ---------------------------------------------------------------------------

/// Builder for [`Runtime`].
pub struct RuntimeBuilder {
    workspace_root: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    data_dir: Option<PathBuf>,
}

impl RuntimeBuilder {
    fn new() -> Self {
        Self {
            workspace_root: None,
            config_dir: None,
            data_dir: None,
        }
    }

    /// Set the workspace root (where the project lives).
    pub fn workspace_root(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(path.into());
        self
    }

    /// Override the config directory (defaults to ~/.config/tidev).
    pub fn config_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_dir = Some(path.into());
        self
    }

    /// Override the data directory (defaults to ~/.local/share/tidev).
    pub fn data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(path.into());
        self
    }

    /// Build the runtime.
    pub async fn build(self) -> Result<Runtime> {
        // 1. Config paths.
        let mut paths = ConfigPaths::discover()?;
        if let Some(ref d) = self.config_dir {
            paths.config_dir = d.clone();
            paths.config_file = d.join("config.toml");
        }
        if let Some(ref d) = self.data_dir {
            paths.data_dir = d.clone();
            paths.auth_file = d.join("auth.json");
            paths.database_file = d.join("sessions.sqlite3");
        }
        paths.ensure_directories()?;

        // 2. Config + auth.
        let config = AppConfig::load(&paths)?;
        let auth = AuthStore::load_or_create(&paths)?;

        // 3. Database + session store.
        let database = tidev_storage::database::Database::open(&paths.database_file)
            .context("failed to open database")?;
        let store = database.create_store()?;

        // 4. LLM client.
        let active_model = config.resolve_active_model(&auth)?;
        let llm = tidev_llm::LlmClient::new(
            config.logging.save_request_body,
            config.logging.max_request_files,
            config.logging.save_response_body,
            config.logging.max_response_files,
        )?;

        // 5. Skills catalog.
        let skills = tidev_tools::SkillCatalog::discover(
            &self.workspace_root.clone().unwrap_or_default(),
            &paths.config_dir,
            &config.skills,
            None,
        );

        // 6. Todo persistence bridge.
        let todo = Arc::new(TodoStore {
            store: store.clone(),
        });

        // 7. Tool registry.
        let max_output_bytes = active_model.max_output_tokens * 2; // heuristic: 2x output tokens ≈ bytes
        let tool_registry = Arc::new(ToolRegistry::new(
            self.workspace_root.clone().unwrap_or_default(),
            paths.config_dir.clone(),
            skills.clone(),
            todo,
            config.websearch.clone(),
            auth.clone(),
            max_output_bytes,
        ));

        // 7. Snapshot service.
        let snapshot_config = Arc::new(config.snapshot.clone());
        let snapshot = if snapshot_config.enabled {
            Some(tidev_snapshot::SnapshotService::new(
                &self.workspace_root.clone().unwrap_or_default(),
                &paths,
                snapshot_config,
            )?)
        } else {
            None
        };

        // Store active model for loop construction.
        let active_model = Arc::new(active_model);

        // 8. Session manager.
        let session_manager = SessionManager::new(store.clone());

        // 9. Channels.
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (perm_tx, perm_rx) = tokio::sync::mpsc::unbounded_channel();

        Ok(Runtime {
            config,
            auth,
            session_manager,
            llm,
            tool_registry,
            skills,
            active_model,
            active_loop_cancel: Arc::new(Mutex::new(None)),
            snapshot,
            buffers: Arc::new(Mutex::new(HashMap::new())),
            context_managers: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            _event_rx: Arc::new(Mutex::new(Some(event_rx))),
            perm_tx,
            _perm_rx: Arc::new(Mutex::new(Some(perm_rx))),
            run_loop_handle: Arc::new(Mutex::new(None)),
            workspace_root: self.workspace_root.unwrap_or_default(),
            config_dir: paths.config_dir,
        })
    }
}
