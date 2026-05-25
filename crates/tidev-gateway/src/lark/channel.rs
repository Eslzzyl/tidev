//! Lark/Feishu channel implementation.
//!
//! Connects via WebSocket with Protobuf-framed protocol for receiving events
//! and uses the REST API for sending replies.
//!
//! Architecture: WebSocket IO and event processing are decoupled.
//! Events are dispatched via `spawn_local` to background tasks so
//! heartbeats and new events are never blocked by long agent runs.

use anyhow::Result;
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use serde_json::Value;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};

use tidev_engine::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use tidev_session::session::{Message, MessageRole};
use tidev_storage::SessionStore;

use crate::channel::Channel;
use crate::channel_core::{ChannelCore, MessageSender};
use crate::commands::parse_command;
use crate::model_selection::{self, ModelSelectionIO, ModelSelectionState};

use super::client::LarkClient;
use super::types::{EventMessage, EventPayload, PbFrame};

pub const GATEWAY_PLATFORM_LARK: &str = "lark";
const WS_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const LARK_TEXT_MAX_LENGTH: usize = 2000;

// ── Shared state (behind Arc<Mutex>, used via spawn_local) ────────────

struct SharedState {
    core: ChannelCore,
    client: LarkClient,
    bot_open_id: Option<String>,
}

/// Lark/Feishu gateway channel implementation.
pub struct LarkChannel {
    shared: Arc<Mutex<SharedState>>,
    pub app_id: String,
    pub mention_only: bool,
    pub use_feishu: bool,
    cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
}

