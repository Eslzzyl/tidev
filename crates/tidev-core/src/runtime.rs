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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_config::{paths::ConfigPaths, AppConfig, AuthStore};
use tidev_config::auth::ActiveModel;
use tidev_search::FileSearchIndex;
use tidev_storage::SessionStore;
use tidev_types::message::{BackendEvent, Message, MessageAttachment, MessageRole};
use tidev_types::prompts::SessionMode;
use tidev_types::tools::TodoItem;

use tidev_agent::{AgentDefinition, TuiRequest};

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
    /// Application configuration (behind RwLock for hot-reload).
    config: Arc<StdRwLock<AppConfig>>,
    /// Authentication store (API keys, web search credentials).
    auth: Arc<StdRwLock<AuthStore>>,
    /// Config paths (directories for config, data, auth file, database).
    paths: ConfigPaths,
    /// Session manager (SQLite).
    pub session_manager: SessionManager,
    /// LLM client.
    pub llm: tidev_llm::LlmClient,
    /// Tool registry.
    pub tool_registry: Arc<ToolRegistry>,
    /// Skills catalog.
    pub skills: tidev_tools::SkillCatalog,
    /// Resolved model (for loop construction). Behind RwLock so the TUI can
    /// update it when the user switches providers.
    active_model: Arc<StdRwLock<tidev_config::auth::ActiveModel>>,
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
    /// Request channel (sender → UI).
    request_tx: UnboundedSender<TuiRequest>,
    _request_rx: Arc<Mutex<Option<UnboundedReceiver<TuiRequest>>>>,

    /// Currently running agent loop handle.
    run_loop_handle: Arc<StdMutex<Option<tokio::task::JoinHandle<()>>>>,
    /// Synchronous busy flag (avoids awaiting the Mutex in async-free contexts).
    loop_busy: Arc<AtomicBool>,

    /// Cancellation token for background cleanup tasks.
    cleanup_cancel: CancellationToken,

    /// Workspace root.
    workspace_root: PathBuf,

    /// File search index for @mention autocomplete.
    /// Lazily initialised on first access.
    file_search_index: OnceLock<Arc<FileSearchIndex>>,
}

