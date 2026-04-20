use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use uuid::Uuid;

use crate::{
    config::{AppConfig, AuthStore},
    llm::LlmClient,
    session::{AssistantTurn, Conversation, Message, MessageRole, ToolCall, ToolExecutionResult},
    storage::SessionStore,
    tooling::ToolRegistry,
};

use super::qq_client::QQClient;

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

pub struct QQGatewayRunner {
    pub workspace_root: PathBuf,
    pub config: AppConfig,
    pub auth: AuthStore,
    pub store: SessionStore,
    pub llm: LlmClient,
    pub tools: ToolRegistry,
    pub instruction_prompt: String,
    pub allowlist: HashSet<String>,
    pub client: QQClient,
    pub session_id: Option<String>,
    pub last_seq: Option<u32>,
}

impl QQGatewayRunner {
    pub async fn run_loop(&mut self) -> Result<()> {
        loop {
            if let Err(e) = self.connect_and_handle().await {
                crate::log_error!("QQ Gateway connection error: {e}. Retrying in 5s...");
                sleep(Duration::from_secs(5)).await;
            }
        }
    }

    async fn connect_and_handle(&mut self) -> Result<()> {
        let gateway_url = self.client.get_gateway_url().await?;
        let (ws_stream, _) = connect_async(gateway_url.as_str()).await?;
        let (mut write, mut read) = ws_stream.split();

        crate::log_info!("QQ Gateway connected to {}", gateway_url);

        let mut heartbeat_interval = 45000;
        let mut _last_heartbeat_ack = Instant::now();

        // Handle Hello
        if let Some(msg) = read.next().await {
            let msg = msg?;
            if let WsMessage::Text(text) = msg {
                let payload: WsPayload = serde_json::from_str(&text)?;
                if payload.op == 10 {
                    let hello: HelloData = serde_json::from_value(payload.d.unwrap())?;
                    heartbeat_interval = hello.heartbeat_interval;
                    crate::log_info!("QQ Hello received, heartbeat: {}ms", heartbeat_interval);
                }
            }
        }

        // Identify or Resume
        let token = self.client.get_access_token().await?;
        let identify = if let (Some(sid), Some(seq)) = (&self.session_id, self.last_seq) {
            crate::log_info!("QQ Attempting resume, session_id: {}, seq: {}", sid, seq);
            serde_json::json!({
                "op": 6,
                "d": {
                    "token": format!("QQBot {}", token),
                    "session_id": sid,
                    "seq": seq,
                }
            })
        } else {
            crate::log_info!("QQ Attempting identify");
            serde_json::json!({
                "op": 2,
                "d": {
                    "token": format!("QQBot {}", token),
                    "intents": 1 << 30, // GUILD_MESSAGES / AT_MESSAGES
                    "shard": [0, 1],
                }
            })
        };

        write
            .send(WsMessage::Text(identify.to_string().into()))
            .await?;

        let mut heartbeat_timer = tokio::time::interval(Duration::from_millis(heartbeat_interval));

        loop {
            tokio::select! {
                _ = heartbeat_timer.tick() => {
                    let hb = serde_json::json!({
                        "op": 1,
                        "d": self.last_seq
                    });
                    write.send(WsMessage::Text(hb.to_string().into())).await?;
                }
                msg = read.next() => {
                    match msg {
                        Some(Ok(WsMessage::Text(text))) => {
                            let payload: WsPayload = serde_json::from_str(&text)?;
                            if let Some(s) = payload.s {
                                self.last_seq = Some(s);
                            }

                            match payload.op {
                                0 => { // Dispatch
                                    if let Some(t) = payload.t.as_deref() {
                                        match t {
                                            "READY" => {
                                                let ready: ReadyData = serde_json::from_value(payload.d.unwrap())?;
                                                self.session_id = Some(ready.session_id);
                                                crate::log_info!("QQ Ready, session_id: {}", self.session_id.as_ref().unwrap());
                                            }
                                            "AT_MESSAGE_CREATE" | "MESSAGE_CREATE" => {
                                                self.handle_message(payload.d.unwrap()).await?;
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                11 => { // Heartbeat ACK
                                    _last_heartbeat_ack = Instant::now();
                                }
                                _ => {}
                            }
                        }
                        Some(Ok(WsMessage::Close(_))) | None => {
                            return Err(anyhow!("QQ WebSocket closed"));
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    async fn handle_message(&mut self, data: serde_json::Value) -> Result<()> {
        let channel_id = data["channel_id"]
            .as_str()
            .context("missing channel_id")?
            .to_string();
        let author_id = data["author"]["id"].as_str().context("missing author id")?;
        let msg_id = data["id"]
            .as_str()
            .context("missing message id")?
            .to_string();
        let content = data["content"].as_str().unwrap_or_default().trim();

        if !self.allowlist.contains(author_id) {
            crate::log_info!("QQ Message from unauthorized user: {}", author_id);
            return Ok(());
        }

        // Clean @bot if present
        let clean_content = if let Some(pos) = content.find(' ') {
            &content[pos + 1..]
        } else {
            content
        };

        crate::log_info!("QQ Message from {}: {}", author_id, clean_content);

        let active_model = self.config.resolve_active_model(&self.auth)?;
        let chat_key = format!("qq:{}", channel_id);

        let mut conversation = self.load_or_create_conversation(&chat_key, &active_model)?;

        let user_message = Message::new(MessageRole::User, clean_content.to_string());
        conversation.push(user_message.clone());
        self.store
            .append_message(conversation.session_id, &user_message)?;

        if conversation.messages.len() == 1 || conversation.title == "Untitled session" {
            conversation.update_title_from_prompt(clean_content);
            self.store
                .update_session_title(conversation.session_id, &conversation.title)?;
        }

        if let Err(error) = self
            .run_agent_with_tools(&channel_id, &msg_id, &mut conversation, &active_model)
            .await
        {
            let error_text = format!("Gateway error: {error}");
            let error_message = Message::new(MessageRole::Error, error_text.clone());
            self.store
                .append_message(conversation.session_id, &error_message)?;
            self.client
                .send_message(&channel_id, &error_text, Some(&msg_id))
                .await?;
        }

        Ok(())
    }

    fn load_or_create_conversation(
        &self,
        chat_key: &str,
        active_model: &crate::config::ActiveModel,
    ) -> Result<Conversation> {
        if let Some(session_id) = self.store.load_gateway_chat_session("qq", chat_key)?
            && let Some(record) = self.store.load_session_record(session_id)? {
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
        let title = "Untitled session".to_string();
        self.store.create_session(
            session_id,
            &self.workspace_root,
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &title,
        )?;
        self.store
            .set_gateway_chat_session("qq", chat_key, session_id)?;

        let now = Utc::now();
        Ok(Conversation {
            session_id,
            parent_session_id: None,
            workspace_root: self.workspace_root.display().to_string(),
            provider_id: active_model.provider_id.clone(),
            provider_display_name: active_model.provider_display_name.clone(),
            model_id: active_model.model_id.clone(),
            model_display_name: active_model.display_name.clone(),
            title,
            created_at: now,
            updated_at: now,
            context_summary: None,
            context_retained_from: 0,
            messages: Vec::new(),
            revert_message_id: None,
        })
    }

    async fn run_agent_with_tools(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        conversation: &mut Conversation,
        active_model: &crate::config::ActiveModel,
    ) -> Result<()> {
        crate::log_info!(
            "Starting QQ agent: channel_id={}, model={}, session={}",
            channel_id,
            active_model.label(),
            conversation.session_id
        );

        let runtime = tokio::runtime::Handle::current();

        for _ in 1..=8 {
            let turn = self
                .run_single_streaming_turn(conversation, active_model)
                .await?;

            if turn.tool_calls.is_empty() {
                let final_text = turn.content.trim();
                if !final_text.is_empty() {
                    self.client
                        .send_message(channel_id, final_text, Some(msg_id))
                        .await?;
                }
                return Ok(());
            }

            self.execute_tool_calls(&runtime, conversation, turn.tool_calls)?;
        }

        bail!("assistant exceeded maximum tool rounds; aborting to prevent loop")
    }

    async fn run_single_streaming_turn(
        &mut self,
        conversation: &mut Conversation,
        active_model: &crate::config::ActiveModel,
    ) -> Result<AssistantTurn> {
        self.tools.set_active_model(active_model.clone());

        let context_manager = crate::context::ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );

        let request_messages = context_manager
            .build_request_messages(conversation, crate::prompts::SessionMode::Build);
        let tool_definitions = self.tools.all_definitions();

        let mut request_model = active_model.clone();
        request_model.system_prompt =
            super::compose_system_prompt(&active_model.system_prompt, &self.instruction_prompt);

        let turn = self
            .llm_completion_turn(&request_model, request_messages, tool_definitions)
            .await?;

        let mut assistant_message = Message::new(MessageRole::Assistant, turn.content.clone());
        assistant_message.tool_calls = turn.tool_calls.clone();
        assistant_message.reasoning = turn.reasoning.clone();

        conversation.push(assistant_message.clone());
        self.store
            .append_message(conversation.session_id, &assistant_message)?;

        Ok(turn)
    }

    async fn llm_completion_turn(
        &self,
        model: &crate::config::ActiveModel,
        messages: Vec<Message>,
        tools: Vec<crate::tooling::ToolDefinition>,
    ) -> Result<AssistantTurn> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let session_id = Uuid::new_v4();
        let request_id = 1;

        let client = self.llm.clone();
        let model = model.clone();

        tokio::spawn(async move {
            client
                .stream_chat(session_id, request_id, model, messages, tools, tx)
                .await;
        });

        let mut turn = AssistantTurn::default();
        while let Some(event) = rx.recv().await {
            match event {
                crate::session::BackendEvent::Delta { content, .. } => {
                    turn.content.push_str(&content);
                }
                crate::session::BackendEvent::ReasoningDelta { content, .. } => {
                    turn.reasoning.push_str(&content);
                }
                crate::session::BackendEvent::ToolCallUpdated { tool_call, .. } => {
                    if let Some(existing) =
                        turn.tool_calls.iter_mut().find(|tc| tc.id == tool_call.id)
                    {
                        *existing = tool_call;
                    } else {
                        turn.tool_calls.push(tool_call);
                    }
                }
                crate::session::BackendEvent::Failed { error, .. } => {
                    bail!("LLM Error: {}", error);
                }
                crate::session::BackendEvent::Finished {
                    turn: assistant_turn,
                    ..
                } => {
                    turn = assistant_turn;
                    break;
                }
                _ => {}
            }
        }

        Ok(turn)
    }

    fn execute_tool_calls(
        &mut self,
        runtime: &tokio::runtime::Handle,
        conversation: &mut Conversation,
        tool_calls: Vec<ToolCall>,
    ) -> Result<()> {
        for tool_call in tool_calls {
            crate::log_info!("Executing tool: {}", tool_call.name);
            let result =
                self.tools
                    .execute_call(runtime, &self.store, conversation.session_id, &tool_call);

            let execution_result = match result {
                Ok(res) => res,
                Err(error) => ToolExecutionResult::new(format!("Error: {error}")),
            };

            let tool_message =
                Message::tool_result(&tool_call.id, &tool_call.name, execution_result);
            conversation.push(tool_message.clone());
            self.store
                .append_message(conversation.session_id, &tool_message)?;
        }
        Ok(())
    }
}
