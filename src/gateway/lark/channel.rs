//! Lark/Feishu channel implementation.
//!
//! Connects via WebSocket with Protobuf-framed protocol for receiving events
//! and uses the REST API for sending replies.

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use prost::Message as ProstMessage;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Instant;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use anyhow::Result;
use serde_json::Value;

use crate::config::{ActiveModel, AppConfig, AuthStore, ConfigPaths};
use crate::session::{Message, MessageRole};
use crate::storage::SessionStore;

use crate::gateway::channel::Channel;
use crate::gateway::channel_core::{ChannelCore, MessageSender};
use crate::gateway::commands::parse_command;
use crate::gateway::model_selection::{self, ModelSelectionIO, ModelSelectionState};

use super::client::LarkClient;
use super::types::{
    EventMessage, EventPayload, PbFrame,
};

pub const GATEWAY_PLATFORM_LARK: &str = "lark";
const WS_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const WS_RECONNECT_DELAY: Duration = Duration::from_secs(5);
const LARK_TEXT_MAX_LENGTH: usize = 2000;

/// Lark/Feishu gateway channel implementation.
pub struct LarkChannel {
    pub core: ChannelCore,
    pub client: LarkClient,
    pub app_id: String,
    pub mention_only: bool,
    pub use_feishu: bool,
    /// Bot's own open_id (resolved at startup for mention detection).
    bot_open_id: Option<String>,
}