/// RAII guard that clears `loop_busy` and `run_loop_handle` on drop
/// (including on task panic).
struct AgentLoopGuard {
    busy: Arc<AtomicBool>,
    handle: Arc<StdMutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl Drop for AgentLoopGuard {
    fn drop(&mut self) {
        self.busy.store(false, Ordering::SeqCst);
        *self.handle.lock().unwrap() = None;
    }
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

    /// Get the request receiver (consumed by the TUI).
    pub async fn request_rx(&self) -> Option<UnboundedReceiver<TuiRequest>> {
        self._request_rx.lock().await.take()
    }

    /// Deprecated — use [`request_rx`] instead.
    #[doc(hidden)]
    pub async fn perm_rx(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedReceiver<crate::PendingToolApproval>> {
        None
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
        &self.paths.config_dir
    }

    /// Get ConfigPaths (for save operations).
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// Get a snapshot of the current application configuration.
    pub fn config(&self) -> AppConfig {
        self.config.read().unwrap().clone()
    }

    /// Atomically update the application configuration.
    /// Optionally saves to disk if `save` is true.
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) {
        f(&mut self.config.write().unwrap());
    }

    /// Save the current config to disk.
    pub fn save_config(&self) -> Result<()> {
        let cfg = self.config();
        cfg.save(&self.paths)?;
        Ok(())
    }

    /// Get a snapshot of the current auth store.
    pub fn auth(&self) -> AuthStore {
        self.auth.read().unwrap().clone()
    }

    /// Atomically update the auth store.
    pub fn update_auth(&self, f: impl FnOnce(&mut AuthStore)) {
        f(&mut self.auth.write().unwrap());
    }

    /// Save the auth store to disk.
    pub fn save_auth(&self) -> Result<()> {
        let auth = self.auth.read().unwrap().clone();
        auth.save(&self.paths)?;
        Ok(())
    }

    /// Get the active model ID string (for display).
    pub fn active_model_id(&self) -> String {
        self.active_model.read().unwrap().model_id.clone()
    }

    /// Get the active provider ID string.
    pub fn active_provider_id(&self) -> String {
        self.active_model.read().unwrap().provider_id.clone()
    }

    /// Get a clone of the currently resolved active model.
    pub fn active_model(&self) -> tidev_config::auth::ActiveModel {
        self.active_model.read().unwrap().clone()
    }

    /// Update the active model (called by TUI on provider/model switch).
    pub fn set_active_model(&self, model: tidev_config::auth::ActiveModel) {
        *self.active_model.write().unwrap() = model;
    }

    /// Get the file search index for @mention autocomplete.
    ///
    /// The index is lazily created on first access and bound to the
    /// workspace root.  Background indexing and file-system watching
    /// are managed by the index itself.
    pub fn file_search_index(&self) -> Arc<FileSearchIndex> {
        self.file_search_index
            .get_or_init(|| {
                let index = Arc::new(FileSearchIndex::new());
                index.ensure_background_indexing(&self.workspace_root);
                index
            })
            .clone()
    }

    /// Save a thinking level preference for an agent type to config.
    pub fn set_model_thinking_level(
        &self,
        _provider_id: &str,
        _model_id: &str,
        thinking_level: &str,
    ) -> Result<()> {
        // Update active model's thinking level if the model matches.
        let mut active = self.active_model.write().unwrap();
        active.thinking_level =
            tidev_types::reasoning::ThinkingLevelType::from_string(thinking_level);
        Ok(())
    }

    /// Create a new session with current workspace and active model settings.
    pub fn create_default_session(&self, title: &str) -> Result<Uuid> {
        let session_id = Uuid::new_v4();
        let model = self.active_model.read().unwrap();
        self.session_manager.create_session(
            session_id,
            &self.workspace_root.to_string_lossy(),
            &model.provider_id,
            &model.provider_display_name,
            &model.model_id,
            &model.display_name,
            title,
        )?;
        Ok(session_id)
    }

    // -----------------------------------------------------------------------
    // Operations
    // -----------------------------------------------------------------------

    /// Append a message to a session (both in-memory buffer and store).
    ///
    /// This is the single correct way for external code (e.g. the TUI's
    /// request handler) to add messages to a session — it keeps the
    /// [`MessageBuffer`] and SQLite in sync so that [`continue_session`]
    /// picks up the new data.
    pub async fn append_message(&self, session_id: Uuid, msg: tidev_types::message::Message) -> Result<()> {
        {
            let buf = self.message_buffer(session_id).await;
            buf.write().await.append(msg.clone());
        }
        self.session_manager.append_message(session_id, &msg)?;
        Ok(())
    }

    /// Submit a user prompt for a session.
    pub async fn submit_prompt(&self, session_id: Uuid, content: String) -> Result<()> {
        self.submit_prompt_with_attachments(session_id, content, Vec::new())
            .await
    }

    /// Submit a user prompt with file/directory/image attachments.
    ///
    /// Attachments are typically built from `@`-references by
    /// [`crate::attachment::build_attachments`] and represent files
    /// the user wants to include with their message.
    pub async fn submit_prompt_with_attachments(
        &self,
        session_id: Uuid,
        content: String,
        attachments: Vec<MessageAttachment>,
    ) -> Result<()> {
        let mut user_msg = Message::new(MessageRole::User, content);
        user_msg.attachments = attachments;

        // 1. Persist the user message.
        {
            let buf = self.message_buffer(session_id).await;
            buf.write().await.append(user_msg.clone());
        }
        self.session_manager.append_message(session_id, &user_msg)?;

        // 2. Check if a loop is already running.
        if self.is_loop_running() {
            // Loop running — new message will be picked up on next turn.
            return Ok(());
        }

        // 3. Build CoreContext + AgentLoopConfig and spawn the loop.
        self.start_agent_loop(session_id).await
    }

    /// Continue an existing session without adding a new user message.
    ///
    /// Used when resuming the parent session after a subagent returns — the
    /// tool result message is already in the store and gets loaded into the
    /// [`MessageBuffer`] here.
    pub async fn continue_session(&self, session_id: Uuid) -> Result<()> {
        if self.is_loop_running() {
            // Already running — new data will be picked up on next turn.
            return Ok(());
        }

        // Reload the buffer from the store so any messages added while the
        // loop wasn't running (e.g. subagent results) are picked up.
        self.reload_message_buffer(session_id).await;

        self.start_agent_loop(session_id).await
    }

    /// Quick synchronous check — is the agent loop active?
    pub fn is_busy(&self) -> bool {
        self.loop_busy.load(Ordering::SeqCst)
    }

    /// Check whether an agent loop is currently running.
    fn is_loop_running(&self) -> bool {
        self.run_loop_handle.lock().unwrap().is_some()
    }

    /// Reload the in-memory [`MessageBuffer`] for a session from the store.
    pub async fn reload_message_buffer(&self, session_id: Uuid) {
        let buf = self.message_buffer(session_id).await;
        if let Ok(messages) = self.session_manager.load_messages(session_id) {
            buf.write().await.replace_all(messages);
        }
    }

    /// Build [`CoreContext`] + [`AgentLoopConfig`] and spawn the agent loop.
    async fn start_agent_loop(&self, session_id: Uuid) -> Result<()> {
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
                    let instructions = self.config.read().unwrap().instructions.clone();
                    let sp = crate::agent_ctx::compose_system_prompt(
                        tidev_types::agent_type::AgentType::General,
                        &instructions,
                        &self.workspace_root,
                        &self.paths.config_dir,
                        SessionMode::Build,
                    );
                    // Persist system prompt to session.
                    self.session_manager
                        .update_session(session_id, None, None)?;
                    sp
                }
            }
        };

        let active_model = self.active_model.read().unwrap().clone();
        let llm_config = crate::agent_ctx::to_llm_provider_config(&active_model);

        let agent_def = AgentDefinition {
            agent_type: tidev_types::agent_type::AgentType::General,
            display_name: "tidev".into(),
            description: "A terminal-based AI coding agent".into(),
            system_prompt: system_prompt.clone(),
            allowed_tools: None,
            temperature: None,
            read_only: false,
        };

        let filtered_tools = self.tool_registry.definitions_for_model(&active_model);
        let ctx = crate::agent_ctx::CoreContext::new(
            self.llm.clone(),
            self.session_manager.clone(),
            self.tool_registry.clone(),
            context_manager,
            buffer,
            self.event_tx.clone(),
            self.request_tx.clone(),
            session_id,
            SessionMode::Build,
            system_prompt,
            llm_config,
            cancel.clone(),
            filtered_tools,
            self.workspace_root.clone(),
            active_model.clone(),
            self.snapshot.clone(),
            self.config.clone(),
            self.auth.clone(),
        );

        let loop_config = tidev_agent::AgentLoopConfig {
            session_id,
            definition: agent_def,
            mode: SessionMode::Build,
            thinking_level: active_model.thinking_level.clone(),
            event_tx: self.event_tx.clone(),
            cancel,
        };

        self.loop_busy.store(true, Ordering::SeqCst);

        let busy_flag = self.loop_busy.clone();
        let handle_slot = self.run_loop_handle.clone();
        let join = tokio::spawn(async move {
            let _guard = AgentLoopGuard { busy: busy_flag, handle: handle_slot };
            if let Err(e) = tidev_agent::run_agent_loop(&ctx, loop_config).await {
                log::error!("agent loop for session {session_id} exited with error: {e}");
            }
        });

        {
            let mut handle = self.run_loop_handle.lock().unwrap();
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

        self.loop_busy.store(false, Ordering::SeqCst);

        // Give cooperative exit a brief window, then force-abort.
        let handle = self.run_loop_handle.lock().unwrap().take();
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

    /// Gracefully shut down background tasks.
    ///
    /// Cancels the output cleanup task and kills any remaining child processes.
    /// Call this when the application exits (e.g., after the TUI event loop).
    pub async fn shutdown(&self) {
        self.cleanup_cancel.cancel();
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

    /// Manually trigger context compaction for a session.
    ///
    /// Used by the `/compact` command. The TUI provides the current `mode`
    /// (active session mode) and an optional `stream_request_id` for streaming
    /// compaction output.
    pub async fn compact_session(
        &self,
        session_id: Uuid,
        mode: SessionMode,
        stream_request_id: Option<u64>,
    ) -> Result<()> {
        use crate::agent_ctx::to_llm_provider_config;

        // 1. Collect the inputs: messages, context manager, model config, tools.
        let messages = {
            let buf = self.message_buffer(session_id).await;
            buf.read().await.load().to_vec()
        };
        let cm = self.context_manager(session_id).await;
        let model_config = {
            let active = self.active_model.read().unwrap();
            to_llm_provider_config(&active)
        };
        let active_model = self.active_model.read().unwrap().clone();
        let tools = self.tool_registry.definitions_for_model(&active_model);

        // 2. Run compaction (async, no locks held on ContextManager).
        let result = {
            let cm_lock = cm.lock().await;
            cm_lock
                .compact(
                    &self.llm,
                    &model_config,
                    &tools,
                    &messages,
                    mode,
                    session_id,
                    Some(self.event_tx.clone()),
                )
                .await?
        };

        // 3. Apply compaction state.
        {
            let mut cm_lock = cm.lock().await;
            cm_lock.apply_compaction(result.summary.clone(), result.retained_from);
        }

        // 4. Persist compaction state to the session store.
        self.session_manager.update_context_state(
            session_id,
            Some(&result.summary),
            result.retained_from,
        )?;

        // 5. Notify the TUI (BackendEvent::ContextCompacted is already sent by
        //    compact() via event_tx when streaming, but for consistency we
        //    always send the final event here as well).
        let model_id = active_model.model_id.clone();
        let _ = self.event_tx.send(BackendEvent::ContextCompacted {
            session_id,
            compacted: true,
            manual: stream_request_id.is_some(),
            summary: Some(result.summary),
            retained_from: result.retained_from,
            model_id: Some(model_id),
            completed_at: Some(Utc::now()),
            error: None,
        });

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
            model_id: None,
            completed_at: None,
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
            model_id: None,
            completed_at: None,
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

    /// Resolve the active model, falling back to the first available model
    /// if the default is not configured.
    fn resolve_fallback_model(
        config: &AppConfig,
        auth: &AuthStore,
    ) -> Result<ActiveModel> {
        if let Ok(model) = config.resolve_active_model(auth) {
            return Ok(model);
        }
        let summary = config
            .available_models()
            .into_iter()
            .next()
            .context("no models are configured")?;
        config.resolve_model_by_ids(auth, &summary.provider_id, &summary.model_id)
    }

    /// Build the runtime.
    pub async fn build(self) -> Result<Runtime> {
        let _start = Instant::now();

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

        // 2. Config + auth (with project-level overlay).
        let workspace_root = self.workspace_root.clone().unwrap_or_default();
        let config = AppConfig::load_with_overlay(&paths, &workspace_root)?;
        let auth = AuthStore::load_or_create(&paths)?;

        // ── Startup initialisation ─────────────────────────────────
        //
        // Everything below is best-effort initialisation that follows
        // the old tidev v0.6.x startup sequence.

        // 3. Logging (file + console via custom TidevLogger).
        tidev_logging::init(&paths.data_dir, &config.logging);
        log::info!("Runtime initialising, workspace={}", workspace_root.display());
        log::info!("startup: config + auth loaded in {:?}", _start.elapsed());

        // 4. Shell detection (must happen before any tool execution).
        tidev_tools::shell::init(config.shell.windows_shell.clone(), Some(&paths));

        // 5. Auto-cleanup of temp files (best-effort, ignore errors).
        if config.tmp.auto_cleanup {
            let max_age = Duration::from_secs(config.tmp.max_age_hours * 3600);
            match tidev_utils::tmp::clean_temp_files(max_age, false) {
                Ok(removed) if !removed.is_empty() => {
                    log::info!("Cleaned up {} temp file(s)", removed.len());
                }
                Ok(_) => {}
                Err(e) => log::warn!("Failed to clean temp files: {e}"),
            }
        }

        // 6. Database + session store.
        let _t_db = Instant::now();
        let database = tidev_storage::database::Database::open(&paths.database_file)
            .context("failed to open database")?;
        let store = database.create_store()?;
        log::info!("startup: database opened in {:?}", _t_db.elapsed());

        // Delete expired tool outputs on startup (best-effort).
        if let Ok(count) = store.delete_expired_tool_outputs(7) {
            if count > 0 {
                log::info!("Cleaned up {count} old tool output(s)");
            }
        }

        // 7. LLM client + model resolution (with fallback).
        let _t_llm = Instant::now();
        let active_model = Self::resolve_fallback_model(&config, &auth)
            .context("no models are configured — set up a provider API key first")?;
        let llm = tidev_llm::LlmClient::new(
            config.logging.save_request_body,
            config.logging.max_request_files,
            config.logging.save_response_body,
            config.logging.max_response_files,
        )?;
        log::info!("startup: LLM client ready in {:?}", _t_llm.elapsed());

        // 8. Skills catalog.
        let _t_skills = Instant::now();
        let skills = tidev_tools::SkillCatalog::discover(
            &workspace_root,
            &paths.config_dir,
            &config.skills,
            None,
        );
        log::info!("startup: skills catalog ready in {:?}", _t_skills.elapsed());

        // 9. Todo persistence bridge.
        let todo = Arc::new(TodoStore {
            store: store.clone(),
        });

        // 10. Tool registry.
        let _t_tools = Instant::now();
        let max_output_bytes = active_model.max_output_tokens * 2; // heuristic: 2x output tokens ≈ bytes
        let tool_registry = Arc::new(ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            skills.clone(),
            todo,
            config.websearch.clone(),
            auth.clone(),
            max_output_bytes,
            config.permissions.clone(),
        ));
        log::info!("startup: tool registry ready in {:?}", _t_tools.elapsed());

        // 11. Snapshot service.
        let _t_snap = Instant::now();
        let snapshot_config = Arc::new(config.snapshot.clone());
        let snapshot = if snapshot_config.enabled {
            Some(tidev_snapshot::SnapshotService::new(
                &workspace_root,
                &paths,
                snapshot_config,
            )?)
        } else {
            None
        };
        log::info!("startup: snapshot service ready in {:?}", _t_snap.elapsed());

        // Store active model for loop construction.
        let active_model = Arc::new(StdRwLock::new(active_model));

        // 12. Session manager.
        let session_manager = SessionManager::new(store.clone());

        // 13. Start background tool-output cleanup (hourly).
        let cleanup_cancel = CancellationToken::new();
        {
            let cancel = cleanup_cancel.clone();
            let cstore = store.clone();
            tokio::spawn(async move {
                let interval = Duration::from_secs(3600);
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(interval) => {
                            if let Ok(count) = cstore.delete_expired_tool_outputs(7) {
                                if count > 0 {
                                    log::info!("Cleaned up {count} old tool output(s)");
                                }
                            }
                        }
                        _ = cancel.cancelled() => break,
                    }
                }
            });
        }

        // 14. Channels.
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let (request_tx, request_rx) = tokio::sync::mpsc::unbounded_channel::<TuiRequest>();

        log::info!("startup: runtime ready in {:?}", _start.elapsed());

        Ok(Runtime {
            config: Arc::new(StdRwLock::new(config)),
            auth: Arc::new(StdRwLock::new(auth)),
            paths,
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
            request_tx,
            _request_rx: Arc::new(Mutex::new(Some(request_rx))),
            run_loop_handle: Arc::new(StdMutex::new(None)),
            loop_busy: Arc::new(AtomicBool::new(false)),
            cleanup_cancel,
            workspace_root,
            file_search_index: OnceLock::new(),
        })
    }
}
