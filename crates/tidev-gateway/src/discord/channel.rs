//! Discord channel implementation.
//!
//! Connects to the Discord Gateway via WebSocket for real-time message
//! reception and uses the REST API for sending replies.
//!
//! Architecture: WebSocket IO and message processing are decoupled.
//! Messages are dispatched via `spawn_local` to background tasks so
//! heartbeats and new messages are never blocked by long agent runs.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};

use tidev_engine::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use tidev_storage::SessionStore;

use crate::channel::Channel;
use crate::channel_core::{ChannelCore, MessageSender};
use crate::commands::parse_command;
use crate::model_selection::{self, ModelSelectionIO, ModelSelectionState};
use tidev_session::session::{Message, MessageRole};

use super::client::DiscordClient;
use super::types::{
    DEFAULT_INTENTS, DiscordMessage, GatewayPayload, HelloData, OP_DISPATCH, OP_HEARTBEAT,
    OP_HEARTBEAT_ACK, OP_HELLO, OP_IDENTIFY,
};

pub const GATEWAY_PLATFORM_DISCORD: &str = "discord";

/// Random acknowledgement emoji reactions.
const ACK_REACTIONS: &[&str] = &["👀", "✅", "👍", "👋", "🤔", "💭", "✨"];

// ── Shared state (behind Arc<Mutex>, used via spawn_local) ────────────

struct SharedState {
    core: ChannelCore,
    client: DiscordClient,
}

/// Discord gateway channel implementation.
pub struct DiscordChannel {
    shared: Arc<Mutex<SharedState>>,
    pub bot_token: String,
    pub bot_user_id: String,
    pub guild_ids: Vec<String>,
    pub channel_ids: Vec<String>,
    pub mention_only: bool,
    pub last_seq: Option<u32>,
    cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
}