impl LarkChannel {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: crate::llm::LlmClient,
        tools: crate::tooling::ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        app_id: String,
        app_secret: String,
        mention_only: bool,
        use_feishu: bool,
        paths: &ConfigPaths,
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
            core,
            client: LarkClient::new(app_id.clone(), app_secret, use_feishu),
            app_id,
            mention_only,
            use_feishu,
            bot_open_id: None,
        }
    }

    /// Parse message content from Lark event into plain text.
    fn parse_content(msg: &EventMessage) -> String {
        match msg.msg_type.as_str() {
            "text" => {
                // content is JSON: {"text": "hello"}
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
                // content is JSON with rich text elements
                Self::extract_post_text(&msg.content)
            }
            _ => String::new(),
        }
    }

    /// Extract plain text from a Lark post (rich text) message.
    fn extract_post_text(content: &str) -> String {
        let Ok(val) = serde_json::from_str::<Value>(content) else {
            return String::new();
        };
        let mut text = String::new();

        // Post format: { "zh_cn": { "content": [[...]] }, "default": ... }
        for lang in &["default", "zh_cn", "en_us", "ja_jp"] {
            if let Some(content_block) = val.get(*lang).and_then(|c| c.get("content")) {
                if let Some(lines) = content_block.as_array() {
                    for line in lines {
                        if let Some(elements) = line.as_array() {
                            for elem in elements {
                                if let Some(tag) = elem.get("tag").and_then(|t| t.as_str()) {
                                    match tag {
                                        "text" | "md" => {
                                            if let Some(txt) = elem.get("text").and_then(|t| t.as_str()) {
                                                text.push_str(txt);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            text.push('\n');
                        }
                    }
                }
                if !text.is_empty() {
                    break;
                }
            }
        }

        text.trim().to_string()
    }

    /// Check if the bot was @mentioned in the message content.
    fn is_mentioned(content: &str, bot_open_id: &str) -> bool {
        let mention_pattern = format!("@_user_{}", bot_open_id);
        content.contains(&mention_pattern)
    }

    // ── WebSocket event loop ─────────────────────────────────────────────────

    async fn run_loop(&mut self) -> Result<()> {
        // Resolve bot open_id at startup
        self.bot_open_id = self.client.get_bot_info().await.ok();
        crate::log_info!("Lark bot open_id: {:?}", self.bot_open_id);

        loop {
            if let Err(e) = self.connect_and_listen().await {
                crate::log_error!("Lark WS connection error: {e}, reconnecting in {}s", WS_RECONNECT_DELAY.as_secs());
                sleep(WS_RECONNECT_DELAY).await;
            }
        }
    }

    async fn connect_and_listen(&mut self) -> Result<()> {
        // Get WS endpoint
        let (ws_url, client_config) = self.client.get_ws_endpoint().await?;
        let ping_interval = Duration::from_secs(client_config.ping_interval.unwrap_or(120).max(10));

        crate::log_info!("Lark connecting to WS endpoint...");
        let (ws_stream, _) = connect_async(&ws_url).await?;
        let (mut write, mut read) = ws_stream.split();

        let mut seq: u64 = 0;
        let mut last_recv = Instant::now();
        let mut hb_interval = tokio::time::interval(ping_interval);
        hb_interval.tick().await; // consume immediate tick

        // Send initial ping
        seq = seq.wrapping_add(1);
        let initial_ping = PbFrame {
            seq_id: seq,
            log_id: 0,
            service: 0,
            method: 0,
            headers: vec![
                super::types::PbHeader { key: "type".into(), value: "ping".into() },
            ],
            payload: None,
        };
        write
            .send(WsMessage::Binary(initial_ping.encode_to_vec().into()))
            .await?;

        loop {
            tokio::select! {
                _ = hb_interval.tick() => {
                    seq = seq.wrapping_add(1);
                    let ping = PbFrame {
                        seq_id: seq, log_id: 0, service: 0, method: 0,
                        headers: vec![
                            super::types::PbHeader { key: "type".into(), value: "ping".into() },
                        ],
                        payload: None,
                    };
                    if write.send(WsMessage::Binary(ping.encode_to_vec().into())).await.is_err() {
                        crate::log_warn!("Lark ping send failed, reconnecting");
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
                                        if frame.header_value("type") == "pong" {
                                            // Pong received, server is alive
                                        }
                                    } else if frame.method == 1 {
                                        // DATA frame → event
                                        if let Some(payload_bytes) = &frame.payload {
                                            self.handle_event(payload_bytes).await?;
                                        }
                                    }
                                }
                                Err(e) => {
                                    crate::log_warn!("Lark WS frame decode error: {e}");
                                }
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) => {
                            crate::log_info!("Lark WS closed, reconnecting...");
                            break;
                        }
                        Some(Ok(WsMessage::Ping(data))) => {
                            let _ = write.send(WsMessage::Pong(data)).await;
                        }
                        Some(Err(e)) => anyhow::bail!("Lark WS error: {e}"),
                        None => break,
                        _ => {}
                    }

                    // Timeout check
                    if last_recv.elapsed() > WS_HEARTBEAT_TIMEOUT {
                        crate::log_warn!("Lark WS heartbeat timeout, reconnecting");
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle a received event payload from the WS DATA frame.
    async fn handle_event(&mut self, payload_bytes: &[u8]) -> Result<()> {
        // The payload is a JSON object with event data
        let event_val: Value = serde_json::from_slice(payload_bytes)?;

        // Extract event type from the payload
        let event_type = event_val
            .get("header")
            .and_then(|h| h.get("event_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Extract event body
        let Some(event) = event_val.get("event") else {
            return Ok(());
        };

        match event_type {
            "im.message.receive_v1" => {
                let payload: EventPayload = serde_json::from_value(event.clone())?;
                self.handle_message_event(payload).await?;
            }
            _ => {
                // Ignore other event types
            }
        }

        Ok(())
    }

    /// Handle an im.message.receive_v1 event.
    async fn handle_message_event(&mut self, event: EventPayload) -> Result<()> {
        // Skip messages from bots (including ourselves)
        if event.sender.sender_type.open_id().is_none() {
            return Ok(());
        }

        let sender_open_id = match event.sender.sender_type.open_id() {
            Some(id) => id.to_string(),
            None => return Ok(()),
        };

        // Skip our own messages
        if Some(&sender_open_id) == self.bot_open_id.as_ref() {
            return Ok(());
        }

        let msg = &event.message;
        let chat_id = &msg.chat_id;
        let message_id = &msg.message_id;

        // Allowlist check
        if !self.core.is_allowed(&sender_open_id) {
            crate::log_info!("Lark unauthorized user: {sender_open_id}");
            return Ok(());
        }

        // Parse content
        let content = Self::parse_content(msg);
        if content.trim().is_empty() {
            return Ok(());
        }

        // Mention-only check for group chats
        if msg.chat_type == "group" && self.mention_only {
            if let Some(ref bot_id) = self.bot_open_id {
                if !Self::is_mentioned(&content, bot_id) {
                    return Ok(());
                }
            }
        }

        // Clean @mention from content
        let clean_content = if let Some(ref bot_id) = self.bot_open_id {
            let mention = format!("@_user_{}", bot_id);
            content.replace(&mention, "").trim().to_string()
        } else {
            content.trim().to_string()
        };

        if clean_content.is_empty() {
            return Ok(());
        }

        crate::log_info!(
            "Lark Message from {sender_open_id} in {chat_id}: {clean_content}"
        );

        // Add ack reaction (best-effort)
        let _ = self.client.add_reaction(message_id, "OK").await;

        // Shell command
        if let Some(cmd) = clean_content.strip_prefix('!') {
            let cmd = cmd.trim();
            if !cmd.is_empty() {
                return self.handle_shell_command(chat_id, message_id, cmd).await;
            }
        }

        // Model selection state
        if let Some(state) = self.core.model_selection_states.get(chat_id).cloned() {
            return self
                .handle_model_selection(chat_id, message_id, &state, &clean_content)
                .await;
        }

        // Parse command
        if let Some(command) = parse_command(&clean_content) {
            if command.name == "model" {
                model_selection::start_model_selection(self, chat_id).await?;
                return Ok(());
            }

            let mut active_model =
                self.core.resolve_chat_model(&format!("lark:{chat_id}"))?;
            let chat_key = format!("lark:{chat_id}");
            let mut conversation =
                self.core.load_or_create_conversation(&chat_key, &active_model)?;
            self.core
                .load_system_prompt(&conversation, &mut active_model);
            self.core
                .mode_manager
                .restore_from_messages(&chat_key, &conversation.messages);

            let mut sender = LarkSender {
                client: &self.client,
            };
            let handled = self
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
        let mut active_model = self.core.resolve_chat_model(&format!("lark:{chat_id}"))?;
        let chat_key = format!("lark:{chat_id}");
        let mut conversation = self.core.load_or_create_conversation(&chat_key, &active_model)?;
        self.core
            .load_system_prompt(&conversation, &mut active_model);
        self.core
            .mode_manager
            .restore_from_messages(&chat_key, &conversation.messages);
        self.core
            .persist_user_message(&mut conversation, &chat_key, &clean_content)?;

        let mut sender = LarkSender {
            client: &self.client,
        };

        if let Err(error) = self
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
            self.core
                .store
                .append_message(conversation.session_id, &error_message)?;
            let _ = self.client.send_text_message(chat_id, &error_text).await;
        }

        Ok(())
    }

    async fn handle_shell_command(
        &mut self,
        chat_id: &str,
        msg_id: &str,
        command: &str,
    ) -> Result<()> {
        let chat_key = format!("lark:{chat_id}");
        let mut sender = LarkSender {
            client: &self.client,
        };
        self.core
            .handle_shell_command(&mut sender, chat_id, Some(msg_id), command, &chat_key)
            .await
    }

    async fn handle_model_selection(
        &mut self,
        chat_id: &str,
        _msg_id: &str,
        state: &ModelSelectionState,
        content: &str,
    ) -> Result<()> {
        let id = chat_id.to_string();
        model_selection::handle_step(self, &id, state, content).await
    }

    /// Send a message choosing between text and interactive card based on length.
    async fn send_message_adaptive(&mut self, recipient: &str, content: &str) -> Result<()> {
        if content.len() <= LARK_TEXT_MAX_LENGTH {
            self.client.send_text_message(recipient, content).await?;
        } else {
            self.client.send_card_message(recipient, content).await?;
        }
        Ok(())
    }
}

// ── Lark MessageSender ───────────────────────────────────────────────────────

struct LarkSender<'a> {
    client: &'a LarkClient,
}

#[async_trait]
impl MessageSender for LarkSender<'_> {
    async fn send_message(
        &mut self,
        recipient: &str,
        text: &str,
        _reply_to: Option<&str>,
    ) -> Result<()> {
        // Short messages use text, long messages use interactive card
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
        let msg_id = self.client.send_text_message(recipient, text).await?;
        Ok(Some(msg_id))
    }

    async fn update_draft(
        &mut self,
        _recipient: &str,
        _msg_id: &str,
        _text: &str,
    ) -> Result<()> {
        // Lark does not support editing messages via the API.
        // We re-send instead (the draft is replaced with a new message).
        Ok(())
    }

    async fn finalize_draft(
        &mut self,
        recipient: &str,
        _msg_id: &str,
        text: &str,
    ) -> Result<()> {
        // Send the final response (draft editing is not supported, send fresh)
        self.send_message(recipient, text, None).await
    }

    async fn cancel_draft(
        &mut self,
        _recipient: &str,
        _msg_id: &str,
    ) -> Result<()> {
        // Cannot delete messages via Lark API; just ignore
        Ok(())
    }
}

// ── ModelSelectionIO for LarkChannel ─────────────────────────────────────────

#[async_trait]
impl ModelSelectionIO for LarkChannel {
    type Id = String;

    async fn send_message(&mut self, id: &String, text: &str) -> Result<()> {
        self.send_message_adaptive(id, text).await
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
impl Channel for LarkChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_LARK
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.core.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            crate::log_info!("Lark channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        self.core.restore_sessions(store)
    }

    fn supports_draft_updates(&self) -> bool {
        false
    }

    async fn send_draft(&mut self, message: &crate::gateway::channel::SendMessage) -> Result<Option<String>> {
        // Lark doesn't support editing, so send as regular message
        let msg_id = self.client.send_text_message(&message.recipient, &message.content).await?;
        Ok(Some(msg_id))
    }

    async fn update_draft(
        &mut self,
        _recipient: &str,
        _message_id: &str,
        _text: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn finalize_draft(
        &mut self,
        recipient: &str,
        _message_id: &str,
        text: &str,
    ) -> Result<()> {
        self.send_message_adaptive(recipient, text).await
    }

    async fn cancel_draft(&mut self, _recipient: &str, _message_id: &str) -> Result<()> {
        Ok(())
    }
}
