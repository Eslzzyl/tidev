//! Shared core state and operations for all gateway channels.
//!
//! Each platform (Telegram, QQ, etc.) creates a [`ChannelCore`] and delegates
//! common operations (session management, model resolution, message handling)
//! to it. Platform-specific IO is provided via the [`MessageSender`] trait.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_types::prompts::{SessionMode, gateway_system_prompt};

use tidev_engine::{

    agent::runtime::AgentRuntime,
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    llm::LlmClient,
    storage::SessionStore,
    tooling::ToolRegistry,
};
use tidev_session::session::{Conversation, Message, MessageRole};

use super::commands::{format_status_summary, gateway_help_text};
use super::model_selection::ModelSelectionState;
use super::shared::ModeManager;
use super::shell;

// ── MessageSender trait ─────────────────────────────────────────────────────

/// Platform-agnostic interface for sending messages through any gateway channel.
#[async_trait]
pub trait MessageSender {
    /// Send a text message. `reply_to` is an optional platform message ID.
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()>;

    /// Whether this platform supports progressive message updates.
    fn supports_draft(&self) -> bool {
        false
    }

    /// Send an initial draft (placeholder) message. Returns a platform message ID.
    async fn send_draft(&mut self, _recipient: &str, _text: &str) -> Result<Option<String>> {
        Ok(None)
    }

