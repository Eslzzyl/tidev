//! Telegram channel implementation.

use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use tokio::sync::mpsc::unbounded_channel;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore},
    context::ContextManager,
    llm::LlmClient,
    prompts::{SessionMode, gateway_system_prompt},
    session::{AssistantTurn, BackendEvent, Conversation, Message, MessageRole, ToolCall},
    storage::SessionStore,
    tooling::ToolRegistry,
};

use super::bot::TelegramBot;
use super::types::TelegramMessage;
use crate::gateway::channel::Channel;
use crate::gateway::channel::SendMessage;
use crate::gateway::commands::{
    CommandInvocation, GATEWAY_COMMANDS, format_status_summary, gateway_help_text, parse_command,
};
use crate::gateway::shared::compose_system_prompt;

pub const GATEWAY_PLATFORM_TELEGRAM: &str = "telegram";
pub const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_DRAFT_EDIT_INTERVAL_MS: u64 = 1200;

/// Interactive model selection state for a chat.
#[derive(Debug, Clone)]
enum ModelSelectionState {
    /// Waiting for user to select a provider (1, 2, 3, ...)
    WaitingForProvider,
    /// Waiting for user to select a model (1, 2, 3, ...) for the given provider.
    WaitingForModel { provider_id: String },
}

/// Interactive balance selection state for a chat.
#[derive(Debug, Clone)]
enum BalanceSelectionState {
    /// Waiting for user to select a provider (1, 2, 3, ...)
    WaitingForProvider,
}

/// Telegram gateway channel implementation.
pub struct TelegramChannel {
    pub workspace_root: PathBuf,
    pub config: AppConfig,
    pub auth: AuthStore,
    pub store: SessionStore,
    pub llm: LlmClient,
    pub tools: ToolRegistry,
    pub instruction_prompt: String,
    pub allowlist: HashSet<String>,
    pub poll_timeout_secs: u64,
    pub bot: TelegramBot,
    pub offset: i64,
    pub request_seq: u64,
    /// Gateway start time for uptime calculation.
    pub start_time: Instant,
    /// Cancellation flags per chat_id for /stop command.
    /// When set to true, the current task will be stopped after current streaming completes.
    cancellation_flags: HashMap<i64, Arc<AtomicBool>>,
    /// Interactive model selection state per chat_id.
    /// When a user is in this state, their next message is handled as selection input,
    /// not sent to the agent.
    model_selection_states: HashMap<i64, ModelSelectionState>,
    /// Interactive balance selection state per chat_id.
    /// When a user is in this state, their next message is handled as selection input,
    /// not sent to the agent.
    balance_selection_states: HashMap<i64, BalanceSelectionState>,
    /// Sessions that are currently compacting.
    compacting_sessions: HashSet<Uuid>,
}

