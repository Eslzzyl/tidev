use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use uuid::Uuid;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore},
    llm::LlmClient,
    session::{AssistantTurn, Conversation, Message, MessageRole, ToolCall, ToolExecutionResult},
    storage::SessionStore,
    tooling::ToolRegistry,
};

use super::channel::Channel;
use super::commands::{CommandInvocation, format_status_summary, gateway_help_text, parse_command};
use super::qq_client::QQClient;
use super::shared;

pub const GATEWAY_PLATFORM_QQ: &str = "qq";

/// Interactive model selection state for a channel.
#[derive(Debug, Clone)]
enum ModelSelectionState {
    /// Waiting for user to select a provider (1, 2, 3, ...)
    WaitingForProvider,
    /// Waiting for user to select a model (1, 2, 3, ...) for the given provider.
    WaitingForModel { provider_id: String },
}

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
    /// Gateway start time for uptime calculation.
    pub start_time: Instant,
    /// Cancellation flags per channel_id for /stop command.
    /// When set to true, the current task will be stopped after current streaming completes.
    cancellation_flags: HashMap<String, Arc<AtomicBool>>,
    /// Interactive model selection state per channel_id.
    /// When a user is in this state, their next message is handled as selection input,
    /// not sent to the agent.
    model_selection_states: HashMap<String, ModelSelectionState>,
}

impl QQChannel {
    /// Create a new QQ channel.
    pub fn new(
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: LlmClient,
        tools: ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        app_id: String,
        app_secret: String,
        sandbox: bool,
    ) -> Self {
        Self {
            workspace_root,
            config,
            auth,
            store,
            llm,
            tools,
            instruction_prompt,
            allowlist,
            client: QQClient::new(app_id, app_secret, sandbox),
            session_id: None,
            last_seq: None,
            start_time: Instant::now(),
            cancellation_flags: HashMap::new(),
            model_selection_states: HashMap::new(),
        }
    }

