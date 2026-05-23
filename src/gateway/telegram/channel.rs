//! Telegram channel implementation.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::time::{Duration, sleep};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    session::{BackendEvent, Conversation, MessageRole},
    storage::SessionStore,
};

use crate::gateway::channel::Channel;
use crate::gateway::channel::SendMessage;
use crate::gateway::channel_core::{ChannelCore, MessageSender};
use crate::gateway::commands::{GATEWAY_COMMANDS, parse_command};
use crate::gateway::model_selection::{self, ModelSelectionIO, ModelSelectionState};
use crate::gateway::telegram::bot::TelegramBot;
use crate::gateway::telegram::types::TelegramMessage;

pub const GATEWAY_PLATFORM_TELEGRAM: &str = "telegram";
pub const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_DRAFT_EDIT_INTERVAL_MS: u64 = 1200;

/// Interactive balance selection state for a chat.
#[derive(Debug, Clone)]
enum BalanceSelectionState {
    WaitingForProvider,
}

/// Telegram gateway channel implementation.
pub struct TelegramChannel {
    pub core: ChannelCore,
    pub bot: TelegramBot,
    pub poll_timeout_secs: u64,
    pub offset: i64,
    pub request_seq: u64,
    /// Telegram-specific balance selection state.
    balance_selection_states: HashMap<i64, BalanceSelectionState>,
}