impl TelegramChannel {
    /// Create a new Telegram channel.
    pub fn new(
        workspace_root: PathBuf,
        config: AppConfig,
        auth: AuthStore,
        store: SessionStore,
        llm: LlmClient,
        tools: ToolRegistry,
        instruction_prompt: String,
        allowlist: HashSet<String>,
        poll_timeout_secs: u64,
        bot_token: String,
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
            poll_timeout_secs,
            bot: TelegramBot::new(bot_token),
            offset: 0,
            request_seq: 0,
            start_time: Instant::now(),
            cancellation_flags: HashMap::new(),
            model_selection_states: HashMap::new(),
            balance_selection_states: HashMap::new(),
            compacting_sessions: HashSet::new(),
        }
    }

    async fn bootstrap_offset(&mut self) -> Result<()> {
        let updates = self.bot.get_updates(0, 0).await?;
        if let Some(last) = updates.last() {
            self.offset = last.update_id.saturating_add(1);
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

    async fn handle_message(&mut self, message: TelegramMessage) -> Result<()> {
        if !self.is_allowed(&message) {
            crate::log_debug!(
                "Message from chat_id={} not in allowlist, skipping",
                message.chat.id
            );
            return Ok(());
        }

        // Add receipt reaction
        if let Err(e) = self
            .bot
            .set_message_reaction(message.chat.id, message.message_id, "👀")
            .await
        {
            crate::log_warn!(
                "Failed to set message reaction for chat_id={}: {}",
                message.chat.id,
                e
            );
        }

        let Some(content) = message
            .text
            .as_deref()
            .or(message.caption.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            crate::log_debug!(
                "Message chat_id={} has no text content, skipping",
                message.chat.id
            );
            return Ok(());
        };

        // Check if user is in interactive model selection state.
        // If so, handle selection input instead of normal message processing.
        if let Some(state) = self.model_selection_states.get(&message.chat.id).cloned() {
            crate::log_info!(
                "Handling model selection input: chat_id={}",
                message.chat.id
            );
            return self.handle_model_selection(&message, &state).await;
        }

        // Check if user is in interactive balance selection state.
        // If so, handle selection input instead of normal message processing.
        if let Some(state) = self.balance_selection_states.get(&message.chat.id).cloned() {
            crate::log_info!(
                "Handling balance selection input: chat_id={}",
                message.chat.id
            );
            return self.handle_balance_selection(&message, &state).await;
        }

        let chat_key = self.chat_key(&message);
        let mut active_model = self.resolve_chat_model(&chat_key)?;
        let mut conversation = self.load_or_create_chat_conversation(&chat_key, &active_model)?;

        crate::log_info!(
            "Processing message: chat_id={}, session_id={}, content_len={}",
            message.chat.id,
            conversation.session_id,
            content.len()
        );

        if let Some(command) = parse_command(content) {
            crate::log_info!("Executing command: /{} {:?}", command.name, command.args);
            if self
                .handle_command(
                    &message,
                    &chat_key,
                    &mut conversation,
                    &mut active_model,
                    command,
                )
                .await?
            {
                return Ok(());
            }
        }

        let user_message = Message::new(MessageRole::User, content.to_string());
        conversation.push(user_message.clone());
        self.store
            .append_message(conversation.session_id, &user_message)?;

        if conversation.messages.len() == 1 || conversation.title == "Untitled session" {
            conversation.update_title_from_prompt(content);
            self.store
                .update_session_title(conversation.session_id, &conversation.title)?;
        }

        if let Err(error) = self
            .run_agent_with_tools(&message, &mut conversation, &active_model)
            .await
        {
            let error_text = format!("Gateway error: {error}");
            let error_message = Message::new(MessageRole::Error, error_text.clone());
            self.store
                .append_message(conversation.session_id, &error_message)?;
            self.send_reply_chunks(&message, &error_text).await?;
        }

        Ok(())
    }

    async fn run_agent_with_tools(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
    ) -> Result<()> {
        crate::log_info!(
            "Starting agent: chat_id={}, model={}, session={}",
            source_message.chat.id,
            active_model.label(),
            conversation.session_id
        );

        // Build recipient string: chat_id:thread_id (if thread exists)
        let recipient = if let Some(thread_id) = source_message.message_thread_id {
            format!("{}:{}", source_message.chat.id, thread_id)
        } else {
            source_message.chat.id.to_string()
        };

        // Send initial draft message
        let draft_message_id = match self
            .send_draft(&SendMessage::new("Thinking...", &recipient))
            .await?
        {
            Some(id) => id,
            None => {
                self.send_reply_chunks(source_message, "Thinking...")
                    .await?;
                return Ok(());
            }
        };

        let mut has_tool_calls = false;

        loop {
            if self.check_cancellation(source_message.chat.id) {
                self.send_reply_chunks(source_message, "Stopped.").await?;
                return Ok(());
            }

            let turn = self
                .run_single_streaming_turn(
                    source_message,
                    conversation,
                    active_model,
                    &draft_message_id,
                )
                .await?;

            if turn.tool_calls.is_empty() {
                let final_text = normalize_assistant_output(&turn.content);
                crate::log_info!(
                    "Agent completed: session={}, content_len={}",
                    conversation.session_id,
                    final_text.len()
                );

                // Persist assistant turn
                let assistant_message = Message::new(MessageRole::Assistant, final_text.clone());
                conversation.push(assistant_message.clone());
                self.store
                    .append_message(conversation.session_id, &assistant_message)?;

                // If we had tool calls, send final response as a new message
                // instead of editing the draft (so it appears at the end)
                if has_tool_calls {
                    // Delete the draft message first
                    let _ = self.cancel_draft(&recipient, &draft_message_id).await;
                    // Send final response as new message
                    self.send_reply_chunks(source_message, &final_text).await?;
                } else {
                    // No tool calls - use finalize_draft to update the initial message
                    if self
                        .finalize_draft(&recipient, &draft_message_id, &final_text)
                        .await
                        .is_err()
                    {
                        // Fallback: send as new message
                        self.send_reply_chunks(source_message, &final_text).await?;
                    }
                }
                return Ok(());
            }

            // Mark that we've had tool calls
            has_tool_calls = true;

            // Show tool call progress - use update_draft_progress
            let status = format!("🔧 Running {} tool call(s)...", turn.tool_calls.len());
            if self
                .update_draft_progress(&recipient, &draft_message_id, &status)
                .await
                .is_err()
            {
                // Fallback: send as new message
                self.send_reply_chunks(source_message, &status).await?;
            }

            self.execute_tool_calls(source_message, conversation, turn.tool_calls)
                .await?;
        }
    }

    async fn run_single_streaming_turn(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
        draft_message_id: &str,
    ) -> Result<AssistantTurn> {
        self.tools.set_active_model(active_model.clone());

        let context_manager = ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );

        let request_messages =
            context_manager.build_request_messages(conversation, SessionMode::Build);
        let tool_definitions = self.tools.all_definitions();

        let mut request_model = active_model.clone();
        request_model.system_prompt =
            compose_system_prompt(&active_model.system_prompt, &self.instruction_prompt);

        self.request_seq = self.request_seq.wrapping_add(1);
        if self.request_seq == 0 {
            self.request_seq = 1;
        }

        let request_id = self.request_seq;
        let session_id = conversation.session_id;

        let (tx, mut rx) = unbounded_channel();
        let llm = self.llm.clone();

        tokio::spawn(async move {
            let thinking_level = request_model.thinking_level.clone();
            llm.stream_chat(
                session_id,
                request_id,
                request_model,
                request_messages,
                tool_definitions,
                tx,
                thinking_level,
            )
            .await;
        });

        let mut turn = AssistantTurn::default();
        let mut streamed_reasoning = String::new();
        let mut last_edit = Instant::now() - Duration::from_millis(TELEGRAM_DRAFT_EDIT_INTERVAL_MS);

        // Build recipient for draft updates
        let recipient = if let Some(thread_id) = source_message.message_thread_id {
            format!("{}:{}", source_message.chat.id, thread_id)
        } else {
            source_message.chat.id.to_string()
        };

        while let Some(event) = rx.recv().await {
            match event {
                BackendEvent::Delta {
                    session_id: event_session_id,
                    request_id: event_req_id,
                    content,
                    ..
                } if event_session_id == session_id && event_req_id == request_id => {
                    if content.is_empty() {
                        continue;
                    }

                    turn.content.push_str(&content);
                    let preview = preview_for_streaming(&turn.content);

                    if streamed_reasoning.len() < turn.content.len() {
                        let new_reasoning = &turn.content[streamed_reasoning.len()..];
                        streamed_reasoning.push_str(new_reasoning);
                    }

                    let now = Instant::now();
                    if now.duration_since(last_edit).as_millis() as u64
                        >= TELEGRAM_DRAFT_EDIT_INTERVAL_MS
                        || turn.content.len() >= TELEGRAM_MAX_MESSAGE_LENGTH
                    {
                        if self
                            .update_draft(&recipient, draft_message_id, &preview)
                            .await
                            .is_err()
                        {
                            // Fallback: send as new message
                            self.send_reply_chunks(source_message, &preview).await?;
                        }
                        last_edit = now;
                    }
                }
                BackendEvent::ReasoningDelta {
                    session_id: event_session_id,
                    request_id: event_req_id,
                    content,
                    ..
                } if event_session_id == session_id && event_req_id == request_id => {
                    turn.reasoning.push_str(&content);
                }
                BackendEvent::ToolCallUpdated {
                    session_id: event_session_id,
                    request_id: event_req_id,
                    tool_call,
                    ..
                } if event_session_id == session_id && event_req_id == request_id => {
                    if let Some(existing) =
                        turn.tool_calls.iter_mut().find(|tc| tc.id == tool_call.id)
                    {
                        *existing = tool_call;
                    } else {
                        turn.tool_calls.push(tool_call);
                    }
                }
                BackendEvent::Failed {
                    session_id: event_session_id,
                    request_id: event_req_id,
                    error,
                    ..
                } if event_session_id == session_id && event_req_id == request_id => {
                    bail!("LLM Error: {}", error);
                }
                BackendEvent::Finished {
                    session_id: event_session_id,
                    request_id: event_req_id,
                    turn: finished_turn,
                    ..
                } if event_session_id == session_id && event_req_id == request_id => {
                    turn = finished_turn;
                    break;
                }
                _ => {}
            }
        }

        Ok(turn)
    }

    async fn execute_tool_calls(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        tool_calls: Vec<ToolCall>,
    ) -> Result<()> {
        let runtime = tokio::runtime::Handle::current();
        for tool_call in tool_calls {
            crate::log_info!("Executing tool: {}", tool_call.name);
            let result =
                self.tools
                    .execute_call(&runtime, &self.store, conversation.session_id, &tool_call, SessionMode::Build, false);

            let execution_result = match result {
                Ok(res) => res,
                Err(error) => crate::session::ToolExecutionResult::new(format!("Error: {error}")),
            };

            let display_result =
                execution_result.preview_for_storage(Some(tool_call.name.as_str()));
            let output_for_tool_event = display_result.output.clone();

            let tool_message =
                Message::tool_result(&tool_call.id, &tool_call.name, execution_result);
            self.store.append_tool_event(
                conversation.session_id,
                tool_message.id,
                &tool_call.name,
                &tool_call.arguments,
                &output_for_tool_event,
            )?;

            conversation.push(tool_message.clone());
            self.store
                .append_message(conversation.session_id, &tool_message)?;

            // Send tool result to user
            let tool_result_text = format!(
                "🔧 *{}*\n```\n{}\n```",
                tool_call.name,
                truncate_for_markdown(&output_for_tool_event)
            );
            self.send_reply_chunks(source_message, &tool_result_text)
                .await?;

            crate::log_debug!(
                "Tool result recorded: name={}, result_len={}",
                tool_call.name,
                output_for_tool_event.len()
            );
        }

        Ok(())
    }

    async fn handle_command(
        &mut self,
        source_message: &TelegramMessage,
        chat_key: &str,
        conversation: &mut Conversation,
        active_model: &mut ActiveModel,
        command: CommandInvocation,
    ) -> Result<bool> {
        match command.name.as_str() {
            "new" => {
                *conversation = self.rotate_chat_session(chat_key, active_model)?;
                self.send_reply_chunks(source_message, "Started a fresh session.")
                    .await?;
                Ok(true)
            }
            "session" => {
                if let Some(new_model) = self
                    .handle_session_command(
                        source_message,
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
                self.handle_model_command(source_message).await?;
                Ok(true)
            }
            "help" => {
                self.send_reply_chunks(source_message, &gateway_help_text())
                    .await?;
                Ok(true)
            }
            "status" => {
                self.handle_status_command(source_message, conversation, active_model)
                    .await?;
                Ok(true)
            }
            "balance" => {
                self.handle_balance_command(source_message).await?;
                Ok(true)
            }
            "stop" => {
                self.handle_stop_command(source_message, chat_key).await?;
                Ok(true)
            }
            "compact" => {
                self.handle_compact_command(source_message, chat_key, conversation, active_model)
                    .await?;
                Ok(true)
            }
            "init" => {
                self.handle_init_command(source_message).await?;
                Ok(true)
            }
            _ => {
                self.send_reply_chunks(
                    source_message,
                    &format!(
                        "Unknown command: {}\n\n{}",
                        command.name,
                        gateway_help_text()
                    ),
                )
                .await?;
                Ok(true)
            }
        }
    }

    async fn handle_session_command(
        &self,
        source_message: &TelegramMessage,
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
                self.send_reply_chunks(source_message, &text).await?;
                Ok(updated_model)
            }
            Some("new") => {
                let new_conversation = self.rotate_chat_session("session:new", active_model)?;
                self.send_reply_chunks(
                    source_message,
                    &format!(
                        "Session rotated. New session_id: {}",
                        new_conversation.session_id
                    ),
                )
                .await?;
                Ok(updated_model)
            }
            Some("clear") => {
                let new_conversation = self.rotate_chat_session("session:clear", active_model)?;
                self.send_reply_chunks(
                    source_message,
                    &format!(
                        "Session cleared. New session_id: {}",
                        new_conversation.session_id
                    ),
                )
                .await?;
                Ok(updated_model)
            }
            Some("title") => {
                if args.len() < 2 {
                    self.send_reply_chunks(source_message, "Usage: /session title <new_title>")
                        .await?;
                    return Ok(updated_model);
                }
                let new_title = args[1..].join(" ");
                self.store
                    .update_session_title(conversation.session_id, &new_title)?;
                self.send_reply_chunks(
                    source_message,
                    &format!("Session title updated: {}", new_title),
                )
                .await?;
                Ok(updated_model)
            }
            _ => {
                self.send_reply_chunks(
                    source_message,
                    "Usage: /session (show | new | clear | title <new_title>)",
                )
                .await?;
                Ok(updated_model)
            }
        }
    }

    /// Get providers that support balance queries and have API keys configured.
    fn get_balance_providers(&self) -> Vec<(&str, &str)> {
        let mut providers = Vec::new();

        // DeepSeek
        if self.auth.api_key("deepseek").is_some() {
            providers.push(("deepseek", "DeepSeek"));
        }

        // SiliconFlow
        if self.auth.api_key("siliconflow-cn").is_some() {
            providers.push(("siliconflow-cn", "SiliconFlow"));
        }

        providers
    }

    /// Format DeepSeek balance for display.
    fn format_deepseek_balance(&self, balance: &crate::balance::DeepSeekBalanceResponse) -> String {
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
    fn format_siliconflow_balance(
        &self,
        balance: &crate::balance::SiliconFlowBalanceResponse,
    ) -> String {
        format!(
            "💰 SiliconFlow Balance\n\nTotal: {} CNY",
            balance.data.total_balance
        )
    }

    async fn handle_balance_command(&mut self, message: &TelegramMessage) -> Result<()> {
        let providers = self.get_balance_providers();

        if providers.is_empty() {
            self.send_reply_chunks(
                message,
                "No providers available for balance queries.\nConfigure API keys for DeepSeek or SiliconFlow.",
            )
            .await?;
            return Ok(());
        }

        // Format provider list
        let mut text = String::from("Select a provider to query balance (enter number):\n\n");
        for (i, (_, name)) in providers.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, name));
        }
        text.push_str("\n(Enter any other number to cancel)");

        self.send_reply_chunks(message, &text).await?;

        // Set state to waiting for provider selection
        self.balance_selection_states
            .insert(message.chat.id, BalanceSelectionState::WaitingForProvider);

        Ok(())
    }

    async fn handle_balance_selection(
        &mut self,
        message: &TelegramMessage,
        state: &BalanceSelectionState,
    ) -> Result<()> {
        let content = message.text.as_deref().unwrap_or_default().trim();

        match state {
            BalanceSelectionState::WaitingForProvider => {
                let providers = self.get_balance_providers();
                let selection: usize = match content.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        self.balance_selection_states.remove(&message.chat.id);
                        self.send_reply_chunks(
                            message,
                            "Invalid selection. Selection cancelled. Send /balance to try again.",
                        )
                        .await?;
                        return Ok(());
                    }
                };

                if selection < 1 || selection > providers.len() {
                    self.balance_selection_states.remove(&message.chat.id);
                    self.send_reply_chunks(
                        message,
                        "Selection cancelled. Send /balance to try again.",
                    )
                    .await?;
                    return Ok(());
                }

                let (provider_id, _provider_name) = providers[selection - 1];

                // Query balance
                let result = self.query_balance_for_provider(provider_id).await;

                self.balance_selection_states.remove(&message.chat.id);

                let text = match result {
                    Ok(info) => info,
                    Err(e) => format!("Failed to query balance: {}", e),
                };

                self.send_reply_chunks(message, &text).await?;
                Ok(())
            }
        }
    }

    async fn query_balance_for_provider(&self, provider_id: &str) -> Result<String> {
        let http = self.llm.http();
        let api_key = self
            .auth
            .api_key(provider_id)
            .context("API key not found")?;

        match provider_id {
            "deepseek" => {
                let balance = crate::balance::query_deepseek_balance(http, api_key).await?;
                Ok(self.format_deepseek_balance(&balance))
            }
            "siliconflow-cn" => {
                let balance = crate::balance::query_siliconflow_balance(http, api_key).await?;
                Ok(self.format_siliconflow_balance(&balance))
            }
            _ => anyhow::bail!("Unsupported provider: {}", provider_id),
        }
    }

    async fn handle_model_command(&mut self, message: &TelegramMessage) -> Result<()> {
        let providers = self.get_available_providers();

        if providers.is_empty() {
            self.send_reply_chunks(
                message,
                "No providers available. Configure API keys in auth.json.",
            )
            .await?;
            return Ok(());
        }

        // Format provider list
        let mut text = String::from("Select a provider (enter number):\n\n");
        for (i, (id, name)) in providers.iter().enumerate() {
            text.push_str(&format!("{}. {} ({})\n", i + 1, name, id));
        }
        text.push_str("\n(Enter any other number to cancel)");

        self.send_reply_chunks(message, &text).await?;

        // Set state to waiting for provider selection
        self.model_selection_states
            .insert(message.chat.id, ModelSelectionState::WaitingForProvider);

        Ok(())
    }

    async fn handle_model_selection(
        &mut self,
        message: &TelegramMessage,
        state: &ModelSelectionState,
    ) -> Result<()> {
        let content = message.text.as_deref().unwrap_or_default().trim();

        match state {
            ModelSelectionState::WaitingForProvider => {
                let providers = self.get_available_providers();
                let selection: usize = match content.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        self.model_selection_states.remove(&message.chat.id);
                        self.send_reply_chunks(
                            message,
                            "Invalid selection. Selection cancelled. Send /model to try again.",
                        )
                        .await?;
                        return Ok(());
                    }
                };

                if selection < 1 || selection > providers.len() {
                    self.model_selection_states.remove(&message.chat.id);
                    self.send_reply_chunks(
                        message,
                        "Selection cancelled. Send /model to try again.",
                    )
                    .await?;
                    return Ok(());
                }

                let (provider_id, _provider_name) = &providers[selection - 1];

                // Format model list
                let mut text = format!("Select a model for {} (enter number):\n\n", provider_id);
                for (i, model) in self.get_models_for_provider(provider_id).iter().enumerate() {
                    text.push_str(&format!("{}. {}\n", i + 1, model.1));
                }
                text.push_str("\n(Enter any other number to cancel)");

                self.send_reply_chunks(message, &text).await?;

                // Set state to waiting for model selection
                self.model_selection_states.insert(
                    message.chat.id,
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
                        self.model_selection_states.remove(&message.chat.id);
                        self.send_reply_chunks(
                            message,
                            "Invalid selection. Selection cancelled. Send /model to try again.",
                        )
                        .await?;
                        return Ok(());
                    }
                };

                let models = self.get_models_for_provider(provider_id);
                if selection < 1 || selection > models.len() {
                    self.model_selection_states.remove(&message.chat.id);
                    self.send_reply_chunks(
                        message,
                        "Selection cancelled. Send /model to try again.",
                    )
                    .await?;
                    return Ok(());
                }

                let (_model_id, _model_name) = &models[selection - 1];

                // Save the model selection
                let chat_key = format!("telegram:{}", message.chat.id);
                self.store.set_gateway_chat_model(
                    GATEWAY_PLATFORM_TELEGRAM,
                    &chat_key,
                    provider_id,
                    _model_id,
                )?;

                // Clear state
                self.model_selection_states.remove(&message.chat.id);

                // Send success message
                let success_text = format!(
                    "Model switched to {}/{}\n\nSend /model to change again.",
                    provider_id, _model_id
                );
                self.send_reply_chunks(message, &success_text).await?;
            }
        }

        Ok(())
    }

    /// Get available providers (user config + bundled) that have valid auth.
    fn get_available_providers(&self) -> Vec<(String, String)> {
        let mut providers = Vec::new();

        // Check user-configured providers
        for (id, config) in &self.config.providers {
            if let Some(auth) = self.auth.providers.get(id)
                && auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty())
            {
                providers.push((id.clone(), config.display_name.clone()));
            }
        }

        // Check bundled providers
        for (id, config) in &self.config.bundled_providers {
            // Skip if already added from user config
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
    fn get_models_for_provider(&self, provider_id: &str) -> Vec<(String, String)> {
        let mut models = Vec::new();

        // Check user-configured providers first
        if let Some(config) = self.config.providers.get(provider_id) {
            for (id, model_config) in &config.models {
                models.push((id.clone(), model_config.display_name.clone()));
            }
        }

        // Check bundled providers if not found
        if models.is_empty()
            && let Some(config) = self.config.bundled_providers.get(provider_id)
        {
            for (id, model_config) in &config.models {
                models.push((id.clone(), model_config.display_name.clone()));
            }
        }

        models
    }

    fn load_or_create_chat_conversation(
        &self,
        chat_key: &str,
        active_model: &ActiveModel,
    ) -> Result<Conversation> {
        if let Some(session_id) = self
            .store
            .load_gateway_chat_session(GATEWAY_PLATFORM_TELEGRAM, chat_key)?
        {
            if let Some(conversation) = self.store.load_conversation(session_id)? {
                return Ok(conversation);
            }

            self.store
                .clear_gateway_chat_session(GATEWAY_PLATFORM_TELEGRAM, chat_key)?;
        }

        let conversation = self.create_gateway_session(active_model)?;
        self.store.set_gateway_chat_session(
            GATEWAY_PLATFORM_TELEGRAM,
            chat_key,
            conversation.session_id,
        )?;

        Ok(conversation)
    }

    fn rotate_chat_session(
        &self,
        chat_key: &str,
        active_model: &ActiveModel,
    ) -> Result<Conversation> {
        let conversation = self.create_gateway_session(active_model)?;
        self.store.set_gateway_chat_session(
            GATEWAY_PLATFORM_TELEGRAM,
            chat_key,
            conversation.session_id,
        )?;
        Ok(conversation)
    }

    fn resolve_chat_model(&self, chat_key: &str) -> Result<ActiveModel> {
        if let Some((provider_id, model_id)) = self
            .store
            .load_gateway_chat_model(GATEWAY_PLATFORM_TELEGRAM, chat_key)?
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
                        .clear_gateway_chat_model(GATEWAY_PLATFORM_TELEGRAM, chat_key)?;
                }
            }
        }

        self.config.resolve_active_model_for_gateway(&self.auth)
    }

    fn create_gateway_session(&self, active_model: &ActiveModel) -> Result<Conversation> {
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
            self.workspace_root.as_path(),
            &active_model.provider_id,
            &active_model.provider_display_name,
            &active_model.model_id,
            &active_model.display_name,
            &conversation.title,
        )?;

        Ok(conversation)
    }

    fn is_allowed(&self, message: &TelegramMessage) -> bool {
        let chat_id = message.chat.id.to_string();
        if self.allowlist.contains(&chat_id) {
            return true;
        }

        message
            .from
            .as_ref()
            .map(|user| {
                let user_id = user.id.to_string();
                if self.allowlist.contains(&user_id) {
                    return true;
                }
                if let Some(ref username) = user.username
                    && self.allowlist.contains(username)
                {
                    return true;
                }
                false
            })
            .unwrap_or(false)
    }

    fn chat_key(&self, message: &TelegramMessage) -> String {
        match message.message_thread_id {
            Some(thread_id) => format!("{}:{thread_id}", message.chat.id),
            None => message.chat.id.to_string(),
        }
    }

    async fn send_reply_chunks(&self, message: &TelegramMessage, text: &str) -> Result<()> {
        let chunks = split_message_for_telegram(text);
        crate::log_info!(
            "Sending reply: chat_id={}, chunks={}, total_len={}",
            message.chat.id,
            chunks.len(),
            text.len()
        );

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

    /// Handle /status command - show session statistics.
    async fn handle_status_command(
        &self,
        source_message: &TelegramMessage,
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

        self.send_reply_chunks(source_message, &text).await
    }

    /// Handle /stop command - set cancellation flag for current task.
    async fn handle_stop_command(
        &mut self,
        source_message: &TelegramMessage,
        _chat_key: &str,
    ) -> Result<()> {
        // Get or create cancellation flag for this chat
        let flag = self
            .cancellation_flags
            .entry(source_message.chat.id)
            .or_insert_with(|| Arc::new(AtomicBool::new(false)));

        // Check if there's already a task running
        if flag.load(Ordering::SeqCst) {
            // Already stopping
            self.send_reply_chunks(source_message, "Already stopping...")
                .await?;
        } else {
            // Set the cancellation flag
            flag.store(true, Ordering::SeqCst);
            // We'll send confirmation after the task actually stops
            // The actual stopping is handled in run_agent_with_tools_inner
        }
        Ok(())
    }

    /// Check and clear cancellation flag, return true if cancelled.
    fn check_cancellation(&self, chat_id: i64) -> bool {
        if let Some(flag) = self.cancellation_flags.get(&chat_id)
            && flag.load(Ordering::SeqCst)
        {
            // Clear the flag
            flag.store(false, Ordering::SeqCst);
            return true;
        }
        false
    }

    /// Finalize draft message with final content.
    async fn finalize_draft(
        &mut self,
        _recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await?;
        Ok(())
    }

    /// Cancel draft by deleting the message.
    async fn cancel_draft(&mut self, _recipient: &str, message_id: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.delete_message(0, msg_id).await?;
        Ok(())
    }
}