    async fn run_loop(&mut self) -> Result<()> {
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

        // Clean @bot if present (QQ mentions are like <@!123456>)
        let clean_content = if let Some(pos) = content.find(' ') {
            &content[pos + 1..]
        } else {
            content
        };

        crate::log_info!("QQ Message from {}: {}", author_id, clean_content);

        // Check if user is in interactive model selection state.
        // If so, handle selection input instead of normal message processing.
        if let Some(state) = self.model_selection_states.get(&channel_id).cloned() {
            crate::log_info!("Handling model selection input: channel_id={}", channel_id);
            return self
                .handle_model_selection(&channel_id, &msg_id, &state, clean_content)
                .await;
        }

        // Check if this is a slash command
        if let Some(command) = parse_command(clean_content) {
            crate::log_info!("QQ Executing command: /{} {:?}", command.name, command.args);
            let mut active_model = self.config.resolve_active_model_for_gateway(&self.auth)?;
            let chat_key = format!("qq:{}", channel_id);
            let mut conversation = self.load_or_create_conversation(&chat_key, &active_model)?;

            let handled = self
                .handle_command(
                    &channel_id,
                    &msg_id,
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

        let active_model = self.config.resolve_active_model_for_gateway(&self.auth)?;
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
        if let Some(session_id) = self.store.load_gateway_chat_session(GATEWAY_PLATFORM_QQ, chat_key)?
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
            .set_gateway_chat_session(GATEWAY_PLATFORM_QQ, chat_key, session_id)?;

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
            // Check for cancellation at the start of each round
            if self.check_cancellation(channel_id) {
                crate::log_info!("Task cancelled by user: channel_id={}", channel_id);

                // Send cancellation confirmation
                self.client
                    .send_message(channel_id, "🛑 Task stopped.", Some(msg_id))
                    .await?;
                return Ok(());
            }

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

                let mut assistant_message =
                    Message::new(MessageRole::Assistant, turn.content.clone());
                assistant_message.reasoning = turn.reasoning.clone();
                conversation.push(assistant_message.clone());
                self.store
                    .append_message(conversation.session_id, &assistant_message)?;

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
            shared::compose_system_prompt(&active_model.system_prompt, &self.instruction_prompt);

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

    async fn handle_command(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        chat_key: &str,
        conversation: &mut Conversation,
        active_model: &mut ActiveModel,
        command: CommandInvocation,
    ) -> Result<bool> {
        match command.name.as_str() {
            "new" => {
                *conversation = self.rotate_chat_session(chat_key, active_model)?;
                self.client
                    .send_message(channel_id, "Started a fresh session.", Some(msg_id))
                    .await?;
                Ok(true)
            }
            "session" => {
                if let Some(new_model) = self
                    .handle_session_command(
                        channel_id,
                        msg_id,
                        chat_key,
                        conversation,
                        active_model,
                        command.args,
                        None,
                    )
                    .await?
                {
                    *active_model = new_model;
                }
                Ok(true)
            }
            "model" => {
                self.handle_model_command(channel_id, msg_id).await?;
                Ok(true)
            }
            "help" => {
                self.client
                    .send_message(channel_id, &gateway_help_text(), Some(msg_id))
                    .await?;
                Ok(true)
            }
            "status" => {
                self.handle_status_command(channel_id, msg_id, conversation, active_model)
                    .await?;
                Ok(true)
            }
            "stop" => {
                self.handle_stop_command(channel_id, msg_id, channel_id).await?;
                Ok(true)
            }
            _ => {
                self.client
                    .send_message(
                        channel_id,
                        &format!(
                            "Unknown command: {}\n\n{}",
                            command.name,
                            gateway_help_text()
                        ),
                        Some(msg_id),
                    )
                    .await?;
                Ok(true)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_session_command(
        &self,
        channel_id: &str,
        msg_id: &str,
        _chat_key: &str,
        conversation: &Conversation,
        active_model: &ActiveModel,
        args: Vec<String>,
        new_active_model: Option<ActiveModel>,
    ) -> Result<Option<ActiveModel>> {
        let updated_model = new_active_model;

        match args.first().map(|s| s.as_str()) {
            None | Some("") => {
                let text = format_session_summary(conversation, active_model);
                self.client
                    .send_message(channel_id, &text, Some(msg_id))
                    .await?;
            }
            _ => {
                self.client
                    .send_message(channel_id, &gateway_help_text(), Some(msg_id))
                    .await?;
            }
        }

        Ok(updated_model)
    }

    /// Handle /model command - start interactive provider/model selection.
    async fn handle_model_command(&mut self, channel_id: &str, msg_id: &str) -> Result<()> {
        // Get available providers (user config + bundled, only those with valid auth)
        let providers = self.get_available_providers();

        if providers.is_empty() {
            self.client
                .send_message(channel_id, "No available providers found. Please check your configuration.", Some(msg_id))
                .await?;
            return Ok(());
        }

        // Format provider list
        let mut text = String::from("Select a provider (enter number):\n\n");
        for (i, provider) in providers.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, provider.1));
        }
        text.push_str("\n(Enter any other number to cancel)");

        self.client.send_message(channel_id, &text, Some(msg_id)).await?;

        // Set state to waiting for provider selection
        self.model_selection_states
            .insert(channel_id.to_string(), ModelSelectionState::WaitingForProvider);

        Ok(())
    }

    /// Handle interactive model selection input.
    async fn handle_model_selection(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        state: &ModelSelectionState,
        content: &str,
    ) -> Result<()> {
        // Check if it's a command - cancel selection if so
        if content.starts_with('/') {
            self.model_selection_states.remove(channel_id);
            self.client
                .send_message(channel_id, "Selection cancelled. Send /model to try again.", Some(msg_id))
                .await?;
            return Ok(());
        }

        match state {
            ModelSelectionState::WaitingForProvider => {
                // Parse provider selection
                let selection: usize = match content.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        self.model_selection_states.remove(channel_id);
                        self.client
                            .send_message(channel_id, "Invalid selection. Selection cancelled. Send /model to try again.", Some(msg_id))
                            .await?;
                        return Ok(());
                    }
                };

                let providers = self.get_available_providers();
                if selection < 1 || selection > providers.len() {
                    self.model_selection_states.remove(channel_id);
                    self.client
                        .send_message(channel_id, "Selection cancelled. Send /model to try again.", Some(msg_id))
                        .await?;
                    return Ok(());
                }

                let (provider_id, _provider_name) = &providers[selection - 1];

                // Get models for selected provider
                let models = self.get_models_for_provider(provider_id);
                if models.is_empty() {
                    self.model_selection_states.remove(channel_id);
                    self.client
                        .send_message(channel_id, "No models available for this provider. Selection cancelled.", Some(msg_id))
                        .await?;
                    return Ok(());
                }

                // Format model list
                let mut text = format!(
                    "Select a model for {} (enter number):\n\n",
                    provider_id
                );
                for (i, model) in models.iter().enumerate() {
                    text.push_str(&format!("{}. {}\n", i + 1, model.1));
                }
                text.push_str("\n(Enter any other number to cancel)");

                self.client.send_message(channel_id, &text, Some(msg_id)).await?;

                // Set state to waiting for model selection
                self.model_selection_states.insert(
                    channel_id.to_string(),
                    ModelSelectionState::WaitingForModel {
                        provider_id: provider_id.clone(),
                    },
                );
            }
            ModelSelectionState::WaitingForModel { provider_id } => {
                // Parse model selection
                let selection: usize = match content.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        self.model_selection_states.remove(channel_id);
                        self.client
                            .send_message(channel_id, "Invalid selection. Selection cancelled. Send /model to try again.", Some(msg_id))
                            .await?;
                        return Ok(());
                    }
                };

                let models = self.get_models_for_provider(provider_id);
                if selection < 1 || selection > models.len() {
                    self.model_selection_states.remove(channel_id);
                    self.client
                        .send_message(channel_id, "Selection cancelled. Send /model to try again.", Some(msg_id))
                        .await?;
                    return Ok(());
                }

                let (model_id, _model_name) = &models[selection - 1];

                // Save the model selection
                let chat_key = format!("qq:{}", channel_id);
                self.store.set_gateway_chat_model(
                    GATEWAY_PLATFORM_QQ,
                    &chat_key,
                    provider_id,
                    model_id,
                )?;

                // Clear state
                self.model_selection_states.remove(channel_id);

                // Send success message
                let success_text = format!(
                    "Model switched to {}/{}\n\nSend /model to change again.",
                    provider_id, model_id
                );
                self.client.send_message(channel_id, &success_text, Some(msg_id)).await?;
            }
        }

