//! Core agent context — implements the [`AgentContext`] trait for tidev.
//!
//! This module ties together all the components built in the other modules:
//! LLM calls, tool execution, message persistence, context compaction, and
//! permission approvals.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_agent::{
    ContextManager, AgentContext, AgentEvent, AgentLoopConfig, llm_event_to_agent_event,
};
use tidev_config::auth::ActiveModel;
use tidev_config::{AppConfig, AuthStore};
use crate::agent_type::{AgentDefinition, AgentType};
use crate::backend_event::{
    agent_event_channel, agent_event_to_backend_event, BackendEvent,
};
use tidev_llm::message::{AssistantTurn, Message, MessageRole, ToolCall, ToolExecutionResult};
use tidev_llm::reasoning::ThinkingLevelType;
use tidev_tools::types::ToolDefinition;
use tidev_tools::ShellOutput;
use tidev_utils::path::{
    extract_boundary_violation_path, extract_sensitive_file_path, load_sensitive_patterns,
};

use tidev_llm::{LlmClient, LlmProviderConfig};
use tidev_snapshot::SnapshotService;
use tidev_storage::MessageAppData;

use crate::mode::Mode;
use crate::approval::{ApprovedTool, ToolCallWithViolations, TuiRequest, TuiRequestKind, TuiResponse};
use crate::message_buf::CoreMessageBuffer;
use crate::registry::ToolRegistry;
use crate::session::SessionManager;
use crate::tool_def::to_llm_tool_def;

// ---------------------------------------------------------------------------
// System prompt composition
// ---------------------------------------------------------------------------

/// Compose the complete system prompt for a session.
///
/// Assembled once at session creation and stored in `AgentLoopConfig.system_prompt`.
/// Includes: base agent prompt + environment info.
/// Instruction files (AGENTS.md etc.) are injected into user messages via
/// `<system-reminder>` tags instead (see `inject_instructions`).
/// Mode reminders are injected into user messages instead (see `inject_mode_reminder`).
pub fn compose_system_prompt(
    agent_type: crate::agent_type::AgentType,
    workspace_root: &std::path::Path,
) -> String {
    let base_prompt = crate::agent_type::system_prompt(agent_type);

    // Environment info (detected once, frozen for the session lifetime).
    let system_info = crate::system_info::SystemInfo::detect();
    let working_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let is_git = crate::system_info::is_git_repo(workspace_root);

    let env_block = format!(
        "\n\nHere is some useful information about the environment:\n\
         <env>\n  \
         Working directory: {}\n  \
         Workspace root folder: {}\n  \
         Is directory a git repo: {}\n  \
         {}\n  \
         Shell: {}\n  \
         Shell path: {}\n\
         </env>",
        working_dir,
        workspace_root.display(),
        if is_git { "yes" } else { "no" },
        system_info.format_env(),
        tidev_tools::shell::get().display_name,
        tidev_tools::shell::get().program,
    );

    let mut prompt = base_prompt;
    prompt.push_str(&env_block);

    prompt
}

// ---------------------------------------------------------------------------
// Tools that are safe to run in parallel.
// ---------------------------------------------------------------------------

/// Tool names that can be read-only and thus run concurrently.
fn read_only_tool_names() -> HashSet<&'static str> {
    ["read", "glob", "grep", "websearch", "webfetch"]
        .into_iter()
        .collect()
}

/// Whether a tool call is read-only (and may thus run in parallel with others).
fn is_read_only(name: &str) -> bool {
    read_only_tool_names().contains(tidev_utils::tool_name::canonical_tool_name(name).unwrap_or(name))
}

