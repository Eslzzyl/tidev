use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::fmt::Write;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::unbounded_channel;
use std::time::Instant;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore},
    context::ContextManager,
    llm::LlmClient,
    prompts::{SessionMode, gateway_system_prompt},
    session::{
        AssistantTurn, BackendEvent, Conversation, Message, MessageRole, ToolCall,
        ToolExecutionResult,
    },
    storage::SessionStore,
    tooling::ToolRegistry,
};

use super::channel::Channel;
use super::commands::{
    CommandInvocation, GATEWAY_COMMANDS, format_status_summary, gateway_help_text, parse_command,
};
use super::shared::compose_system_prompt;

pub const GATEWAY_PLATFORM_TELEGRAM: &str = "telegram";
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_DRAFT_EDIT_INTERVAL_MS: u64 = 1200;
const MAX_TOOL_ROUNDS: usize = 8;

/// Interactive model selection state for a chat.
#[derive(Debug, Clone)]
enum ModelSelectionState {
    /// Waiting for user to select a provider (1, 2, 3, ...)
    WaitingForProvider,
    /// Waiting for user to select a model (1, 2, 3, ...) for the given provider.
    WaitingForModel { provider_id: String },
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
            crate::log_info!("Handling model selection input: chat_id={}", message.chat.id);
            return self
                .handle_model_selection(&message, &state)
                .await;
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

        let draft = self
            .bot
            .send_message(
                source_message.chat.id,
                source_message.message_thread_id,
                "Thinking...",
                Some(source_message.message_id),
            )
            .await?;
        let draft_message_id = draft.message_id;
        crate::log_debug!("Sent draft message: msg_id={}", draft_message_id);

        if let Err(error) = self
            .run_agent_with_tools_inner(
                source_message,
                conversation,
                active_model,
                draft_message_id,
            )
            .await
        {
            let error_text = format!("Gateway error: {error}");
            crate::log_error!(
                "Agent failed: chat_id={}, error={error}",
                source_message.chat.id
            );
            let _ = self
                .bot
                .edit_message_text(source_message.chat.id, draft_message_id, &error_text)
                .await;
            return Err(error);
        }