    /// Update a draft message with new accumulated content.
    async fn update_draft(&mut self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> {
        Ok(())
    }

    /// Show a one-line progress / tool status update.
    async fn update_draft_progress(
        &mut self,
        _recipient: &str,
        _msg_id: &str,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Finalise a draft with the complete response.
    async fn finalize_draft(&mut self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> {
        Ok(())
    }

    /// Cancel / delete a draft message.
    async fn cancel_draft(&mut self, _recipient: &str, _msg_id: &str) -> Result<()> {
        Ok(())
    }
}

// ── ChannelCore ─────────────────────────────────────────────────────────────

/// Shared state and operations for a gateway channel.
///
/// Every concrete channel holds a `ChannelCore` and delegates to it for
/// session management, model resolution, command handling, and the
/// agent loop.
pub struct ChannelCore {
    pub workspace_root: PathBuf,
    pub config: AppConfig,
    pub auth: AuthStore,
    pub config_paths: ConfigPaths,
    pub store: SessionStore,
    pub llm: LlmClient,
    pub tools: ToolRegistry,
    pub agent: AgentRuntime,
    pub instruction_prompt: String,
    pub start_time: Instant,
    pub allowlist: HashSet<String>,
    /// Platform name constant (e.g. "telegram", "qq").
    pub platform_name: &'static str,

    // ── Per-chat state (keyed by stringified chat_id) ──
    pub cancellation_tokens: HashMap<String, CancellationToken>,
    pub model_selection_states: HashMap<String, ModelSelectionState>,
    pub compacting_sessions: HashSet<Uuid>,
    pub mode_manager: ModeManager,
}

impl ChannelCore {
    fn build_agent(
        workspace_root: &Path,
        paths: &ConfigPaths,
        config: &AppConfig,
        auth: &AuthStore,
        store: &SessionStore,
        tools: &ToolRegistry,
        llm: &LlmClient,
    ) -> AgentRuntime {
        AgentRuntime {
            workspace_root: workspace_root.to_path_buf(),
            config_dir: paths.config_dir.clone(),
            config_paths: paths.clone(),
            config: config.clone(),
            auth: auth.clone(),
            store: Arc::new(tokio::sync::Mutex::new(store.clone())),
            llm_client: llm.clone(),
            tools: tools.clone(),
            instructions: config.instructions.clone(),
            instruction_content_cache: std::collections::HashMap::new(),
            queued_messages: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::new(),
            )),
            auto_approve_permissions: false,
            hooks: tidev_engine::hooks::HookEngine::new(
                config.hooks.clone(),
                workspace_root.to_path_buf(),
            ),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        platform_name: &'static str,
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        paths: &ConfigPaths,
        store: SessionStore,
        llm: LlmClient,
        tools: ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
    ) -> Self {
        let agent = Self::build_agent(&workspace_root, paths, &config, &auth, &store, &tools, &llm);
        let default_mode = config.gateway.parsed_default_mode();
        Self {
            workspace_root,
            config,
            auth,
            config_paths: paths.clone(),
            store,
            llm,
            tools,
            agent,
            instruction_prompt,
            start_time: Instant::now(),
            allowlist,
            platform_name,
            cancellation_tokens: HashMap::new(),
            model_selection_states: HashMap::new(),
            compacting_sessions: HashSet::new(),
            mode_manager: ModeManager::new(default_mode),
        }
    }

    #[inline]
    pub fn platform(&self) -> &str {
        self.platform_name
    }

    // ── Session management ─────────────────────────────────────────────────

    /// Load an existing conversation for `chat_key`, or create a fresh one.
    pub fn load_or_create_conversation(
        &self,
        chat_key: &str,
        active_model: &ActiveModel,
    ) -> Result<Conversation> {
        if let Some(session_id) = self
            .store
            .load_gateway_chat_session(self.platform(), chat_key)?
            && let Some(record) = self.store.load_session_record(session_id)?
        {
            let messages = self.store.load_messages(session_id)?;
            return Ok(Conversation {
                session_id,
                parent_session_id: record.parent_session_id,
                workspace_root: record.workspace_root,
                provider_id: record.provider_id,
                provider_display_name: record.provider_display_name,
                model_id: record.model_id,
                model_display_name: record.model_display_name,
                title: record.title,
                created_at: record.created_at,
                updated_at: record.updated_at,
                context_summary: record.context_summary,
                context_retained_from: record.context_retained_from,
                messages,
                revert_message_id: None,
            });
        }

        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            active_model.provider_id.clone(),
            active_model.provider_display_name.clone(),
            active_model.model_id.clone(),
            active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            &self.workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &conversation.title,
        )?;

        let static_prompt = self
            .agent
            .compose_static_system_prompt(&active_model.system_prompt);
        if let Err(e) = self
            .store
            .update_session_system_prompt(session_id, &static_prompt)
        {
            log::warn!("failed to persist static system prompt: {}", e);
        }

        self.store
            .set_gateway_chat_session(self.platform(), chat_key, session_id)?;
        Ok(conversation)
    }

    /// Create a fresh session for `chat_key`, replacing any existing one.
    pub fn rotate_chat_session(
        &self,
        chat_key: &str,
        active_model: &ActiveModel,
    ) -> Result<Conversation> {
        let conversation = self.create_gateway_session(active_model)?;
        self.store
            .set_gateway_chat_session(self.platform(), chat_key, conversation.session_id)?;
        Ok(conversation)
    }

    /// Create a new gateway session in the database.
    pub fn create_gateway_session(&self, active_model: &ActiveModel) -> Result<Conversation> {
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            active_model.provider_id.clone(),
            active_model.provider_display_name.clone(),
            active_model.model_id.clone(),
            active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            &self.workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &conversation.title,
        )?;

        let static_prompt = self
            .agent
            .compose_static_system_prompt(&active_model.system_prompt);
        if let Err(e) = self
            .store
            .update_session_system_prompt(session_id, &static_prompt)
        {
            log::warn!("failed to persist static system prompt: {}", e);
        }
        Ok(conversation)
    }

    /// Restore sessions from persistent storage, closing orphaned user turns.
    pub fn restore_sessions(&self, store: SessionStore) -> Result<usize> {
        let sessions = store.list_gateway_chat_sessions(self.platform())?;
        let mut count = 0;
        let mut orphans_closed = 0;

        for (chat_key, session_id) in sessions {
            if let Some(_conversation) = store.load_conversation(session_id)? {
                let messages = store.load_messages(session_id)?;
                if let Some(last) = messages.last()
                    && last.role == MessageRole::User
                {
                    let marker = Message::new(
                        MessageRole::Assistant,
                        "[Session interrupted — not continuing this request]".to_string(),
                    );
                    store.append_message(session_id, &marker)?;
                    orphans_closed += 1;
                }
                count += 1;
                log::info!(
                    "Restored {} session: chat_key={}, session_id={}, messages={}",
                    self.platform(),
                    chat_key,
                    session_id,
                    messages.len()
                );
            }
        }

        if count > 0 {
            log::info!(
                "Restored {} {} session(s) from disk",
                count,
                self.platform()
            );
        }
        if orphans_closed > 0 {
            log::info!(
                "Closed {} orphaned session turn(s) from previous crash",
                orphans_closed
            );
        }
        Ok(count)
    }