        Ok(())
    }

    /// Get available providers (user config + bundled) that have valid auth.
    fn get_available_providers(&self) -> Vec<(String, String)> {
        let mut providers = Vec::new();

        // Check user-configured providers
        for (id, config) in &self.config.providers {
            if let Some(auth) = self.auth.providers.get(id) {
                if auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty()) {
                    providers.push((id.clone(), config.display_name.clone()));
                }
            }
        }

        // Check bundled providers
        for (id, config) in &self.config.bundled_providers {
            // Skip if already added from user config
            if self.config.providers.contains_key(id) {
                continue;
            }
            if let Some(auth) = self.auth.providers.get(id) {
                if auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty()) {
                    providers.push((id.clone(), config.display_name.clone()));
                }
            }
        }

        providers
    }

    /// Get models for a specific provider.
    fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)> {
        let mut models = Vec::new();

        // Check user-configured providers first
        if let Some(config) = self.config.providers.get(provider_id) {
            for (id, model_config) in &config.models {
                models.push((id.clone(), model_config.display_name.clone()));
            }
        }

        // Check bundled providers if not found
        if models.is_empty() {
            if let Some(config) = self.config.bundled_providers.get(provider_id) {
                for (id, model_config) in &config.models {
                    models.push((id.clone(), model_config.display_name.clone()));
                }
            }
        }

        models
    }

    fn rotate_chat_session(
        &self,
        chat_key: &str,
        active_model: &ActiveModel,
    ) -> Result<Conversation> {
        let conversation = self.create_gateway_session(active_model)?;
        self.store
            .set_gateway_chat_session(GATEWAY_PLATFORM_QQ, chat_key, conversation.session_id)?;
        Ok(conversation)
    }

    fn create_gateway_session(&self, active_model: &ActiveModel) -> Result<Conversation> {
        let session_id = Uuid::new_v4();
        let title = "Untitled session".to_string();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            active_model.provider_id.clone(),
            active_model.provider_display_name.clone(),
            active_model.model_id.clone(),
            active_model.display_name.clone(),
            title,
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
        Ok(conversation)
    }

    /// Handle /status command - show session statistics.
    async fn handle_status_command(
        &self,
        channel_id: &str,
        msg_id: &str,
        conversation: &Conversation,
        active_model: &ActiveModel,
    ) -> Result<()> {
        // Count messages by role
        let user_message_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .count();
        let assistant_message_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Assistant)
            .count();

        // Get tool call count from database
        let tool_call_count = self
            .store
            .count_tool_events(conversation.session_id)
            .unwrap_or(0);

        // Get token stats from database
        let token_stats = self
            .store
            .get_session_token_stats(conversation.session_id)
            .unwrap_or(crate::storage::SessionTokenStats {
                input_tokens: 0,
                output_tokens: 0,
            });

        let text = format_status_summary(
            &conversation.session_id.to_string(),
            &conversation.title,
            conversation.messages.len(),
            user_message_count,
            assistant_message_count,
            tool_call_count,
            &active_model.provider_id,
            &active_model.model_id,
            active_model.context_window,
            token_stats.input_tokens,
            token_stats.output_tokens,
            self.start_time,
            None, // Average response time - could be tracked if needed
        );

        self.client.send_message(channel_id, &text, Some(msg_id)).await
    }

    /// Handle /stop command - set cancellation flag for current task.
    async fn handle_stop_command(
        &mut self,
        channel_id: &str,
        msg_id: &str,
        _chat_key: &str,
    ) -> Result<()> {
        // Get or create cancellation flag for this channel
        let flag = self
            .cancellation_flags
            .entry(channel_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)));

        // Check if there's already a task running
        if flag.load(Ordering::SeqCst) {
            // Already stopping
            self.client
                .send_message(channel_id, "Already stopping...", Some(msg_id))
                .await?;
        } else {
            // Set the cancellation flag
            flag.store(true, Ordering::SeqCst);
            // We'll send confirmation after the task actually stops
            // The actual stopping is handled in run_agent_with_tools
        }
        Ok(())
    }

    /// Check and clear cancellation flag, return true if cancelled.
    fn check_cancellation(&self, channel_id: &str) -> bool {
        if let Some(flag) = self.cancellation_flags.get(channel_id) {
            if flag.load(Ordering::SeqCst) {
                // Clear the flag
                flag.store(false, Ordering::SeqCst);
                return true;
            }
        }
        false
    }
}