impl DiscordChannel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: tidev_llm::LlmClient,
        tools: tidev_engine::tooling::ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        bot_token: String,
        guild_ids: Vec<String>,
        channel_ids: Vec<String>,
        mention_only: bool,
        paths: &ConfigPaths,
        cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
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
            shared: Arc::new(Mutex::new(SharedState {
                core,
                client: DiscordClient::new(bot_token.clone()),
            })),
            bot_token,
            bot_user_id: bot_user_id.unwrap_or_default(),
            guild_ids,
            channel_ids,
            mention_only,
            last_seq: None,
            cron_rx,
        }
    }

    /// Extract the bot user ID from a Discord bot token.
    fn extract_bot_user_id(token: &str) -> Option<String> {
        use base64::Engine as _;
        let part = token.split('.').next()?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(part)
            .ok()?;
        let id = std::str::from_utf8(&bytes).ok()?;
        Some(id.to_string())
    }

    fn is_bot_mentioned(content: &str, bot_user_id: &str) -> bool {
        let mention_pattern = format!("<@{}>", bot_user_id);
        if content.contains(&mention_pattern) {
            return true;
        }
        // Also check for role @everyone
        if content.contains("@everyone") {
            return true;
        }
        false
    }

    fn random_ack_reaction() -> &'static str {
        ACK_REACTIONS[0]
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

    #[allow(clippy::collapsible_match)]
    async fn connect_and_listen(&mut self) -> Result<()> {
        let gw_url = {
            let guard = self.shared.lock().await;
            guard.client.get_gateway_url().await?
        };
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

        // ── Heartbeat: independent timer task ─────────────────────────
        let hb_ms = heartbeat_interval.as_millis() as u64;
        let grace_ms = (hb_ms / 10).min(5000);
        let effective_interval = hb_ms + grace_ms;
        let (hb_tx, mut hb_rx) = tokio::sync::mpsc::channel::<()>(1);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(effective_interval));
            interval.tick().await;
            loop {
                interval.tick().await;
                if hb_tx.send(()).await.is_err() {
                    break;
                }
            }
        });

        const MAX_MISSED_ACKS: u32 = 3;
        let mut missed_ack_count: u32 = 0;

        // ── Shared state for spawned tasks ────────────────────────────
        let shared = self.shared.clone();
        let guild_ids = self.guild_ids.clone();
        let channel_ids = self.channel_ids.clone();
        let mention_only = self.mention_only;
        let bot_user_id = self.bot_user_id.clone();

        // Event loop
        loop {
            tokio::select! {
                _ = hb_rx.recv() => {
                    if missed_ack_count > 0 {
                        if missed_ack_count >= MAX_MISSED_ACKS {
                            log::error!(
                                "Discord heartbeat timeout after {} consecutive missed ACKs; reconnecting",
                                missed_ack_count,
                            );
                            break;
                        }
                        log::warn!(
                            "Discord heartbeat ACK missed ({}/{}); tolerating",
                            missed_ack_count, MAX_MISSED_ACKS,
                        );
                    }
                    let hb = json!({ "op": OP_HEARTBEAT, "d": self.last_seq });
                    if write.send(WsMessage::Text(hb.to_string().into())).await.is_err() {
                        log::warn!("Discord heartbeat send failed; reconnecting");
                        break;
                    }
                    missed_ack_count += 1;
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
                                                if let Some(d) = payload.d
                                                    && let Ok(discord_msg) = serde_json::from_value::<DiscordMessage>(d) {
                                                        let shared = shared.clone();
                                                        let guild_ids = guild_ids.clone();
                                                        let channel_ids = channel_ids.clone();
                                                        let bot_user_id = bot_user_id.clone();
                                                        tokio::task::spawn_local(async move {
                                                            if let Err(e) = handle_message(
                                                                shared, discord_msg,
                                                                &guild_ids, &channel_ids,
                                                                mention_only, &bot_user_id,
                                                            ).await {
                                                                log::error!("Discord handle_message error: {e}");
                                                            }
                                                        });
                                                }
                                            }
                                            "READY" => {
                                                log::info!("Discord gateway ready");
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                OP_HEARTBEAT_ACK => {
                                    missed_ack_count = 0;
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
                            if write.send(WsMessage::Pong(data)).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(e)) => anyhow::bail!("Discord WS error: {e}"),
                        None => { log::warn!("Discord WS stream ended; reconnecting"); break; }
                        _ => {}
                    }
                }
            }

            // Drain any pending cron delivery messages.
            self.drain_cron_messages().await;
        }

        Ok(())
    }

    /// Drain any pending cron job delivery messages and send them.
    async fn drain_cron_messages(&mut self) {
        use tokio::sync::broadcast::error::TryRecvError;

        let Some(ref mut rx) = self.cron_rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if msg.delivery.channel.as_deref() != Some("discord") {
                        continue;
                    }
                    let Some(to) = msg.delivery.to.as_ref() else {
                        continue;
                    };
                    log::info!(
                        "Delivering cron job '{}' result to discord {to}",
                        msg.job_name
                    );
                    let shared = self.shared.clone();
                    let output = msg.output.clone();
                    let to = to.clone();
                    tokio::task::spawn_local(async move {
                        let guard = shared.lock().await;
                        let _ = guard.client.send_message(&to, &output).await;
                    });
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed) => {
                    self.cron_rx = None;
                    break;
                }
                Err(TryRecvError::Lagged(n)) => {
                    log::warn!("Cron delivery receiver lagged by {n} messages");
                    continue;
                }
            }
        }
    }
}

// ── Background message handler ─────────────────────────────────────────

