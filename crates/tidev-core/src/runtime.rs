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
//! // Subscribe to events (each call registers a new subscriber)
//! let mut events = rt.event_rx().await;
//! while let Some(event) = events.recv().await { ... }
//! ```

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, Notify, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::mode::Mode;
use tidev_config::auth::ActiveModel;
use tidev_config::{AppConfig, AuthStore, SendWhileBusy, paths::ConfigPaths};
use tidev_llm::message::{Message, MessageAttachment, MessageRole};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_search::FileSearchIndex;
use tidev_storage::{MessageAppData, SessionStore};
use tidev_tools::types::TodoItem;

use tidev_agent::{AgentContext, ContextManager};

use crate::approval::{ApprovalBroker, FrontendRequest, FrontendResponse};
use crate::backend_event::{BackendEvent, CoreEventBus};
use crate::event_hub::EventHub;
use crate::mcp::McpManager;
use crate::message_buf::CoreMessageBuffer;
use crate::registry::ToolRegistry;
use crate::session::SessionManager;
use crate::tool_def::to_llm_tool_def;

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
        self.store.load_todos(session_id)
    }

    fn replace_todos(&self, session_id: Uuid, todos: &[TodoItem]) -> anyhow::Result<()> {
        self.store.save_todos(session_id, todos)
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
    /// Per-session cancellation tokens for active agent loops.
    active_loop_cancels: Arc<std::sync::Mutex<HashMap<Uuid, CancellationToken>>>,
    /// Snapshot service for undo/redo (optional).
    snapshot: Option<tidev_snapshot::SnapshotService>,
    /// Read-only Git service for the user's workspace repository.
    git: crate::git::GitService,

    /// Per-session message buffers, keyed by session ID.
    buffers: Arc<Mutex<HashMap<Uuid, Arc<RwLock<CoreMessageBuffer>>>>>,
    /// Per-session context managers, keyed by session ID.
    context_managers: Arc<Mutex<HashMap<Uuid, Arc<Mutex<ContextManager>>>>>,

    /// Event channel (sender → frontend hub).
    event_tx: UnboundedSender<BackendEvent>,
    /// Ordered event buses keyed by session. Agent and backend events for a
    /// session must enter the frontend channel through the same FIFO queue.
    event_buses: Arc<Mutex<HashMap<Uuid, CoreEventBus>>>,
    /// Ordered, replayable frontend event distribution.
    event_hub: EventHub,
    /// Frontend-neutral approval broker. Requests are fanned out to every
    /// registered subscriber while responses are routed by request ID.
    approval_broker: ApprovalBroker,
    /// Registered request subscribers. Each call to [`Runtime::request_rx`]
    /// adds one; dead senders are pruned by the fan-out task.
    request_subscribers: Arc<Mutex<Vec<UnboundedSender<FrontendRequest>>>>,

    /// Currently running agent loop handles, keyed by session ID.
    run_loop_handles: Arc<std::sync::Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
    /// Set of session IDs with active agent loops.
    busy_sessions: Arc<std::sync::Mutex<HashSet<Uuid>>>,

    /// Per-session queues of user messages submitted while the session's
    /// agent loop was busy.
    ///
    /// Steering entries are persisted to the message buffer immediately by
    /// `submit_prompt_with_attachments` and only serve as a keep-alive
    /// signal for the running loop (see `steer_signals`). Queueing entries
    /// are drained by the host after the loop exits, persisted, and start
    /// the next turn.
    pending_prompts: Arc<std::sync::Mutex<HashMap<Uuid, VecDeque<PendingPrompt>>>>,
    /// Per-session steering signals. Set when a steering message is
    /// submitted while the loop is running; consumed by the loop at the
    /// end of a turn without tool calls so it keeps running instead of
    /// exiting.
    steer_signals: Arc<std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,

    /// Completion notifications for frontends that await a full session run.
    session_idle_notifies: Arc<StdMutex<HashMap<Uuid, Arc<Notify>>>>,
    /// Error from the most recent full session run, if any.
    session_outcomes: Arc<StdMutex<HashMap<Uuid, Option<String>>>>,

    /// Guards the check-and-start sequence in submit_prompt / continue_session
    /// to prevent a TOCTOU race where two tasks both see is_session_busy=false
    /// and both call start_agent_loop for the same session.
    session_start_lock: Arc<std::sync::Mutex<()>>,

    /// Serializes prompt submission through the shared Runtime. This makes a
    /// client retry with the same message ID idempotent and gives concurrent
    /// frontends one authoritative insertion order.
    prompt_submission_lock: Arc<Mutex<()>>,

    /// Cancellation token for background cleanup tasks.
    cleanup_cancel: CancellationToken,

    /// Workspace root.
    workspace_root: PathBuf,

    /// File search index for @mention autocomplete.
    /// Lazily initialised on first access.
    file_search_index: OnceLock<Arc<FileSearchIndex>>,
}

/// RAII guard that removes a session from `busy_sessions` and
/// `run_loop_handles` on drop (including on task panic).
struct SessionLoopGuard {
    session_id: Uuid,
    busy_sessions: Arc<std::sync::Mutex<HashSet<Uuid>>>,
    handles: Arc<std::sync::Mutex<HashMap<Uuid, tokio::task::JoinHandle<()>>>>,
    cancels: Arc<std::sync::Mutex<HashMap<Uuid, CancellationToken>>>,
    steer_signals: Arc<std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
    idle_notify: Arc<Notify>,
}

impl Drop for SessionLoopGuard {
    fn drop(&mut self) {
        self.handles.lock().unwrap().remove(&self.session_id);
        self.cancels.lock().unwrap().remove(&self.session_id);
        self.busy_sessions.lock().unwrap().remove(&self.session_id);
        self.steer_signals.lock().unwrap().remove(&self.session_id);
        self.idle_notify.notify_waiters();
    }
}

/// How a user message submitted while the agent loop is busy is delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeliveryMode {
    /// Wait until the current turn (all requests and tool calls) finishes,
    /// then start a new turn with the message.
    Queue,
    /// Persist immediately and insert into the running turn at the next
    /// request boundary, without interrupting the in-flight stream.
    Steer,
}

/// A user message submitted while the session's agent loop was busy.
///
/// The host decides delivery from the `send_while_busy` config: steering
/// entries are persisted to the buffer at submission time (the entry is
/// only a keep-alive signal), queueing entries are persisted after the
/// current turn exits.
#[derive(Clone, Debug)]
struct PendingPrompt {
    message_id: Uuid,
    delivery: DeliveryMode,
    mode: Mode,
    content: String,
    attachments: Vec<MessageAttachment>,
    thinking_level: Option<ThinkingLevelType>,
}

/// A frontend-neutral user prompt submission.
///
/// Clients generate `message_id` once and reuse it when retrying a command.
/// Runtime uses it as the durable idempotency key for the user message.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PromptSubmission {
    pub message_id: Uuid,
    pub content: String,
    pub mode: Mode,
    pub attachments: Vec<MessageAttachment>,
    pub thinking_level: Option<ThinkingLevelType>,
}