// ===========================================================================
// Helper functions
// ===========================================================================

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

fn normalize_assistant_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed.to_string()
    }
}

#[allow(dead_code)]
fn trim_for_telegram(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(240) {
        out.push(ch);
    }
    if value.chars().count() > 240 {
        out.push_str("...");
    }
    out
}

fn truncate_for_markdown(value: &str) -> String {
    const MAX_CHARS: usize = 500;
    let mut out = String::new();
    for ch in value.chars().take(MAX_CHARS) {
        // Escape backticks to avoid breaking markdown code blocks
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

fn preview_for_streaming(text: &str) -> String {
    let normalized = normalize_assistant_output(text);
    if normalized.chars().count() <= TELEGRAM_MAX_MESSAGE_LENGTH {
        return normalized;
    }

    let mut preview = String::new();
    for ch in normalized
        .chars()
        .take(TELEGRAM_MAX_MESSAGE_LENGTH.saturating_sub(3))
    {
        preview.push(ch);
    }
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
        let mut chunk_end = search_area
            .rfind('\n')
            .or_else(|| search_area.rfind(' '))
            .unwrap_or(split_at);

        if chunk_end == 0 {
            chunk_end = split_at;
        }

        let chunk = remaining[..chunk_end].trim();
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }

        remaining = remaining[chunk_end..].trim_start();
    }

    if chunks.is_empty() {
        vec!["(no content)".to_string()]
    } else {
        chunks
    }
}