impl Channel for QQChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_QQ
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            crate::log_info!("QQ channel ready");
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let sessions = store.list_gateway_chat_sessions(GATEWAY_PLATFORM_QQ)?;
        let mut count = 0;
        let mut orphans_closed = 0;

        for (chat_key, session_id) in sessions {
            if let Some(_conversation) = store.load_conversation(session_id)? {
                let messages = store.load_messages(session_id)?;

                // Check for orphaned user turn (crash mid-query)
                if let Some(last) = messages.last() {
                    if last.role == MessageRole::User {
                        // Close orphan with marker to prevent LLM from continuing the old request
                        let marker = Message::new(
                            MessageRole::Assistant,
                            "[Session interrupted — not continuing this request]".to_string(),
                        );
                        store.append_message(session_id, &marker)?;
                        orphans_closed += 1;
                    }
                }

                count += 1;
                crate::log_info!(
                    "Restored QQ session: chat_key={}, session_id={}, messages={}",
                    chat_key,
                    session_id,
                    messages.len()
                );
            }
        }

        if count > 0 {
            crate::log_info!("Restored {} QQ session(s) from disk", count);
        }
        if orphans_closed > 0 {
            crate::log_info!(
                "Closed {} orphaned session turn(s) from previous crash",
                orphans_closed
            );
        }

        Ok(count)
    }
}

fn format_session_summary(conversation: &Conversation, active_model: &ActiveModel) -> String {
    format!(
        "Session status\n- session_id: {}\n- title: {}\n- message_count: {}\n- model: {}/{}",
        conversation.session_id,
        conversation.title,
        conversation.messages.len(),
        active_model.provider_id,
        active_model.model_id
    )
}
