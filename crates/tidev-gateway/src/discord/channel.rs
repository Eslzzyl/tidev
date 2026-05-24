//! Discord channel implementation.
//!
//! Connects to the Discord Gateway via WebSocket for real-time message
//! reception and uses the REST API for sending replies.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use rand::RngExt;
use serde_json::json;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};

use tidev_engine::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use tidev_engine::storage::SessionStore;

use crate::channel::Channel;
use crate::channel_core::{ChannelCore, MessageSender};
use crate::commands::parse_command;
use crate::model_selection::{self, ModelSelectionIO, ModelSelectionState};
use tidev_engine::session::{Message, MessageRole};

use super::client::{DISCORD_MAX_MESSAGE_LENGTH, DiscordClient};
use super::types::{
    DEFAULT_INTENTS, DiscordMessage, GatewayPayload, HelloData, OP_DISPATCH, OP_HEARTBEAT,
    OP_HEARTBEAT_ACK, OP_HELLO, OP_IDENTIFY,
};

pub const GATEWAY_PLATFORM_DISCORD: &str = "discord";
const DISCORD_DRAFT_EDIT_INTERVAL_MS: u64 = 1200;

/// Random acknowledgement emoji reactions.
const ACK_REACTIONS: &[&str] = &["👀", "✅", "👍", "👋", "🤔", "💭", "✨"];

/// Discord gateway channel implementation.
pub struct DiscordChannel {
    pub core: ChannelCore,
    pub client: DiscordClient,
    pub bot_token: String,
    pub bot_user_id: String,
    pub guild_ids: Vec<String>,
    pub channel_ids: Vec<String>,
    pub mention_only: bool,
    pub last_seq: Option<u32>,
    last_heartbeat_ack: Instant,
}