// ===========================================================================
// Channel trait implementation
// ===========================================================================

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_TELEGRAM
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            crate::log_info!("Telegram channel ready");
            self.bootstrap_offset().await?;
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let sessions = store.list_gateway_chat_sessions(GATEWAY_PLATFORM_TELEGRAM)?;
        let mut count = 0;
        let mut orphans_closed = 0;

        for (_chat_key, session_id) in sessions {
            if let Some(_conversation) = store.load_conversation(session_id)? {
                let messages = store.load_messages(session_id)?;

                // Check for orphaned user turn (crash mid-query)
                if let Some(last) = messages.last()
                    && last.role == MessageRole::User
                {
                    crate::log_info!("Found orphaned user turn in session {}", session_id);
                    orphans_closed += 1;
                }

                count += 1;
            }
        }

        crate::log_info!(
            "Telegram channel restored {} session(s), closed {} orphaned session(s)",
            count,
            orphans_closed
        );

        Ok(count)
    }

    fn supports_draft_updates(&self) -> bool {
        true
    }

    async fn send_draft(&mut self, message: &SendMessage) -> Result<Option<String>> {
        let chat_id = message
            .recipient
            .parse::<i64>()
            .context("invalid chat_id")?;
        let thread_id = message
            .thread_ts
            .as_ref()
            .and_then(|s| s.parse::<i64>().ok());
        let sent = self
            .bot
            .send_message(chat_id, thread_id, &message.content, None)
            .await?;

        Ok(Some(sent.message_id.to_string()))
    }

    async fn update_draft(&mut self, _recipient: &str, message_id: &str, text: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await?;
        Ok(())
    }

    async fn update_draft_progress(
        &mut self,
        _recipient: &str,
        message_id: &str,
        status: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, status).await?;
        Ok(())
    }

    async fn finalize_draft(
        &mut self,
        _recipient: &str,
        message_id: &str,
        text: &str,
    ) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.edit_message_text_html(0, msg_id, text).await?;
        Ok(())
    }

    async fn cancel_draft(&mut self, _recipient: &str, message_id: &str) -> Result<()> {
        let msg_id: i64 = message_id.parse().context("invalid message_id")?;
        self.bot.delete_message(0, msg_id).await?;
        Ok(())
    }
}