/// Recover a subagent association from the assistant tool call that produced
/// a result. The association is application data and must never enter the
/// protocol-level tool result.
fn child_session_id_for_tool_call(
    buffer: &CoreMessageBuffer,
    tool_call_id: &str,
) -> Option<Uuid> {
    buffer
        .load()
        .iter()
        .rev()
        .find(|message| {
            message.role == MessageRole::Assistant
                && message
                    .tool_calls
                    .iter()
                    .any(|tool_call| tool_call.id == tool_call_id)
        })
        .and_then(|message| buffer.app_data(message.id))
        .and_then(|data| data.child_session_id)
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// Build an [`LlmProviderConfig`] from a resolved [`ActiveModel`].
pub fn to_llm_provider_config(model: &ActiveModel) -> LlmProviderConfig {
    LlmProviderConfig {
        provider_id: model.provider_id.clone(),
        api_type: model.api_type,
        api_key: model.api_key.clone(),
        base_url: model.base_url.clone(),
        model_id: model.model_id.clone(),
        request_model_id: Some(model.request_model_id.clone()),
        system_prompt: Some(model.system_prompt.clone()),
        thinking_level: model.thinking_level.clone(),
        extra_body: model.extra_body.clone(),
        max_output_tokens: model.max_output_tokens,
        context_window: model.context_window,
        temperature: model.temperature,
        supports_images: model.supports_images,
        supports_parallel_tool_calls: model.supports_parallel_tool_calls,
    }
}

// ---------------------------------------------------------------------------
// CancelPersistGuard
// ---------------------------------------------------------------------------

/// Ensures "User cancelled" tool results are persisted even when the agent
/// loop task is force-aborted via [`JoinHandle::abort`].
///
/// When the task is aborted, all local variables are destroyed by running
/// their destructors. This guard's [`Drop`] implementation persists synthetic
/// cancellation results to the database and in-memory buffer, guaranteeing
/// the parent agent sees the cancellation signal on the next turn.
struct CancelPersistGuard {
    session_manager: SessionManager,
    buffer: Arc<RwLock<CoreMessageBuffer>>,
    session_id: Uuid,
    /// The tool calls that were pending when the guard was created.
    tool_calls: Vec<ToolCall>,
    /// When `true`, [`Drop`] skips persistence (normal path handled it).
    disarmed: bool,
}

impl Drop for CancelPersistGuard {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }

        // Task was aborted — persist "User cancelled" for every task tool
        // call, retaining any child-session association established before
        // the child was aborted.
        let child_session_ids: HashMap<String, Option<Uuid>> = self
            .buffer
            .try_read()
            .ok()
            .map(|buffer| {
                self.tool_calls
                    .iter()
                    .map(|tc| {
                        let child_session_id = buffer
                            .load()
                            .iter()
                            .rev()
                            .find(|message| {
                                message.role == MessageRole::Assistant
                                    && message
                                        .tool_calls
                                        .iter()
                                        .any(|tool_call| tool_call.id == tc.id)
                            })
                            .and_then(|message| buffer.app_data(message.id))
                            .and_then(|data| data.child_session_id);
                        (tc.id.clone(), child_session_id)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut messages = Vec::with_capacity(self.tool_calls.len());
        let mut app_data = HashMap::with_capacity(self.tool_calls.len());
        for tc in &self.tool_calls {
            let result = ToolExecutionResult::new("User cancelled the request");
            let msg = Message::tool_result(&tc.id, &tc.name, result);
            app_data.insert(
                msg.id,
                MessageAppData {
                    child_session_id: child_session_ids.get(&tc.id).copied().flatten(),
                    ..MessageAppData::default()
                },
            );
            messages.push(msg);
        }

        let _ = self.session_manager.append_messages_with_app_data(
            self.session_id,
            &messages,
            &app_data,
        );
        if let Ok(mut buf) = self.buffer.try_write() {
            for msg in messages {
                let data = app_data.get(&msg.id).cloned().unwrap_or_default();
                buf.append_with_app_data(msg, data);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// CoreContext
// ---------------------------------------------------------------------------

/// The concrete implementation of [`AgentContext`] for tidev.
pub struct CoreContext {
    /// LLM client for streaming chat turns.
    llm: LlmClient,
    /// Session persistence (SQLite).
    session_manager: SessionManager,
    /// Tool registry for execution.
    tool_registry: Arc<ToolRegistry>,
    /// Context compaction state.
    context_manager: Arc<Mutex<ContextManager>>,
    /// Per-session message buffer (the in-memory cache / single source of truth).
    buffer: Arc<RwLock<CoreMessageBuffer>>,
    /// Channel for sending events to the UI.
    event_tx: UnboundedSender<BackendEvent>,
    /// Channel for sending UI requests (tool approval etc.).
    request_tx: UnboundedSender<TuiRequest>,
    /// This loop's session ID.
    session_id: Uuid,
    /// Current session mode.
    mode: Mode,
    /// Pre-composed system prompt (session-scoped, immutable after creation).
    system_prompt: String,
    /// Resolved model config for the LLM call.
    model_config: LlmProviderConfig,
    /// Co-operative cancellation token.
    cancel: CancellationToken,
    /// Tools available to this session.
    tools: Vec<ToolDefinition>,
    /// Workspace root (used for tool execution and child contexts).
    workspace_root: PathBuf,
    /// Resolved active model (for subagents / provider info).
    active_model: ActiveModel,
    /// Workspace snapshot service for undo/redo.
    snapshot: Option<SnapshotService>,
    /// Pre-round snapshot hash, captured when the assistant message with
    /// tool calls is saved. Consumed when tool results are saved, to compute
    /// the diff of files changed in this round.
    pre_round_hash: Arc<Mutex<Option<String>>>,
    /// Session-start snapshot hash, captured on the very first snapshot
    /// of the session. Never updated afterwards. Used to compute cumulative
    /// sidebar diffs that match `git diff` from session start.
    session_start_hash: Arc<Mutex<Option<String>>>,
    /// Application config (shared, hot-reloadable).
    config: Arc<StdRwLock<AppConfig>>,
    /// Auth store (shared, hot-reloadable).
    auth: Arc<StdRwLock<AuthStore>>,
    /// Cached instruction file contents to avoid redundant I/O.
    /// Key: canonical path, Value: file content.
    instruction_content_cache: Arc<Mutex<HashMap<String, String>>>,
    /// Config directory path (for instruction file lookup).
    config_dir: PathBuf,
    /// Instruction sources discovered during tool execution. They are
    /// persisted immediately, while the replay notification is appended after
    /// the corresponding tool results to preserve message order.
    pending_instruction_sources: Arc<Mutex<Vec<String>>>,
}

impl CoreContext {
    /// Create a new CoreContext with all required resources.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        llm: LlmClient,
        session_manager: SessionManager,
        tool_registry: Arc<ToolRegistry>,
        context_manager: Arc<Mutex<ContextManager>>,
        buffer: Arc<RwLock<CoreMessageBuffer>>,
        event_tx: UnboundedSender<BackendEvent>,
        request_tx: UnboundedSender<TuiRequest>,
        session_id: Uuid,
        mode: Mode,
        system_prompt: String,
        model_config: LlmProviderConfig,
        cancel: CancellationToken,
        tools: Vec<ToolDefinition>,
        workspace_root: PathBuf,
        active_model: ActiveModel,
        snapshot: Option<SnapshotService>,
        config: Arc<StdRwLock<AppConfig>>,
        auth: Arc<StdRwLock<AuthStore>>,
        session_start_hash: Option<String>,
        config_dir: PathBuf,
    ) -> Self {
        Self {
            llm,
            session_manager,
            tool_registry,
            context_manager,
            buffer,
            event_tx,
            request_tx,
            session_id,
            mode,
            system_prompt,
            model_config,
            cancel,
            tools,
            workspace_root,
            active_model,
            snapshot,
            pre_round_hash: Arc::new(Mutex::new(None)),
            session_start_hash: Arc::new(Mutex::new(session_start_hash)),
            config,
            auth,
            instruction_content_cache: Arc::new(Mutex::new(HashMap::new())),
            config_dir,
            pending_instruction_sources: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Helper: emit an event; logging the error is sufficient (UI may have gone away).
    fn emit(&self, event: BackendEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Inject new instruction files into the last user message.
    ///
    /// Loads all workspace-root instruction files (AGENTS.md, CLAUDE.md, etc.),
    /// compares against already-injected sources in the DB, and prepends
    /// `<system-reminder>` blocks for new sources to the last user message.
    ///
    /// Called once per agent loop turn, before `stream_turn`.  After the first
    /// turn all workspace-root sources are injected, so subsequent turns are
    /// no-ops (unless new sources are discovered via tool results).
    pub(crate) async fn inject_instructions_impl(
        &self,
        session_id: Uuid,
        messages: &mut [Message],
    ) -> Result<Vec<String>> {
        // 1. Load all instruction sources (with content cache).
        let instructions = self.config.read().unwrap().instructions.clone();
        let mut cache = self.instruction_content_cache.lock().await;
        let (_, all_sources, new_cache) = tidev_instructions::system_prompt_and_sources_with_cache(
            &self.workspace_root,
            &self.config_dir,
            &instructions,
            &cache,
        )
        .unwrap_or_default();
        *cache = new_cache;
        drop(cache);

        // 2. Load already-injected sources from DB.
        let already_injected = self
            .session_manager
            .store()
            .load_instruction_sources(session_id)?;

        if all_sources.is_empty() {
            return Ok(already_injected);
        }

        // 3. Find the last user message.
        let last_user_idx = match messages.iter().rposition(|m| m.role == MessageRole::User) {
            Some(idx) => idx,
            None => return Ok(already_injected),
        };

        // 4. Find new sources (paths from system_prompt_and_sources_with_cache
        //    are already canonical — see system_paths + canonicalize_display).
        let new_sources: Vec<&String> = all_sources
            .iter()
            .filter(|s| !already_injected.contains(s))
            .collect();

        if new_sources.is_empty() {
            return Ok(already_injected);
        }

        // 5. Build <system-reminder> block from new sources.
        let cache = self.instruction_content_cache.lock().await;
        let mut sections: Vec<String> = Vec::new();
        for source in &new_sources {
            if let Some(content) = cache.get(*source) {
                sections.push(format!("Instructions from: {}\n{}", source, content));
            }
        }
        drop(cache);

        if sections.is_empty() {
            return Ok(already_injected);
        }

        let injection = format!(
            "<system-reminder>\n{}\n</system-reminder>",
            sections.join("\n\n"),
        );

        // 6. Safety check: avoid double injection if <system-reminder> already
        //    present (should never happen given DB tracking, but be defensive).
        if messages[last_user_idx]
            .content
            .contains("<system-reminder>")
        {
            return Ok(already_injected);
        }

        // 7. Prepend injection to the last user message (same pattern as
        //    inject_mode_reminder in loop_.rs).
        let new_content = format!("{}\n\n{}", injection, messages[last_user_idx].content);
        let msg_id = messages[last_user_idx].id;
        messages[last_user_idx].content = new_content.clone();

        // Persist to store + buffer via the existing dual-write method.
        self.update_message_content(session_id, msg_id, new_content)
            .await?;

        // 8. Persist new sources to DB so subsequent turns don't re-inject.
        // Merge with already-injected sources (save_instruction_sources replaces ALL).
        let mut updated = already_injected;
        updated.extend(new_sources.iter().map(|s| (*s).clone()));
        self.session_manager
            .store()
            .save_instruction_sources(session_id, &updated)?;

        // 9. Notify frontend.
        let string_sources: Vec<String> = new_sources.iter().map(|s| (*s).clone()).collect();
        self.emit(BackendEvent::InstructionsLoaded {
            session_id,
            sources: string_sources,
        });

        log::info!(
            "injected {} new instruction file(s) into user message {}",
            new_sources.len(),
            msg_id,
        );

        // 10. Persist "Loaded instructions from" notification for
        //     cross-session replay (only the first time each source
        //     is injected).
        let display_paths: Vec<String> = new_sources
            .iter()
            .map(|s| {
                let path = std::path::Path::new(s);
                path.strip_prefix(&self.workspace_root)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .collect();
        let display_content = if display_paths.len() == 1 {
            format!("Loaded instructions from {}", display_paths[0])
        } else {
            format!(
                "Loaded {} instruction files: {}",
                display_paths.len(),
                display_paths.join(", ")
            )
        };
        self.session_manager.append_message(
            session_id,
            &Message::new(MessageRole::System, &display_content),
        )?;

        Ok(updated)
    }

    /// Inject the mode reminder into the last user message when the mode
    /// changes, preserving the existing message prefix and persistence order.
    async fn inject_mode_reminder_impl(
        &self,
        session_id: Uuid,
        messages: &mut [Message],
    ) -> Result<()> {
        let last_user_idx = match messages.iter().rposition(|m| m.role == MessageRole::User) {
            Some(idx) => idx,
            None => return Ok(()),
        };

        let buffer = self.buffer.read().await;
        let prev_mode = messages[..last_user_idx]
            .iter()
            .rev()
            .filter(|m| m.role == MessageRole::User)
            .find_map(|m| {
                buffer
                    .app_data(m.id)
                    .and_then(|data| data.mode.as_deref()?.parse::<Mode>().ok())
            });
        drop(buffer);
        let is_first_user = prev_mode.is_none();
        let reminder = match (is_first_user, prev_mode) {
            (true, _) => Some(crate::prompts::mode_reminder(self.mode)),
            (false, Some(previous)) if previous != self.mode => Some(match self.mode {
                Mode::Plan => crate::prompts::plan_switch_reminder(),
                Mode::Build => crate::prompts::build_switch_reminder(),
            }),
            _ => None,
        };

        let Some(text) = reminder else {
            return Ok(());
        };
        if messages[last_user_idx].content.starts_with(&text) {
            return Ok(());
        }

        let new_content = format!("{text}\n\n{}", messages[last_user_idx].content);
        let message_id = messages[last_user_idx].id;
        messages[last_user_idx].content = new_content.clone();
        self.update_message_content(session_id, message_id, new_content)
            .await?;

        log::info!(
            "injected mode reminder into user message {} (mode={:?}, is_first={})",
            message_id,
            self.mode,
            is_first_user,
        );
        Ok(())
    }

    /// Update the content of an existing message in both the buffer and store.
    async fn update_message_content(
        &self,
        session_id: Uuid,
        message_id: Uuid,
        content: String,
    ) -> Result<()> {
        {
            let mut buf = self.buffer.write().await;
            buf.update_content(message_id, content.clone());
        }
        self.session_manager
            .update_message_content(session_id, message_id, &content)?;
        Ok(())
    }

    /// Move instruction sources collected by tools into persistent session
    /// state while deferring their replay notice until tool results are saved.
    async fn collect_instruction_sources(&self, session_id: Uuid) -> Result<()> {
        let sources = self.tool_registry.take_instruction_sources(session_id);
        if sources.is_empty() {
            return Ok(());
        }

        self.emit(BackendEvent::InstructionsLoaded {
            session_id,
            sources: sources.clone(),
        });

        let already_injected = self
            .session_manager
            .store()
            .load_instruction_sources(session_id)?;
        self.session_manager
            .store()
            .append_instruction_sources(session_id, &sources)?;

        let mut unique = sources;
        unique.sort();
        unique.dedup();
        let new_sources: Vec<String> = unique
            .into_iter()
            .filter(|source| !already_injected.contains(source))
            .collect();
        if !new_sources.is_empty() {
            self.pending_instruction_sources
                .lock()
                .await
                .extend(new_sources);
        }
        Ok(())
    }

    async fn finish_tool_execution(
        &self,
        session_id: Uuid,
        results: Vec<(ToolCall, ToolExecutionResult)>,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        self.collect_instruction_sources(session_id).await?;
        Ok(results)
    }

    /// Append the replay notice after the tool result messages have been
    /// persisted. This matches the previous loop-level ordering.
    async fn append_pending_instruction_message(&self, session_id: Uuid) -> Result<()> {
        let sources = {
            let mut pending = self.pending_instruction_sources.lock().await;
            std::mem::take(&mut *pending)
        };
        if sources.is_empty() {
            return Ok(());
        }

        let display: Vec<String> = sources
            .iter()
            .map(|source| {
                std::path::Path::new(source)
                    .strip_prefix(&self.workspace_root)
                    .unwrap_or(std::path::Path::new(source))
                    .display()
                    .to_string()
            })
            .collect();
        let content = if display.len() == 1 {
            format!("Loaded instructions from {}", display[0])
        } else {
            format!(
                "Loaded {} instruction files: {}",
                display.len(),
                display.join(", ")
            )
        };
        let message = Message::new(MessageRole::System, &content);
        self.buffer.write().await.append(message.clone());
        self.session_manager.append_message(session_id, &message)?;
        Ok(())
    }

    async fn request_tool_approval(
        &self,
        tool_calls: &[ToolCall],
        read_only: bool,
    ) -> Result<Vec<ApprovedTool>> {
        let mode = if read_only { Mode::Plan } else { Mode::Build };
        let sensitive_patterns = load_sensitive_patterns(&self.workspace_root);
        let access_control = {
            let cfg = self.config.read().unwrap();
            cfg.access_control.clone()
        };

        let mut approved = Vec::with_capacity(tool_calls.len());
        let mut pending = Vec::new();
        for tc in tool_calls {
            if !self.tool_registry.can_execute(&tc.name, mode) {
                approved.push(ApprovedTool {
                    tool_call: tc.clone(),
                    rejection: Some(ToolExecutionResult::new(format!(
                        "Tool '{}' is disabled in {} mode.",
                        tc.name,
                        mode.as_str(),
                    ))),
                    child_session_id: None,
                    allow_outside: false,
                    sensitive_file_approved: false,
                    user_reason: None,
                });
                continue;
            }

            let arguments: Value = serde_json::from_str(&tc.arguments).unwrap_or(Value::Null);
            let boundary_violation = if access_control.allow_outside_workspace_access {
                None
            } else {
                extract_boundary_violation_path(&self.workspace_root, &tc.name, &arguments)
            };
            let sensitive_violation = if access_control.allow_sensitive_file_access {
                None
            } else {
                extract_sensitive_file_path(
                    &self.workspace_root,
                    &tc.name,
                    &arguments,
                    &sensitive_patterns,
                )
            };

            if tidev_utils::tool_name::canonical_tool_name(&tc.name) == Some("question") {
                pending.push(ToolCallWithViolations {
                    tool_call: tc.clone(),
                    workspace_boundary_violation: None,
                    sensitive_file_violation: None,
                });
                continue;
            }

            if boundary_violation.is_none() && sensitive_violation.is_none() {
                approved.push(ApprovedTool {
                    tool_call: tc.clone(),
                    rejection: None,
                    child_session_id: None,
                    allow_outside: access_control.allow_outside_workspace_access,
                    sensitive_file_approved: access_control.allow_sensitive_file_access,
                    user_reason: None,
                });
            } else {
                pending.push(ToolCallWithViolations {
                    tool_call: tc.clone(),
                    workspace_boundary_violation: boundary_violation,
                    sensitive_file_violation: sensitive_violation,
                });
            }
        }

        if pending.is_empty() {
            return Ok(approved);
        }

        let (response_tx, mut response_rx) = tokio::sync::mpsc::unbounded_channel();
        self.request_tx
            .send(TuiRequest {
                session_id: self.session_id,
                kind: TuiRequestKind::ToolApproval(pending),
                response_tx,
            })
            .map_err(|_| anyhow::anyhow!("UI request channel closed — UI may have exited"))?;

        let user_approved = tokio::select! {
            _ = self.cancel.cancelled() => Vec::new(),
            result = response_rx.recv() => match result {
                Some(TuiResponse::ToolApproval(tools)) => tools,
                None => Vec::new(),
            },
        };
        approved.extend(user_approved);
        Ok(approved)
    }
}

// ---------------------------------------------------------------------------
// AgentContext trait implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl AgentContext for CoreContext {
    // -----------------------------------------------------------------------
    fn tools(&self) -> Vec<tidev_llm::ToolDefinition> {
        self.tools.iter().map(to_llm_tool_def).collect()
    }

    fn event_tx(&self) -> UnboundedSender<AgentEvent> {
        agent_event_channel(self.session_id, self.event_tx.clone())
    }

    fn workspace_root(&self) -> &std::path::Path {
        &self.workspace_root
    }

    // -----------------------------------------------------------------------
    async fn stream_turn(
        &self,
        messages: &[Message],
        system_prompt: &str,
        thinking_level: &ThinkingLevelType,
        request_id: u64,
    ) -> Result<AssistantTurn> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let llm = self.llm.clone();
        let mut model = self.model_config.clone();
        model.system_prompt = Some(system_prompt.to_string());
        let llm_tools: Vec<tidev_llm::ToolDefinition> =
            self.tools.iter().map(to_llm_tool_def).collect();
        let tl = thinking_level.clone();
        let msgs = messages.to_vec();

        // Spawn the streaming LLM call.
        let handle = tokio::spawn(async move {
            llm.stream_chat(model, msgs, llm_tools, tx, tl)
                .await;
        });

        let mut turn = AssistantTurn {
            created_at: Some(Utc::now()),
            ..Default::default()
        };

        // Race: cancel token vs LLM events.
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    handle.abort();
                    self.emit(BackendEvent::StreamEnd {
                        session_id: self.session_id,
                        request_id,
                        reasoning_started_at: None,
                        reasoning_completed_at: None,
                    });
                    return Err(anyhow::anyhow!("Stream cancelled by user"));
                }
                event = rx.recv() => {
                    match event {
                        Some(ev) => {
                            let ev = llm_event_to_agent_event(ev, request_id);
                            self.emit(agent_event_to_backend_event(
                                ev.clone(),
                                self.session_id,
                            ));
                            match ev {
                                AgentEvent::Delta { content, .. } => {
                                    turn.content.push_str(&content);
                                    // reasoning → content transition
                                    if turn.reasoning_started_at.is_some()
                                        && turn.reasoning_completed_at.is_none()
                                    {
                                        turn.reasoning_completed_at = Some(Utc::now());
                                    }
                                }
                                AgentEvent::ReasoningDelta { content, .. } => {
                                    if turn.reasoning_started_at.is_none() {
                                        turn.reasoning_started_at = Some(Utc::now());
                                    }
                                    turn.reasoning.push_str(&content);
                                }
                                AgentEvent::ToolCallUpdated { tool_call, .. } => {
                                    turn.upsert_tool_call(tool_call);
                                    // reasoning → tool-call transition
                                    if turn.reasoning_started_at.is_some()
                                        && turn.reasoning_completed_at.is_none()
                                    {
                                        turn.reasoning_completed_at = Some(Utc::now());
                                    }
                                }
                                AgentEvent::UsageStats {
                                    input_tokens,
                                    output_tokens,
                                    total_tokens,
                                    cache_read_tokens,
                                    cache_write_tokens,
                                    model_id,
                                    duration_ms,
                                    ..
                                } => {
                                    turn.input_tokens = Some(input_tokens);
                                    turn.output_tokens = Some(output_tokens);
                                    turn.total_tokens = Some(total_tokens);
                                    turn.cache_read_tokens = Some(cache_read_tokens);
                                    turn.cache_write_tokens = Some(cache_write_tokens);
                                    turn.model_id = Some(model_id);
                                    if let Some(ms) = duration_ms {
                                        turn.tokens_per_second =
                                            Some(output_tokens as f32 / (ms as f32 / 1000.0));
                                    }
                                }
                                AgentEvent::Finished { turn: finished_turn, .. } => {
                                    // The Responses provider carries opaque output items on the
                                    // final event because they are not represented by deltas.
                                    turn.responses_output_items =
                                        finished_turn.responses_output_items.clone();
                                    // reasoning -> turn-end transition
                                    if turn.reasoning_started_at.is_some()
                                        && turn.reasoning_completed_at.is_none()
                                    {
                                        turn.reasoning_completed_at = Some(Utc::now());
                                    }
                                    break;
                                }
                                AgentEvent::StreamEnd { .. } => {
                                    // reasoning → turn-end transition
                                    if turn.reasoning_started_at.is_some()
                                        && turn.reasoning_completed_at.is_none()
                                    {
                                        turn.reasoning_completed_at = Some(Utc::now());
                                    }
                                    break;
                                }
                                AgentEvent::Failed { error, .. } => {
                                    return Err(anyhow::anyhow!("LLM error: {error}"));
                                }
                                _ => {}
                            }
                        }
                        None => break,
                    }
                }
            }
        }

        turn.completed_at = Some(Utc::now());
        Ok(turn)
    }

    // -----------------------------------------------------------------------
    async fn execute_tools(
        &self,
        tool_calls: &[ToolCall],
        session_id: Uuid,
        request_id: u64,
    ) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
        let approved_tools = self
            .request_tool_approval(tool_calls, self.mode == Mode::Plan)
            .await?;

        let mut results: Vec<(ToolCall, ToolExecutionResult)> = Vec::new();
        for approved in &approved_tools {
            if let Some(rejection) = &approved.rejection {
                self.emit(BackendEvent::ToolCompleted {
                    session_id,
                    request_id,
                    tool_call: approved.tool_call.clone(),
                    result: Box::new(rejection.clone()),
                    child_session_id: None,
                });
                results.push((approved.tool_call.clone(), rejection.clone()));
            }
        }

        // Separate approved calls into task, read-only, and write groups.
        let mut task_calls: Vec<(ToolCall, Option<Uuid>)> = Vec::new();
        let mut read_only: Vec<(ToolCall, bool, bool)> = Vec::new();
        let mut write: Vec<(ToolCall, bool, bool)> = Vec::new();

        for approved in approved_tools {
            if approved.rejection.is_some() {
                continue;
            }

            let tc = approved.tool_call.clone();

            if tidev_utils::tool_name::canonical_tool_name(&tc.name) == Some("task") {
                task_calls.push((tc, approved.child_session_id));
            } else if is_read_only(&tc.name) {
                read_only.push((tc, approved.allow_outside, approved.sensitive_file_approved));
            } else {
                write.push((tc, approved.allow_outside, approved.sensitive_file_approved));
            }
        }

        // --- Read-only tools: parallel execution with immediate cancellation ---
        //
        // Uses JoinSet so that on cancellation every spawned tool task is
        // aborted (dropping in-flight HTTP connections, blocking reads, etc.)
        // and a synthetic "User cancelled" result is emitted for each.
        if !read_only.is_empty() {
            if self.cancel.is_cancelled() {
                for (tc, _, _) in &read_only {
                    self.emit(BackendEvent::ToolCompleted {
                        session_id,
                        request_id,
                        tool_call: tc.clone(),
                        result: Box::new(ToolExecutionResult::new("User cancelled the request")),
                        child_session_id: None,
                    });
                    results.push((
                        tc.clone(),
                        ToolExecutionResult::new("User cancelled the request"),
                    ));
                }
                return self.finish_tool_execution(session_id, results).await;
            }

            let mut pending_tcs: Vec<ToolCall> =
                read_only.iter().map(|(tc, _, _)| tc.clone()).collect();
            let mut join_set = tokio::task::JoinSet::new();

            // Notify TUI that read-only tools have started executing.
            for (tc, _, _) in &read_only {
                self.emit(BackendEvent::ToolStarting {
                    session_id,
                    request_id,
                    tool_call: tc.clone(),
                });
            }

            for (tc, allow_outside, sensitive_approved) in read_only {
                let reg = self.tool_registry.clone();
                let sid = session_id;
                let mode = self.mode;
                let cancel = self.cancel.clone();
                join_set.spawn(async move {
                    let result = reg
                        .execute_via_agent(
                            &tc,
                            sid,
                            request_id,
                            mode,
                            allow_outside,
                            sensitive_approved,
                            &cancel,
                        )
                        .await;
                    (tc, result)
                });
            }

            loop {
                tokio::select! {
                    _ = self.cancel.cancelled() => {
                        // Abort every in-flight tool task immediately.
                        join_set.abort_all();
                        for tc in pending_tcs {
                            self.emit(BackendEvent::ToolCompleted {
                                session_id,
                                request_id,
                                tool_call: tc.clone(),
                                result: Box::new(ToolExecutionResult::new("User cancelled the request")),
                                child_session_id: None,
                            });
                            results.push((
                                tc,
                                ToolExecutionResult::new("User cancelled the request"),
                            ));
                        }
                        return self.finish_tool_execution(session_id, results).await;
                    }
                    result = join_set.join_next() => {
                        match result {
                            Some(Ok((tc, result))) => {
                                pending_tcs.retain(|t| t.id != tc.id);
                                self.emit(BackendEvent::ToolCompleted {
                                    session_id,
                                    request_id,
                                    tool_call: tc.clone(),
                                    result: Box::new(result.clone()),
                                    child_session_id: None,
                                });
                                results.push((tc, result));
                            }
                            Some(Err(join_err)) => {
                                return Err(anyhow::anyhow!("Task join error: {join_err}"));
                            }
                            None => break,
                        }
                    }
                }
            }
        }

        // --- Write tools: serial execution with immediate cancellation ---
        for (tc, allow_outside, sensitive_approved) in write {
            if self.cancel.is_cancelled() {
                self.emit(BackendEvent::ToolCompleted {
                    session_id,
                    request_id,
                    tool_call: tc.clone(),
                    result: Box::new(ToolExecutionResult::new("User cancelled the request")),
                    child_session_id: None,
                });
                results.push((
                    tc,
                    ToolExecutionResult::new("User cancelled the request"),
                ));
                continue;
            }

            // Notify TUI that this tool has started executing.
            self.emit(BackendEvent::ToolStarting {
                session_id,
                request_id,
                tool_call: tc.clone(),
            });

            let (shell_tx, shell_rx) = tokio::sync::mpsc::unbounded_channel::<ShellOutput>();
            let tool_fut = self.tool_registry.execute(
                &tc,
                session_id,
                request_id,
                self.mode,
                allow_outside,
                sensitive_approved,
                &self.cancel,
                Some(shell_tx),
            );

            // Guard ensures ToolCompleted is sent if this future is
            // force-dropped (e.g. by JoinHandle::abort()), preventing the
            // TUI's running_tools from leaking.
            let mut guard = ToolCompletedGuard::new(
                session_id,
                request_id,
                Some(self.event_tx.clone()),
                shell_rx,
                tc.clone(),
            );

            // Directly await the tool. We do NOT use a select! with a cancel
            // branch here because doing so would drop the in-flight JoinHandle
            // for spawned tools (shell), causing the spawned task's result
            // to be lost. Instead, the tool receives the CancellationToken
            // internally and responds to cancellation itself (shell kills
            // the child process and returns partial output). Other write tools
            // (write/edit/apply_patch/todowrite) are fast spawn_blocking
            // operations that complete in milliseconds anyway.
            let result = tool_fut.await;
            guard.drain_shell_output();
            guard.disarm();
            self.emit(BackendEvent::ToolCompleted {
                session_id,
                request_id,
                tool_call: tc.clone(),
                result: Box::new(result.clone()),
                child_session_id: None,
            });
            results.push((tc, result));
        }

        // --- Task tools (subagents): parallel with immediate cancellation ---
        // When subagent is disabled by config, return an error instead of spawning.
        if !task_calls.is_empty() && !self.config.read().unwrap().subagent.enabled {
            for (tc, child_session_id) in task_calls.drain(..) {
                let result = ToolExecutionResult::new(
                    "User has temporarily disabled the subagent (task) tool.",
                );
                self.emit(BackendEvent::ToolCompleted {
                    session_id,
                    request_id,
                    tool_call: tc.clone(),
                    result: Box::new(result.clone()),
                    child_session_id,
                });
                results.push((tc, result));
            }
        }

        if !task_calls.is_empty() {
            let mut pending_tcs = task_calls.clone();
            // Drop guard persists "User cancelled" if h.abort() kills this task.
            let mut cancel_guard = CancelPersistGuard {
                session_manager: self.session_manager.clone(),
                buffer: self.buffer.clone(),
                session_id,
                tool_calls: pending_tcs.iter().map(|(tc, _)| tc.clone()).collect(),
                disarmed: false,
            };
            let mut join_set = tokio::task::JoinSet::new();
            for (tc, child_session_id) in task_calls {
                let cancel = self.cancel.child_token();
                let spawner = SubagentSpawner {
                    session_manager: self.session_manager.clone(),
                    tool_registry: self.tool_registry.clone(),
                    llm: self.llm.clone(),
                    active_model: self.active_model.clone(),
                    workspace_root: self.workspace_root.clone(),
                    config_dir: self.config_dir.clone(),
                    event_tx: self.event_tx.clone(),
                    mode: self.mode,
                    system_prompt: self.system_prompt.clone(),
                    snapshot: self.snapshot.clone(),
                    config: self.config.clone(),
                    auth: self.auth.clone(),
                    session_start_hash: self.session_start_hash.lock().await.clone(),
                    buffer: self.buffer.clone(),
                };
                join_set.spawn(async move {
                    execute_task_tool(
                        spawner,
                        SubagentConfig {
                            tool_call: tc.clone(),
                            child_session_id,
                            parent_session_id: session_id,
                            parent_request_id: request_id,
                            cancel_token: cancel,
                        },
                    )
                    .await
                    .map(|(result, child_session_id)| (tc, result, child_session_id))
                });
            }

            loop {
                tokio::select! {
                    _ = self.cancel.cancelled() => {
                        // Abort every subagent task. Because JoinSet owns the
                        // tasks, dropping it cascades: the subagent future is
                        // dropped, which drops the child agent loop, which
                        // drops the child JoinSet, which aborts the child's
                        // tools — recursively to any depth.
                        join_set.abort_all();
                        for (tc, child_session_id) in pending_tcs {
                            self.emit(BackendEvent::ToolCompleted {
                                session_id,
                                request_id,
                                tool_call: tc.clone(),
                                result: Box::new(ToolExecutionResult::new("User cancelled the request")),
                                child_session_id,
                            });
                            results.push((
                                tc,
                                ToolExecutionResult::new("User cancelled the request"),
                            ));
                        }
                        cancel_guard.disarmed = true;
                        return self.finish_tool_execution(session_id, results).await;
                    }
                    result = join_set.join_next() => {
                        match result {
                            Some(Ok(Ok((tc, result, child_session_id)))) => {
                                pending_tcs.retain(|(t, _)| t.id != tc.id);
                                if self.cancel.is_cancelled() {
                                    // Already cancelled — push synthetic result.
                                    results.push((
                                        tc,
                                        ToolExecutionResult::new("User cancelled the request"),
                                    ));
                                } else {
                                    self.emit(BackendEvent::ToolCompleted {
                                        session_id,
                                        request_id,
                                        tool_call: tc.clone(),
                                        result: Box::new(result.clone()),
                                        child_session_id: Some(child_session_id),
                                    });
                                    results.push((tc, result));
                                }
                            }
                            Some(Ok(Err(e))) => {
                                if self.cancel.is_cancelled() {
                                    // Cancelled subagent with error: push
                                    // synthetic results for all remaining.
                                    join_set.abort_all();
                                    for (tc, child_session_id) in pending_tcs {
                                        self.emit(BackendEvent::ToolCompleted {
                                            session_id,
                                            request_id,
                                            tool_call: tc.clone(),
                                            result: Box::new(ToolExecutionResult::new(
                                                "User cancelled the request",
                                            )),
                                            child_session_id,
                                        });
                                        results.push((
                                            tc,
                                            ToolExecutionResult::new(
                                                "User cancelled the request",
                                            ),
                                        ));
                                    }
                                    cancel_guard.disarmed = true;
                                    return self.finish_tool_execution(session_id, results).await;
                                } else {
                                    cancel_guard.disarmed = true;
                                    return Err(e);
                                }
                            }
                            Some(Err(join_err)) => {
                                cancel_guard.disarmed = true;
                                return Err(anyhow::anyhow!(
                                    "Subagent join error: {join_err}"
                                ));
                            }
                            None => break,
                        }
                    }
                }
            }
            cancel_guard.disarmed = true;
        }

        self.finish_tool_execution(session_id, results).await
    }

    // -----------------------------------------------------------------------
    async fn save_messages(
        &self,
        session_id: Uuid,
        messages: &[Message],
    ) -> Result<()> {
        // ── Phase 1: Round-level snapshot tracking ──────────────────────
        //
        // When the assistant message with tool calls is saved, a round is
        // starting — capture the pre-round workspace snapshot.
        let has_tool_calls = messages
            .iter()
            .any(|m| m.role == MessageRole::Assistant && !m.tool_calls.is_empty());

        // If every tool in this round is read-only no files can change,
        // so snapshot tracking is unnecessary.
        let all_read_only = has_tool_calls
            && messages
                .iter()
                .filter(|m| m.role == MessageRole::Assistant)
                .flat_map(|m| &m.tool_calls)
                .all(|tc| is_read_only(&tc.name));

        if has_tool_calls
            && !all_read_only
            && let Some(ref snap) = self.snapshot
            && let Ok(Some(hash)) = snap.track()
        {
            *self.pre_round_hash.lock().await = Some(hash.clone());
            // Initialize session_start_hash on the very first snapshot.
            let mut ssh = self.session_start_hash.lock().await;
            if ssh.is_none() {
                let persist_hash = hash.clone();
                *ssh = Some(hash);
                // Persist to DB so restored sessions retain the correct baseline.
                let _ = self
                    .session_manager
                    .update_session_start_hash(session_id, &persist_hash);
            }
        } else if has_tool_calls && all_read_only {
            // Read-only round: clear any stale pre-round hash so the
            // post-round diff branch below does not run an unnecessary
            // git diff against an old snapshot.
            *self.pre_round_hash.lock().await = None;
        }

        // When tool result messages are saved, the round has finished —
        // capture the post-round snapshot and diff against pre-round.
        let has_tool_results = messages.iter().any(|m| m.role == MessageRole::Tool);
        let mut enriched = messages.to_vec();
        let mut app_data: HashMap<Uuid, MessageAppData> = enriched
            .iter()
            .map(|message| (message.id, MessageAppData::default()))
            .collect();
        if messages.iter().any(|message| message.role == MessageRole::Tool) {
            let buffer = self.buffer.read().await;
            for message in messages {
                if message.role != MessageRole::Tool {
                    continue;
                }
                let Some(tool_call_id) = message.tool_call_id.as_deref() else {
                    continue;
                };
                if let Some(child_session_id) =
                    child_session_id_for_tool_call(&buffer, tool_call_id)
                {
                    if let Some(data) = app_data.get_mut(&message.id) {
                        data.child_session_id = Some(child_session_id);
                    }
                }
            }
        }
        if has_tool_results {
            let pre = { self.pre_round_hash.lock().await.clone() };
            if let Some(ref pre) = pre
                && let Some(ref snap) = self.snapshot
                && let Ok(Some(post_hash)) = snap.track()
                && let Ok(diffs) = snap.diff_lightweight(pre, &post_hash).await
            {
                let files: Vec<String> = diffs
                    .iter()
                    .map(|d| {
                        self.workspace_root
                            .join(&d.file)
                            .to_string_lossy()
                            .replace('\\', "/")
                    })
                    .collect();
                if !files.is_empty() {
                    *self.pre_round_hash.lock().await = Some(post_hash.clone());
                    let step_patch = serde_json::json!([{
                        "hash": pre,
                        "files": files,
                        "step": 1,
                    }]);
                    if let Some(last) = enriched.last_mut() {
                        let data = app_data.entry(last.id).or_default();
                        data.snapshot_hash = Some(post_hash.clone());
                        data.patch_files = Some(step_patch.to_string());
                        // Serialize lightweight diffs for sidebar display.
                        // Use session_start_hash as baseline so the diff
                        // matches `git diff` from session start.
                        let start = { self.session_start_hash.lock().await.clone() };
                        let baseline = start.as_ref().unwrap_or(pre);
                        if let Ok(cumulative_diffs) =
                            snap.diff_lightweight(baseline, &post_hash).await
                            && let Ok(diffs_json) = serde_json::to_string(&cumulative_diffs)
                        {
                            data.file_diffs = Some(diffs_json.clone());
                            self.emit(BackendEvent::SidebarSnapshotReady {
                                session_id,
                                request_id: 0,
                                tool_call_id: last.tool_call_id.clone().unwrap_or_default(),
                                file_diffs_json: diffs_json,
                            });
                        }
                    }
                }
            }
        }

        // ── Phase 2: Write to buffer + DB ───────────────────────────────
        {
            let mut buf = self.buffer.write().await;
            for msg in &enriched {
                let data = app_data.get(&msg.id).cloned().unwrap_or_default();
                buf.append_with_app_data(msg.clone(), data);
            }
        }
        self.session_manager
            .append_messages_with_app_data(session_id, &enriched, &app_data)?;
        if has_tool_results {
            self.append_pending_instruction_message(session_id).await?;
        }
        Ok(())
    }

    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        // 1. Check if compaction is needed (brief lock).
        let (needs_compact, msgs_to_compact) = {
            let cm = self.context_manager.lock().await;
            let buf = self.buffer.read().await;
            let needs = cm.needs_compaction(
                buf.protocol(),
                self.model_config.context_window,
                self.model_config.max_output_tokens,
            );
            let msgs = if needs {
                buf.load().to_vec()
            } else {
                Vec::new()
            };
            (needs, msgs)
        };

        // 2. If compaction is needed, perform it (no locks held during LLM call).
        if needs_compact {
            // Capture prior compaction state before compact overwrites it.
            let (prior_summary, prior_retained_from) = {
                let cm = self.context_manager.lock().await;
                (cm.summary.clone(), cm.retained_from)
            };
            let tools: Vec<tidev_llm::ToolDefinition> = self
                .tool_registry
                .definitions_for_model(&self.active_model)
                .iter()
                .map(to_llm_tool_def)
                .collect();
            let result = {
                let mut compact_model = self.model_config.clone();
                compact_model.system_prompt = Some(self.system_prompt.clone());
                let cm = self.context_manager.lock().await;
                cm.compact(
                    &self.llm,
                    &compact_model,
                    &tools,
                    &msgs_to_compact,
                    self.session_id,
                    None,
                )
                .await?
            };

            // 3. Update state + persist + append marker + emit event.
            {
                let mut cm = self.context_manager.lock().await;
                cm.apply_compaction(result.summary.clone(), result.retained_from);
            }
            self.session_manager.update_context_state(
                self.session_id,
                Some(&result.summary),
                result.retained_from,
            )?;

            // Append compaction marker for undo support.
            {
                let mut marker = Message::compaction(&result.summary);
                marker.metadata.prior_summary = prior_summary;
                marker.metadata.prior_retained_from = Some(prior_retained_from);
                self.buffer.write().await.append(marker.clone());
                self.session_manager
                    .append_message(self.session_id, &marker)?;
            }
            let model_id = self.active_model.model_id.clone();
            self.emit(BackendEvent::ContextCompacted {
                session_id: self.session_id,
                compacted: true,
                manual: false,
                summary: Some(result.summary),
                retained_from: result.retained_from,
                model_id: Some(model_id),
                completed_at: Some(Utc::now()),
                error: None,
            });
        }

        // 4. Return the prepared message view.
        let cm = self.context_manager.lock().await;
        let buf = self.buffer.read().await;
        let mut messages = cm.build_request_messages(buf.protocol());
        drop(buf);
        drop(cm);

        // Keep both injections in the same order as the original agent loop:
        // instruction files first, then the mode reminder.
        self.inject_instructions_impl(session_id, &mut messages).await?;
        self.inject_mode_reminder_impl(session_id, &mut messages).await?;
        Ok(messages)
    }

}

// ---------------------------------------------------------------------------
// RAII guard that ensures ToolCompleted is always emitted on abort.
// ---------------------------------------------------------------------------

/// RAII guard: sends `BackendEvent::ToolCompleted` on drop if not disarmed.
///
/// Ensures the TUI is always notified that a tool has completed, even when
/// the `execute_tools` future is force-dropped (e.g. by `JoinHandle::abort()`).
struct ToolCompletedGuard {
    session_id: Uuid,
    request_id: u64,
    event_tx: Option<UnboundedSender<BackendEvent>>,
    shell_rx: Option<UnboundedReceiver<ShellOutput>>,
    tool_call: ToolCall,
    disarmed: bool,
}

impl ToolCompletedGuard {
    fn new(
        session_id: Uuid,
        request_id: u64,
        event_tx: Option<UnboundedSender<BackendEvent>>,
        shell_rx: UnboundedReceiver<ShellOutput>,
        tool_call: ToolCall,
    ) -> Self {
        Self {
            session_id,
            request_id,
            event_tx,
            shell_rx: Some(shell_rx),
            tool_call,
            disarmed: false,
        }
    }

    fn drain_shell_output(&mut self) {
        let Some(shell_rx) = self.shell_rx.as_mut() else {
            return;
        };
        while let Ok(output) = shell_rx.try_recv() {
            if let Some(ref tx) = self.event_tx {
                let _ = tx.send(BackendEvent::ShellOutput {
                    session_id: output.session_id,
                    tool_call_id: output.tool_call_id,
                    content: output.content,
                    finished: output.finished,
                    exit_code: output.exit_code,
                });
            }
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for ToolCompletedGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            self.drain_shell_output();
            if let Some(ref tx) = self.event_tx {
                let _ = tx.send(BackendEvent::ToolCompleted {
                    session_id: self.session_id,
                    request_id: self.request_id,
                    tool_call: self.tool_call.clone(),
                    result: Box::new(ToolExecutionResult::new("User cancelled the request")),
                    child_session_id: None,
                });
            }
        }
    }
}

#[cfg(test)]
mod tool_event_order_tests {
    use super::*;

    #[tokio::test]
    async fn guard_drains_shell_output_before_tool_completed() {
        let session_id = Uuid::new_v4();
        let (backend_tx, mut backend_rx) = tokio::sync::mpsc::unbounded_channel();
        let (shell_tx, shell_rx) = tokio::sync::mpsc::unbounded_channel();
        shell_tx
            .send(ShellOutput {
                session_id,
                request_id: 7,
                tool_call_id: "call-1".to_string(),
                content: "partial output".to_string(),
                finished: true,
                exit_code: Some(0),
            })
            .unwrap();
        drop(shell_tx);

        drop(ToolCompletedGuard::new(
            session_id,
            7,
            Some(backend_tx),
            shell_rx,
            ToolCall::default(),
        ));

        assert!(matches!(
            backend_rx.recv().await,
            Some(BackendEvent::ShellOutput { .. })
        ));
        assert!(matches!(
            backend_rx.recv().await,
            Some(BackendEvent::ToolCompleted { request_id: 7, .. })
        ));
    }
}

// ---------------------------------------------------------------------------
// Subagent support — private helpers used by execute_tools for task tool calls.
// ---------------------------------------------------------------------------

/// Holds all the resources a subagent needs (owned, 'static-capable).
struct SubagentSpawner {
    session_manager: SessionManager,
    buffer: Arc<RwLock<CoreMessageBuffer>>,
    tool_registry: Arc<ToolRegistry>,
    llm: LlmClient,
    active_model: ActiveModel,
    workspace_root: PathBuf,
    config_dir: PathBuf,
    event_tx: UnboundedSender<BackendEvent>,
    mode: Mode,
    system_prompt: String,
    snapshot: Option<SnapshotService>,
    config: Arc<StdRwLock<AppConfig>>,
    auth: Arc<StdRwLock<AuthStore>>,
    session_start_hash: Option<String>,
}

/// Parameters describing a subagent invocation.
struct SubagentConfig {
    tool_call: ToolCall,
    child_session_id: Option<Uuid>,
    parent_session_id: Uuid,
    parent_request_id: u64,
    cancel_token: CancellationToken,
}

/// Execute a `task` tool call by spawning a subagent loop.
async fn execute_task_tool(
    spawner: SubagentSpawner,
    config: SubagentConfig,
) -> Result<(ToolExecutionResult, Uuid)> {
    // 1. Parse the task arguments.
    let args: Value = serde_json::from_str(&config.tool_call.arguments)
        .context("failed to parse task tool arguments")?;

    let subagent_type_str = args
        .get("subagent_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("task tool missing 'subagent_type'"))?;

    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let description = args
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();

    let agent_type = AgentType::parse(subagent_type_str)
        .ok_or_else(|| anyhow::anyhow!("unknown subagent type '{subagent_type_str}'"))?;

    // Plan mode rejects delegation to fixer subagents (they perform writes).
    // Moved here from tidev-tools task.rs: the main loop intercepts all task
    // calls in execute_tools, so this check only takes effect in core.
    if spawner.mode == Mode::Plan && agent_type == AgentType::Fixer {
        anyhow::bail!(
            "Task delegation to fixer subagent rejected: Plan mode is read-only and does not allow write operations. \
            You may delegate to read-only subagents (explorer, librarian, oracle) in plan mode. \
            Switch to build mode to use the fixer subagent."
        );
    }

    // 2. Build agent definition.
    let agent_def = build_agent_def(agent_type, &spawner.system_prompt);

    // 3. Resolve child model: check [agent.models] config, fall back to parent model.
    let child_model = {
        let agent_type_name = agent_type.display_name();
        let cfg = spawner.config.read().unwrap();
        let auth = spawner.auth.read().unwrap();
        match cfg.resolve_agent_active_model(&auth, agent_type_name) {
            Ok(Some(model)) => model,
            _ => {
                let mut m = spawner.active_model.clone();
                m.system_prompt = agent_def.system_prompt.clone();
                m.thinking_level = ThinkingLevelType::default();
                m
            }
        }
    };
    let child_model_config = to_llm_provider_config(&child_model);

    // 4. Filter tools based on child model, then agent type + mode.
    let model_tools = spawner.tool_registry.definitions_for_model(&child_model);
    let sub_tools = filter_subagent_tools(&model_tools, agent_type, spawner.mode)?;

    // 5. Create child session.
    let child_session_id = match config.child_session_id {
        Some(id) => id,
        None => {
            let child_session_id = Uuid::new_v4();
            spawner
                .session_manager
                .create_session(
                    child_session_id,
                    &spawner.workspace_root.to_string_lossy(),
                    &child_model.provider_id,
                    &child_model.provider_display_name,
                    &child_model.model_id,
                    &child_model.display_name,
                    &format!("subagent:{:?} {}", agent_type, description),
                    Some(config.parent_session_id),
                    None,
                )
                .context("failed to create child session")?;
            child_session_id
        }
    };

    // 5. Create child buffer + seed with the user prompt.
    let child_buffer = Arc::new(RwLock::new(CoreMessageBuffer::empty()));
    let user_msg = Message::new(tidev_llm::message::MessageRole::User, prompt);
    child_buffer.write().await.append(user_msg.clone());
    spawner
        .session_manager
        .append_message(child_session_id, &user_msg)
        .context("failed to seed child session")?;

    // 6. Emit SubagentStatus that the child has started (parent session).
    let _ = spawner.event_tx.send(BackendEvent::SubagentStatus {
        session_id: config.parent_session_id,
        request_id: config.parent_request_id,
        tool_call_id: config.tool_call.id.clone(),
        child_session_id,
        status_text: format!("Started {:?} subagent", agent_type),
        current_tool_call: None,
        assistant_message: Box::new(None),
        content_delta: None,
        reasoning_delta: None,
    });

    // Persist the child association as application data on the parent
    // assistant message so it survives session reloads without entering the
    // protocol metadata sent to an LLM.
    let assistant_message_id = {
        let buffer = spawner.buffer.read().await;
        buffer
            .load()
            .iter()
            .find(|message| {
                message.role == MessageRole::Assistant
                    && message
                        .tool_calls
                        .iter()
                        .any(|tool_call| tool_call.id == config.tool_call.id)
            })
            .map(|message| message.id)
    };
    if let Some(message_id) = assistant_message_id {
        let mut app_data = {
            let buffer = spawner.buffer.read().await;
            buffer.app_data(message_id).cloned().unwrap_or_default()
        };
        app_data.child_session_id = Some(child_session_id);
        spawner
            .buffer
            .write()
            .await
            .set_app_data(message_id, app_data);
        let _ = spawner.session_manager.update_message_child_session_id(
            config.parent_session_id,
            message_id,
            child_session_id,
        );
    }

    // 7. Create child CoreContext.
    let child_thinking_level = child_model.thinking_level.clone();
    let child_ctx = CoreContext::new(
        spawner.llm,
        spawner.session_manager,
        spawner.tool_registry,
        Arc::new(Mutex::new(ContextManager::new())),
        child_buffer.clone(),
        spawner.event_tx.clone(),
        // Subagents auto-approve (parent handles UI permissions).
        tokio::sync::mpsc::unbounded_channel().0,
        child_session_id,
        spawner.mode,
        agent_def.system_prompt.clone(),
        child_model_config,
        config.cancel_token.clone(),
        sub_tools,
        spawner.workspace_root,
        child_model,
        spawner.snapshot,
        spawner.config,
        spawner.auth,
        spawner.session_start_hash,
        spawner.config_dir,
    );

    let loop_config = AgentLoopConfig {
        session_id: child_session_id,
        system_prompt: agent_def.system_prompt.clone(),
        thinking_level: child_thinking_level,
        event_tx: child_ctx.event_tx(),
        cancel: config.cancel_token.clone(),
        // Subagents run in isolation — they don't process main-session
        // user messages, so give them a fresh empty queue.
        queued_messages: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
    };

    // 8. Run the inner loop.
    let result = tidev_agent::run_agent_loop(&child_ctx, loop_config).await;

    // 9. Collect the final assistant message from the child buffer.
    let final_output = {
        let buf = child_buffer.read().await;
        buf.load()
            .iter()
            .rev()
            .find(|m| m.role == tidev_llm::message::MessageRole::Assistant)
            .cloned()
    };

    // 10. Build the protocol result separately from the child-session
    // association. The parent emits ToolCompleted for the result, while the
    // association remains host-owned application data.
    let final_result = match result {
        Ok(()) => match final_output {
            Some(msg) if !msg.content.is_empty() => ToolExecutionResult::new(msg.content),
            _ => ToolExecutionResult::new("(Subagent completed without text output)".to_string()),
        },
        Err(e) => ToolExecutionResult::new(format!("Subagent failed: {e}")),
    };
    Ok((final_result, child_session_id))
}

fn build_agent_def(agent_type: AgentType, parent_prompt: &str) -> AgentDefinition {
    AgentDefinition {
        agent_type,
        display_name: agent_type.display_name().to_string(),
        description: agent_type.description().to_string(),
        system_prompt: format!(
            "{}\n\nYou are running as a subagent ({}). \
             Your output will be reviewed by the parent agent. \
             Be concise and complete in your specialized role.",
            parent_prompt,
            agent_type.display_name()
        ),
        allowed_tools: agent_type
            .default_tool_restrictions()
            .map(|t| t.iter().map(|s| s.to_string()).collect()),
        temperature: Some(agent_type.default_temperature()),
        read_only: agent_type.is_read_only(),
    }
}

fn filter_subagent_tools(
    parent_tools: &[ToolDefinition],
    agent_type: AgentType,
    mode: Mode,
) -> Result<Vec<ToolDefinition>> {
    let allowed = agent_type.default_tool_restrictions();
    let read_only = agent_type.is_read_only();

    let filtered: Vec<ToolDefinition> = parent_tools
        .iter()
        .filter(|def| {
            let name = &def.name;
            let canonical = tidev_utils::tool_name::canonical_tool_name(name).unwrap_or(name.as_str());

            // Plan mode or read-only agent: only read tools.
            if mode == Mode::Plan || read_only {
                return is_read_tool(canonical);
            }

            // Agent type restrictions.
            match allowed {
                Some(list) => list.contains(&canonical),
                None => true,
            }
        })
        .filter(|def| {
            let name = &def.name;
            let canonical = tidev_utils::tool_name::canonical_tool_name(name).unwrap_or(name.as_str());
            // Extra safety: never include write tools for read-only agents.
            if read_only && is_write_tool(canonical) {
                return false;
            }
            true
        })
        // Never include the task tool — subagents must not spawn further subagents.
        .filter(|def| {
            let canonical = tidev_utils::tool_name::canonical_tool_name(&def.name).unwrap_or(&def.name);
            canonical != "task"
        })
        .cloned()
        .collect();
    Ok(filtered)
}

fn is_read_tool(name: &str) -> bool {
    matches!(
        name,
        "read" | "glob" | "grep" | "websearch" | "webfetch" | "question"
    )
}

fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "write" | "edit" | "apply_patch" | "shell" | "todowrite"
    )
}

#[cfg(test)]
mod child_session_app_data_tests {
    use super::*;

    #[test]
    fn finds_child_session_from_matching_assistant_tool_call() {
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "task".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        };
        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(tool_call);
        let child_session_id = Uuid::new_v4();
        let mut buffer = CoreMessageBuffer::empty();
        buffer.append_with_app_data(
            assistant,
            MessageAppData {
                child_session_id: Some(child_session_id),
                ..MessageAppData::default()
            },
        );

        assert_eq!(
            child_session_id_for_tool_call(&buffer, "call-1"),
            Some(child_session_id)
        );
        assert_eq!(child_session_id_for_tool_call(&buffer, "missing"), None);
    }
}