async fn handle_message(
    shared: Arc<Mutex<SharedState>>,
    msg: DiscordMessage,
    guild_ids: &[String],
    channel_ids: &[String],
    mention_only: bool,
    bot_user_id: &str,
) -> Result<()> {
    // Skip the bot's own messages
    if msg.author.id == bot_user_id {
        return Ok(());
    }
    if msg.author.bot.unwrap_or(false) {
        return Ok(());
    }

    let channel_id = &msg.channel_id;
    let author_id = &msg.author.id;

    // Guild/channel filter (inline since guild_ids/channel_ids are borrowed)
    let passes = if guild_ids.is_empty() && channel_ids.is_empty() {
        true
    } else if !guild_ids.is_empty() {
        if let Some(gid) = msg.guild_id.as_deref() {
            guild_ids.iter().any(|g| g == gid)
        } else {
            true // DM
        }
    } else {
        channel_ids.iter().any(|c| c == channel_id)
    };
    if !passes {
        return Ok(());
    }

    // ── Lock shared state ─────────────────────────────────────────────
    let mut guard = shared.lock().await;

    // Allowlist check
    if !guard.core.is_allowed(author_id) {
        log::info!("Discord unauthorized user: {author_id}");
        return Ok(());
    }

    // Mention-only check
    let content = msg.content.trim();
    if mention_only && !DiscordChannel::is_bot_mentioned(content, bot_user_id) {
        return Ok(());
    }

    // Clean content
    let clean_content = if mention_only || content.starts_with(&format!("<@{}>", bot_user_id)) {
        let mention = format!("<@{}>", bot_user_id);
        content.replace(&mention, "").trim().to_string()
    } else {
        content.to_string()
    };
    if clean_content.is_empty() {
        return Ok(());
    }

    log::info!("Discord Message from {author_id} in {channel_id}: {clean_content}");

    // Send typing indicator
    let _ = guard.client.trigger_typing(channel_id).await;

    // Add a reaction acknowledgment
    let reaction = DiscordChannel::random_ack_reaction();
    let msg_id = &msg.id;
    let _ = guard
        .client
        .add_reaction(channel_id, msg_id, reaction)
        .await;

    // Shell command
    if let Some(cmd) = clean_content.strip_prefix('!') {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            let chat_key = format!("discord:{channel_id}");
            let mut sender = DiscordSender {
                client: guard.client.clone(),
            };
            return guard
                .core
                .handle_shell_command(&mut sender, channel_id, Some(msg_id), cmd, &chat_key)
                .await;
        }
    }

    // Model selection state (drop lock, run via snapshot IO)
    let model_state = guard.core.model_selection_states.get(channel_id).cloned();
    if let Some(state) = model_state {
        drop(guard);
        let mut io = DiscordModelSelectionIO::new(&shared).await;
        model_selection::handle_step(&mut io, channel_id, &state, &clean_content).await?;
        io.finalize().await;
        return Ok(());
    }

    // Parse command
    if let Some(command) = parse_command(&clean_content) {
        if command.name == "model" {
            drop(guard);
            let mut io = DiscordModelSelectionIO::new(&shared).await;
            model_selection::start_model_selection(&mut io, channel_id).await?;
            io.finalize().await;
            return Ok(());
        }

        let chat_key = format!("discord:{channel_id}");
        let mut active_model = guard.core.resolve_chat_model(&chat_key)?;
        let mut conversation = guard
            .core
            .load_or_create_conversation(&chat_key, &active_model)?;
        guard
            .core
            .load_system_prompt(&conversation, &mut active_model);
        guard
            .core
            .mode_manager
            .restore_from_messages(&chat_key, &conversation.messages);

        let mut sender = DiscordSender {
            client: guard.client.clone(),
        };
        let handled = guard
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
    let chat_key = format!("discord:{channel_id}");
    let mut active_model = guard.core.resolve_chat_model(&chat_key)?;
    let mut conversation = guard
        .core
        .load_or_create_conversation(&chat_key, &active_model)?;
    guard
        .core
        .load_system_prompt(&conversation, &mut active_model);
    guard
        .core
        .mode_manager
        .restore_from_messages(&chat_key, &conversation.messages);
    guard
        .core
        .persist_user_message(&mut conversation, &chat_key, &clean_content)?;

    // Send typing indicator while processing
    let client = guard.client.clone();
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
        client: guard.client.clone(),
    };

    if let Err(error) = guard
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
        guard
            .core
            .store
            .append_message(conversation.session_id, &error_message)?;
        let _ = guard.client.send_message(channel_id, &error_text).await;
    } else {
        typing_handle.abort();
    }

    Ok(())
}

// ── DiscordSender (owned) ──────────────────────────────────────────────

struct DiscordSender {
    client: DiscordClient,
}