impl TelegramChannel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_root: std::path::PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: crate::llm::LlmClient,
        tools: crate::tooling::ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        poll_timeout_secs: u64,
        bot_token: String,
        paths: &ConfigPaths,
    ) -> Self {
        let core = ChannelCore::new(
            GATEWAY_PLATFORM_TELEGRAM,
            workspace_root,
            config,
            auth,
            paths,
            store,
            llm,
            tools,
            instruction_prompt,
            allowlist,
        );
        Self {
            core,
            bot: TelegramBot::new(bot_token),
            poll_timeout_secs,
            offset: 0,
            request_seq: 0,
            balance_selection_states: HashMap::new(),
        }
    }

    // ── Telegram-specific event loop ──────────────────────────────────────

    async fn bootstrap_offset(&mut self) -> Result<()> {
        crate::log_info!("Telegram bootstrapping offset...");
        let updates = self.bot.get_updates(0, 0).await?;
        if let Some(last) = updates.last() {
            self.offset = last.update_id.saturating_add(1);
            crate::log_info!("Telegram bootstrap offset set to {}", self.offset);
        }
        self.register_commands().await?;
        Ok(())
    }

    async fn register_commands(&self) -> Result<()> {
        let commands: Vec<(String, String)> = GATEWAY_COMMANDS
            .iter()
            .map(|spec| (spec.name.to_string(), spec.description.to_string()))
            .collect();
        self.bot.set_my_commands(commands).await?;
        Ok(())
    }

    async fn run_loop(&mut self) -> Result<()> {
        loop {
            let updates = match self
                .bot
                .get_updates(self.offset, self.poll_timeout_secs)
                .await
            {
                Ok(updates) => updates,
                Err(error) => {
                    crate::log_error!("Telegram getUpdates failed: {error}");
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            if updates.is_empty() {
                continue;
            }

            for update in updates {
                self.offset = update.update_id.saturating_add(1);
                let Some(message) = update.message else {
                    continue;
                };

                crate::log_info!(
                    "Received message: chat_id={}, msg_id={}, user={}",
                    message.chat.id,
                    message.message_id,
                    message.from.as_ref().map(|u| &u.id).unwrap_or(&0)
                );

                if let Err(error) = self.handle_message(message).await {
                    crate::log_error!("Message handling failed: {error}");
                }
            }
        }
    }

    // ── Message handling ──────────────────────────────────────────────────

    async fn handle_message(&mut self, message: TelegramMessage) -> Result<()> {
        // Allowlist check
        let chat_id = message.chat.id;
        let chat_id_str = chat_id.to_string();
        if !self.core.is_allowed(&chat_id_str) {
            let allowed_by_user = message
                .from
                .as_ref()
                .map(|u| {
                    self.core.is_allowed(&u.id.to_string())
                        || u.username
                            .as_ref()
                            .is_some_and(|un| self.core.is_allowed(un))
                })
                .unwrap_or(false);
            if !allowed_by_user {
                crate::log_debug!(
                    "Message from chat_id={} not in allowlist, skipping",
                    chat_id
                );
                return Ok(());
            }
        }

        // Add receipt reaction
        if let Err(e) = self
            .bot
            .set_message_reaction(chat_id, message.message_id, "👀")
            .await
        {
            crate::log_warn!(
                "Failed to set message reaction for chat_id={}: {}",
                chat_id,
                e
            );
        }

        let content = message
            .text
            .as_deref()
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Ok(());
        }

        // Balance selection state
        if let Some(state) = self.balance_selection_states.get(&chat_id).cloned() {
            self.balance_selection_states.remove(&chat_id);
            return self
                .handle_balance_selection(&message, &state, &content)
                .await;
        }

        // Model selection state
        if let Some(state) = self.core.model_selection_states.get(&chat_id_str).cloned() {
            crate::log_info!("Handling model selection input: chat_id={}", chat_id);
            return self
                .handle_model_selection(&message, &state, &content)
                .await;
        }

        // Shell command
        if let Some(cmd) = content.strip_prefix('!') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                return self.handle_shell_command(&message, cmd).await;
            }
        }

        // Parse command
        if let Some(command) = parse_command(&content) {
            crate::log_info!(
                "Telegram executing command: /{} {:?}",
                command.name,
                command.args
            );
            let chat_key = self.chat_key(&message);
            let mut active_model = self.core.resolve_chat_model(&chat_key)?;
            let mut conversation = self
                .core
                .load_or_create_conversation(&chat_key, &active_model)?;
            self.core
                .load_system_prompt(&conversation, &mut active_model);
            self.core
                .mode_manager
                .restore_from_messages(&chat_key, &conversation.messages);

            let mut sender = TelegramSender {
                bot: &self.bot,
                chat_id,
                thread_id: message.message_thread_id,
                reply_to_msg_id: Some(message.message_id),
            };

            // Handle balance and model commands specially (platform-specific ModelSelectionIO)
            if command.name == "balance" {
                return self.handle_balance_command(&message).await;
            }
            if command.name == "model" {
                return self.handle_model_command(&message).await;
            }

            let handled = self
                .core
                .handle_command(
                    &mut sender,
                    &chat_id_str,
                    Some(&message.message_id.to_string()),
                    &chat_key,
                    &mut conversation,
                    &mut active_model,
                    command,
                )
                .await?;

            if handled {
                return Ok(());
            }
        }

        // Regular message → run agent with streaming
        let chat_key = self.chat_key(&message);
        let mut active_model = self.core.resolve_chat_model(&chat_key)?;
        let mut conversation = self
            .core
            .load_or_create_conversation(&chat_key, &active_model)?;
        self.core
            .load_system_prompt(&conversation, &mut active_model);
        self.core
            .mode_manager
            .restore_from_messages(&chat_key, &conversation.messages);
        self.core
            .persist_user_message(&mut conversation, &chat_key, &content)?;

        self.run_agent_with_tools(&message, &mut conversation, &active_model)
            .await
    }

    // ── Shell command ─────────────────────────────────────────────────────

    async fn handle_shell_command(
        &mut self,
        message: &TelegramMessage,
        command: &str,
    ) -> Result<()> {
        let chat_key = self.chat_key(message);
        let active_model = self.core.resolve_chat_model(&chat_key)?;
        let conversation = self
            .core
            .load_or_create_conversation(&chat_key, &active_model)?;
        let session_id = conversation.session_id;

        let (content, exit_code) = crate::gateway::shell::execute_shell(command);
        let formatted_db = crate::gateway::shell::format_shell_output(&content, exit_code);
        let formatted_html = crate::gateway::shell::format_shell_output_html(&content, exit_code);

        crate::gateway::shell::persist_shell_messages(
            &self.core.store,
            session_id,
            command,
            &formatted_db,
        )?;

        self.send_reply_chunks(message, &formatted_html).await
    }

    // ── Agent loop with streaming ─────────────────────────────────────────

    /// Run the full agent loop with Telegram-specific streaming draft editing.
    async fn run_agent_with_tools(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
    ) -> Result<()> {
        crate::log_info!(
            "Telegram agent: chat_id={}, model={}, session={}",
            source_message.chat.id,
            active_model.label(),
            conversation.session_id
        );

        // Build recipient string
        let recipient = if let Some(thread_id) = source_message.message_thread_id {
            format!("{}:{}", source_message.chat.id, thread_id)
        } else {
            source_message.chat.id.to_string()
        };

        // Send initial draft message
        let draft_message_id = match self.send_draft_message(&recipient, "Thinking...").await? {
            Some(id) => id,
            None => {
                self.send_reply_chunks(source_message, "Thinking...")
                    .await?;
                return Ok(());
            }
        };

        // Set up cancellation
        let cancel_token = CancellationToken::new();
        self.core
            .cancellation_tokens
            .insert(source_message.chat.id.to_string(), cancel_token.clone());

        // Ensure tools have the active model
        self.core.tools.set_active_model(active_model.clone());

        // Build context manager
        let mut context_manager = crate::context::ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );

        let session_id = conversation.session_id;
        let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

        // Clone resources for event handler task
        let bot = self.bot.clone();
        let chat_id = source_message.chat.id;
        let thread_id = source_message.message_thread_id;
        let msg_id = source_message.message_id;
        let draft_id = draft_message_id.clone();

        // Spawn event handler for real-time draft updates and tool results
        let event_handle = tokio::task::spawn_local(async move {
            let mut streamed = String::new();
            let mut last_edit =
                Instant::now() - Duration::from_millis(TELEGRAM_DRAFT_EDIT_INTERVAL_MS);
            let draft_id: i64 = match draft_id.parse() {
                Ok(id) => id,
                Err(_) => return,
            };

            while let Some(event) = event_rx.recv().await {
                match event {
                    BackendEvent::Delta { content, .. } => {
                        streamed.push_str(&content);
                        let preview = preview_for_streaming(&streamed);
                        let now = Instant::now();
                        if now.duration_since(last_edit).as_millis() as u64
                            >= TELEGRAM_DRAFT_EDIT_INTERVAL_MS
                            || streamed.len() >= TELEGRAM_MAX_MESSAGE_LENGTH
                        {
                            let _ = bot.edit_message_text_html(0, draft_id, &preview).await;
                            last_edit = now;
                        }
                    }
                    BackendEvent::ToolCompleted {
                        tool_call, result, ..
                    } => {
                        let display = result.preview_for_storage(Some(tool_call.name.as_str()));
                        let text = format!(
                            "🔧 <b>{}</b>\n<pre><code class=\"language-text\">{}</code></pre>",
                            tool_call.name,
                            truncate_for_html(&display.output)
                        );
                        let _ = bot
                            .send_message_html(chat_id, thread_id, &text, Some(msg_id))
                            .await;
                    }
                    _ => {}
                }
            }
        });

        // Run the agent loop
        let chat_key = self.chat_key(source_message);
        let current_mode = self.core.mode_manager.get(&chat_key);
        let result = self
            .core
            .agent
            .run_agent_loop(crate::agent::runtime::AgentLoopConfig {
                session_id,
                model: active_model.clone(),
                context_manager: &mut context_manager,
                mode: current_mode,
                thinking_level: active_model.thinking_level.clone(),
                event_tx,
                cancel_token: Some(cancel_token.clone()),
            })
            .await;

        // Clean up cancellation token
        self.core
            .cancellation_tokens
            .remove(&source_message.chat.id.to_string());

        // Wait for event handler
        let _ = event_handle.await;

        // Handle errors
        if let Err(ref e) = result {
            crate::log_error!("Telegram agent loop failed: {}", e);
            let error_msg = format!("Error: {e}");
            let _ = self
                .cancel_draft_message(&recipient, &draft_message_id)
                .await;
            self.send_reply_chunks(source_message, &error_msg).await?;
            return Ok(());
        }

        // Handle cancellation
        if cancel_token.is_cancelled() {
            self.send_reply_chunks(source_message, "Stopped.").await?;
            return Ok(());
        }

        // Send final response
        if let Ok(messages) = self.core.store.load_messages(session_id)
            && let Some(last_msg) = messages.last()
            && last_msg.role == MessageRole::Assistant
            && !last_msg.content.trim().is_empty()
        {
            let final_text = normalize_assistant_output(&last_msg.content);

            if last_msg.tool_calls.is_empty() {
                // No tool calls — finalize the draft message
                if self
                    .finalize_draft_message(&recipient, &draft_message_id, &final_text)
                    .await
                    .is_err()
                {
                    self.send_reply_chunks(source_message, &final_text).await?;
                }
            } else {
                // Had tool calls — delete draft and send as new message
                let _ = self
                    .cancel_draft_message(&recipient, &draft_message_id)
                    .await;
                self.send_reply_chunks(source_message, &final_text).await?;
            }
        }

        Ok(())
    }

    // ── Balance command (Telegram-specific) ───────────────────────────────

    async fn handle_balance_command(&mut self, message: &TelegramMessage) -> Result<()> {
        let providers = self.get_balance_providers();

        if providers.is_empty() {
            self.send_reply_chunks(
                message,
                "No providers available for balance queries.\nConfigure API keys for DeepSeek or SiliconFlow.",
            ).await?;
            return Ok(());
        }

        let mut text = String::from("Select a provider to query balance (enter number):\n\n");
        for (i, (_, name)) in providers.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, name));
        }
        text.push_str("\n(Enter any other number to cancel)");

        self.send_reply_chunks(message, &text).await?;
        self.balance_selection_states
            .insert(message.chat.id, BalanceSelectionState::WaitingForProvider);
        Ok(())
    }

    fn get_balance_providers(&self) -> Vec<(&str, &str)> {
        let mut providers = Vec::new();
        if self.core.auth.api_key("deepseek").is_some() {
            providers.push(("deepseek", "DeepSeek"));
        }
        if self.core.auth.api_key("siliconflow-cn").is_some() {
            providers.push(("siliconflow-cn", "SiliconFlow"));
        }
        providers
    }

    async fn handle_balance_selection(
        &mut self,
        message: &TelegramMessage,
        _state: &BalanceSelectionState,
        content: &str,
    ) -> Result<()> {
        let providers = self.get_balance_providers();
        let selection: usize = match content.trim().parse() {
            Ok(n) => n,
            Err(_) => {
                self.send_reply_chunks(message, "Invalid number. Balance query cancelled.")
                    .await?;
                return Ok(());
            }
        };

        if selection == 0 || selection > providers.len() {
            self.send_reply_chunks(message, "Balance query cancelled.")
                .await?;
            return Ok(());
        }

        let (provider_id, _) = providers[selection - 1];
        let http = self.core.llm.http();
        let api_key = self
            .core
            .auth
            .api_key(provider_id)
            .context("API key not found")?;

        match provider_id {
            "deepseek" => match crate::balance::query_deepseek_balance(http, api_key).await {
                Ok(balance) => {
                    let text = self.core.format_deepseek_balance(&balance);
                    self.send_reply_chunks(message, &text).await?;
                }
                Err(e) => {
                    self.send_reply_chunks(
                        message,
                        &format!("❌ Failed to query DeepSeek balance: {e}"),
                    )
                    .await?;
                }
            },
            "siliconflow-cn" => {
                match crate::balance::query_siliconflow_balance(http, api_key).await {
                    Ok(balance) => {
                        let text = self.core.format_siliconflow_balance(&balance);
                        self.send_reply_chunks(message, &text).await?;
                    }
                    Err(e) => {
                        self.send_reply_chunks(
                            message,
                            &format!("❌ Failed to query SiliconFlow balance: {e}"),
                        )
                        .await?;
                    }
                }
            }
            _ => {
                self.send_reply_chunks(
                    message,
                    &format!(
                        "Balance query for '{}' is not yet implemented.",
                        provider_id
                    ),
                )
                .await?;
            }
        }

        Ok(())
    }

    // ── Model selection ───────────────────────────────────────────────────

    async fn handle_model_command(&mut self, message: &TelegramMessage) -> Result<()> {
        model_selection::start_model_selection(self, &message.chat.id).await
    }

    async fn handle_model_selection(
        &mut self,
        message: &TelegramMessage,
        state: &ModelSelectionState,
        content: &str,
    ) -> Result<()> {
        model_selection::handle_step(self, &message.chat.id, state, content).await
    }

    // ── Chat helpers ──────────────────────────────────────────────────────

    fn chat_key(&self, message: &TelegramMessage) -> String {
        match message.message_thread_id {
            Some(thread_id) => format!("{}:{}", message.chat.id, thread_id),
            None => message.chat.id.to_string(),
        }
    }

    // ── Sending ───────────────────────────────────────────────────────────
    async fn send_reply_chunks(&self, message: &TelegramMessage, text: &str) -> Result<()> {
        let chunks = split_message_for_telegram(text);
        for (index, chunk) in chunks.iter().enumerate() {
            self.bot
                .send_message_html(
                    message.chat.id,
                    message.message_thread_id,
                    chunk,
                    if index == 0 {
                        Some(message.message_id)
                    } else {
                        None
                    },
                )
                .await?;
        }
        Ok(())
    }

    async fn send_draft_message(&mut self, recipient: &str, text: &str) -> Result<Option<String>> {
        let chat_id: i64 = recipient.parse::<i64>().unwrap_or(0);
        let sent = self.bot.send_message(chat_id, None, text, None).await?;
        Ok(Some(sent.message_id.to_string()))
    }

    async fn finalize_draft_message(
        &mut self,
        _recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await
    }

    async fn cancel_draft_message(&mut self, _recipient: &str, message_id: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.delete_message(0, msg_id).await
    }
}