    /// Load the session's static system prompt onto the model.
    pub fn load_system_prompt(&self, conversation: &Conversation, active_model: &mut ActiveModel) {
        match self
            .store
            .load_session_system_prompt(conversation.session_id)
        {
            Ok(stored) if !stored.is_empty() => active_model.system_prompt = stored,
            _ => {
                let composed = self
                    .agent
                    .compose_static_system_prompt(&active_model.system_prompt);
                if let Err(e) = self
                    .store
                    .update_session_system_prompt(conversation.session_id, &composed)
                {
                    log::warn!("failed to persist static system prompt: {}", e);
                }
                active_model.system_prompt = composed;
            }
        }
    }

    /// Persist a user message, update title on first message.
    pub fn persist_user_message(
        &self,
        conversation: &mut Conversation,
        chat_key: &str,
        content: &str,
    ) -> Result<()> {
        let mut user_message = Message::new(MessageRole::User, content);
        user_message.mode = Some(self.mode_manager.get(chat_key));
        conversation.push(user_message.clone());
        self.store
            .append_message(conversation.session_id, &user_message)?;

        if conversation.messages.len() == 1 || conversation.title == "Untitled session" {
            conversation.update_title_from_prompt(content);
            self.store
                .update_session_title(conversation.session_id, &conversation.title)?;
        }
        Ok(())
    }

    // ── Model management ───────────────────────────────────────────────────

