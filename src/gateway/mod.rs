use anyhow::{Context, Result, anyhow, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::unbounded_channel;
use tokio::task::block_in_place;
use tokio::time::{Duration, Instant, sleep};
use uuid::Uuid;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    instructions,
    llm::LlmClient,
    mcp::McpManager,
    prompts::SessionMode,
    session::{
        AssistantTurn, BackendEvent, Conversation, Message, MessageRole, ToolCall,
        ToolExecutionResult,
    },
    storage::SessionStore,
    tooling::{FileReadTracker, ToolRegistry},
};

const GATEWAY_PLATFORM_TELEGRAM: &str = "telegram";
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;
const TELEGRAM_DRAFT_EDIT_INTERVAL_MS: u64 = 1200;
const MAX_TOOL_ROUNDS: usize = 8;
const MAX_MODEL_LIST_LINES: usize = 48;

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    runtime.block_on(run_async())
}

async fn run_async() -> Result<()> {
    let workspace_root = env::current_dir().context("failed to determine workspace root")?;
    let paths = ConfigPaths::discover()?;
    let config = AppConfig::load_or_create(&paths)?;
    crate::logging::init(&paths.data_dir, config.logging.clone());
    let auth = AuthStore::load_or_create(&paths)?;

    if !config.gateway.telegram.enabled {
        bail!("gateway.telegram.enabled is false; set it to true in config.toml");
    }

    let allowlist = config
        .gateway
        .telegram
        .allowlist
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<HashSet<_>>();

    if allowlist.is_empty() {
        bail!("gateway.telegram.allowlist is empty; configure at least one Telegram user/chat id");
    }

    let bot_token = auth
        .telegram_bot_token()
        .context("missing Telegram bot token in auth.json for channel 'telegram'")?
        .to_string();

    let default_model = config.resolve_active_model(&auth)?;
    let instruction_prompt = compose_instruction_prompt(&workspace_root, &paths, &config);
    let llm = LlmClient::new()?;
    let store = SessionStore::open(paths.default_database_path())?;

    let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
    let file_read_tracker = Arc::new(FileReadTracker::new());
    let mut tools = ToolRegistry::new(
        workspace_root.clone(),
        paths.config_dir.clone(),
        config.skills.clone(),
        mcp,
        config.permissions.clone(),
        file_read_tracker,
    );
    tools.set_active_model(default_model.clone());

    let poll_timeout_secs = config.gateway.telegram.poll_timeout_secs.max(1);

    let mut runner = TelegramGatewayRunner {
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
    };

    runner.bootstrap_offset().await?;
    runner.run_loop().await
}

fn compose_instruction_prompt(
    workspace_root: &Path,
    paths: &ConfigPaths,
    config: &AppConfig,
) -> String {
    let (instruction_prompt, _) = instructions::system_prompt_and_sources(
        workspace_root,
        &paths.config_dir,
        &config.instructions,
    )
    .unwrap_or_default();

    instruction_prompt
}

fn compose_system_prompt(base_system_prompt: &str, instruction_prompt: &str) -> String {
    let mut prompt = String::new();
    if !base_system_prompt.trim().is_empty() {
        prompt.push_str(base_system_prompt.trim());
    }

    if !instruction_prompt.trim().is_empty() {
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(instruction_prompt.trim());
    }

    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }

    prompt.push_str(SessionMode::Build.reminder());
    prompt
}

struct TelegramGatewayRunner {
    workspace_root: PathBuf,
    config: AppConfig,
    auth: AuthStore,
    store: SessionStore,
    llm: LlmClient,
    tools: ToolRegistry,
    instruction_prompt: String,
    allowlist: HashSet<String>,
    poll_timeout_secs: u64,
    bot: TelegramBot,
    offset: i64,
    request_seq: u64,
}