// ── Telegram MessageSender ──────────────────────────────────────────────────

struct TelegramSender<'a> {
    bot: &'a TelegramBot,
    chat_id: i64,
    thread_id: Option<i64>,
    reply_to_msg_id: Option<i64>,
}

#[async_trait]
impl MessageSender for TelegramSender<'_> {
    async fn send_message(
        &mut self,
        _recipient: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        // Split long messages into chunks for Telegram
        let chunks = split_message_for_telegram(text);
        for (index, chunk) in chunks.iter().enumerate() {
            self.bot
                .send_message_html(
                    self.chat_id,
                    self.thread_id,
                    chunk,
                    if index == 0 {
                        self.reply_to_msg_id
                    } else {
                        None
                    },
                )
                .await?;
        }
        Ok(())
    }
}

// ── ModelSelectionIO for TelegramChannel ────────────────────────────────────

#[async_trait]
impl ModelSelectionIO for TelegramChannel {
    type Id = i64;

    async fn send_message(&mut self, id: &i64, text: &str) -> Result<()> {
        self.bot.send_message_html(*id, None, text, None).await?;
        Ok(())
    }

    fn get_state(&self, id: &i64) -> Option<ModelSelectionState> {
        self.core
            .model_selection_states
            .get(&id.to_string())
            .cloned()
    }