impl PromptSubmission {
    pub fn new(content: String, mode: Mode) -> Self {
        Self {
            message_id: Uuid::new_v4(),
            content,
            mode,
            attachments: Vec::new(),
            thinking_level: None,
        }
    }
}

/// Receipt returned for a prompt submission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromptSubmissionReceipt {
    pub message_id: Uuid,
    /// True when a retry matched a prompt already accepted by Runtime.
    pub duplicate: bool,
}

impl Runtime {
    /// Create a new builder.
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::new()
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Subscribe to backend events using the legacy event-only receiver.
    ///
    /// New frontends should prefer [`Runtime::subscribe_events`] so they can
    /// resume from an event cursor after reconnecting.
    pub async fn event_rx(&self) -> UnboundedReceiver<BackendEvent> {
        let subscription = self.subscribe_events(None).await;
        let mut events = subscription.into_receiver();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(envelope) = events.recv().await {
                if tx.send(envelope.event).is_err() {
                    break;
                }
            }
        });
        rx
    }

    /// Subscribe to ordered frontend events. The returned replay result and
    /// live receiver are captured atomically, so a reconnecting client can
    /// process replay before draining live events without a gap.
    pub async fn subscribe_events(
        &self,
        after: Option<crate::EventCursor>,
    ) -> crate::EventSubscription {
        self.event_hub.subscribe(after).await
    }

    /// Get (or create) the ordered event bus for a session.
    pub(crate) async fn event_bus(&self, session_id: Uuid) -> CoreEventBus {
        let mut buses = self.event_buses.lock().await;
        buses
            .entry(session_id)
            .or_insert_with(|| CoreEventBus::new(self.event_tx.clone(), session_id))
            .clone()
    }

    /// Subscribe to frontend requests (tool approval etc.).
    ///
    /// Every call registers a new subscriber and returns its receiver;
    /// requests are broadcast to all subscribers concurrently. Each
    /// Subscribers answer through [`Runtime::respond_to_request`]. The core
    /// broker guarantees that the first response wins.
    pub async fn request_rx(&self) -> UnboundedReceiver<FrontendRequest> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        self.request_subscribers.lock().await.push(tx);
        rx
    }

    /// Answer a frontend request, routing the response to the waiting agent
    /// loop. Multiple frontends may observe a request, but only the first
    /// response is accepted.
    pub fn respond_to_request(
        &self,
        request_id: Uuid,
        response: FrontendResponse,
    ) -> Result<(), crate::ApprovalError> {
        self.approval_broker.respond(request_id, response)
    }

    /// Get (or create) the message buffer for a session.
    pub async fn message_buffer(&self, session_id: Uuid) -> Arc<RwLock<CoreMessageBuffer>> {
        let mut bufs = self.buffers.lock().await;
        if let Some(buf) = bufs.get(&session_id) {
            return buf.clone();
        }
        // Load from DB and create.
        let messages = self
            .session_manager
            .load_session_messages(session_id)
            .unwrap_or_default();
        let buf = Arc::new(RwLock::new(CoreMessageBuffer::from_session_messages(
            messages,
        )));
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

    /// Get the MCP manager (delegates to tool registry).
    pub fn mcp_manager(&self) -> &McpManager {
        self.tool_registry.mcp_manager()
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

    /// Get a cloneable read-only Git service for the workspace repository.
    pub fn git(&self) -> crate::git::GitService {
        self.git.clone()
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

    /// Atomically update the application configuration and refresh consumers
    /// that keep derived runtime state.
    pub fn update_config(&self, f: impl FnOnce(&mut AppConfig)) {
        let logging = {
            let mut config = self.config.write().unwrap();
            f(&mut config);
            config.logging.clone()
        };

        self.llm.update_debug_config(tidev_llm::LlmDebugConfig {
            save_request_body: logging.save_request_body,
            max_request_files: logging.max_request_files,
            save_response_body: logging.save_response_body,
            max_response_files: logging.max_response_files,
        });
        tidev_logging::reload(&self.paths.data_dir, &logging);
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

    /// Resolve the configured active model with its persisted thinking level.
    pub fn resolve_active_model(&self) -> Result<ActiveModel> {
        let config = self.config();
        let auth = self.auth();
        let mut model = config.resolve_active_model(&auth)?;
        self.apply_saved_thinking_level(&mut model);
        Ok(model)
    }

    fn apply_saved_thinking_level(&self, model: &mut ActiveModel) {
        if let Ok(Some(level)) = self
            .session_manager
            .store()
            .load_model_thinking_level(&model.provider_id, &model.model_id)
        {
            model.thinking_level = ThinkingLevelType::from_string(&level);
        }
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
        provider_id: &str,
        model_id: &str,
        thinking_level: &str,
    ) -> Result<()> {
        let mut active = self.active_model.write().unwrap();
        active.thinking_level =
            tidev_llm::reasoning::ThinkingLevelType::from_string(thinking_level);
        if let Err(e) = self.session_manager.store().save_model_thinking_level(
            provider_id,
            model_id,
            thinking_level,
        ) {
            log::warn!("failed to save thinking level preference: {}", e);
        }
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
            None,
            None,
        )?;
        Ok(session_id)
    }

    /// Update the title of an existing session.
    pub fn update_session_title(&self, session_id: Uuid, title: &str) -> Result<()> {
        self.session_manager
            .update_session(session_id, Some(title), None)
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
    pub async fn append_message(
        &self,
        session_id: Uuid,
        msg: tidev_llm::message::Message,
    ) -> Result<()> {
        {
            let buf = self.message_buffer(session_id).await;
            buf.write().await.append(msg.clone());
        }
        self.session_manager.append_message(session_id, &msg)?;
        Ok(())
    }

    /// Submit a user prompt for a session.
    pub async fn submit_prompt(&self, session_id: Uuid, content: String, mode: Mode) -> Result<()> {
        self.submit_prompt_submission(session_id, PromptSubmission::new(content, mode))
            .await
            .map(|_| ())
    }

    /// Submit a user prompt with file/directory/image attachments.
    ///
    /// Attachments are typically built from `@`-references by
    /// [`crate::attachment::build_attachments`] and represent files
    /// the user wants to include with their message.
    ///
    /// `mode` is the session mode at submission time, used to tag the message
    /// and to construct the agent loop context.
    ///
    /// When the session's agent loop is already running, the `send_while_busy`
    /// config decides the delivery:
    ///
    /// - `steer`: the message is persisted immediately (with a
    ///   `<system-reminder>` suffix) and inserted into the running turn at the
    ///   next request boundary, without interrupting the in-flight stream.
    /// - `queue`: the message is held in the pending queue and only persisted
    ///   after the current turn exits, starting the next turn.
    pub async fn submit_prompt_with_attachments(
        &self,
        session_id: Uuid,
        mode: Mode,
        content: String,
        attachments: Vec<MessageAttachment>,
        thinking_level: Option<ThinkingLevelType>,
    ) -> Result<()> {
        let mut submission = PromptSubmission::new(content, mode);
        submission.attachments = attachments;
        submission.thinking_level = thinking_level;
        self.submit_prompt_submission(session_id, submission)
            .await
            .map(|_| ())
    }

    /// Submit a prompt with a caller-provided message ID.
    ///
    /// The ID is an idempotency key: repeating an already accepted submission
    /// returns a duplicate receipt without changing the buffer, persistence,
    /// queue, or future LLM request bytes.
    pub async fn submit_prompt_submission(
        &self,
        session_id: Uuid,
        submission: PromptSubmission,
    ) -> Result<PromptSubmissionReceipt> {
        let _submission_guard = self.prompt_submission_lock.lock().await;
        if self
            .find_existing_prompt_submission(session_id, &submission)
            .await?
        {
            return Ok(PromptSubmissionReceipt {
                message_id: submission.message_id,
                duplicate: true,
            });
        }

        let PromptSubmission {
            message_id,
            content,
            mode,
            attachments,
            thinking_level,
        } = submission;

        // Decide delivery before mutating anything. start_agent_loop
        // re-checks busy under a mutex for atomicity.
        let delivery = if self.is_session_busy(session_id) {
            match self.config.read().unwrap().ui.send_while_busy {
                SendWhileBusy::Steer => Some(DeliveryMode::Steer),
                SendWhileBusy::Queue => Some(DeliveryMode::Queue),
            }
        } else {
            None
        };

        // 1. If undo revert is active, discard hidden messages first.
        //    Only when starting a new turn — truncating the buffer while a
        //    loop is running would change the running loop's request bytes.
        if delivery.is_none()
            && let Ok(Some((revert_msg_id, _))) = self.session_manager.load_revert_state(session_id)
            && revert_msg_id != Uuid::nil()
        {
            let buf = self.message_buffer(session_id).await;
            let pos = {
                let mut write_buf = buf.write().await;
                let pos = write_buf
                    .load()
                    .iter()
                    .position(|m| m.id == revert_msg_id)
                    .unwrap_or(write_buf.len());
                let to_remove: Vec<Uuid> = write_buf.load()[pos..].iter().map(|m| m.id).collect();
                write_buf.truncate(pos);
                if !to_remove.is_empty() {
                    self.session_manager
                        .delete_messages(session_id, &to_remove)?;
                };
                pos
            };
            self.session_manager
                .save_revert_state(session_id, Uuid::nil(), None)?;
            let _ =
                self.event_bus(session_id)
                    .await
                    .send_backend(BackendEvent::MessagesTruncated {
                        session_id,
                        kept_count: pos,
                    });
        }

        // 2. Build the user message. Steering messages carry a
        //    system-reminder suffix so the model keeps advancing the task
        //    while adjusting direction.
        let mut user_msg = Message::new(MessageRole::User, content);
        user_msg.id = message_id;
        user_msg.attachments = attachments;
        user_msg.thinking_level = thinking_level;
        if delivery == Some(DeliveryMode::Steer) {
            user_msg.content = format!(
                "{}\n\n{}",
                user_msg.content,
                crate::prompts::steer_reminder()
            );
        }
        let user_app_data = MessageAppData {
            mode: Some(mode.as_str().to_string()),
            ..Default::default()
        };

        let event_app_data = user_app_data.clone();

        match delivery {
            // 4a. Steering: persist now and signal the running loop. The
            //     next load_messages() in the loop picks the message up.
            Some(DeliveryMode::Steer) => {
                let buf = self.message_buffer(session_id).await;
                self.persist_user_message(session_id, &buf, &user_msg, &user_app_data)
                    .await?;
                let _ = self.event_bus(session_id).await.send_backend(
                    BackendEvent::UserMessageCreated {
                        session_id,
                        message: Box::new(user_msg),
                        app_data: Box::new(event_app_data),
                        queued: true,
                    },
                );
                self.push_pending_prompt(
                    session_id,
                    PendingPrompt {
                        message_id,
                        delivery: DeliveryMode::Steer,
                        mode,
                        content: String::new(),
                        attachments: Vec::new(),
                        thinking_level: None,
                    },
                );
                Ok(PromptSubmissionReceipt {
                    message_id,
                    duplicate: false,
                })
            }
            // 4b. Queueing: hold the content in the pending queue. The host
            //     persists it after the current turn exits and starts the
            //     next turn.
            Some(DeliveryMode::Queue) => {
                let q_content = user_msg.content.clone();
                let q_attachments = user_msg.attachments.clone();
                let q_thinking = user_msg.thinking_level.clone();
                let _ = self.event_bus(session_id).await.send_backend(
                    BackendEvent::UserMessageCreated {
                        session_id,
                        message: Box::new(user_msg),
                        app_data: Box::new(event_app_data),
                        queued: true,
                    },
                );
                self.push_pending_prompt(
                    session_id,
                    PendingPrompt {
                        message_id,
                        delivery: DeliveryMode::Queue,
                        mode,
                        content: q_content,
                        attachments: q_attachments,
                        thinking_level: q_thinking,
                    },
                );
                Ok(PromptSubmissionReceipt {
                    message_id,
                    duplicate: false,
                })
            }
            // 4c. Idle: persist and spawn the loop.
            None => {
                let buf = self.message_buffer(session_id).await;
                self.persist_user_message(session_id, &buf, &user_msg, &user_app_data)
                    .await?;
                let _ = self.event_bus(session_id).await.send_backend(
                    BackendEvent::UserMessageCreated {
                        session_id,
                        message: Box::new(user_msg),
                        app_data: Box::new(event_app_data),
                        queued: false,
                    },
                );
                self.start_agent_loop(session_id, mode).await?;
                Ok(PromptSubmissionReceipt {
                    message_id,
                    duplicate: false,
                })
            }
        }
    }

    /// Check whether Runtime has already accepted a prompt submission with the
    /// caller-provided message ID. A reused ID with different content is a
    /// protocol error rather than a second user message.
    async fn find_existing_prompt_submission(
        &self,
        session_id: Uuid,
        submission: &PromptSubmission,
    ) -> Result<bool> {
        let buffer = self.message_buffer(session_id).await;
        {
            let buffer = buffer.read().await;
            if let Some(message) = buffer
                .load()
                .iter()
                .find(|message| message.id == submission.message_id)
            {
                let mode_matches = buffer
                    .app_data(message.id)
                    .and_then(|data| data.mode.as_deref())
                    == Some(submission.mode.as_str());
                if prompt_message_matches_submission(message, submission) && mode_matches {
                    return Ok(true);
                }
                anyhow::bail!(
                    "message ID {} was already accepted with different content",
                    submission.message_id
                );
            }
        }

        let queued = self
            .pending_prompts
            .lock()
            .expect("pending prompt mutex poisoned");
        if let Some(prompt) = queued.get(&session_id).and_then(|prompts| {
            prompts
                .iter()
                .find(|prompt| prompt.message_id == submission.message_id)
        }) {
            if prompt.delivery == DeliveryMode::Queue
                && prompt.mode == submission.mode
                && prompt.content == submission.content
                && prompt.attachments == submission.attachments
                && prompt.thinking_level == submission.thinking_level
            {
                return Ok(true);
            }
            anyhow::bail!(
                "message ID {} was already accepted with different content",
                submission.message_id
            );
        }
        Ok(false)
    }

    /// Persist a user message to the in-memory buffer and the store,
    /// paired with its application data.
    async fn persist_user_message(
        &self,
        session_id: Uuid,
        buf: &Arc<RwLock<CoreMessageBuffer>>,
        msg: &Message,
        app_data: &MessageAppData,
    ) -> Result<()> {
        buf.write()
            .await
            .append_with_app_data(msg.clone(), app_data.clone());
        self.session_manager.append_messages_with_app_data(
            session_id,
            std::slice::from_ref(msg),
            &[(msg.id, app_data.clone())].into_iter().collect(),
        )
    }

    /// Register a pending prompt for a busy session.
    ///
    /// Steering entries set the session's steering signal so the running
    /// loop keeps going (their content is already persisted); queueing
    /// entries are drained by the host after the loop exits.
    fn push_pending_prompt(&self, session_id: Uuid, prompt: PendingPrompt) {
        let delivery = prompt.delivery;
        let mut queue = self.pending_prompts.lock().unwrap();
        queue.entry(session_id).or_default().push_back(prompt);
        if delivery == DeliveryMode::Steer
            && let Some(signal) = self.steer_signals.lock().unwrap().get(&session_id)
        {
            signal.store(true, Ordering::SeqCst);
        }
    }

    /// Continue an existing session without adding a new user message.
    ///
    /// Used when resuming the parent session after a subagent returns — the
    /// tool result message is already in the store and gets loaded into the
    /// [`MessageBuffer`] here.
    ///
    /// `mode` should be the session's current mode; it is read from the last
    /// user message's `mode` field if `None` is passed.
    pub async fn continue_session(&self, session_id: Uuid, mode: Option<Mode>) -> Result<()> {
        // Fast path: avoid DB reload if the session is already running.
        // The atomic check in start_agent_loop prevents TOCTOU.
        if self.is_session_busy(session_id) {
            return Ok(());
        }

        // Reload the buffer from the store so any messages added while the
        // loop wasn't running (e.g. subagent results) are picked up.
        self.reload_message_buffer(session_id).await;

        // Resolve mode from the last user message if not provided.
        let mode = match mode {
            Some(m) => m,
            None => {
                let messages = self.session_manager.load_session_messages(session_id)?;
                messages
                    .iter()
                    .rev()
                    .find(|m| m.role == MessageRole::User)
                    .and_then(|m| m.mode())
                    .unwrap_or(Mode::Build)
            }
        };

        self.start_agent_loop(session_id, mode).await
    }

    /// Quick synchronous check — is any session's agent loop active?
    pub fn is_busy(&self) -> bool {
        !self.busy_sessions.lock().unwrap().is_empty()
    }

    /// Check whether a specific session has an agent loop running.
    pub fn is_session_busy(&self, session_id: Uuid) -> bool {
        self.busy_sessions.lock().unwrap().contains(&session_id)
    }

    /// Wait until the session's outer agent loop has completely exited.
    pub async fn wait_for_session(&self, session_id: Uuid) -> Result<()> {
        let notify = {
            let mut notifies = self.session_idle_notifies.lock().unwrap();
            notifies
                .entry(session_id)
                .or_insert_with(|| Arc::new(Notify::new()))
                .clone()
        };

        loop {
            let notified = notify.notified();
            if !self.is_session_busy(session_id) {
                if let Some(error) = self
                    .session_outcomes
                    .lock()
                    .unwrap()
                    .get(&session_id)
                    .cloned()
                    .flatten()
                {
                    anyhow::bail!(error);
                }
                return Ok(());
            }
            notified.await;
        }
    }

    /// List all session IDs with active agent loops.
    pub fn active_sessions(&self) -> Vec<Uuid> {
        self.busy_sessions.lock().unwrap().iter().copied().collect()
    }

    /// Reload the in-memory [`MessageBuffer`] for a session from the store.
    pub async fn reload_message_buffer(&self, session_id: Uuid) {
        let buf = self.message_buffer(session_id).await;
        if let Ok(messages) = self.session_manager.load_session_messages(session_id) {
            buf.write()
                .await
                .replace_all_with_session_messages(messages);
        }
    }

    /// Set the in-memory [`MessageBuffer`] for a session with pre-loaded messages.
    ///
    /// Avoids a redundant DB read when the caller already has the messages
    /// in hand (e.g., after loading them for the UI).
    pub async fn set_message_buffer(&self, session_id: Uuid, messages: Vec<Message>) {
        let mut bufs = self.buffers.lock().await;
        if let Some(buf) = bufs.get(&session_id) {
            buf.write().await.replace_all(messages);
        } else {
            let buf = Arc::new(RwLock::new(CoreMessageBuffer::new(messages)));
            bufs.insert(session_id, buf);
        }
    }

    /// Set the in-memory message buffer from protocol messages paired with app data.
    pub async fn set_session_message_buffer(
        &self,
        session_id: Uuid,
        messages: Vec<crate::SessionMessage>,
    ) {
        let mut bufs = self.buffers.lock().await;
        if let Some(buf) = bufs.get(&session_id) {
            buf.write()
                .await
                .replace_all_with_session_messages(messages);
        } else {
            let buf = Arc::new(RwLock::new(CoreMessageBuffer::from_session_messages(
                messages,
            )));
            bufs.insert(session_id, buf);
        }
    }

    /// Build [`CoreContext`] + [`AgentLoopConfig`] and spawn the agent loop.
    ///
    /// Uses `session_start_lock` to prevent a TOCTOU race: only one task
    /// per session gets past the busy check and marks the session as busy.
    async fn start_agent_loop(&self, session_id: Uuid, mode: Mode) -> Result<()> {
        // ── Check-and-claim: atomic under session_start_lock ──────────
        {
            let _lock = self.session_start_lock.lock().unwrap();
            if self.is_session_busy(session_id) {
                // Another task already started a loop for this session.
                // The caller's message was already persisted; it will be
                // picked up by the running loop's next turn.
                return Ok(());
            }
            // Mark busy immediately so no other concurrent submit_prompt
            // or continue_session can also start a loop.
            self.busy_sessions.lock().unwrap().insert(session_id);
        }
        // lock released — remaining work is async but no longer racy.

        // Create a fresh cancellation token for this loop.
        let cancel = CancellationToken::new();
        self.active_loop_cancels
            .lock()
            .unwrap()
            .insert(session_id, cancel.clone());
        let context_manager = self.context_manager(session_id).await;
        let event_bus = self.event_bus(session_id).await;

        // Compose or load the system prompt (mode-agnostic — see inject_mode_reminder).
        let (system_prompt, session_start_hash) = {
            let session = self.session_manager.load_session(session_id)?;
            let ssh = session.as_ref().and_then(|s| s.snapshot_start_hash.clone());
            match session {
                Some(s) if !s.system_prompt.is_empty() => (s.system_prompt, ssh),
                _ => {
                    let sp = crate::agent_ctx::compose_system_prompt(
                        crate::agent_type::AgentType::General,
                        &self.workspace_root,
                        &self.skills,
                    );
                    // Persist system prompt to the session record.
                    self.session_manager.update_system_prompt(session_id, &sp)?;
                    (sp, ssh)
                }
            }
        };

        let active_model = self.active_model.read().unwrap().clone();
        let llm_config = crate::agent_ctx::to_llm_provider_config(&active_model);

        let filtered_tools = self.tool_registry.definitions_for_model(&active_model);
        // The buffer is shared with the spawned task (for persisting queued
        // prompts after a turn) and moved into the CoreContext.
        let buffer = self.message_buffer(session_id).await;
        let buffer_for_queued = buffer.clone();
        let session_manager = self.session_manager.clone();
        let ctx = crate::agent_ctx::CoreContext::new(
            self.llm.clone(),
            self.session_manager.clone(),
            self.tool_registry.clone(),
            context_manager,
            buffer,
            event_bus,
            self.approval_broker.clone(),
            session_id,
            mode,
            system_prompt.clone(),
            llm_config,
            cancel.clone(),
            filtered_tools,
            self.workspace_root.clone(),
            active_model.clone(),
            self.snapshot.clone(),
            self.config.clone(),
            self.auth.clone(),
            session_start_hash,
            self.paths.config_dir.clone(),
        );

        // Create the steering signal for this loop run. Steering messages
        // submitted while the loop is busy set it; run_agent_loop consumes
        // it at the end of a turn without tool calls so the loop keeps
        // running instead of exiting.
        let steer_signal = Arc::new(AtomicBool::new(false));
        self.steer_signals
            .lock()
            .unwrap()
            .insert(session_id, steer_signal.clone());
        self.session_outcomes
            .lock()
            .unwrap()
            .insert(session_id, None);
        let idle_notify = self
            .session_idle_notifies
            .lock()
            .unwrap()
            .entry(session_id)
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone();

        // Extract the per-session pending prompts for this loop run.
        // Steering entries are consumed as keep-alive signals by the loop;
        // queueing entries are persisted by the host after the loop exits.
        let per_session_queue: Arc<std::sync::Mutex<VecDeque<PendingPrompt>>> = {
            let mut qmap = self.pending_prompts.lock().unwrap();
            Arc::new(std::sync::Mutex::new(
                qmap.remove(&session_id).unwrap_or_default(),
            ))
        };

        let loop_config = tidev_agent::AgentLoopConfig {
            session_id,
            system_prompt,
            thinking_level: active_model.thinking_level.clone(),
            event_tx: ctx.event_tx(),
            cancel: cancel.clone(),
            steer_signal: steer_signal.clone(),
        };

        // Clone Arcs for the guard before they move into the spawn.
        let busy_sessions = self.busy_sessions.clone();
        let handles = self.run_loop_handles.clone();
        let cancels = self.active_loop_cancels.clone();
        let steer_signals = self.steer_signals.clone();
        let qmap_restore = self.pending_prompts.clone();
        let session_outcomes = self.session_outcomes.clone();

        let join = tokio::spawn(async move {
            let _guard = SessionLoopGuard {
                session_id,
                busy_sessions: busy_sessions.clone(),
                handles: handles.clone(),
                cancels: cancels.clone(),
                steer_signals: steer_signals.clone(),
                idle_notify,
            };
            let mut loop_error = None;
            // The outer loop keeps the session busy across turns: after
            // run_agent_loop exits (the model stopped responding), queued
            // prompts are persisted and the loop runs again for the next
            // turn. Steering messages never reach this point — they were
            // persisted at submission time and the loop consumed their
            // keep-alive signals.
            loop {
                if cancel.is_cancelled() {
                    break;
                }
                if let Err(e) = tidev_agent::run_agent_loop(&ctx, loop_config.clone()).await {
                    loop_error = Some(e.to_string());
                    log::error!("agent loop for session {session_id} exited with error: {e}");
                }
                // Drain prompts queued while the loop was running. Steering
                // entries were already consumed as keep-alive signals and
                // their messages are persisted — only queueing entries
                // remain to be persisted here.
                let queued: Vec<PendingPrompt> = per_session_queue
                    .lock()
                    .unwrap()
                    .drain(..)
                    .filter(|p| p.delivery == DeliveryMode::Queue)
                    .collect();
                if queued.is_empty() {
                    break;
                }
                // Persist queued prompts (buffer + store) so the next
                // loop iteration's load_messages() picks them up.
                let mut failed = false;
                for prompt in queued {
                    let mut msg = Message::new(MessageRole::User, prompt.content);
                    msg.id = prompt.message_id;
                    msg.attachments = prompt.attachments;
                    msg.thinking_level = prompt.thinking_level;
                    let app_data = MessageAppData {
                        mode: Some(prompt.mode.as_str().to_string()),
                        ..Default::default()
                    };
                    buffer_for_queued
                        .write()
                        .await
                        .append_with_app_data(msg.clone(), app_data.clone());
                    if let Err(e) = session_manager.append_messages_with_app_data(
                        session_id,
                        std::slice::from_ref(&msg),
                        &[(msg.id, app_data)].into_iter().collect(),
                    ) {
                        log::error!(
                            "failed to persist queued prompt for session {session_id}: {e}"
                        );
                        failed = true;
                        break;
                    }
                }
                if failed {
                    break;
                }
                // Loop again — the next run_agent_loop loads the persisted
                // prompts and starts the next turn.
            }
            // On exit, restore any remaining pending prompts back to the
            // per-session map so they aren't lost (e.g. on cancellation or
            // a persistence failure).
            let remaining: Vec<PendingPrompt> =
                per_session_queue.lock().unwrap().drain(..).collect();
            if !remaining.is_empty() {
                let mut map = qmap_restore.lock().unwrap();
                let q = map.entry(session_id).or_default();
                for prompt in remaining {
                    q.push_back(prompt);
                }
            }
            session_outcomes
                .lock()
                .unwrap()
                .insert(session_id, loop_error);
        });

        // Store handle, then mark session busy.
        self.run_loop_handles
            .lock()
            .unwrap()
            .insert(session_id, join);
        self.busy_sessions.lock().unwrap().insert(session_id);

        Ok(())
    }

    /// Cancel the current operation.
    ///
    /// Signals cooperative cancellation for all sessions via their cancellation
    /// tokens. Agent loops detect this at the next checkpoint and shut down
    /// gracefully, allowing in-flight tools to persist their results and emit
    /// final events before the loop exits.
    ///
    /// The `run_loop_handles` entries are dropped (not aborted), so the spawned
    /// tasks remain alive long enough to complete their cleanup. The RAII
    /// guard in `execute_tools` ensures `ToolCompleted` events are sent even
    /// if the task is unexpectedly dropped during shutdown.
    pub async fn cancel(&self) {
        // 1. Signal cooperative cancellation for ALL sessions.
        let tokens: Vec<CancellationToken> = self
            .active_loop_cancels
            .lock()
            .unwrap()
            .drain()
            .map(|(_, t)| t)
            .collect();
        for token in tokens {
            token.cancel();
        }

        self.busy_sessions.lock().unwrap().clear();
        // Queued (non-steered) prompts are abandoned on cancellation —
        // steering messages were already persisted and cannot be retracted.
        self.pending_prompts.lock().unwrap().clear();

        // 2. Drop handles without aborting — the loops will exit naturally
        //    after detecting the cancellation token.
        self.run_loop_handles.lock().unwrap().clear();

        // 3. Force-kill any lingering child processes whose session's loop
        //    may have exited without cleaning them up.
        tidev_tools::kill_all_children();
    }

    /// Cancel a specific session's agent loop.
    ///
    /// Like [`cancel`] but only affects the given session. Other sessions'
    /// loops continue running undisturbed.
    pub async fn cancel_session(&self, session_id: Uuid) {
        // 1. Signal cooperative cancellation for this session.
        if let Some(token) = self.active_loop_cancels.lock().unwrap().remove(&session_id) {
            token.cancel();
        }

        self.busy_sessions.lock().unwrap().remove(&session_id);
        // Queued (non-steered) prompts for this session are abandoned on
        // cancellation — steering messages were already persisted and
        // cannot be retracted.
        self.pending_prompts.lock().unwrap().remove(&session_id);
        self.steer_signals.lock().unwrap().remove(&session_id);

        // 2. Drop the handle without aborting — the loop will detect the
        //    cancellation token and shut down gracefully, letting in-flight
        //    tools persist results and emit final events.
        self.run_loop_handles.lock().unwrap().remove(&session_id);
        // Note: child processes for THIS session are handled by the
        // CancellationToken chain inside the shell tool. We don't call
        // kill_all_children() here because that would kill other sessions'
        // processes too.
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
        let messages = buf.read().await.session_messages();
        drop(buf);

        // Determine target: previous user message (or one more back if in revert state).
        let target_id = match self.session_manager.load_revert_state(session_id)? {
            Some((current, _)) if current != Uuid::nil() => {
                crate::undo::prev_user_message_before(&messages, current)
            }
            _ => crate::undo::last_visible_user_message(&messages),
        };
        let Some(target_id) = target_id else {
            return Ok(());
        };

        self.revert_to_message(session_id, &messages, target_id)
            .await?;
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
        let messages = buf.read().await.session_messages();
        drop(buf);

        // Is there a next user message to move forward to?
        if let Some(next_id) = crate::undo::next_user_message_after(&messages, current_id) {
            // Move the undo point FORWARD to the next message.
            self.revert_to_message(session_id, &messages, next_id)
                .await?;
        } else {
            // At the end of history — restore the original pre-undo state.
            self.unrevert(session_id, redo_snapshot.as_deref().unwrap_or_default())
                .await?;
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
        stream_request_id: Option<u64>,
    ) -> Result<()> {
        use crate::agent_ctx::to_llm_provider_config;

        // 1. Collect the inputs: messages, context manager, model config, tools.
        let mut messages = {
            let buf = self.message_buffer(session_id).await;
            buf.read().await.load().to_vec()
        };
        crate::agent_ctx::restore_full_tool_output_semantics(&mut messages);
        let cm = self.context_manager(session_id).await;
        let model_config = {
            let active = self.active_model.read().unwrap();
            to_llm_provider_config(&active)
        };
        // Match the session's system prompt used by normal requests.
        let compact_model = {
            let session = self.session_manager.load_session(session_id)?;
            let mut m = model_config;
            if let Some(s) = session
                && !s.system_prompt.is_empty()
            {
                m.system_prompt = Some(s.system_prompt);
            }
            m
        };
        let active_model = self.active_model.read().unwrap().clone();
        let tools: Vec<tidev_llm::ToolDefinition> = self
            .tool_registry
            .definitions_for_model(&active_model)
            .iter()
            .map(to_llm_tool_def)
            .collect();

        // 2. Run compaction (async, no locks held on ContextManager).
        //    Capture prior compaction state before it gets overwritten.
        let event_bus = self.event_bus(session_id).await;
        let (result, prior_summary, prior_retained_from) = {
            let cm_lock = cm.lock().await;
            let prior_summary = cm_lock.summary.clone();
            let prior_retained_from = cm_lock.retained_from;
            let result = cm_lock
                .compact(
                    &self.llm,
                    &compact_model,
                    &tools,
                    &messages,
                    session_id,
                    Some(event_bus.agent_sender()),
                )
                .await
                .inspect_err(|e| {
                    let _ = event_bus.send_backend(BackendEvent::ContextCompacted {
                        session_id,
                        compacted: false,
                        manual: stream_request_id.is_some(),
                        summary: None,
                        retained_from: 0,
                        model_id: None,
                        completed_at: Some(Utc::now()),
                        error: Some(e.to_string()),
                    });
                })?;
            (result, prior_summary, prior_retained_from)
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

        // 5. Append a compaction marker message for undo support.
        //     Stores the prior state so revert_to_message can restore it.
        {
            let mut marker = Message::compaction(&result.summary);
            marker.metadata.prior_summary = prior_summary;
            marker.metadata.prior_retained_from = Some(prior_retained_from);
            let buf = self.message_buffer(session_id).await;
            buf.write().await.append(marker.clone());
            self.session_manager.append_message(session_id, &marker)?;
        }

        // 6. Notify the TUI (BackendEvent::ContextCompacted is already sent by
        //    compact() via event_tx when streaming, but for consistency we
        //    always send the final event here as well).
        let model_id = active_model.model_id.clone();
        let _ = event_bus.send_backend(BackendEvent::ContextCompacted {
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
        messages: &[crate::SessionMessage],
        target_id: Uuid,
    ) -> Result<()> {
        // 1. Reuse existing redo_snapshot if one exists (maintains undo chain),
        //    otherwise capture current workspace as the redo point.
        let redo_hash: Option<Vec<u8>> = match self.session_manager.load_revert_state(session_id)? {
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
        if !patches.is_empty()
            && let Some(ref snap) = self.snapshot
        {
            snap.revert(&patches).await?;
        }

        // 3. Adjust context compaction state.
        let cm = self.context_manager(session_id).await;
        let mut cm_lock = cm.lock().await;
        let mut summary = cm_lock.summary.clone();
        let mut retained_from = cm_lock.retained_from;

        // Determine whether the target lies within the compacted range
        // (index < retained_from) or the visible range (index >= retained_from).
        let target_idx = messages.iter().position(|m| m.id == target_id);
        let target_in_compacted = target_idx.map(|i| i < retained_from).unwrap_or(false);

        if target_in_compacted {
            // Target was covered by a previous compaction — restore the context
            // state that was active when the target was created by walking forward
            // to the first compaction marker after it.
            if !crate::undo::restore_context_from_compaction(
                messages,
                target_id,
                &mut summary,
                &mut retained_from,
            ) {
                // No compaction marker found — target predates any compaction.
                summary = None;
                retained_from = 0;
            }
        }
        // Otherwise the target is in the visible (uncompacted) range and the
        // current compaction state was already active when the target was
        // created — keep it unchanged.

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
        let _ = self
            .event_bus(session_id)
            .await
            .send_backend(BackendEvent::ContextCompacted {
                session_id,
                compacted: true,
                manual: false,
                summary: None,
                retained_from: 0,
                model_id: None,
                completed_at: None,
                error: None,
            });

        let message_content = messages
            .iter()
            .find(|m| m.id == target_id)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        let _ = self
            .event_bus(session_id)
            .await
            .send_backend(BackendEvent::UndoCompleted {
                session_id,
                target_id,
                message_content,
            });

        Ok(())
    }

    /// Full unrevert: restore the pre-undo workspace snapshot and clear state.
    async fn unrevert(&self, session_id: Uuid, redo_snapshot: &[u8]) -> Result<()> {
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

        let _ = self
            .event_bus(session_id)
            .await
            .send_backend(BackendEvent::ContextCompacted {
                session_id,
                compacted: true,
                manual: false,
                summary: None,
                retained_from: 0,
                model_id: None,
                completed_at: None,
                error: None,
            });

        let _ = self
            .event_bus(session_id)
            .await
            .send_backend(BackendEvent::UndoCompleted {
                session_id,
                target_id: Uuid::nil(),
                message_content: String::new(),
            });

        Ok(())
    }
}

fn prompt_message_matches_submission(message: &Message, submission: &PromptSubmission) -> bool {
    let steering_content = format!(
        "{}\n\n{}",
        submission.content,
        crate::prompts::steer_reminder()
    );
    message.role == MessageRole::User
        && (message.content == submission.content || message.content == steering_content)
        && message.attachments == submission.attachments
        && message.thinking_level == submission.thinking_level
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
    fn resolve_fallback_model(config: &AppConfig, auth: &AuthStore) -> Result<ActiveModel> {
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
        log::info!(
            "Runtime initialising, workspace={}",
            workspace_root.display()
        );
        log::info!("startup: config + auth loaded in {:?}", _start.elapsed());

        // 4. Shell detection (must happen before any tool execution).
        tidev_tools::shell::init(
            config.shell.windows_shell.clone(),
            config.shell.unix_shell.clone(),
        );

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
        if let Ok(count) = store.delete_expired_tool_outputs(7)
            && count > 0
        {
            log::info!("Cleaned up {count} old tool output(s)");
        }

        // 7. LLM client + model resolution (with fallback).
        let _t_llm = Instant::now();
        let mut active_model = Self::resolve_fallback_model(&config, &auth)
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

        // 9b. MCP manager.
        let _t_mcp = Instant::now();
        let mcp_manager = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        log::info!("startup: MCP manager ready in {:?}", _t_mcp.elapsed());

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
            mcp_manager,
        ));
        log::info!("startup: tool registry ready in {:?}", _t_tools.elapsed());

        // 10b. Best-effort MCP server refresh (don't block startup on failures).
        let _t_mcp_refresh = Instant::now();
        if !config.mcp.is_empty() {
            if let Err(e) = tool_registry.mcp_manager().refresh_all().await {
                log::warn!("MCP server refresh (best-effort): {e:#}");
            }
            log::info!(
                "startup: MCP refresh done in {:?}",
                _t_mcp_refresh.elapsed()
            );
        }

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

        // Load saved thinking level preference from database (if any).
        if let Ok(Some(level_str)) =
            store.load_model_thinking_level(&active_model.provider_id, &active_model.model_id)
        {
            active_model.thinking_level = ThinkingLevelType::from_string(&level_str);
        }

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
                            if let Ok(count) = cstore.delete_expired_tool_outputs(7)
                                && count > 0
                            {
                                log::info!("Cleaned up {count} old tool output(s)");
                            }
                        }
                        _ = cancel.cancelled() => break,
                    }
                }
            });
        }

        // 13b. Start background snapshot GC (hourly, with initial 60s delay).
        if let Some(ref svc) = snapshot {
            let cancel = cleanup_cancel.clone();
            let svc = svc.clone();
            tokio::spawn(async move {
                // Wait a bit before first GC so startup isn't slowed down.
                tokio::time::sleep(Duration::from_secs(60)).await;
                loop {
                    if let Err(e) = svc.cleanup().await {
                        log::warn!("snapshot cleanup failed: {e}");
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(3600)) => {},
                        _ = cancel.cancelled() => break,
                    }
                }
            });
        }

        // 14. Channels. The runtime owns the primary (unbounded) channels;
        //     a fan-out task per channel forwards every message to all
        //     registered subscribers so multiple frontends (TUI, ACP, web
        //     server, ...) can consume the same stream concurrently and
        //     subscribe/unsubscribe at any time.
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel::<BackendEvent>();
        let event_hub = EventHub::new();
        {
            let event_hub = event_hub.clone();
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    event_hub.publish(event).await;
                }
            });
        }

        let (request_tx, mut request_rx) =
            tokio::sync::mpsc::unbounded_channel::<FrontendRequest>();
        let approval_broker = ApprovalBroker::new(request_tx);
        let request_subscribers: Arc<Mutex<Vec<UnboundedSender<FrontendRequest>>>> =
            Arc::new(Mutex::new(Vec::new()));
        {
            let subscribers = request_subscribers.clone();
            tokio::spawn(async move {
                while let Some(request) = request_rx.recv().await {
                    let mut subs = subscribers.lock().await;
                    subs.retain(|tx| tx.send(request.clone()).is_ok());
                }
            });
        }

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
            active_loop_cancels: Arc::new(std::sync::Mutex::new(HashMap::new())),
            snapshot,
            git: crate::git::GitService::new(workspace_root.clone()),
            buffers: Arc::new(Mutex::new(HashMap::new())),
            context_managers: Arc::new(Mutex::new(HashMap::new())),
            event_tx,
            event_buses: Arc::new(Mutex::new(HashMap::new())),
            event_hub,
            approval_broker,
            request_subscribers,
            run_loop_handles: Arc::new(std::sync::Mutex::new(HashMap::new())),
            busy_sessions: Arc::new(std::sync::Mutex::new(HashSet::new())),
            pending_prompts: Arc::new(std::sync::Mutex::new(HashMap::new())),
            steer_signals: Arc::new(std::sync::Mutex::new(HashMap::new())),
            session_idle_notifies: Arc::new(StdMutex::new(HashMap::new())),
            session_outcomes: Arc::new(StdMutex::new(HashMap::new())),
            session_start_lock: Arc::new(std::sync::Mutex::new(())),
            prompt_submission_lock: Arc::new(Mutex::new(())),
            cleanup_cancel,
            workspace_root,
            file_search_index: OnceLock::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_config::SendWhileBusy;

    /// Build a runtime in a fresh temp directory using the bundled
    /// deepseek provider preset (no API key required for metadata).
    async fn make_test_runtime() -> Runtime {
        let dir = std::env::temp_dir().join(format!("tidev-runtime-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let config_toml =
            "default_provider = \"deepseek\"\ndefault_model = \"deepseek-v4-flash\"\n";
        std::fs::write(dir.join("config.toml"), config_toml).expect("config should be written");
        Runtime::builder()
            .workspace_root(dir.clone())
            .config_dir(dir.clone())
            .data_dir(dir.clone())
            .build()
            .await
            .expect("runtime should build")
    }

    async fn recv_created_event(
        rx: &mut tokio::sync::mpsc::UnboundedReceiver<BackendEvent>,
    ) -> BackendEvent {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("user message event should arrive")
            .expect("channel should stay open")
    }

    #[tokio::test]
    async fn busy_queue_holds_prompt_without_persisting() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("queue test").unwrap();
        let mut events = rt.event_rx().await;

        // Simulate a running loop for this session.
        rt.busy_sessions.lock().unwrap().insert(sid);
        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Queue;

        rt.submit_prompt_with_attachments(
            sid,
            Mode::Build,
            "queued message".into(),
            Vec::new(),
            None,
        )
        .await
        .expect("submit should succeed");

        // The message must NOT be persisted — it waits for the turn to end.
        let buf = rt.message_buffer(sid).await;
        let messages = buf.read().await.load().to_vec();
        assert!(
            messages.is_empty(),
            "queued prompt must not enter the buffer"
        );

        // It must be held in the pending queue with delivery=Queue.
        let pending = rt.pending_prompts.lock().unwrap().get(&sid).cloned();
        let pending = pending.expect("queued prompt should be registered");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].delivery, DeliveryMode::Queue);
        assert_eq!(pending[0].content, "queued message");

        // The frontend must be told it is queued.
        match recv_created_event(&mut events).await {
            BackendEvent::UserMessageCreated { queued, .. } => assert!(queued),
            other => panic!("expected UserMessageCreated, got {other:?}"),
        }

        // Cancellation abandons queued prompts.
        rt.cancel_session(sid).await;
        assert!(rt.pending_prompts.lock().unwrap().get(&sid).is_none());
    }

    #[tokio::test]
    async fn busy_steer_persists_with_reminder_and_sets_signal() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("steer test").unwrap();
        let mut events = rt.event_rx().await;

        // Simulate a running loop that holds the steering signal.
        rt.busy_sessions.lock().unwrap().insert(sid);
        let signal = Arc::new(AtomicBool::new(false));
        rt.steer_signals.lock().unwrap().insert(sid, signal.clone());
        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Steer;

        rt.submit_prompt_with_attachments(
            sid,
            Mode::Build,
            "steer message".into(),
            Vec::new(),
            None,
        )
        .await
        .expect("submit should succeed");

        // The message must be persisted immediately, with a
        // system-reminder suffix appended.
        let buf = rt.message_buffer(sid).await;
        let messages = buf.read().await.load().to_vec();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.starts_with("steer message"));
        assert!(
            messages[0].content.contains("<system-reminder>"),
            "steering message must carry a system-reminder: {}",
            messages[0].content
        );

        // The keep-alive signal must be set for the running loop.
        assert!(signal.load(Ordering::SeqCst));

        // The frontend must be told the message is pending (queued=true).
        match recv_created_event(&mut events).await {
            BackendEvent::UserMessageCreated { queued, .. } => assert!(queued),
            other => panic!("expected UserMessageCreated, got {other:?}"),
        }

        rt.cancel_session(sid).await;
    }

    #[tokio::test]
    async fn idle_submit_persists_and_starts_loop() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("idle test").unwrap();
        let mut events = rt.event_rx().await;

        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Steer;

        rt.submit_prompt_with_attachments(sid, Mode::Build, "fresh turn".into(), Vec::new(), None)
            .await
            .expect("submit should succeed");

        // Idle submission persists without a reminder and starts a loop.
        let buf = rt.message_buffer(sid).await;
        let messages = buf.read().await.load().to_vec();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "fresh turn");
        assert!(!messages[0].content.contains("<system-reminder>"));
        assert!(rt.is_session_busy(sid), "loop should be running");

        match recv_created_event(&mut events).await {
            BackendEvent::UserMessageCreated { queued, .. } => assert!(!queued),
            other => panic!("expected UserMessageCreated, got {other:?}"),
        }

        rt.cancel_session(sid).await;
        // Cancellation must not leave stale steering signals behind.
        assert!(rt.steer_signals.lock().unwrap().get(&sid).is_none());
    }

    #[tokio::test]
    async fn busy_steer_without_signal_falls_back_to_next_submission() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("signal-less steer").unwrap();

        // Busy, but no loop holds a signal (e.g. the loop exited just
        // before the submission). The entry is still registered so the
        // next start_agent_loop drains it as a keep-alive signal.
        rt.busy_sessions.lock().unwrap().insert(sid);
        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Steer;

        rt.submit_prompt_with_attachments(sid, Mode::Build, "late steer".into(), Vec::new(), None)
            .await
            .expect("submit should succeed");

        let pending = rt.pending_prompts.lock().unwrap().get(&sid).cloned();
        let pending = pending.expect("steer entry should be registered");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].delivery, DeliveryMode::Steer);

        rt.cancel_session(sid).await;
    }

    #[tokio::test]
    async fn prompt_submission_is_idempotent_while_queued() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("idempotent queue").unwrap();
        rt.busy_sessions.lock().unwrap().insert(sid);
        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Queue;

        let submission = PromptSubmission {
            message_id: Uuid::new_v4(),
            content: "keep this exact prompt".into(),
            mode: Mode::Build,
            attachments: Vec::new(),
            thinking_level: None,
        };
        let first = rt
            .submit_prompt_submission(sid, submission.clone())
            .await
            .expect("first submit should succeed");
        let retry = rt
            .submit_prompt_submission(sid, submission.clone())
            .await
            .expect("retry should succeed");

        assert!(!first.duplicate);
        assert!(retry.duplicate);
        assert_eq!(first.message_id, submission.message_id);
        let pending = rt
            .pending_prompts
            .lock()
            .unwrap()
            .get(&sid)
            .cloned()
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].message_id, submission.message_id);

        rt.cancel_session(sid).await;
    }

    #[tokio::test]
    async fn prompt_submission_rejects_conflicting_message_id() {
        let rt = make_test_runtime().await;
        let sid = rt.create_default_session("idempotent conflict").unwrap();
        rt.busy_sessions.lock().unwrap().insert(sid);
        rt.config.write().unwrap().ui.send_while_busy = SendWhileBusy::Queue;

        let message_id = Uuid::new_v4();
        rt.submit_prompt_submission(
            sid,
            PromptSubmission {
                message_id,
                content: "first content".into(),
                mode: Mode::Build,
                attachments: Vec::new(),
                thinking_level: None,
            },
        )
        .await
        .expect("first submit should succeed");

        let error = rt
            .submit_prompt_submission(
                sid,
                PromptSubmission {
                    message_id,
                    content: "different content".into(),
                    mode: Mode::Build,
                    attachments: Vec::new(),
                    thinking_level: None,
                },
            )
            .await
            .expect_err("conflicting reuse must fail");
        assert!(error.to_string().contains("different content"));

        rt.cancel_session(sid).await;
    }
}
