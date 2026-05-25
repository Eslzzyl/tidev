use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};

use tidev_engine::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use tidev_session::session::{Message, MessageRole};
use tidev_storage::SessionStore;

use super::client::QQClient;
use crate::channel::Channel;
use crate::channel_core::{ChannelCore, MessageSender};
use crate::commands::parse_command;
use crate::model_selection::{self, ModelSelectionIO, ModelSelectionState};
pub const GATEWAY_PLATFORM_QQ: &str = "qq";

// ── Gateway opcodes (same numbering as QQ/Discord spec) ────────────────────

const OP_DISPATCH: u8 = 0;
const OP_HEARTBEAT: u8 = 1;
#[allow(dead_code)]
const OP_IDENTIFY: u8 = 2;
#[allow(dead_code)]
const OP_RESUME: u8 = 6;
const OP_RECONNECT: u8 = 7;
const OP_INVALID_SESSION: u8 = 9;
const OP_HELLO: u8 = 10;
const OP_HEARTBEAT_ACK: u8 = 11;

#[derive(Debug, Serialize, Deserialize)]
struct WsPayload {
    op: u8,
    d: Option<serde_json::Value>,
    s: Option<u32>,
    t: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HelloData {
    heartbeat_interval: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReadyData {
    version: u32,
    session_id: String,
    user: serde_json::Value,
}

/// QQ gateway channel implementation.
pub struct QQChannel {
    pub core: ChannelCore,
    pub client: QQClient,
    pub session_id: Option<String>,
    pub last_seq: Option<u32>,
    pub msg_seq: u32,
}

impl QQChannel {
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
        sandbox: bool,
        paths: &ConfigPaths,
    ) -> Self {
        let core = ChannelCore::new(
            GATEWAY_PLATFORM_QQ,
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
            client: QQClient::new(app_id, app_secret, sandbox),
            session_id: None,
            last_seq: None,
            msg_seq: 0,
        }
    }

    async fn send_markdown(
        &mut self,
        recipient: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<()> {
        self.msg_seq += 1;
        if let Some(openid) = recipient.strip_prefix("user:") {
            self.client
                .send_c2c_message_markdown(openid, content, msg_id, self.msg_seq)
                .await
        } else {
            self.client
                .send_message_markdown(recipient, content, msg_id, self.msg_seq)
                .await
        }
    }

    // ── QQ WebSocket event loop ───────────────────────────────────────────

    async fn run_loop(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.connect_and_handle().await {
                log::error!("QQ Gateway connection error: {e}. Retrying in 5s...");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn connect_and_handle(&mut self) -> Result<()> {
        let gateway_url = self.client.get_gateway_url().await?;
        let (ws_stream, _) = connect_async(gateway_url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();

        log::info!("QQ Gateway connected to {}", gateway_url);

        // ── Handle Hello ──────────────────────────────────────────────
        let mut heartbeat_interval = 45000u64;
        if let Some(msg) = read.next().await {
            let msg = msg?;
            if let WsMessage::Text(text) = msg {
                let payload: WsPayload = serde_json::from_str(&text)?;
                if payload.op == OP_HELLO {
                    let hello: HelloData = serde_json::from_value(payload.d.unwrap())?;
                    heartbeat_interval = hello.heartbeat_interval;
                    log::info!("QQ Hello received, heartbeat: {}ms", heartbeat_interval);
                }
            }
        }

        // ── Identify or Resume ────────────────────────────────────────
        let token = self.client.get_access_token().await?;
        let identify = if let (Some(sid), Some(seq)) = (&self.session_id, self.last_seq) {
            serde_json::json!({
                "op": 6, "d": { "token": format!("QQBot {}", token), "session_id": sid, "seq": seq }
            })
        } else {
            serde_json::json!({
                "op": 2, "d": {
                    "token": format!("QQBot {}", token),
                    "intents": (1 << 25) | (1 << 30),
                    "shard": [0, 1],
                }
            })
        };

        write
            .send(WsMessage::Text(identify.to_string().into()))
            .await?;

        // ── Heartbeat: independent timer task ─────────────────────────
        // Add a small grace period (10 % of interval, capped at 5 s) so
        // that slightly-delayed ACKs do not immediately count as missed.
        let grace_ms = (heartbeat_interval / 10).min(5000);
        let effective_interval = heartbeat_interval + grace_ms;
        let (hb_tx, mut hb_rx) = mpsc::channel::<()>(1);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(effective_interval));
            // The first tick is immediate; skip it so we don't send a
            // heartbeat right after Identify.
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

        loop {
            tokio::select! {
                // Heartbeat timer tick from the independent task
                _ = hb_rx.recv() => {
                    if missed_ack_count > 0 {
                        if missed_ack_count >= MAX_MISSED_ACKS {
                            log::error!(
                                "QQ heartbeat timeout after {} consecutive missed ACKs; \
                                 reconnecting with resume",
                                missed_ack_count,
                            );
                            break;
                        }
                        log::warn!(
                            "QQ heartbeat ACK missed ({}/{}); tolerating transient delay",
                            missed_ack_count,
                            MAX_MISSED_ACKS,
                        );
                    }
                    let hb = serde_json::json!({ "op": 1, "d": self.last_seq });
                    if write.send(WsMessage::Text(hb.to_string().into())).await.is_err() {
                        log::error!("QQ heartbeat write failed; reconnecting");
                        break;
                    }
                    missed_ack_count += 1;
                }

                // WebSocket incoming message
                msg = read.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let payload: WsPayload = serde_json::from_str(&text)?;
                            if let Some(s) = payload.s { self.last_seq = Some(s); }
                            match payload.op {
                                OP_DISPATCH => {
                                    if let Some(t) = payload.t.as_deref() {
                                        match t {
                                            "READY" => {
                                                let ready: ReadyData = serde_json::from_value(payload.d.unwrap())?;
                                                log::info!("QQ Ready: session_id={}", ready.session_id);
                                                self.session_id = Some(ready.session_id);
                                            }
                                            "AT_MESSAGE_CREATE" | "MESSAGE_CREATE" | "C2C_MESSAGE_CREATE" => {
                                                self.handle_message(t, payload.d.unwrap()).await?;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                // Server requests an immediate heartbeat
                                OP_HEARTBEAT => {
                                    let hb = serde_json::json!({ "op": 1, "d": self.last_seq });
                                    if write.send(WsMessage::Text(hb.to_string().into())).await.is_err() {
                                        break;
                                    }
                                }
                                // Server requests graceful reconnect — break to let run_loop
                                // reconnect with Resume (op 6), preserving session state.
                                OP_RECONNECT => {
                                    log::info!("QQ Reconnect (op 7) received; reconnecting with resume");
                                    break;
                                }
                                // Invalid session — clear state so next connection uses fresh Identify.
                                OP_INVALID_SESSION => {
                                    log::warn!("QQ Invalid Session (op 9) received; clearing session for fresh auth");
                                    self.session_id = None;
                                    self.last_seq = None;
                                    break;
                                }
                                OP_HEARTBEAT_ACK => {
                                    // Reset missed-ACK counter — the connection is alive
                                    missed_ack_count = 0;
                                }
                                op => { log::warn!("QQ unknown opcode: {op}"); }
                            }
                        }
                        Some(Ok(WsMessage::Close(frame))) => {
                            let (code, reason) = frame
                                .as_ref()
                                .map(|f| (f.code.to_string(), f.reason.to_string()))
                                .unwrap_or_else(|| ("unknown".into(), "none".into()));
                            log::info!(
                                "QQ WebSocket closed (code={code}, reason=\"{reason}\"); \
                                 reconnecting with resume"
                            );
                            break;
                        }
                        Some(Ok(WsMessage::Ping(payload))) => {
                            // QQ gateway may send Pings; respond promptly
                            if write.send(WsMessage::Pong(payload)).await.is_err() {
                                break;
                            }
                        }
                        Some(Err(e)) => anyhow::bail!("QQ WS error: {e}"),
                        None => {
                            log::warn!("QQ WebSocket stream ended; reconnecting");
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    // ── Message handling ──────────────────────────────────────────────────

    async fn handle_message(&mut self, event_type: &str, data: serde_json::Value) -> Result<()> {
        let (channel_id, author_id) = if event_type == "C2C_MESSAGE_CREATE" {
            let openid = data["author"]["user_openid"]
                .as_str()
                .context("missing user_openid")?
                .to_string();
            (format!("user:{openid}"), openid)
        } else {
            let cid = data["channel_id"]
                .as_str()
                .context("missing channel_id")?
                .to_string();
            let aid = data["author"]["id"]
                .as_str()
                .context("missing author id")?
                .to_string();
            (cid, aid)
        };
        let msg_id = data["id"]
            .as_str()
            .context("missing message id")?
            .to_string();
        let content = data["content"].as_str().unwrap_or_default().trim();

        // Allowlist check
        if !self.core.is_allowed(&author_id) {
            log::info!("QQ unauthorized user: {author_id}");
            return Ok(());
        }

        // Strip @mention
        let clean_content = content.split(' ').next_back().unwrap_or(content);

        log::info!("QQ Message from {author_id}: {clean_content}");

        // Shell command
        if let Some(cmd) = clean_content.strip_prefix('!') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                return self.handle_shell_command(&channel_id, &msg_id, cmd).await;
            }
        }

        // Model selection state
        if let Some(state) = self.core.model_selection_states.get(&channel_id).cloned() {
            return self
                .handle_model_selection(&channel_id, &msg_id, &state, clean_content)
                .await;
        }

        // Parse command
        if let Some(command) = parse_command(clean_content) {
            // Handle model command specially (needs ModelSelectionIO impl)
            if command.name == "model" {
                let id = channel_id.to_string();
                model_selection::start_model_selection(self, &id).await?;
                return Ok(());
            }

            let mut active_model = self.core.resolve_chat_model(&format!("qq:{channel_id}"))?;
            let chat_key = format!("qq:{channel_id}");
            let mut conversation = self
                .core
                .load_or_create_conversation(&chat_key, &active_model)?;
            self.core
                .load_system_prompt(&conversation, &mut active_model);
            self.core
                .mode_manager
                .restore_from_messages(&chat_key, &conversation.messages);

            let mut sender = QQSender {
                client: &mut self.client,
                msg_seq: &mut self.msg_seq,
                recipient: &channel_id,
                msg_id: Some(&msg_id),
            };
            let handled = self
                .core
                .handle_command(
                    &mut sender,
                    &channel_id,
                    Some(&msg_id),
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
        let mut active_model = self.core.resolve_chat_model(&format!("qq:{channel_id}"))?;
        let chat_key = format!("qq:{channel_id}");
        let mut conversation = self
            .core
            .load_or_create_conversation(&chat_key, &active_model)?;
        self.core
            .load_system_prompt(&conversation, &mut active_model);
        self.core
            .mode_manager
            .restore_from_messages(&chat_key, &conversation.messages);
        self.core
            .persist_user_message(&mut conversation, &chat_key, clean_content)?;

        let mut sender = QQSender {
            client: &mut self.client,
            msg_seq: &mut self.msg_seq,
            recipient: &channel_id,
            msg_id: Some(&msg_id),
        };

        if let Err(error) = self
            .core
            .run_agent_loop_simple(
                &mut sender,
                &channel_id,
                Some(&msg_id),
                &chat_key,
                &mut conversation,
                &active_model,
            )
            .await
        {
            let error_text = format!("Gateway error: {error}");
            let error_message = Message::new(MessageRole::Error, error_text.clone());
            self.core
                .store
                .append_message(conversation.session_id, &error_message)?;
            let _ = self
                .send_markdown(&channel_id, &error_text, Some(&msg_id))
                .await;
        }

        Ok(())
    }

    async fn handle_shell_command(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        command: &str,
    ) -> Result<()> {
        let chat_key = format!("qq:{channel_id}");
        let mut sender = QQSender {
            client: &mut self.client,
            msg_seq: &mut self.msg_seq,
            recipient: channel_id,
            msg_id: Some(msg_id),
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
}

// ── QQ MessageSender ────────────────────────────────────────────────────────

struct QQSender<'a> {
    client: &'a mut QQClient,
    msg_seq: &'a mut u32,
    #[allow(dead_code)]
    recipient: &'a str,
    msg_id: Option<&'a str>,
}

#[async_trait]
impl MessageSender for QQSender<'_> {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        *self.msg_seq += 1;
        if let Some(openid) = recipient.strip_prefix("user:") {
            self.client
                .send_c2c_message_markdown(openid, text, reply_to.or(self.msg_id), *self.msg_seq)
                .await
        } else {
            self.client
                .send_message_markdown(recipient, text, reply_to.or(self.msg_id), *self.msg_seq)
                .await
        }
    }
}

// ── ModelSelectionIO for QQChannel ──────────────────────────────────────────

#[async_trait]
impl ModelSelectionIO for QQChannel {
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
impl Channel for QQChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_QQ
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.core.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            log::info!("QQ channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        self.core.restore_sessions(store)
    }
}

// ── Helper functions ────────────────────────────────────────────────────────

#[allow(dead_code)]
fn truncate_for_markdown(value: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut out = String::new();
    for ch in value.chars().take(MAX_CHARS) {
        if ch == '`' {
            out.push_str("\\`");
        } else {
            out.push(ch);
        }
    }
    if value.chars().count() > MAX_CHARS {
        out.push_str("\n... (truncated)");
    }
    out
}