        Ok(())
    }

    async fn run_agent_with_tools_inner(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
        draft_message_id: i64,
    ) -> Result<()> {
        for round in 1..=MAX_TOOL_ROUNDS {
            // Check for cancellation at the start of each round
            if self.check_cancellation(source_message.chat.id) {
                crate::log_info!("Task cancelled by user: chat_id={}", source_message.chat.id);

                // Send cancellation confirmation
                self.send_reply_chunks(source_message, "🛑 Task stopped.")
                    .await?;
                return Ok(());
            }

            crate::log_debug!(
                "Tool round {}/{}: chat_id={}, session={}",
                round,
                MAX_TOOL_ROUNDS,
                source_message.chat.id,
                conversation.session_id
            );

            let turn = self
                .run_single_streaming_turn(
                    source_message,
                    conversation,
                    active_model,
                    draft_message_id,
                )
                .await?;

            if turn.tool_calls.is_empty() {
                let final_text = normalize_assistant_output(&turn.content);
                crate::log_info!(
                    "Agent completed: chat_id={}, response_len={}",
                    source_message.chat.id,
                    final_text.len()
                );

                let assistant_message =
                    Message::new(MessageRole::Assistant, final_text.clone());
                conversation.push(assistant_message.clone());
                self.store.append_message(conversation.session_id, &assistant_message)?;

                self.finalize_draft_response(source_message, draft_message_id, &final_text)
                    .await?;
                return Ok(());
            }

            let status = format!("Running {} tool call(s)...", turn.tool_calls.len());
            self.try_edit_draft_text(source_message.chat.id, draft_message_id, &status)
                .await;

            self.execute_tool_calls(source_message, conversation, turn.tool_calls).await?;
        }

        bail!(
            "assistant exceeded maximum tool rounds ({MAX_TOOL_ROUNDS}); aborting to prevent loop"
        )
    }

    async fn run_single_streaming_turn(
        &mut self,
        source_message: &TelegramMessage,
        conversation: &mut Conversation,
        active_model: &ActiveModel,
        draft_message_id: i64,
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
            llm.stream_chat(
                session_id,
                request_id,
                request_model,
                request_messages,
                tool_definitions,
                tx,
            )
            .await;
        });

        let mut streamed_content = String::new();
        let mut streamed_reasoning = String::new();
        let mut last_edit = Instant::now() - Duration::from_millis(TELEGRAM_DRAFT_EDIT_INTERVAL_MS);
        let mut final_turn: Option<AssistantTurn> = None;

        while let Some(event) = rx.recv().await {
            match event {
                BackendEvent::Delta {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    content,
                } if event_session_id == session_id && event_request_id == request_id => {
                    streamed_content.push_str(&content);

                    if last_edit.elapsed() >= Duration::from_millis(TELEGRAM_DRAFT_EDIT_INTERVAL_MS)
                    {
                        let preview = preview_for_streaming(&streamed_content);
                        self.try_edit_draft_text(
                            source_message.chat.id,
                            draft_message_id,
                            &preview,
                        )
                        .await;
                        last_edit = Instant::now();
                    }
                }
                BackendEvent::ReasoningDelta {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    content,
                } if event_session_id == session_id && event_request_id == request_id => {
                    streamed_reasoning.push_str(&content);
                }
                BackendEvent::ToolCallUpdated {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    tool_call,
                } if event_session_id == session_id && event_request_id == request_id => {
                    if final_turn.is_none() {
                        final_turn = Some(AssistantTurn {
                            content: streamed_content.clone(),
                            reasoning: streamed_reasoning.clone(),
                            tool_calls: vec![tool_call],
                            finish_reason: None,
                        });
                    } else if let Some(ref mut turn) = final_turn {
                        turn.tool_calls.push(tool_call);
                    }
                }
                BackendEvent::Finished {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    turn: finished_turn,
                } if event_session_id == session_id && event_request_id == request_id => {
                    if let Some(turn) = final_turn.take() {
                        return Ok(turn);
                    }
                    // If no turn was being built, return the finished turn
                    return Ok(finished_turn);
                }
                _ => {}
            }
        }

        bail!("streaming ended without TurnEnd event")
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
                    .execute_call(&runtime, &self.store, conversation.session_id, &tool_call);

            let execution_result = match result {
                Ok(res) => res,
                Err(error) => ToolExecutionResult::new(format!("Error: {error}")),
            };

            let display_result =
                execution_result.preview_for_storage(Some(tool_call.name.as_str()));
            let output_for_tool_event = display_result.output.clone();
            let tool_message =
                Message::tool_result(tool_call.id.clone(), tool_call.name.clone(), display_result);

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
            self.send_reply_chunks(source_message, &tool_result_text).await?;

            crate::log_debug!(
                "Tool result recorded: name={}, result_len={}",
                tool_call.name,
                output_for_tool_event.len()
            );
        }

        Ok(())
    }

  async fn finalize_draft_response(
        &self,
        source_message: &TelegramMessage,
        draft_message_id: i64,
        text: &str,
    ) -> Result<()> {
        let chunks = split_message_for_telegram(text);
        let Some(first_chunk) = chunks.first() else {
            return Ok(());
        };

        self.bot
            .edit_message_text_html(source_message.chat.id, draft_message_id, first_chunk)
            .await?;
        crate::log_debug!(
            "Sent final response: chat_id={}, msg_id={}",
            source_message.chat.id,
            draft_message_id
        );

        for chunk in chunks.iter().skip(1) {
            self.bot
                .send_message_html(
                    source_message.chat.id,
                    source_message.message_thread_id,
                    chunk,
                    None,
                )
                .await?;
        }

        crate::log_info!(
            "Reply sent: chunks={}",
            chunks.len()
        );

        Ok(())
    }

    async fn try_edit_draft_text(&self, chat_id: i64, message_id: i64, text: &str) {
        if let Err(error) = self.bot.edit_message_text_html(chat_id, message_id, text).await {
            crate::log_warn!("Edit message failed: msg_id={}, error={error}", message_id);
        }
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
            "stop" => {
                self.handle_stop_command(source_message, chat_key).await?;
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
            }
            _ => {
                self.send_reply_chunks(source_message, &gateway_help_text())
                    .await?;
            }
        }

        Ok(updated_model)
    }

    /// Handle /model command - start interactive provider/model selection.
    async fn handle_model_command(&mut self, source_message: &TelegramMessage) -> Result<()> {
        // Get available providers (user config + bundled, only those with valid auth)
        let providers = self.get_available_providers();

        if providers.is_empty() {
            self.send_reply_chunks(
                source_message,
                "No available providers found. Please check your configuration.",
            )
            .await?;
            return Ok(());
        }

        // Format provider list
        let mut text = String::from("Select a provider (enter number):\n\n");
        for (i, provider) in providers.iter().enumerate() {
            text.push_str(&format!("{}. {}\n", i + 1, provider.1));
        }
        text.push_str("\n(Enter any other number to cancel)");

        self.send_reply_chunks(source_message, &text).await?;

        // Set state to waiting for provider selection
        self.model_selection_states
            .insert(source_message.chat.id, ModelSelectionState::WaitingForProvider);

        Ok(())
    }

    /// Handle interactive model selection input.
    async fn handle_model_selection(
        &mut self,
        message: &TelegramMessage,
        state: &ModelSelectionState,
    ) -> Result<()> {
        let content = message
            .text
            .as_deref()
            .or(message.caption.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("");

        // Check if it's a command - cancel selection if so
        if content.starts_with('/') {
            self.model_selection_states.remove(&message.chat.id);
            self.send_reply_chunks(message, "Selection cancelled. Send /model to try again.")
                .await?;
            return Ok(());
        }

        match state {
            ModelSelectionState::WaitingForProvider => {
                // Parse provider selection
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

                let providers = self.get_available_providers();
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

                // Get models for selected provider
                let models = self.get_models_for_provider(provider_id);
                if models.is_empty() {
                    self.model_selection_states.remove(&message.chat.id);
                    self.send_reply_chunks(
                        message,
                        "No models available for this provider. Selection cancelled.",
                    )
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
                && auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty()) {
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
                && auth.api_key.as_ref().is_some_and(|k| !k.trim().is_empty()) {
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
            && let Some(config) = self.config.bundled_providers.get(provider_id) {
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
            self.send_reply_chunks(source_message, "Already stopping...").await?;
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
            && flag.load(Ordering::SeqCst) {
                // Clear the flag
                flag.store(false, Ordering::SeqCst);
                return true;
            }
        false
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

pub struct TelegramBot {
    token: String,
    http: Client,
}

impl TelegramBot {
    pub fn new(token: String) -> Self {
        Self {
            token,
            http: Client::new(),
        }
    }

    /// Convert Markdown text to Telegram HTML format.
    fn markdown_to_telegram_html(text: &str) -> String {

        let lines: Vec<&str> = text.split('\n').collect();
        let mut result_lines: Vec<String> = Vec::new();

        for line in &lines {
            let trimmed_line = line.trim_start();
            if trimmed_line.starts_with("```") {
                result_lines.push(trimmed_line.to_string());
                continue;
            }

            let mut line_out = String::new();

            // Handle headers: ## Title → <b>Title</b>
            let stripped = line.trim_start_matches('#');
            let header_level = line.len() - stripped.len();
            if header_level > 0 && line.starts_with('#') && stripped.starts_with(' ') {
                let title = Self::escape_html(stripped.trim());
                result_lines.push(format!("<b>{title}</b>"));
                continue;
            }

            // Inline formatting
            let mut i = 0;
            let bytes = line.as_bytes();
            let len = bytes.len();
            while i < len {
                // Bold: **text** or __text__
                if i + 1 < len
                    && bytes[i] == b'*'
                    && bytes[i + 1] == b'*'
                    && let Some(end) = line[i + 2..].find("**")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<b>{inner}</b>");
                    i += 4 + end;
                    continue;
                }
                if i + 1 < len
                    && bytes[i] == b'_'
                    && bytes[i + 1] == b'_'
                    && let Some(end) = line[i + 2..].find("__")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<b>{inner}</b>");
                    i += 4 + end;
                    continue;
                }
                // Italic: *text* or _text_ (single)
                if bytes[i] == b'*'
                    && (i == 0 || bytes[i - 1] != b'*')
                    && let Some(end) = line[i + 1..].find('*')
                    && end > 0
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<i>{inner}</i>");
                    i += 2 + end;
                    continue;
                }
                if bytes[i] == b'_'
                    && (i == 0 || bytes[i - 1] != b'_')
                    && let Some(end) = line[i + 1..].find('_')
                    && end > 0
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<i>{inner}</i>");
                    i += 2 + end;
                    continue;
                }
                // Inline code: `code`
                if bytes[i] == b'`'
                    && (i == 0 || bytes[i - 1] != b'`')
                    && let Some(end) = line[i + 1..].find('`')
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<code>{inner}</code>");
                    i += 2 + end;
                    continue;
                }
                // Markdown link: [text](url)
                if bytes[i] == b'['
                    && let Some(bracket_end) = line[i + 1..].find(']')
                {
                    let text_part = &line[i + 1..i + 1 + bracket_end];
                    let after_bracket = i + 1 + bracket_end + 1;
                    if after_bracket < len
                        && bytes[after_bracket] == b'('
                        && let Some(paren_end) = line[after_bracket + 1..].find(')')
                    {
                        let url = &line[after_bracket + 1..after_bracket + 1 + paren_end];
                        if url.starts_with("http://") || url.starts_with("https://") {
                            let text_html = Self::escape_html(text_part);
                            let url_html = Self::escape_html(url);
                            let _ = write!(line_out, "<a href=\"{url_html}\">{text_html}</a>");
                            i = after_bracket + 1 + paren_end + 1;
                            continue;
                        }
                    }
                }
                // Strikethrough: ~~text~~
                if i + 1 < len
                    && bytes[i] == b'~'
                    && bytes[i + 1] == b'~'
                    && let Some(end) = line[i + 2..].find("~~")
                {
                    let inner = Self::escape_html(&line[i + 2..i + 2 + end]);
                    let _ = write!(line_out, "<s>{inner}</s>");
                    i += 4 + end;
                    continue;
                }
                // Default: escape HTML entities
                let ch = line[i..].chars().next().unwrap();
                match ch {
                    '<' => line_out.push_str("&lt;"),
                    '>' => line_out.push_str("&gt;"),
                    '&' => line_out.push_str("&amp;"),
                    '"' => line_out.push_str("&quot;"),
                    '\'' => line_out.push_str("&#39;"),
                    _ => line_out.push(ch),
                }
                i += ch.len_utf8();
            }
            result_lines.push(line_out);
        }

        // Second pass: handle ``` code blocks across lines
        let joined = result_lines.join("\n");
        let mut final_out = String::with_capacity(joined.len());
        let mut in_code_block = false;
        let mut code_buf = String::new();

        for line in joined.split('\n') {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_code_block {
                    in_code_block = false;
                    let escaped = code_buf.trim_end_matches('\n');
                    let _ = writeln!(final_out, "<pre><code>{escaped}</code></pre>");
                    code_buf.clear();
                } else {
                    in_code_block = true;
                    code_buf.clear();
                }
            } else if in_code_block {
                code_buf.push_str(line);
                code_buf.push('\n');
            } else {
                final_out.push_str(line);
                final_out.push('\n');
            }
        }
        if in_code_block && !code_buf.is_empty() {
            let _ = writeln!(final_out, "<pre><code>{}</code></pre>", code_buf.trim_end());
        }

        final_out.trim_end_matches('\n').to_string()
    }

    /// Escape HTML special characters.
    fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }

    async fn get_updates(&self, offset: i64, timeout_secs: u64) -> Result<Vec<TelegramUpdate>> {
        let body = serde_json::json!({
            "offset": offset,
            "timeout": timeout_secs,
            "allowed_updates": ["message"],
        });

        let response = self
            .http
            .post(self.api_url("getUpdates"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram getUpdates")?;

        let payload: TelegramApiResponse<Vec<TelegramUpdate>> = response
            .json()
            .await
            .context("failed to parse Telegram getUpdates response")?;

        payload.into_result("getUpdates")
    }

    async fn send_message(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        text: &str,
        reply_to_message_id: Option<i64>,
    ) -> Result<TelegramSentMessage> {
        let body = SendMessageRequest {
            chat_id,
            text,
            parse_mode: None,
            message_thread_id,
            reply_to_message_id,
        };

        let response = self
            .http
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram sendMessage")?;

        let payload: TelegramApiResponse<TelegramSentMessage> = response
            .json()
            .await
            .context("failed to parse Telegram sendMessage response")?;

        payload.into_result("sendMessage")
    }

    /// Send message with HTML parse mode for Markdown rendering.
    async fn send_message_html(
        &self,
        chat_id: i64,
        message_thread_id: Option<i64>,
        text: &str,
        reply_to_message_id: Option<i64>,
    ) -> Result<TelegramSentMessage> {
        let html_text = Self::markdown_to_telegram_html(text);
        let body = SendMessageRequest {
            chat_id,
            text: &html_text,
            parse_mode: Some("HTML".to_string()),
            message_thread_id,
            reply_to_message_id,
        };

        let response = self
            .http
            .post(self.api_url("sendMessage"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram sendMessage (HTML)")?;

        let payload: TelegramApiResponse<TelegramSentMessage> = response
            .json()
            .await
            .context("failed to parse Telegram sendMessage response")?;

        payload.into_result("sendMessage")
    }

    async fn edit_message_text(&self, chat_id: i64, message_id: i64, text: &str) -> Result<()> {
        let body = EditMessageTextRequest {
            chat_id,
            message_id,
            text,
            parse_mode: None,
        };

        let response = self
            .http
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram editMessageText")?;

        let payload: TelegramApiResponse<serde_json::Value> = response
            .json()
            .await
            .context("failed to parse Telegram editMessageText response")?;

        if payload.ok {
            return Ok(());
        }

        if payload
            .description
            .as_deref()
            .is_some_and(|description| description.contains("message is not modified"))
        {
            return Ok(());
        }

        match payload.error_code {
            Some(code) => bail!(
                "telegram editMessageText failed ({code}): {}",
                payload
                    .description
                    .unwrap_or_else(|| "unknown telegram api error".to_string())
            ),
            None => bail!(
                "telegram editMessageText failed: {}",
                payload
                    .description
                    .unwrap_or_else(|| "unknown telegram api error".to_string())
            ),
        }
    }

    /// Edit message text with HTML parse mode for Markdown rendering.
    async fn edit_message_text_html(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<()> {
        let html_text = Self::markdown_to_telegram_html(text);
        let body = EditMessageTextRequest {
            chat_id,
            message_id,
            text: &html_text,
            parse_mode: Some("HTML".to_string()),
        };

        let response = self
            .http
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram editMessageText (HTML)")?;

        let payload: TelegramApiResponse<serde_json::Value> = response
            .json()
            .await
            .context("failed to parse Telegram editMessageText response")?;

        if payload.ok {
            return Ok(());
        }

        if payload
            .description
            .as_deref()
            .is_some_and(|description| description.contains("message is not modified"))
        {
            return Ok(());
        }

        match payload.error_code {
            Some(code) => bail!(
                "telegram editMessageText HTML failed ({code}): {}",
                payload
                    .description
                    .unwrap_or_else(|| "unknown telegram api error".to_string())
            ),
            None => bail!(
                "telegram editMessageText HTML failed: {}",
                payload
                    .description
                    .unwrap_or_else(|| "unknown telegram api error".to_string())
            ),
        }
    }

    pub async fn set_my_commands(&self, commands: Vec<(String, String)>) -> Result<()> {
        let body = SetMyCommandsRequest {
            commands: commands
                .into_iter()
                .map(|(command, description)| BotCommand {
                    command,
                    description,
                })
                .collect(),
        };

        let response = self
            .http
            .post(self.api_url("setMyCommands"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram setMyCommands")?;

        let payload: TelegramApiResponse<bool> = response
            .json()
            .await
            .context("failed to parse Telegram setMyCommands response")?;

        payload.into_result("setMyCommands")?;
        Ok(())
    }

    pub async fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<()> {
        let body = SetMessageReactionRequest {
            chat_id,
            message_id,
            reaction: vec![ReactionType::Emoji {
                emoji: emoji.to_string(),
            }],
            is_big: None,
        };

        let response = self
            .http
            .post(self.api_url("setMessageReaction"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram setMessageReaction")?;

        let payload: TelegramApiResponse<bool> = response
            .json()
            .await
            .context("failed to parse Telegram setMessageReaction response")?;

        payload.into_result("setMessageReaction")?;
        Ok(())
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
    error_code: Option<i64>,
}

impl<T> TelegramApiResponse<T> {
    fn into_result(self, method: &str) -> Result<T> {
        if self.ok {
            return self
                .result
                .with_context(|| format!("telegram {method} response missing result"));
        }

        let description = self
            .description
            .unwrap_or_else(|| "unknown telegram api error".to_string());
        match self.error_code {
            Some(code) => bail!("telegram {method} failed ({code}): {description}"),
            None => bail!("telegram {method} failed: {description}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
    #[serde(default)]
    from: Option<TelegramUser>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    caption: Option<String>,
    #[serde(default)]
    message_thread_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramSentMessage {
    message_id: i64,
}

#[derive(Debug, Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_thread_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
}

#[derive(Debug, Serialize)]
struct EditMessageTextRequest<'a> {
    chat_id: i64,
    message_id: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<String>,
}

#[derive(Debug, Serialize)]
struct SetMyCommandsRequest {
    commands: Vec<BotCommand>,
}

#[derive(Debug, Serialize)]
struct BotCommand {
    command: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct SetMessageReactionRequest {
    chat_id: i64,
    message_id: i64,
    reaction: Vec<ReactionType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_big: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ReactionType {
    #[serde(rename = "emoji")]
    Emoji { emoji: String },
}

impl Channel for TelegramChannel {
    fn name(&self) -> &'static str {
        GATEWAY_PLATFORM_TELEGRAM
    }

    fn store(&self) -> Option<&SessionStore> {
        Some(&self.store)
    }

    fn run(&mut self) -> Pin<Box<dyn Future<Output = Result<()>> + '_>> {
        Box::pin(async move {
            self.bootstrap_offset().await?;
            crate::log_info!("Telegram channel ready, offset={}", self.offset);
            self.run_loop().await
        })
    }

    fn restore_sessions(&mut self, store: SessionStore) -> Result<usize> {
        let sessions = store.list_gateway_chat_sessions(GATEWAY_PLATFORM_TELEGRAM)?;
        let mut count = 0;
        let mut orphans_closed = 0;

        for (chat_key, session_id) in sessions {
            if let Some(_conversation) = store.load_conversation(session_id)? {
                let messages = store.load_messages(session_id)?;

                // Check for orphaned user turn (crash mid-query)
                if let Some(last) = messages.last()
                    && last.role == MessageRole::User {
                        // Close orphan with marker to prevent LLM from continuing the old request
                        let marker = Message::new(
                            MessageRole::Assistant,
                            "[Session interrupted — not continuing this request]".to_string(),
                        );
                        store.append_message(session_id, &marker)?;
                        orphans_closed += 1;
                    }

                count += 1;
                crate::log_info!(
                    "Restored Telegram session: chat_key={}, session_id={}, messages={}",
                    chat_key,
                    session_id,
                    messages.len()
                );
            }
        }

        if count > 0 {
            crate::log_info!("Restored {} Telegram session(s) from disk", count);
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