impl TelegramChannel {
    /// Handle /compact command - compact session context.
    async fn handle_compact_command(
        &mut self,
        source_message: &TelegramMessage,
        _chat_key: &str,
        conversation: &Conversation,
        active_model: &ActiveModel,
    ) -> Result<()> {
        use crate::context::ContextManager;

        let session_id = conversation.session_id;

        // Check if already compacting
        if self.compacting_sessions.contains(&session_id) {
            self.send_reply_chunks(source_message, "Already compacting session. Please wait...")
                .await?;
            return Ok(());
        }

        self.compacting_sessions.insert(session_id);

        self.send_reply_chunks(
            source_message,
            "Compacting session context... This may take a moment.",
        )
        .await?;

        // Clone required data for async operation
        let llm = self.llm.clone();
        let store = self.store.clone();
        let session_id_for_compact = session_id;
        let active_model_for_compact = active_model.clone();
        let conversation_for_compact = conversation.clone();

        // Spawn compaction task
        tokio::spawn(async move {
            let mut context_manager = ContextManager::new();

            let result = context_manager
                .compact(
                    &llm,
                    &active_model_for_compact,
                    &conversation_for_compact,
                    true,
                    None,
                )
                .await;

            match result {
                Ok(true) => {
                    let summary = context_manager.summary.clone();
                    let retained_from = context_manager.retained_from;

                    // Save compacted context state
                    if let Some(summary) = &summary {
                        let _ = store.update_session_context_state(
                            session_id_for_compact,
                            Some(summary),
                            retained_from,
                        );
                    }

                    // Send success message
                    let text = format!(
                        "✅ Session context compacted.\n\
                         Messages retained: {}\n\
                         Summary: {}",
                        retained_from,
                        summary.as_deref().unwrap_or("(none)")
                    );
                    let _ = store.append_message(
                        session_id_for_compact,
                        &crate::session::Message::new(crate::session::MessageRole::System, text),
                    );
                }
                Ok(false) => {
                    let text = "ℹ️ No compaction needed (context already compact)".to_string();
                    let _ = store.append_message(
                        session_id_for_compact,
                        &crate::session::Message::new(crate::session::MessageRole::System, text),
                    );
                }
                Err(e) => {
                    let text = format!("❌ Compaction failed: {}", e);
                    let _ = store.append_message(
                        session_id_for_compact,
                        &crate::session::Message::new(crate::session::MessageRole::System, text),
                    );
                }
            }
        });

        Ok(())
    }

    /// Handle /init command - load init prompt for project analysis.
    async fn handle_init_command(&self, source_message: &TelegramMessage) -> Result<()> {
        let init_prompt = crate::prompts::init_command();
        let text = format!(
            "📁 Project Analysis Prompt\n\n\
             Copy and send this prompt to analyze your project:\n\n\
             ```\n{}\n```",
            init_prompt
        );
        self.send_reply_chunks(source_message, &text).await?;
        Ok(())
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