#[async_trait]
impl MessageSender for DiscordSender {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        self.client.send_message(recipient, text).await?;
        Ok(())
    }

    fn supports_draft(&self) -> bool {
        true
    }

    async fn send_draft(&mut self, recipient: &str, text: &str) -> Result<Option<String>> {
        self.client.send_message(recipient, text).await.map(Some)
    }

    async fn update_draft(&mut self, recipient: &str, msg_id: &str, text: &str) -> Result<()> {
        self.client.edit_message(recipient, msg_id, text).await?;
        Ok(())
    }

    async fn finalize_draft(&mut self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> {
        Ok(())
    }

    async fn cancel_draft(&mut self, recipient: &str, msg_id: &str) -> Result<()> {
        self.client.delete_message(recipient, msg_id).await?;
        Ok(())
    }
}

// ── DiscordModelSelectionIO (snapshot-based) ───────────────────────────

struct DiscordModelSelectionIO {
    shared: Arc<Mutex<SharedState>>,
    states: std::collections::HashMap<String, ModelSelectionState>,
    config: AppConfig,
    config_modified: bool,
}

impl DiscordModelSelectionIO {
    async fn new(shared: &Arc<Mutex<SharedState>>) -> Self {
        let guard = shared.lock().await;
        Self {
            shared: shared.clone(),
            states: guard.core.model_selection_states.clone(),
            config: guard.core.config.clone(),
            config_modified: false,
        }
    }

    async fn finalize(mut self) {
        let mut guard = self.shared.lock().await;
        std::mem::swap(&mut guard.core.model_selection_states, &mut self.states);
        if self.config_modified {
            guard.core.config = self.config;
        }
    }
}

#[async_trait]
impl ModelSelectionIO for DiscordModelSelectionIO {
    type Id = String;

    async fn send_message(&mut self, id: &String, text: &str) -> Result<()> {
        let guard = self.shared.lock().await;
        guard.client.send_message(id, text).await?;
        Ok(())
    }

    fn get_state(&self, id: &String) -> Option<ModelSelectionState> {
        self.states.get(id).cloned()
    }

    fn set_state(&mut self, id: String, state: ModelSelectionState) {
        self.states.insert(id, state);
    }

    fn remove_state(&mut self, id: &String) {
        self.states.remove(id);
    }

    fn chat_key(&self, id: &String) -> String {
        format!("{}:{}", GATEWAY_PLATFORM_DISCORD, id)
    }

    fn platform(&self) -> &'static str {
        GATEWAY_PLATFORM_DISCORD
    }

    fn config(&self) -> &AppConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut AppConfig {
        self.config_modified = true;
        &mut self.config
    }

    fn config_paths(&self) -> &ConfigPaths {
        panic!("DiscordModelSelectionIO::config_paths not available in snapshot mode")
    }

    fn auth(&self) -> &AuthStore {
        panic!("DiscordModelSelectionIO::auth not available in snapshot mode")
    }

    fn store(&self) -> &SessionStore {
        panic!("DiscordModelSelectionIO::store not available in snapshot mode")
    }

    fn get_available_providers(&self) -> Vec<(String, String)> {
        self.shared
            .try_lock()
            .ok()
            .map(|g| g.core.get_available_providers())
            .unwrap_or_default()
    }

    fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)> {
        self.shared
            .try_lock()
            .ok()
            .map(|g| g.core.get_models_for_provider(provider_id))
            .unwrap_or_default()
    }

    fn resolve_chat_model(&self, chat_key: &str) -> Result<ActiveModel> {
        self.shared
            .try_lock()
            .map_err(|_| anyhow::anyhow!("failed to lock for resolve_chat_model"))?
            .core
            .resolve_chat_model(chat_key)
    }
}

// ── Channel trait implementation ────────────────────────────────────────

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_DISCORD
    }

    fn store(&self) -> Option<&SessionStore> {
        None
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            log::info!("Discord channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let guard = self.shared.try_lock().map_err(|_| {
            anyhow::anyhow!("DiscordChannel::restore_sessions: failed to lock shared state")
        })?;
        guard.core.restore_sessions(store)
    }
}