impl DiscordChannel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: tidev_engine::llm::LlmClient,
        tools: tidev_engine::tooling::ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        bot_token: String,
        guild_ids: Vec<String>,
        channel_ids: Vec<String>,
        mention_only: bool,
        paths: &ConfigPaths,
    ) -> Self {
        let bot_user_id = Self::extract_bot_user_id(&bot_token);
        let core = ChannelCore::new(
            GATEWAY_PLATFORM_DISCORD,
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
            client: DiscordClient::new(bot_token.clone()),
            bot_token,
            bot_user_id: bot_user_id.unwrap_or_default(),
            guild_ids,
            channel_ids,
            mention_only,
            last_seq: None,
            last_heartbeat_ack: Instant::now(),
        }
    }

    /// Extract the bot user ID from a Discord bot token.
    /// Discord tokens are base64(user_id).timestamp.hmac
    fn extract_bot_user_id(token: &str) -> Option<String> {
        use base64::Engine as _;
        let part = token.split('.').next()?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(part)
            .ok()?;
        let id = String::from_utf8_lossy(&bytes).to_string();
        // Ensure it's numeric
        if id.chars().all(|c| c.is_ascii_digit()) {
            Some(id)
        } else {
            None
        }
    }

    /// Check if a channel passes the guild/channel filter.
    fn channel_passes_filter(&self, guild_id: Option<&str>, channel_id: &str) -> bool {
        // Guild filter
        if !self.guild_ids.is_empty() {
            match guild_id {
                Some(gid) if !self.guild_ids.iter().any(|g| g == gid) => return false,
                None => return false, // DM but guild filter is set
                _ => {}
            }
        }
        // Channel filter
        if !self.channel_ids.is_empty() && !self.channel_ids.iter().any(|c| c == channel_id) {
            return false;
        }
        true
    }

    /// Check if the bot was mentioned in the message content.
    fn is_bot_mentioned(content: &str, bot_user_id: &str) -> bool {
        let mention = format!("<@{}>", bot_user_id);
        content.contains(&mention)
    }

    /// Pick a random reaction emoji.
    fn random_ack_reaction() -> &'static str {
        let mut rng = rand::rng();
        let idx = rng.random_range(0..ACK_REACTIONS.len());
        ACK_REACTIONS[idx]
    }

    /// Split long content into chunks at paragraph boundaries (Discord 2000 char limit).
    fn chunk_content(content: &str) -> Vec<String> {
        if content.len() <= DISCORD_MAX_MESSAGE_LENGTH {
            return vec![content.to_string()];
        }

        let mut chunks = Vec::new();
        let mut start = 0;
        let bytes = content.as_bytes();

        while start < bytes.len() {
            let end = (start + DISCORD_MAX_MESSAGE_LENGTH).min(bytes.len());
            // Try to break at a paragraph boundary (double newline)
            let mut split_at = end;
            if end < bytes.len() {
                // Search backwards for \n\n
                if let Some(pos) = content[start..end].rfind("\n\n") {
                    split_at = start + pos + 2; // include the \n\n
                } else if let Some(pos) = content[start..end].rfind('\n') {
                    split_at = start + pos + 1;
                }
                // Make sure we make progress
                if split_at <= start {
                    split_at = end;
                }
            }
            chunks.push(content[start..split_at].to_string());
            start = split_at;
        }

        chunks
    }

    // ── WebSocket event loop ─────────────────────────────────────────────────

    async fn run_loop(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.connect_and_listen().await {
                log::error!("Discord connection error: {e}, reconnecting in 5s...");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn connect_and_listen(&mut self) -> Result<()> {
        let gw_url = self.client.get_gateway_url().await?;
        let ws_url = format!("{gw_url}/?v=10&encoding=json");

        log::info!("Discord connecting to gateway...");
        let (ws_stream, _) = connect_async(&ws_url).await?;
        let (mut write, mut read) = ws_stream.split();

        // Read Hello (opcode 10)
        let hello_msg = read
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("Discord: gateway closed before Hello"))??;

        let hello_payload: GatewayPayload = serde_json::from_str(&hello_msg.to_string())?;
        if hello_payload.op != OP_HELLO {
            anyhow::bail!(
                "Discord: expected Hello (op 10), got op {}",
                hello_payload.op
            );
        }
        let hello_data: HelloData =
            serde_json::from_value(hello_payload.d.context("Discord: Hello missing data")?)?;
        let heartbeat_interval = Duration::from_millis(hello_data.heartbeat_interval);

        // Send Identify (opcode 2)
        let identify = json!({
            "op": OP_IDENTIFY,
            "d": {
                "token": self.bot_token,
                "intents": DEFAULT_INTENTS,
                "properties": {
                    "os": "linux",
                    "browser": "tidev",
                    "device": "tidev"
                }
            }
        });
        write
            .send(WsMessage::Text(identify.to_string().into()))
            .await?;

        // Heartbeat timer
        let mut heartbeat_timer = tokio::time::interval(heartbeat_interval);
        // Tick immediately on first cycle
        heartbeat_timer.tick().await;

        // Event loop
        loop {
            tokio::select! {
                _ = heartbeat_timer.tick() => {
                    let hb = json!({ "op": OP_HEARTBEAT, "d": self.last_seq });
                    if let Err(e) = write.send(WsMessage::Text(hb.to_string().into())).await {
                        log::warn!("Discord heartbeat send failed: {e}");
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let payload: GatewayPayload = serde_json::from_str(&text)?;
                            if let Some(s) = payload.s {
                                self.last_seq = Some(s);
                            }
                            match payload.op {
                                OP_DISPATCH => {
                                    if let Some(event_type) = payload.t.as_deref() {
                                        match event_type {
                                            "MESSAGE_CREATE" => {
                                                if let Some(d) = payload.d {
                                                    let discord_msg: DiscordMessage = serde_json::from_value(d)?;
                                                    if let Err(e) = self.handle_message(discord_msg).await {
                                                        log::error!("Discord handle_message error: {e}");
                                                    }
                                                }
                                            }
                                            "READY" => {
                                                log::info!("Discord gateway ready");
                                            }
                                            _ => {
                                                // Ignore other event types
                                            }
                                        }
                                    }
                                }
                                OP_HEARTBEAT_ACK => {
                                    self.last_heartbeat_ack = Instant::now();
                                }
                                op => {
                                    log::warn!("Discord unknown opcode: {op}");
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            log::info!("Discord WebSocket closed, reconnecting...");
                            break;
                        }
                        Some(Ok(WsMessage::Ping(data))) => {
                            if let Err(e) = write.send(WsMessage::Pong(data)).await {
                                log::warn!("Discord pong failed: {e}");
                                break;
                            }
                        }
                        Some(Err(e)) => anyhow::bail!("Discord WS error: {e}"),
                        None => break,
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }

    // ── Message handling ────────────────────────────────────────────────────

    async fn handle_message(&mut self, msg: DiscordMessage) -> Result<()> {
        // Skip the bot's own messages
        if msg.author.id == self.bot_user_id {
            return Ok(());
        }

        // Skip other bots (unless we want to listen to them)
        if msg.author.bot.unwrap_or(false) {
            return Ok(());
        }

        let channel_id = &msg.channel_id;
        let author_id = &msg.author.id;

        // Guild/channel filter
        if !self.channel_passes_filter(msg.guild_id.as_deref(), channel_id) {
            return Ok(());
        }

        // Allowlist check
        if !self.core.is_allowed(author_id) {
            log::info!("Discord unauthorized user: {author_id}");
            return Ok(());
        }

        // Mention-only check
        let content = msg.content.trim();
        if self.mention_only && !Self::is_bot_mentioned(content, &self.bot_user_id) {
            return Ok(());
        }

        // Clean content (strip bot mention from the beginning)
        let clean_content =
            if self.mention_only || content.starts_with(&format!("<@{}>", self.bot_user_id)) {
                let mention = format!("<@{}>", self.bot_user_id);
                content.replace(&mention, "").trim().to_string()
            } else {
                content.to_string()
            };

        if clean_content.is_empty() {
            return Ok(());
        }

        log::info!("Discord Message from {author_id} in {channel_id}: {clean_content}");

        // Send typing indicator
        let _ = self.client.trigger_typing(channel_id).await;

        // Add a reaction acknowledgment
        let reaction = Self::random_ack_reaction();
        let msg_id = &msg.id;
        let _ = self.client.add_reaction(channel_id, msg_id, reaction).await;

        // Shell command
        if let Some(cmd) = clean_content.strip_prefix('!') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                return self.handle_shell_command(channel_id, msg_id, cmd).await;
            }
        }

        // Model selection state
        if let Some(state) = self.core.model_selection_states.get(channel_id).cloned() {
            return self
                .handle_model_selection(channel_id, msg_id, &state, &clean_content)
                .await;
        }

        // Parse command
        if let Some(command) = parse_command(&clean_content) {
            if command.name == "model" {
                let id = channel_id.to_string();
                model_selection::start_model_selection(self, &id).await?;
                return Ok(());
            }

            let mut active_model = self
                .core
                .resolve_chat_model(&format!("discord:{channel_id}"))?;
            let chat_key = format!("discord:{channel_id}");
            let mut conversation = self
                .core
                .load_or_create_conversation(&chat_key, &active_model)?;
            self.core
                .load_system_prompt(&conversation, &mut active_model);
            self.core
                .mode_manager
                .restore_from_messages(&chat_key, &conversation.messages);

            let mut sender = DiscordSender {
                client: &self.client,
            };
            let handled = self
                .core
                .handle_command(
                    &mut sender,
                    channel_id,
                    Some(msg_id),
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

        // Regular message → run agent
        let mut active_model = self
            .core
            .resolve_chat_model(&format!("discord:{channel_id}"))?;
        let chat_key = format!("discord:{channel_id}");
        let mut conversation = self
            .core
            .load_or_create_conversation(&chat_key, &active_model)?;
        self.core
            .load_system_prompt(&conversation, &mut active_model);
        self.core
            .mode_manager
            .restore_from_messages(&chat_key, &conversation.messages);
        self.core
            .persist_user_message(&mut conversation, &chat_key, &clean_content)?;

        // Send typing indicator while processing
        let client = self.client();
        let typing_channel = channel_id.to_string();
        let typing_handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if client.trigger_typing(&typing_channel).await.is_err() {
                    break;
                }
            }
        });

        let mut sender = DiscordSender {
            client: &self.client,
        };

        if let Err(error) = self
            .core
            .run_agent_loop_simple(
                &mut sender,
                channel_id,
                Some(msg_id),
                &chat_key,
                &mut conversation,
                &active_model,
            )
            .await
        {
            typing_handle.abort();
            let error_text = format!("Gateway error: {error}");
            let error_message = Message::new(MessageRole::Error, error_text.clone());
            self.core
                .store
                .append_message(conversation.session_id, &error_message)?;
            let _ = self.client.send_message(channel_id, &error_text).await;
        } else {
            typing_handle.abort();
        }

        Ok(())
    }

    fn client(&self) -> DiscordClient {
        DiscordClient::new(self.bot_token.clone())
    }

    async fn handle_shell_command(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        command: &str,
    ) -> Result<()> {
        let chat_key = format!("discord:{channel_id}");
        let mut sender = DiscordSender {
            client: &self.client,
        };
        self.core
            .handle_shell_command(&mut sender, channel_id, Some(msg_id), command, &chat_key)
            .await
    }

    async fn handle_model_selection(
        &mut self,
        channel_id: &str,
        _msg_id: &str,
        state: &ModelSelectionState,
        content: &str,
    ) -> Result<()> {
        let id = channel_id.to_string();
        model_selection::handle_step(self, &id, state, content).await
    }

    /// Send a message to a Discord channel, handling chunking for the 2000 char limit.
    async fn send_markdown(
        &mut self,
        recipient: &str,
        content: &str,
        _msg_id: Option<&str>,
    ) -> Result<()> {
        let chunks = Self::chunk_content(content);
        for chunk in &chunks {
            self.client.send_message(recipient, chunk).await?;
        }
        Ok(())
    }
}

// ── Discord MessageSender ────────────────────────────────────────────────────

struct DiscordSender<'a> {
    client: &'a DiscordClient,
}

#[async_trait]
impl MessageSender for DiscordSender<'_> {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        // Handle chunking for long messages
        let chunks = DiscordChannel::chunk_content(text);
        for chunk in &chunks {
            self.client.send_message(recipient, chunk).await?;
        }
        Ok(())
    }

    fn supports_draft(&self) -> bool {
        true
    }

    async fn send_draft(&mut self, recipient: &str, text: &str) -> Result<Option<String>> {
        // Send first chunk as the draft
        let content = if text.len() > DISCORD_MAX_MESSAGE_LENGTH {
            &text[..DISCORD_MAX_MESSAGE_LENGTH]
        } else {
            text
        };
        let msg_id = self.client.send_message(recipient, content).await?;
        Ok(Some(msg_id))
    }

    async fn update_draft(&mut self, recipient: &str, msg_id: &str, text: &str) -> Result<()> {
        // Discord has a 2000 char limit on edits too
        let content = if text.len() > DISCORD_MAX_MESSAGE_LENGTH {
            format!(
                "{}...\n*(truncated, full response below)*",
                &text[..DISCORD_MAX_MESSAGE_LENGTH - 50]
            )
        } else {
            text.to_string()
        };
        self.client.edit_message(recipient, msg_id, &content).await
    }

    async fn finalize_draft(&mut self, recipient: &str, msg_id: &str, text: &str) -> Result<()> {
        // Finalize with the complete content (edit the draft + send remaining chunks)
        let chunks = DiscordChannel::chunk_content(text);
        if let Some(first) = chunks.first() {
            let content = if first.len() > DISCORD_MAX_MESSAGE_LENGTH {
                format!("{}...", &first[..DISCORD_MAX_MESSAGE_LENGTH - 3])
            } else {
                first.to_string()
            };
            self.client
                .edit_message(recipient, msg_id, &content)
                .await?;
        }
        // Send remaining chunks
        for chunk in chunks.iter().skip(1) {
            self.client.send_message(recipient, chunk).await?;
        }
        Ok(())
    }

    async fn cancel_draft(&mut self, recipient: &str, msg_id: &str) -> Result<()> {
        self.client.delete_message(recipient, msg_id).await
    }
}