    /// Get available providers (user config + bundled) that have valid auth.
    pub fn get_available_providers(&self) -> Vec<(String, String)> {
        let mut providers = Vec::new();
        for (id, config) in &self.config.providers {
            if let Some(auth) = self.auth.providers.get(id)
                && auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty())
            {
                providers.push((id.clone(), config.display_name.clone()));
            }
        }
        for (id, config) in &self.config.bundled_providers {
            if self.config.providers.contains_key(id) {
                continue;
            }
            if let Some(auth) = self.auth.providers.get(id)
                && auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty())
            {
                providers.push((id.clone(), config.display_name.clone()));
            }
        }
        providers
    }

    /// Get models for a specific provider.
    pub fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)> {
        let mut models = Vec::new();
        if let Some(config) = self.config.providers.get(provider_id) {
            for (id, model_config) in &config.models {
                models.push((id.clone(), model_config.display_name.clone()));
            }
        }
        if models.is_empty()
            && let Some(config) = self.config.bundled_providers.get(provider_id)
        {
            for (id, model_config) in &config.models {
                models.push((id.clone(), model_config.display_name.clone()));
            }
        }
        models
    }

    /// Resolve the active model for a chat, falling back to the default.
    pub fn resolve_chat_model(&self, chat_key: &str) -> Result<ActiveModel> {
        if let Some((provider_id, model_id)) = self
            .store
            .load_gateway_chat_model(self.platform(), chat_key)?
        {
            match self
                .config
                .resolve_model_by_ids(&self.auth, &provider_id, &model_id)
            {
                Ok(mut model) => {
                    model.system_prompt = gateway_system_prompt();
                    return Ok(model);
                }
                Err(_) => {
                    self.store
                        .clear_gateway_chat_model(self.platform(), chat_key)?;
                }
            }
        }
        self.config.resolve_active_model_for_gateway(&self.auth)
    }

    // ── Shell command handling ─────────────────────────────────────────────

    /// Execute a `!` shell command, persist the result, and send it back.
    pub async fn handle_shell_command(
        &self,
        sender: &mut dyn MessageSender,
        recipient: &str,
        reply_to: Option<&str>,
        command: &str,
        chat_key: &str,
    ) -> Result<()> {
        let active_model = self.resolve_chat_model(chat_key)?;
        let conversation = self.load_or_create_conversation(chat_key, &active_model)?;
        let session_id = conversation.session_id;
        let (content, exit_code) = shell::execute_shell(command);
        let formatted = shell::format_shell_output(&content, exit_code);
        shell::persist_shell_messages(&self.store, session_id, command, &formatted)?;
        sender.send_message(recipient, &formatted, reply_to).await
    }

    // ── Command handling ───────────────────────────────────────────────────

    /// Handle a parsed slash command.
    ///
    /// Returns `true` if the command was handled (no further message processing needed).
    #[allow(clippy::too_many_arguments)]
    pub async fn handle_command(
        &mut self,
        sender: &mut dyn MessageSender,
        recipient: &str,
        reply_to: Option<&str>,
        chat_key: &str,
        conversation: &mut Conversation,
        active_model: &mut ActiveModel,
        command: crate::commands::CommandInvocation,
    ) -> Result<bool> {
        match command.name.as_str() {
            "new" => {
                *conversation = self.rotate_chat_session(chat_key, active_model)?;
                self.mode_manager.reset(chat_key);
                let mode = self.mode_manager.get(chat_key);
                sender
                    .send_message(
                        recipient,
                        &format!("Started a fresh session in {} mode.", mode.title()),
                        reply_to,
                    )
                    .await?;
                Ok(true)
            }
            "plan" | "p" => {
                self.mode_manager.set(chat_key, SessionMode::Plan);
                sender
                    .send_message(
                        recipient,
                        ModeManager::switch_message(SessionMode::Plan),
                        reply_to,
                    )
                    .await?;
                Ok(true)
            }
            "build" | "b" => {
                self.mode_manager.set(chat_key, SessionMode::Build);
                sender
                    .send_message(
                        recipient,
                        ModeManager::switch_message(SessionMode::Build),
                        reply_to,
                    )
                    .await?;
                Ok(true)
            }
            "mode" => {
                let mode = self.mode_manager.get(chat_key);
                sender
                    .send_message(
                        recipient,
                        &format!(
                            "Current mode: **{}** — {}",
                            mode.title(),
                            mode.description()
                        ),
                        reply_to,
                    )
                    .await?;
                Ok(true)
            }
            "session" => {
                match command.args.first().map(|s| s.as_str()) {
                    None | Some("") => {
                        let text = self.format_session_summary(conversation, active_model);
                        sender.send_message(recipient, &text, reply_to).await?;
                    }
                    _ => {
                        sender
                            .send_message(recipient, &gateway_help_text(), reply_to)
                            .await?;
                    }
                }
                Ok(true)
            }
            "model" => {
                // Note: Model selection is handled by the channel directly because
                // it needs to implement ModelSelectionIO (which requires Send)
                // We delegate back through the sender or handle it separately.
                sender
                    .send_message(recipient, "Use /model in your platform channel.", reply_to)
                    .await?;
                Ok(true)
            }
            "help" => {
                sender
                    .send_message(recipient, &gateway_help_text(), reply_to)
                    .await?;
                Ok(true)
            }
            "status" => {
                let text = self.format_session_status(conversation, active_model);
                sender.send_message(recipient, &text, reply_to).await?;
                Ok(true)
            }
            "stop" => {
                let chat_id = recipient.to_string();
                if let Some(token) = self.cancellation_tokens.get(&chat_id) {
                    token.cancel();
                    sender
                        .send_message(recipient, "🛑 Stopping...", reply_to)
                        .await?;
                } else {
                    sender
                        .send_message(recipient, "No active task to stop.", reply_to)
                        .await?;
                }
                Ok(true)
            }
            "compact" => {
                if self.compacting_sessions.contains(&conversation.session_id) {
                    sender
                        .send_message(recipient, "⏳ Already compacting this session...", reply_to)
                        .await?;
                    return Ok(true);
                }
                self.compacting_sessions.insert(conversation.session_id);

                // Save prior state for compaction message metadata (used by undo).
                let prior_summary = conversation.context_summary.clone();
                let prior_retained_from = conversation.context_retained_from;
                let session_id = conversation.session_id;

                // Build context manager from existing state so we preserve the
                // current summary (if any).  Creating a fresh ContextManager
                // would lose it and break prefix caching.
                let mut context_manager = tidev_engine::context::ContextManager::from_state(
                    conversation.context_summary.clone(),
                    conversation.context_retained_from,
                );

                self.tools.set_active_model(active_model.clone());
                let tools = self.tools.all_definitions();
                let current_mode = self.mode_manager.get(chat_key);

                sender
                    .send_message(
                        recipient,
                        "Compacting session context... This may take a moment.",
                        reply_to,
                    )
                    .await?;

                // Run compaction inline (non-streaming).  This is safe because
                // /compact is a low-frequency manual command; the LLM call is
                // the dominant latency and blocking the handler for a few
                // seconds is acceptable.
                let result = context_manager
                    .compact(tidev_engine::context::CompactionConfig {
                        llm: &self.llm,
                        model: active_model,
                        conversation: &*conversation,
                        manual: true,
                        stream_ctx: None,
                        tools: &tools,
                        mode: current_mode,
                    })
                    .await;

                match result {
                    Ok(true) => {
                        if let Some(summary) = &context_manager.summary {
                            let mut compact_msg = Message::compaction(summary);
                            compact_msg.metadata.prior_summary = prior_summary;
                            compact_msg.metadata.prior_retained_from =
                                Some(prior_retained_from);

                            let _ = self.store.append_message(session_id, &compact_msg);
                            let _ = self.store.update_session_context_state(
                                session_id,
                                Some(summary),
                                context_manager.retained_from,
                            );
                        }
                        sender
                            .send_message(recipient, "✅ Session context compacted.", reply_to)
                            .await?;
                    }
                    Ok(false) => {
                        sender
                            .send_message(
                                recipient,
                                "ℹ️ No compaction needed (context already compact).",
                                reply_to,
                            )
                            .await?;
                    }
                    Err(e) => {
                        let text = format!("❌ Compaction failed: {}", e);
                        sender
                            .send_message(recipient, &text, reply_to)
                            .await?;
                    }
                }

                self.compacting_sessions.remove(&session_id);
                Ok(true)
            }
            "init" => {
                let init_prompt = tidev_types::prompts::init_command();
                let text = format!(
                    "📁 Project Analysis Prompt\n\nCopy and send this prompt to analyze your project:\n\n```\n{}\n```",
                    init_prompt
                );
                sender.send_message(recipient, &text, reply_to).await?;
                Ok(true)
            }
            _ => {
                sender
                    .send_message(
                        recipient,
                        &format!(
                            "Unknown command: {}\n\n{}",
                            command.name,
                            &gateway_help_text()
                        ),
                        reply_to,
                    )
                    .await?;
                Ok(true)
            }
        }
    }

    // ── Status helpers ─────────────────────────────────────────────────────

    pub fn format_session_status(
        &self,
        conversation: &Conversation,
        active_model: &ActiveModel,
    ) -> String {
        let user_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .count();
        let asst_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();
        let tool_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Tool)
            .count();
        let token_stats = self
            .store
            .get_session_token_stats(conversation.session_id)
            .unwrap_or(tidev_engine::storage::SessionTokenStats {
                input_tokens: 0,
                output_tokens: 0,
            });

        format_status_summary(&crate::commands::SessionStats {
            session_id: &conversation.session_id.to_string(),
            title: &conversation.title,
            message_count: conversation.messages.len(),
            user_message_count: user_count,
            assistant_message_count: asst_count,
            tool_call_count: tool_count,
            provider_id: &active_model.provider_id,
            model_id: &active_model.model_id,
            context_window: active_model.context_window,
            input_tokens: token_stats.input_tokens,
            output_tokens: token_stats.output_tokens,
            start_time: self.start_time,
            avg_response_time_ms: None,
        })
    }

    pub fn format_session_summary(
        &self,
        conversation: &Conversation,
        active_model: &ActiveModel,
    ) -> String {
        format!(
            "Session status\n- session_id: {}\n- title: {}\n- message_count: {}\n- model: {}/{}",
            conversation.session_id,
            conversation.title,
            conversation.messages.len(),
            active_model.provider_id,
            active_model.model_id
        )
    }

    /// Check whether a chat_id is in the allowlist.
    pub fn is_allowed(&self, chat_id: &str) -> bool {
        self.allowlist.contains(chat_id) || self.allowlist.contains("*")
    }

    /// Run the basic agent loop (simple version, without streaming draft editing).
    #[allow(clippy::too_many_arguments)]
    pub async fn run_agent_loop_simple(
        &mut self,
        sender: &mut dyn MessageSender,
        recipient: &str,
        reply_to: Option<&str>,
        chat_key: &str,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
    ) -> Result<()> {
        log::info!(
            "Agent: recipient={}, model={}, session={}",
            recipient,
            active_model.label(),
            conversation.session_id
        );

        let cancel_token = CancellationToken::new();
        self.cancellation_tokens
            .insert(recipient.to_string(), cancel_token.clone());

        self.tools.set_active_model(active_model.clone());

        let mut context_manager = tidev_engine::context::ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );

        let session_id = conversation.session_id;
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let current_mode = self.mode_manager.get(chat_key);

        let result = self
            .agent
            .run_agent_loop(tidev_engine::agent::runtime::AgentLoopConfig {
                session_id,
                model: active_model.clone(),
                context_manager: &mut context_manager,
                mode: current_mode,
                thinking_level: active_model.thinking_level.clone(),
                event_tx,
                cancel_token: Some(cancel_token.clone()),
            })
            .await;

        self.cancellation_tokens.remove(recipient);

        if let Err(ref e) = result {
            log::error!("Agent loop failed: {}", e);
            sender
                .send_message(recipient, &format!("Error: {e}"), reply_to)
                .await?;
            return Ok(());
        }

        if cancel_token.is_cancelled() {
            sender
                .send_message(recipient, "🛑 Task stopped.", reply_to)
                .await?;
            return Ok(());
        }

        if let Ok(messages) = self.store.load_messages(session_id)
            && let Some(last_msg) = messages.last()
            && last_msg.role == MessageRole::Assistant
            && !last_msg.content.trim().is_empty()
        {
            let final_text = normalize_assistant_output(&last_msg.content);
            sender
                .send_message(recipient, &final_text, reply_to)
                .await?;
        }

        Ok(())
    }

    // ── Balance query helpers (used by Telegram) ─────────────────────────

    /// List providers that support balance queries and have configured API keys.
    pub fn get_balance_providers(&self) -> Vec<(&str, &str)> {
        let mut providers = Vec::new();
        if self.auth.api_key("deepseek").is_some() {
            providers.push(("deepseek", "DeepSeek"));
        }
        if self.auth.api_key("siliconflow-cn").is_some() {
            providers.push(("siliconflow-cn", "SiliconFlow"));
        }
        providers
    }

    /// Format DeepSeek balance for display.
    pub fn format_deepseek_balance(
        &self,
        balance: &tidev_session::balance::DeepSeekBalanceResponse,
    ) -> String {
        let mut text = String::from("💰 DeepSeek Balance\n\n");
        if !balance.is_available {
            text.push_str("Account is not available.\n");
            return text;
        }
        for info in &balance.balance_infos {
            text.push_str(&format!("Currency: {}\n", info.currency));
            text.push_str(&format!(
                "Total: {} {}\n",
                info.total_balance, info.currency
            ));
            text.push_str(&format!(
                "Granted: {} {}\n",
                info.granted_balance, info.currency
            ));
            text.push_str(&format!(
                "Topped Up: {} {}\n",
                info.topped_up_balance, info.currency
            ));
        }
        text
    }

    /// Format SiliconFlow balance for display.
    pub fn format_siliconflow_balance(
        &self,
        balance: &tidev_session::balance::SiliconFlowBalanceResponse,
    ) -> String {
        format!(
            "💰 SiliconFlow Balance\n\nTotal: {} CNY",
            balance.data.total_balance
        )
    }
}

fn normalize_assistant_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed.to_string()
    }
}