impl LarkChannel {
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
        app_id: String,
        app_secret: String,
        mention_only: bool,
        use_feishu: bool,
        paths: &ConfigPaths,
        cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
    ) -> Self {
        let core = ChannelCore::new(
            GATEWAY_PLATFORM_LARK,
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
                client: LarkClient::new(app_id.clone(), app_secret, use_feishu),
                bot_open_id: None,
            })),
            app_id,
            mention_only,
            use_feishu,
            cron_rx,
        }
    }

    /// Parse message content from Lark event into plain text.
    fn parse_content(msg: &EventMessage) -> String {
        match msg.msg_type.as_str() {
            "text" => {
                if let Ok(val) = serde_json::from_str::<Value>(&msg.content) {
                    val.get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                }
            }
            "post" => {
                if let Ok(val) = serde_json::from_str::<Value>(&msg.content) {
                    Self::extract_post_text(&val)
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    fn extract_post_text(val: &Value) -> String {
        let mut text = String::new();
        if let Some(paragraphs) = val.get("content").and_then(|c| c.as_array()) {
            for paragraph in paragraphs {
                if let Some(elements) = paragraph.as_array() {
                    for element in elements {
                        if let Some(t) = element.get("insert").and_then(|i| i.as_str()) {
                            text.push_str(t);
                        }
                    }
                }
            }
        }
        text.trim().to_string()
    }

    fn is_mentioned(content: &str, bot_open_id: &str) -> bool {
        let mention_pattern = format!("@_user_{}", bot_open_id);
        content.contains(&mention_pattern)
    }

    /// Send a message choosing between text and interactive card based on length.
    async fn send_message_adaptive(
        client: &LarkClient,
        recipient: &str,
        content: &str,
    ) -> Result<()> {
        if content.len() <= LARK_TEXT_MAX_LENGTH {
            client.send_text_message(recipient, content).await?;
        } else {
            client.send_card_message(recipient, content).await?;
        }
        Ok(())
    }

    // ── WebSocket event loop ─────────────────────────────────────────────────

    async fn run_loop(&mut self) -> Result<()> {
        // Resolve bot open_id at startup
        {
            let mut guard = self.shared.lock().await;
            guard.bot_open_id = guard.client.get_bot_info().await.ok();
            log::info!("Lark bot open_id: {:?}", guard.bot_open_id);
        }

        loop {
            if let Err(e) = self.connect_and_listen().await {
                log::error!(
                    "Lark WS connection error: {e}, reconnecting in {}s",
                    WS_RECONNECT_DELAY.as_secs()
                );
                sleep(WS_RECONNECT_DELAY).await;
            }
        }
    }

    async fn connect_and_listen(&mut self) -> Result<()> {
        let shared = self.shared.clone();

        // Get WS endpoint using a temporary lock
        let (ws_url, client_config) = {
            let guard = shared.lock().await;
            guard.client.get_ws_endpoint().await?
        };
        let ping_interval = Duration::from_secs(client_config.ping_interval.unwrap_or(120).max(10));

        log::info!("Lark connecting to WS endpoint...");
        let (ws_stream, _) = connect_async(&ws_url).await?;
        let (mut write, mut read) = ws_stream.split();

        let mut seq: u64 = 0;
        let mut last_recv = Instant::now();

        // Send initial ping
        seq = seq.wrapping_add(1);
        let initial_ping = PbFrame {
            seq_id: seq,
            log_id: 0,
            service: 0,
            method: 0,
            headers: vec![super::types::PbHeader {
                key: "type".into(),
                value: "ping".into(),
            }],
            payload: None,
        };
        write
            .send(WsMessage::Binary(initial_ping.encode_to_vec().into()))
            .await?;

        // ── Heartbeat: independent timer task ─────────────────────────
        let ping_ms = ping_interval.as_millis() as u64;
        let grace_ms = (ping_ms / 10).min(5000);
        let effective_interval = ping_ms + grace_ms;
        let (hb_tx, mut hb_rx) = mpsc::channel::<()>(1);
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

        // ── Shared state for spawned tasks ────────────────────────────
        let mention_only = self.mention_only;

        loop {
            tokio::select! {
                _ = hb_rx.recv() => {
                    seq = seq.wrapping_add(1);
                    let ping = PbFrame {
                        seq_id: seq, log_id: 0, service: 0, method: 0,
                        headers: vec![
                            super::types::PbHeader { key: "type".into(), value: "ping".into() },
                        ],
                        payload: None,
                    };
                    if write.send(WsMessage::Binary(ping.encode_to_vec().into())).await.is_err() {
                        log::warn!("Lark ping send failed, reconnecting");
                        break;
                    }
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(WsMessage::Binary(data))) => {
                            match PbFrame::decode(&data[..]) {
                                Ok(frame) => {
                                    last_recv = Instant::now();
                                    if frame.method == 0 {
                                        // CONTROL frame (ping/pong)
                                    } else if frame.method == 1 {
                                        // DATA frame → event
                                        if let Some(payload_bytes) = &frame.payload {
                                            let shared = shared.clone();
                                            let payload_bytes = payload_bytes.clone();
                                            tokio::task::spawn_local(async move {
                                                if let Err(e) = handle_event(
                                                    shared, &payload_bytes, mention_only,
                                                ).await {
                                                    log::error!("Lark handle_event error: {e}");
                                                }
                                            });
                                        }
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Lark WS frame decode error: {e}");
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            log::info!("Lark WS closed, reconnecting...");
                            break;
                        }
                        Some(Ok(WsMessage::Ping(data))) => {
                            let _ = write.send(WsMessage::Pong(data)).await;
                        }
                        Some(Err(e)) => anyhow::bail!("Lark WS error: {e}"),
                        None => { log::warn!("Lark WS stream ended; reconnecting"); break; }
                        _ => {}
                    }

                    // Timeout check
                    if last_recv.elapsed() > WS_HEARTBEAT_TIMEOUT {
                        log::warn!("Lark WS heartbeat timeout, reconnecting");
                        break;
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
                    if msg.delivery.channel.as_deref() != Some("lark") {
                        continue;
                    }
                    let Some(to) = msg.delivery.to.as_ref() else {
                        continue;
                    };
                    log::info!("Delivering cron job '{}' result to lark {to}", msg.job_name);
                    let shared = self.shared.clone();
                    let output = msg.output.clone();
                    let to = to.clone();
                    tokio::task::spawn_local(async move {
                        let guard = shared.lock().await;
                        let _ = guard.client.send_text_message(&to, &output).await;
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

// ── Background event handler ───────────────────────────────────────────

async fn handle_event(
    shared: Arc<Mutex<SharedState>>,
    payload_bytes: &[u8],
    mention_only: bool,
) -> Result<()> {
    let event_val: Value = serde_json::from_slice(payload_bytes)?;
    let event_type = event_val
        .get("header")
        .and_then(|h| h.get("event_type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let Some(event) = event_val.get("event") else {
        return Ok(());
    };

    if event_type != "im.message.receive_v1" {
        return Ok(());
    }

    let payload: EventPayload = serde_json::from_value(event.clone())?;

    // Skip messages from bots
    if payload.sender.sender_type.open_id().is_none() {
        return Ok(());
    }
    let sender_open_id = match payload.sender.sender_type.open_id() {
        Some(id) => id.to_string(),
        None => return Ok(()),
    };

    // ── Lock shared state ─────────────────────────────────────────────
    let mut guard = shared.lock().await;

    // Skip our own messages
    if Some(&sender_open_id) == guard.bot_open_id.as_ref() {
        return Ok(());
    }

    let msg = &payload.message;
    let chat_id = &msg.chat_id;
    let message_id = &msg.message_id;

    // Allowlist check
    if !guard.core.is_allowed(&sender_open_id) {
        log::info!("Lark unauthorized user: {sender_open_id}");
        return Ok(());
    }

    let content = LarkChannel::parse_content(msg);
    if content.trim().is_empty() {
        return Ok(());
    }

    // Mention-only check for group chats
    if msg.chat_type == "group"
        && mention_only
        && let Some(ref bot_id) = guard.bot_open_id
        && !LarkChannel::is_mentioned(&content, bot_id)
    {
        return Ok(());
    }

    // Clean @mention
    let clean_content = if let Some(ref bot_id) = guard.bot_open_id {
        let mention = format!("@_user_{}", bot_id);
        content.replace(&mention, "").trim().to_string()
    } else {
        content.trim().to_string()
    };
    if clean_content.is_empty() {
        return Ok(());
    }

    log::info!("Lark Message from {sender_open_id} in {chat_id}: {clean_content}");

    // Add ack reaction
    let _ = guard.client.add_reaction(message_id, "OK").await;

    // Shell command
    if let Some(cmd) = clean_content.strip_prefix('!') {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            let chat_key = format!("lark:{chat_id}");
            let mut sender = LarkSender {
                client: guard.client.clone(),
            };
            return guard
                .core
                .handle_shell_command(&mut sender, chat_id, Some(message_id), cmd, &chat_key)
                .await;
        }
    }

    // Model selection state (drop lock, run via snapshot IO)
    let model_state = guard.core.model_selection_states.get(chat_id).cloned();
    if let Some(state) = model_state {
        drop(guard);
        let mut io = LarkModelSelectionIO::new(&shared).await;
        model_selection::handle_step(&mut io, chat_id, &state, &clean_content).await?;
        io.finalize().await;
        return Ok(());
    }

    // Parse command
    if let Some(command) = parse_command(&clean_content) {
        if command.name == "model" {
            drop(guard);
            let mut io = LarkModelSelectionIO::new(&shared).await;
            model_selection::start_model_selection(&mut io, chat_id).await?;
            io.finalize().await;
            return Ok(());
        }

        let chat_key = format!("lark:{chat_id}");
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

        let mut sender = LarkSender {
            client: guard.client.clone(),
        };
        let handled = guard
            .core
            .handle_command(
                &mut sender,
                chat_id,
                Some(message_id),
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
    let chat_key = format!("lark:{chat_id}");
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

    let mut sender = LarkSender {
        client: guard.client.clone(),
    };

    if let Err(error) = guard
        .core
        .run_agent_loop_simple(
            &mut sender,
            chat_id,
            Some(message_id),
            &chat_key,
            &mut conversation,
            &active_model,
        )
        .await
    {
        let error_text = format!("Gateway error: {error}");
        let error_message = Message::new(MessageRole::Error, error_text.clone());
        let _ = guard
            .core
            .store
            .append_message(conversation.session_id, &error_message);
        let _ = LarkChannel::send_message_adaptive(&guard.client, chat_id, &error_text).await;
    }

    Ok(())
}

// ── LarkSender (owned) ─────────────────────────────────────────────────

struct LarkSender {
    client: LarkClient,
}

#[async_trait]
impl MessageSender for LarkSender {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        if text.len() <= LARK_TEXT_MAX_LENGTH {
            self.client.send_text_message(recipient, text).await?;
        } else {
            self.client.send_card_message(recipient, text).await?;
        }
        Ok(())
    }

    fn supports_draft(&self) -> bool {
        true
    }

    async fn send_draft(&mut self, recipient: &str, text: &str) -> Result<Option<String>> {
        self.client
            .send_text_message(recipient, text)
            .await
            .map(Some)
    }

    async fn update_draft(&mut self, _recipient: &str, _msg_id: &str, _text: &str) -> Result<()> {
        Ok(())
    }

    async fn finalize_draft(&mut self, recipient: &str, _msg_id: &str, text: &str) -> Result<()> {
        self.send_message(recipient, text, None).await
    }

    async fn cancel_draft(&mut self, _recipient: &str, _msg_id: &str) -> Result<()> {
        Ok(())
    }
}

// ── LarkModelSelectionIO (snapshot-based) ──────────────────────────────

struct LarkModelSelectionIO {
    shared: Arc<Mutex<SharedState>>,
    states: std::collections::HashMap<String, ModelSelectionState>,
    config: AppConfig,
    config_modified: bool,
}

impl LarkModelSelectionIO {
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
impl ModelSelectionIO for LarkModelSelectionIO {
    type Id = String;

    async fn send_message(&mut self, id: &String, text: &str) -> Result<()> {
        let guard = self.shared.lock().await;
        LarkChannel::send_message_adaptive(&guard.client, id, text).await
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
        format!("{}:{}", GATEWAY_PLATFORM_LARK, id)
    }

    fn platform(&self) -> &'static str {
        GATEWAY_PLATFORM_LARK
    }

    fn config(&self) -> &AppConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut AppConfig {
        self.config_modified = true;
        &mut self.config
    }

    fn config_paths(&self) -> &ConfigPaths {
        panic!("LarkModelSelectionIO::config_paths not available in snapshot mode")
    }

    fn auth(&self) -> &AuthStore {
        panic!("LarkModelSelectionIO::auth not available in snapshot mode")
    }

    fn store(&self) -> &SessionStore {
        panic!("LarkModelSelectionIO::store not available in snapshot mode")
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
impl Channel for LarkChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_LARK
    }

    fn store(&self) -> Option<&SessionStore> {
        None
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            log::info!("Lark channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let guard = self.shared.try_lock().map_err(|_| {
            anyhow::anyhow!("LarkChannel::restore_sessions: failed to lock shared state")
        })?;
        guard.core.restore_sessions(store)
    }
}