// ── ModelSelectionIO for DiscordChannel ──────────────────────────────────────

#[async_trait]
impl ModelSelectionIO for DiscordChannel {
    type Id = String;

    async fn send_message(&mut self, id: &String, text: &str) -> Result<()> {
        self.send_markdown(id, text, None).await
    }

    fn get_state(&self, id: &String) -> Option<ModelSelectionState> {
        self.core.model_selection_states.get(id).cloned()
    }

    fn set_state(&mut self, id: String, state: ModelSelectionState) {
        self.core.model_selection_states.insert(id, state);
    }

    fn remove_state(&mut self, id: &String) {
        self.core.model_selection_states.remove(id);
    }

    fn chat_key(&self, id: &String) -> String {
        format!("{}:{}", self.core.platform_name, id)
    }

    fn platform(&self) -> &'static str {
        self.core.platform_name
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
impl Channel for DiscordChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_DISCORD
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.core.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            log::info!("Discord channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        self.core.restore_sessions(store)
    }

    fn supports_draft_updates(&self) -> bool {
        true
    }

    fn supports_multi_message_streaming(&self) -> bool {
        true
    }

    fn multi_message_delay_ms(&self) -> u64 {
        DISCORD_DRAFT_EDIT_INTERVAL_MS
    }

    async fn send_draft(
        &mut self,
        message: &crate::channel::SendMessage,
    ) -> Result<Option<String>> {
        let content = if message.content.len() > DISCORD_MAX_MESSAGE_LENGTH {
            &message.content[..DISCORD_MAX_MESSAGE_LENGTH]
        } else {
            &message.content
        };
        let msg_id = self
            .client
            .send_message(&message.recipient, content)
            .await?;
        Ok(Some(msg_id))
    }

    async fn update_draft(&mut self, recipient: &str, message_id: &str, text: &str) -> Result<()> {
        self.client.edit_message(recipient, message_id, text).await
    }

    async fn finalize_draft(
        &mut self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        // Edit draft + send remaining chunks
        let chunks = DiscordChannel::chunk_content(text);
        if let Some(first) = chunks.first() {
            self.client
                .edit_message(recipient, message_id, first)
                .await?;
        }
        for chunk in chunks.iter().skip(1) {
            self.client.send_message(recipient, chunk).await?;
        }
        Ok(())
    }

    async fn cancel_draft(&mut self, recipient: &str, message_id: &str) -> Result<()> {
        self.client.delete_message(recipient, message_id).await
    }
}