impl TelegramGatewayRunner {
    async fn bootstrap_offset(&mut self) -> Result<()> {
        let updates = self.bot.get_updates(0, 0).await?;
        if let Some(last) = updates.last() {
            self.offset = last.update_id.saturating_add(1);
        }
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
                    eprintln!("telegram getUpdates failed: {error}");
                    sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            for update in updates {
                self.offset = update.update_id.saturating_add(1);
                let Some(message) = update.message else {
                    continue;
                };

                if let Err(error) = self.handle_message(message).await {
                    eprintln!("telegram message handling failed: {error}");
                }
            }
        }
    }

    async fn handle_message(&mut self, message: TelegramMessage) -> Result<()> {
        if !self.is_allowed(&message) {
            return Ok(());
        }

        let Some(content) = message
            .text
            .as_deref()
            .or(message.caption.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };

        let chat_key = self.chat_key(&message);
        let mut active_model = self.resolve_chat_model(&chat_key)?;
        let mut conversation = self.load_or_create_chat_conversation(&chat_key, &active_model)?;

        if let Some(command) = parse_command(content) {
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
        for _ in 0..MAX_TOOL_ROUNDS {
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
                self.finalize_draft_response(source_message, draft_message_id, &final_text)
                    .await?;
                return Ok(());
            }

            let status = format!("Running {} tool call(s)...", turn.tool_calls.len());
            self.try_edit_draft_text(source_message.chat.id, draft_message_id, &status)
                .await;

            self.execute_tool_calls(conversation, turn.tool_calls)?;
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
                BackendEvent::Retrying {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    attempt,
                    max_attempts,
                    reason,
                    ..
                } if event_session_id == session_id && event_request_id == request_id => {
                    let status = format!(
                        "Network retry {attempt}/{max_attempts}: {}",
                        trim_for_telegram(&reason)
                    );
                    self.try_edit_draft_text(source_message.chat.id, draft_message_id, &status)
                        .await;
                }
                BackendEvent::Finished {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    turn,
                } if event_session_id == session_id && event_request_id == request_id => {
                    final_turn = Some(turn);
                    break;
                }
                BackendEvent::Failed {
                    session_id: event_session_id,
                    request_id: event_request_id,
                    error,
                } if event_session_id == session_id && event_request_id == request_id => {
                    return Err(anyhow!(error));
                }
                _ => {}
            }
        }

        let mut turn = final_turn.ok_or_else(|| anyhow!("LLM stream ended without completion"))?;

        if turn.content.trim().is_empty() && !streamed_content.trim().is_empty() {
            turn.content = streamed_content;
        }

        if turn.reasoning.trim().is_empty() && !streamed_reasoning.trim().is_empty() {
            turn.reasoning = streamed_reasoning;
        }

        turn.content = normalize_assistant_output(&turn.content);

        let mut assistant_message = Message::new(MessageRole::Assistant, turn.content.clone());
        assistant_message.reasoning = turn.reasoning.clone();
        assistant_message.tool_calls = turn.tool_calls.clone();
        conversation.push(assistant_message.clone());
        self.store
            .append_message(conversation.session_id, &assistant_message)?;

        Ok(turn)
    }

    fn execute_tool_calls(
        &mut self,
        conversation: &mut Conversation,
        tool_calls: Vec<ToolCall>,
    ) -> Result<()> {
        let runtime_handle = tokio::runtime::Handle::current();

        for tool_call in tool_calls {
            let result = block_in_place(|| {
                self.tools.execute_call(
                    &runtime_handle,
                    &self.store,
                    conversation.session_id,
                    &tool_call,
                )
            })
            .unwrap_or_else(|error| ToolExecutionResult::new(format!("Tool failed: {error}")));

            self.record_tool_result(conversation, tool_call, result)?;
        }

        Ok(())
    }

    fn record_tool_result(
        &self,
        conversation: &mut Conversation,
        tool_call: ToolCall,
        result: ToolExecutionResult,
    ) -> Result<()> {
        let display_result = result.preview_for_storage(Some(tool_call.name.as_str()));
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
            .edit_message_text(source_message.chat.id, draft_message_id, first_chunk)
            .await?;

        for chunk in chunks.iter().skip(1) {
            self.bot
                .send_message(
                    source_message.chat.id,
                    source_message.message_thread_id,
                    chunk,
                    None,
                )
                .await?;
        }

        Ok(())
    }

    async fn try_edit_draft_text(&self, chat_id: i64, message_id: i64, text: &str) {
        if let Err(error) = self.bot.edit_message_text(chat_id, message_id, text).await {
            eprintln!("telegram editMessageText failed: {error}");
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
                self.handle_session_command(
                    source_message,
                    chat_key,
                    conversation,
                    active_model,
                    command.args,
                )
                .await?;
                Ok(true)
            }
            "model" => {
                self.handle_model_command(source_message, chat_key, active_model, command.args)
                    .await?;
                Ok(true)
            }
            "help" | "start" => {
                self.send_reply_chunks(source_message, &gateway_help_text())
                    .await?;
                Ok(true)
            }
            _ => {
                let text = format!(
                    "Unknown command '/{}'.\n\n{}",
                    command.name,
                    gateway_help_text()
                );
                self.send_reply_chunks(source_message, &text).await?;
                Ok(true)
            }
        }
    }

    async fn handle_session_command(
        &mut self,
        source_message: &TelegramMessage,
        chat_key: &str,
        conversation: &mut Conversation,
        active_model: &mut ActiveModel,
        args: Vec<String>,
    ) -> Result<()> {
        let action = args
            .first()
            .map(|value| value.as_str())
            .unwrap_or("current");

        match action {
            "new" => {
                *conversation = self.rotate_chat_session(chat_key, active_model)?;
                self.send_reply_chunks(source_message, "Started a fresh session.")
                    .await?;
            }
            "current" | "show" | "status" => {
                let summary = format_session_summary(conversation, active_model);
                self.send_reply_chunks(source_message, &summary).await?;
            }
            "reset-model" => {
                self.store
                    .clear_gateway_chat_model(GATEWAY_PLATFORM_TELEGRAM, chat_key)?;
                *active_model = self.config.resolve_active_model(&self.auth)?;
                let text = format!(
                    "Model preference cleared for this chat.\nNow using default model: {}/{}",
                    active_model.provider_id, active_model.model_id
                );
                self.send_reply_chunks(source_message, &text).await?;
            }
            "help" => {
                self.send_reply_chunks(source_message, &session_help_text())
                    .await?;
            }
            _ => {
                let text = format!(
                    "Unknown /session action '{action}'.\n\n{}",
                    session_help_text()
                );
                self.send_reply_chunks(source_message, &text).await?;
            }
        }

        Ok(())
    }

    async fn handle_model_command(
        &mut self,
        source_message: &TelegramMessage,
        chat_key: &str,
        active_model: &mut ActiveModel,
        args: Vec<String>,
    ) -> Result<()> {
        if args.is_empty() {
            let text = format!(
                "Current model: {}/{}\n\nUsage:\n/model list\n/model <provider:model>\n/model reset",
                active_model.provider_id, active_model.model_id
            );
            self.send_reply_chunks(source_message, &text).await?;
            return Ok(());
        }

        match args[0].as_str() {
            "list" => {
                let list = self.format_model_list();
                self.send_reply_chunks(source_message, &list).await?;
            }
            "reset" | "default" => {
                self.store
                    .clear_gateway_chat_model(GATEWAY_PLATFORM_TELEGRAM, chat_key)?;
                *active_model = self.config.resolve_active_model(&self.auth)?;
                let text = format!(
                    "Model reset to default: {}/{}",
                    active_model.provider_id, active_model.model_id
                );
                self.send_reply_chunks(source_message, &text).await?;
            }
            _ => {
                let selector = args.join(" ");
                let selected = self
                    .config
                    .resolve_model(&self.auth, Some(selector.as_str()))
                    .with_context(|| format!("invalid model selector '{selector}'"))?;

                self.store.set_gateway_chat_model(
                    GATEWAY_PLATFORM_TELEGRAM,
                    chat_key,
                    &selected.provider_id,
                    &selected.model_id,
                )?;

                *active_model = selected;
                let text = format!(
                    "Switched model for this chat to {}/{}",
                    active_model.provider_id, active_model.model_id
                );
                self.send_reply_chunks(source_message, &text).await?;
            }
        }

        Ok(())
    }

    fn format_model_list(&self) -> String {
        let models = self.config.available_models();
        let total = models.len();
        let mut lines = Vec::new();

        for model in models.iter().take(MAX_MODEL_LIST_LINES) {
            lines.push(format!("- {}/{}", model.provider_id, model.model_id));
        }

        if total > MAX_MODEL_LIST_LINES {
            lines.push(format!(
                "... and {} more model(s)",
                total - MAX_MODEL_LIST_LINES
            ));
        }

        format!(
            "Available models ({total}):\n{}\n\nUse: /model <provider:model>",
            lines.join("\n")
        )
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
                Ok(model) => return Ok(model),
                Err(_) => {
                    self.store
                        .clear_gateway_chat_model(GATEWAY_PLATFORM_TELEGRAM, chat_key)?;
                }
            }
        }

        self.config.resolve_active_model(&self.auth)
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
                self.allowlist.contains(&user_id)
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
        for (index, chunk) in chunks.iter().enumerate() {
            self.bot
                .send_message(
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

fn gateway_help_text() -> String {
    [
        "Gateway command help",
        "/new - start a fresh session",
        "/session - show current session status",
        "/session new - start a fresh session",
        "/session reset-model - clear model override for this chat",
        "/model - show current model",
        "/model list - list available models",
        "/model <provider:model> - switch model for this chat",
        "/model reset - reset to default model",
    ]
    .join("\n")
}

fn session_help_text() -> String {
    [
        "Session command help",
        "/session",
        "/session current",
        "/session new",
        "/session reset-model",
    ]
    .join("\n")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandInvocation {
    name: String,
    args: Vec<String>,
}

fn parse_command(content: &str) -> Option<CommandInvocation> {
    let mut parts = content.split_whitespace();
    let first = parts.next()?;
    if !first.starts_with('/') {
        return None;
    }

    let raw_name = first.trim_start_matches('/');
    if raw_name.is_empty() {
        return None;
    }

    let name = raw_name
        .split('@')
        .next()
        .unwrap_or(raw_name)
        .trim()
        .to_ascii_lowercase();

    if name.is_empty() {
        return None;
    }

    Some(CommandInvocation {
        name,
        args: parts.map(str::to_string).collect(),
    })
}

fn normalize_assistant_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed.to_string()
    }
}

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

#[derive(Clone, Debug)]
struct TelegramBot {
    token: String,
    http: Client,
}

impl TelegramBot {
    fn new(token: String) -> Self {
        Self {
            token,
            http: Client::new(),
        }
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

    async fn edit_message_text(&self, chat_id: i64, message_id: i64, text: &str) -> Result<()> {
        let body = EditMessageTextRequest {
            chat_id,
            message_id,
            text,
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
    message_thread_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
}

#[derive(Debug, Serialize)]
struct EditMessageTextRequest<'a> {
    chat_id: i64,
    message_id: i64,
    text: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_command_with_bot_mention() {
        let cmd = parse_command("/model@my_bot deepseek:deepseek-chat").expect("command");
        assert_eq!(cmd.name, "model");
        assert_eq!(cmd.args, vec!["deepseek:deepseek-chat"]);
    }

    #[test]
    fn parses_session_command_without_args() {
        let cmd = parse_command("/session").expect("command");
        assert_eq!(cmd.name, "session");
        assert!(cmd.args.is_empty());
    }

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
