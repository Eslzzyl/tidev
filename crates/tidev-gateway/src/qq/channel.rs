use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;
use tokio::sync::broadcast;
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

// ── Gateway opcodes ─────────────────────────────────────────────────────

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

// ── Shared state (behind Arc<Mutex>, used via spawn_local) ────────────

struct SharedState {
    core: ChannelCore,
    client: QQClient,
    msg_seq: Arc<AtomicU32>,
}

// ── QQ gateway channel ────────────────────────────────────────────────

pub struct QQChannel {
    shared: Arc<Mutex<SharedState>>,
    pub session_id: Option<String>,
    pub last_seq: Option<u32>,
    cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
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
        cron_rx: Option<broadcast::Receiver<tidev_scheduler::CronDeliveryMessage>>,
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
            shared: Arc::new(Mutex::new(SharedState {
                core,
                client: QQClient::new(app_id, app_secret, sandbox),
                msg_seq: Arc::new(AtomicU32::new(0)),
            })),
            session_id: None,
            last_seq: None,
            cron_rx,
        }
    }

    async fn send_md(
        guard: &mut tokio::sync::MutexGuard<'_, SharedState>,
        recipient: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let seq = guard.msg_seq.fetch_add(1, Ordering::Relaxed);
        if let Some(openid) = recipient.strip_prefix("user:") {
            guard
                .client
                .send_c2c_message_markdown(openid, text, reply_to, seq)
                .await
        } else {
            guard
                .client
                .send_message_markdown(recipient, text, reply_to, seq)
                .await
        }
    }

    // ── WebSocket event loop ───────────────────────────────────────────

    async fn run_loop(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.connect_and_handle().await {
                log::error!("QQ Gateway connection error: {e}. Retrying in 5s...");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    #[allow(clippy::collapsible_match)]
    async fn connect_and_handle(&mut self) -> Result<()> {
        let shared = self.shared.clone();
        let client_for_gw = shared.lock().await.client.clone();
        drop(shared);

        let gateway_url = client_for_gw.get_gateway_url().await?;
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
        let token = client_for_gw.get_access_token().await?;
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
        let grace_ms = (heartbeat_interval / 10).min(5000);
        let effective_interval = heartbeat_interval + grace_ms;
        let (hb_tx, mut hb_rx) = mpsc::channel::<()>(1);
        // Use tokio::spawn (Send required) for the timer — it's simple.
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

        loop {
            tokio::select! {
                _ = hb_rx.recv() => {
                    if missed_ack_count > 0 {
                        if missed_ack_count >= MAX_MISSED_ACKS {
                            log::error!("QQ heartbeat timeout after {} consecutive missed ACKs; reconnecting", missed_ack_count);
                            break;
                        }
                        log::warn!("QQ heartbeat ACK missed ({}/{}); tolerating", missed_ack_count, MAX_MISSED_ACKS);
                    }
                    let hb = serde_json::json!({ "op": 1, "d": self.last_seq });
                    if write.send(WsMessage::Text(hb.to_string().into())).await.is_err() {
                        log::error!("QQ heartbeat write failed; reconnecting");
                        break;
                    }
                    missed_ack_count += 1;
                }
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
                                                if let Some(d) = payload.d
                                                    && let Ok(ready) = serde_json::from_value::<ReadyData>(d) {
                                                        log::info!("QQ Ready: session_id={}", ready.session_id);
                                                        self.session_id = Some(ready.session_id);
                                                }
                                                // Drain any pending cron delivery messages.
                                                self.drain_cron_messages().await;
                                            }
                                            "AT_MESSAGE_CREATE" | "MESSAGE_CREATE" | "C2C_MESSAGE_CREATE" => {
                                                if let Some(data) = payload.d {
                                                    let shared = self.shared.clone();
                                                    let event_type = t.to_string();
                                                    // Use spawn_local (no Send required) so the
                                                    // message handler can access ChannelCore
                                                    // (which contains !Sync SessionStore).
                                                    tokio::task::spawn_local(async move {
                                                        if let Err(e) = handle_message(shared, &event_type, data).await {
                                                            log::error!("QQ handle_message error: {e}");
                                                        }
                                                    });
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                OP_HEARTBEAT => {
                                    let hb = serde_json::json!({ "op": 1, "d": self.last_seq });
                                    if write.send(WsMessage::Text(hb.to_string().into())).await.is_err() { break; }
                                }
                                OP_RECONNECT => {
                                    log::info!("QQ Reconnect (op 7) received; reconnecting with resume");
                                    break;
                                }
                                OP_INVALID_SESSION => {
                                    log::warn!("QQ Invalid Session (op 9) received; clearing session");
                                    self.session_id = None;
                                    self.last_seq = None;
                                    break;
                                }
                                OP_HEARTBEAT_ACK => { missed_ack_count = 0; }
                                op => { log::warn!("QQ unknown opcode: {op}"); }
                            }
                        }
                        Some(Ok(WsMessage::Close(frame))) => {
                            let (code, reason) = frame.as_ref()
                                .map(|f| (f.code.to_string(), f.reason.to_string()))
                                .unwrap_or_else(|| ("unknown".into(), "none".into()));
                            log::info!("QQ WebSocket closed (code={code}, reason=\"{reason}\"); reconnecting with resume");
                            break;
                        }
                        Some(Ok(WsMessage::Ping(payload))) => {
                            if write.send(WsMessage::Pong(payload)).await.is_err() { break; }
                        }
                        Some(Err(e)) => anyhow::bail!("QQ WS error: {e}"),
                        None => { log::warn!("QQ WebSocket stream ended; reconnecting"); break; }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    /// Drain any pending cron job delivery messages and send them via QQ.
    async fn drain_cron_messages(&mut self) {
        use tokio::sync::broadcast::error::TryRecvError;

        let Some(ref mut rx) = self.cron_rx else {
            return;
        };
        loop {
            match rx.try_recv() {
                Ok(msg) => {
                    if msg.delivery.channel.as_deref() != Some("qq") {
                        continue;
                    }
                    let Some(to) = msg.delivery.to.as_ref() else {
                        continue;
                    };
                    log::info!("Delivering cron job '{}' result to QQ {to}", msg.job_name);
                    let shared = self.shared.clone();
                    // Send via QQ client under lock
                    let output = msg.output.clone();
                    let to = to.clone();
                    tokio::task::spawn_local(async move {
                        let guard = shared.lock().await;
                        if to.starts_with("user:") {
                            let _ = guard
                                .client
                                .send_c2c_message_markdown(
                                    to.trim_start_matches("user:"),
                                    &output,
                                    None,
                                    0,
                                )
                                .await;
                        } else {
                            let _ = guard
                                .client
                                .send_message_markdown(&to, &output, None, 0)
                                .await;
                        }
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

// ── Background message handler (runs via spawn_local) ───────────────────

async fn handle_message(
    shared: Arc<Mutex<SharedState>>,
    event_type: &str,
    data: serde_json::Value,
) -> Result<()> {
    // ── Extract message metadata (no lock needed) ─────────────────────
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
    let clean_content = content.split(' ').next_back().unwrap_or(content);

    log::info!("QQ Message from {author_id}: {clean_content}");

    // ── Lock shared state ─────────────────────────────────────────────
    let mut guard = shared.lock().await;

    // ── Allowlist check ───────────────────────────────────────────────
    if !guard.core.is_allowed(&author_id) {
        log::info!("QQ unauthorized user: {author_id}");
        return Ok(());
    }

    // ── Shell command ─────────────────────────────────────────────────
    if let Some(cmd) = clean_content.strip_prefix('!') {
        let cmd = cmd.trim();
        if !cmd.is_empty() {
            let chat_key = format!("qq:{channel_id}");
            let mut sender = QQSender::new(&guard);
            return guard
                .core
                .handle_shell_command(&mut sender, &channel_id, Some(&msg_id), cmd, &chat_key)
                .await;
        }
    }

    // ── Model selection state (drop lock, run via snapshot IO) ────────
    let model_state = guard.core.model_selection_states.get(&channel_id).cloned();
    if let Some(state) = model_state {
        drop(guard);
        let mut io = QqModelSelectionIO::new(&shared).await;
        model_selection::handle_step(&mut io, &channel_id, &state, clean_content).await?;
        io.finalize().await;
        return Ok(());
    }

    // ── Parse command ─────────────────────────────────────────────────
    if let Some(command) = parse_command(clean_content) {
        if command.name == "model" {
            drop(guard);
            let mut io = QqModelSelectionIO::new(&shared).await;
            model_selection::start_model_selection(&mut io, &channel_id).await?;
            io.finalize().await;
            return Ok(());
        }

        let chat_key = format!("qq:{channel_id}");
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

        let mut sender = QQSender::new(&guard);
        let handled = guard
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

    // ── Regular message → run agent ───────────────────────────────────
    let chat_key = format!("qq:{channel_id}");
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
        .persist_user_message(&mut conversation, &chat_key, clean_content)?;

    let mut sender = QQSender::new(&guard);

    if let Err(error) = guard
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
        let _ = guard
            .core
            .store
            .append_message(conversation.session_id, &error_message);
        let _ = QQChannel::send_md(&mut guard, &channel_id, &error_text, Some(&msg_id)).await;
    }

    Ok(())
}

// ── QQSender (owned QQClient + Arc<AtomicU32>) ─────────────────────────

struct QQSender {
    client: QQClient,
    msg_seq: Arc<AtomicU32>,
}

impl QQSender {
    fn new(guard: &tokio::sync::MutexGuard<'_, SharedState>) -> Self {
        Self {
            client: guard.client.clone(),
            msg_seq: guard.msg_seq.clone(),
        }
    }
}

#[async_trait]
impl MessageSender for QQSender {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<()> {
        let seq = self.msg_seq.fetch_add(1, Ordering::Relaxed);
        if let Some(openid) = recipient.strip_prefix("user:") {
            self.client
                .send_c2c_message_markdown(openid, text, reply_to, seq)
                .await
        } else {
            self.client
                .send_message_markdown(recipient, text, reply_to, seq)
                .await
        }
    }
}

// ── QqModelSelectionIO (snapshot-based, no long-lived borrow) ─────────

struct QqModelSelectionIO {
    shared: Arc<Mutex<SharedState>>,
    states: HashMap<String, ModelSelectionState>,
    /// Snapshot of config — written back on finalize.
    config: AppConfig,
    config_modified: bool,
}

impl QqModelSelectionIO {
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
impl ModelSelectionIO for QqModelSelectionIO {
    type Id = String;

    async fn send_message(&mut self, id: &String, text: &str) -> Result<()> {
        let mut guard = self.shared.lock().await;
        QQChannel::send_md(&mut guard, id, text, None).await
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
        format!("{}:{}", GATEWAY_PLATFORM_QQ, id)
    }

    fn platform(&self) -> &'static str {
        GATEWAY_PLATFORM_QQ
    }

    fn config(&self) -> &AppConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut AppConfig {
        self.config_modified = true;
        &mut self.config
    }

    fn config_paths(&self) -> &ConfigPaths {
        // Snapshot — config_paths is accessed rarely; use stored clone.
        // Actually, we can't return &ConfigPaths from a temporary guard.
        // Panic — this code path is only reached on Agent/Memory target selection,
        // which is unlikely from QQ (the user typically selects a chat model).
        panic!("QqModelSelectionIO::config_paths called; not available in snapshot mode")
    }

    fn auth(&self) -> &AuthStore {
        panic!("QqModelSelectionIO::auth called; not available in snapshot mode")
    }

    fn store(&self) -> &SessionStore {
        panic!("QqModelSelectionIO::store called; not available in snapshot mode")
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
            .map_err(|_| anyhow::anyhow!("failed to lock core for resolve_chat_model"))?
            .core
            .resolve_chat_model(chat_key)
    }
}

// ── Channel trait implementation ────────────────────────────────────────

#[async_trait]
impl Channel for QQChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_QQ
    }

    fn store(&self) -> Option<&SessionStore> {
        None
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            log::info!("QQ channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let guard = self.shared.try_lock().map_err(|_| {
            anyhow::anyhow!("QQChannel::restore_sessions: failed to lock shared state")
        })?;
        guard.core.restore_sessions(store)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

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