    fn set_state(&mut self, id: i64, state: ModelSelectionState) {
        self.core
            .model_selection_states
            .insert(id.to_string(), state);
    }

    fn remove_state(&mut self, id: &i64) {
        self.core.model_selection_states.remove(&id.to_string());
    }

    fn chat_key(&self, id: &i64) -> String {
        format!("{}:{}", self.core.platform_name, id)
    }

    fn platform(&self) -> &'static str {
        GATEWAY_PLATFORM_TELEGRAM
    }
    fn config(&self) -> &AppConfig {
        &self.core.config
    }
    fn config_mut(&mut self) -> &mut AppConfig {
        &mut self.core.config
    }
    fn config_paths(&self) -> &ConfigPaths {
        &self.core.config_paths
    }
    fn auth(&self) -> &AuthStore {
        &self.core.auth
    }
    fn store(&self) -> &SessionStore {
        &self.core.store
    }

    fn get_available_providers(&self) -> Vec<(String, String)> {
        self.core.get_available_providers()
    }

    fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)> {
        self.core.get_models_for_provider(provider_id)
    }

    fn resolve_chat_model(&self, chat_key: &str) -> Result<ActiveModel> {
        self.core.resolve_chat_model(chat_key)
    }
}

// ── Channel trait implementation ────────────────────────────────────────────

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_TELEGRAM
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.core.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            crate::log_info!("Telegram channel ready");
            self.bootstrap_offset().await?;
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        self.core.restore_sessions(store)
    }

    fn supports_draft_updates(&self) -> bool {
        true
    }

    async fn send_draft(&mut self, message: &SendMessage) -> Result<Option<String>> {
        let chat_id: i64 = message.recipient.parse().unwrap_or(0);
        let sent = self
            .bot
            .send_message(chat_id, None, &message.content, None)
            .await?;
        Ok(Some(sent.message_id.to_string()))
    }

    async fn update_draft(&mut self, _recipient: &str, message_id: &str, text: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await
    }

    async fn update_draft_progress(
        &mut self,
        _recipient: &str,
        message_id: &str,
        status: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, status).await
    }

    async fn finalize_draft(
        &mut self,
        _recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await
    }

    async fn cancel_draft(&mut self, _recipient: &str, message_id: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.delete_message(0, msg_id).await
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

fn normalize_assistant_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed.to_string()
    }
}

