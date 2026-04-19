use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::env;
use std::path::{Path, PathBuf};
use tokio::runtime::Runtime;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

use crate::{
    config::{ActiveModel, AppConfig, AuthStore, ConfigPaths},
    context::ContextManager,
    instructions,
    llm::LlmClient,
    prompts::SessionMode,
    session::{Conversation, Message, MessageRole},
    storage::SessionStore,
};

const GATEWAY_PLATFORM_TELEGRAM: &str = "telegram";
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

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

    let bot_token = auth
        .telegram_bot_token()
        .context("missing Telegram bot token in auth.json for provider 'telegram'")?
        .to_string();

    let active_model = config.resolve_active_model(&auth)?;
    let system_prompt = compose_system_prompt(&workspace_root, &paths, &config, &active_model);
    let llm = LlmClient::new()?;
    let store = SessionStore::open(paths.default_database_path())?;
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
    let poll_timeout_secs = config.gateway.telegram.poll_timeout_secs.max(1);

    let mut runner = TelegramGatewayRunner {
        workspace_root,
        store,
        llm,
        active_model,
        system_prompt,
        allowlist,
        poll_timeout_secs,
        bot: TelegramBot::new(bot_token),
        offset: 0,
    };

    runner.bootstrap_offset().await?;
    runner.run_loop().await
}

fn compose_system_prompt(
    workspace_root: &Path,
    paths: &ConfigPaths,
    config: &AppConfig,
    model: &ActiveModel,
) -> String {
    let (instruction_prompt, _) = instructions::system_prompt_and_sources(
        workspace_root,
        &paths.config_dir,
        &config.instructions,
    )
    .unwrap_or_default();

    let mut prompt = String::new();
    if !model.system_prompt.trim().is_empty() {
        prompt.push_str(model.system_prompt.trim());
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
    store: SessionStore,
    llm: LlmClient,
    active_model: ActiveModel,
    system_prompt: String,
    allowlist: HashSet<String>,
    poll_timeout_secs: u64,
    bot: TelegramBot,
    offset: i64,
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

        if is_new_command(content) {
            self.rotate_chat_session(&chat_key)?;
            self.bot
                .send_message(
                    message.chat.id,
                    message.message_thread_id,
                    "Started a fresh session.",
                    Some(message.message_id),
                )
                .await?;
            return Ok(());
        }

        let mut conversation = self.load_or_create_chat_conversation(&chat_key)?;

        let user_message = Message::new(MessageRole::User, content.to_string());
        conversation.push(user_message.clone());
        self.store
            .append_message(conversation.session_id, &user_message)?;

        if conversation.messages.len() == 1 || conversation.title == "Untitled session" {
            conversation.update_title_from_prompt(content);
            self.store
                .update_session_title(conversation.session_id, &conversation.title)?;
        }

        let context_manager = ContextManager::from_state(
            conversation.context_summary.clone(),
            conversation.context_retained_from,
        );
        let mut model = self.active_model.clone();
        model.system_prompt = self.system_prompt.clone();
        let messages = context_manager.build_request_messages(&conversation, SessionMode::Build);

        let assistant_output = match self.llm.complete_with_messages(model, messages).await {
            Ok(output) => output,
            Err(error) => {
                let error_text = format!("Gateway error: {error}");
                let error_message = Message::new(MessageRole::Error, error_text.clone());
                self.store
                    .append_message(conversation.session_id, &error_message)?;
                self.send_reply_chunks(&message, &error_text).await?;
                return Ok(());
            }
        };

        let response_text = normalize_assistant_output(&assistant_output);
        let assistant_message = Message::new(MessageRole::Assistant, response_text.clone());
        self.store
            .append_message(conversation.session_id, &assistant_message)?;

        self.send_reply_chunks(&message, &response_text).await
    }

    fn load_or_create_chat_conversation(&self, chat_key: &str) -> Result<Conversation> {
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

        let conversation = self.create_gateway_session()?;
        self.store.set_gateway_chat_session(
            GATEWAY_PLATFORM_TELEGRAM,
            chat_key,
            conversation.session_id,
        )?;
        Ok(conversation)
    }

    fn rotate_chat_session(&self, chat_key: &str) -> Result<Conversation> {
        let conversation = self.create_gateway_session()?;
        self.store.set_gateway_chat_session(
            GATEWAY_PLATFORM_TELEGRAM,
            chat_key,
            conversation.session_id,
        )?;
        Ok(conversation)
    }

    fn create_gateway_session(&self) -> Result<Conversation> {
        let session_id = Uuid::new_v4();
        let conversation = Conversation::new(
            session_id,
            self.workspace_root.display().to_string(),
            self.active_model.provider_id.clone(),
            self.active_model.provider_display_name.clone(),
            self.active_model.model_id.clone(),
            self.active_model.display_name.clone(),
            "Untitled session",
        );

        self.store.create_session(
            session_id,
            self.workspace_root.as_path(),
            &self.active_model.provider_id,
            &self.active_model.provider_display_name,
            &self.active_model.model_id,
            &self.active_model.display_name,
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

fn normalize_assistant_output(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "(no content)".to_string()
    } else {
        trimmed.to_string()
    }
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

fn is_new_command(content: &str) -> bool {
    let Some(command) = content.split_whitespace().next() else {
        return false;
    };

    let command = command.trim();
    command.eq_ignore_ascii_case("/new") || command.to_ascii_lowercase().starts_with("/new@")
}

#[derive(Clone)]
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
    ) -> Result<()> {
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

        let _ = payload.into_result("sendMessage")?;
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
}

#[derive(Debug, Deserialize)]
struct TelegramSentMessage {
    #[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_new_command_variants() {
        assert!(is_new_command("/new"));
        assert!(is_new_command("/new please"));
        assert!(is_new_command("/new@my_bot"));
        assert!(is_new_command("/new@my_bot please"));

        assert!(!is_new_command("/new-session"));
        assert!(!is_new_command("hello /new"));
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