fn truncate_for_html(value: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut out = String::new();
    for ch in value.chars().take(MAX_CHARS) {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    if value.chars().count() > MAX_CHARS {
        out.push_str("\n... (truncated)");
    }
    out
}

fn preview_for_streaming(text: &str) -> String {
    let normalized = normalize_assistant_output(text);
    if normalized.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return normalized;
    }
    let mut preview: String = normalized
        .chars()
        .take(TELEGRAM_MAX_MESSAGE_LENGTH.saturating_sub(3))
        .collect();
    preview.push_str("...");
    preview
}

fn split_message_for_telegram(message: &str) -> Vec<String> {
    if message.trim().is_empty() {
        return vec!["(no content)".to_string()];
    }
    if message.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return vec![message.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = message;

    while !remaining.is_empty() {
        if remaining.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
            chunks.push(remaining.to_string());
            break;
        }

        let split_at = remaining
            .char_indices()
            .nth(TELEGRAM_MAX_MESSAGE_LENGTH)
            .map_or(remaining.len(), |(idx, _)| idx);
        let search_area = &remaining[..split_at];
        let chunk_end = search_area
            .rfind('\n')
            .or_else(|| search_area.rfind(' '))
            .unwrap_or(split_at);

        if chunk_end == 0 {
            let chunk = &remaining[..split_at].trim();
            if !chunk.is_empty() {
                chunks.push(chunk.to_string());
            }
            remaining = remaining[split_at..].trim_start();
        } else {
            let chunk = &remaining[..chunk_end].trim();
            if !chunk.is_empty() {
                chunks.push(chunk.to_string());
            }
            remaining = remaining[chunk_end..].trim_start();
        }
    }

    if chunks.is_empty() {
        vec!["(no content)".to_string()]
    } else {
        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_message_respects_telegram_limit() {
        let source = "x".repeat(TELEGRAM_MAX_MESSAGE_LENGTH + 50);
        let chunks = split_message_for_telegram(&source);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|chunk| {
            chunk.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH && !chunk.is_empty()
        }));
        assert_eq!(chunks.concat(), source);
    }
}
