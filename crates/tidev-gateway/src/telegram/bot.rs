//! Telegram bot API client.

use std::fmt::Write;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::Client;

use super::types::{SendMessageRequest, TelegramApiResponse, TelegramSentMessage, TelegramUpdate};

/// Telegram bot API client.
#[derive(Clone)]
pub struct TelegramBot {
    token: String,
    http: Client,
}

impl TelegramBot {
    /// Create a new bot client with the given token.
    ///
    /// `request_timeout_secs` is the maximum time allowed for a single HTTP
    /// request (including the long-poll wait).  A reasonable value is the
    /// `getUpdates` poll timeout plus a small buffer (e.g. `poll_timeout + 10`).
    pub fn new(token: String, request_timeout_secs: u64) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(request_timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .unwrap_or_else(|e| {
                log::warn!("Failed to build Telegram HTTP client with custom timeouts: {e}. Falling back to default client.");
                Client::new()
            });
        Self { token, http }
    }

    /// Build API URL for an endpoint.
    fn api_url(&self, endpoint: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, endpoint)
    }

    /// Convert Markdown text to Telegram HTML format.
    pub fn markdown_to_telegram_html(text: &str) -> String {
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
                    && let Some(end) = line[i + 1..].find('_')
                    && end > 0
                {
                    let inner = Self::escape_html(&line[i + 1..i + 1 + end]);
                    let _ = write!(line_out, "<i>{inner}</i>");
                    i += 2 + end;
                    continue;
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

    /// Probe getUpdates with `timeout=0` to claim the polling slot at startup.
    ///
    /// Returns the highest `update_id + 1` to use as the initial offset, or
    /// `offset` unchanged if no updates are pending.
    pub async fn probe_updates(&self, offset: i64) -> Result<i64> {
        let body = serde_json::json!({
            "offset": offset,
            "timeout": 0,
            "allowed_updates": ["message"],
        });

        let response = self
            .http
            .post(self.api_url("getUpdates"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram getUpdates probe")?;

        let payload: TelegramApiResponse<Vec<TelegramUpdate>> = response
            .json()
            .await
            .context("failed to parse Telegram getUpdates probe response")?;

        let updates = payload.into_result("getUpdates probe")?;
        let new_offset = updates
            .iter()
            .map(|u| u.update_id)
            .max()
            .map(|id| id.saturating_add(1))
            .unwrap_or(offset);
        Ok(new_offset)
    }

    /// Fetch updates from Telegram.
    pub async fn get_updates(&self, offset: i64, timeout_secs: u64) -> Result<Vec<TelegramUpdate>> {
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

    /// Send a plain text message.
    pub async fn send_message(
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
    pub async fn send_message_html(
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

    /// Edit message text with HTML parse mode for Markdown rendering.
    pub async fn edit_message_text_html(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
    ) -> Result<()> {
        let html_text = Self::markdown_to_telegram_html(text);
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": html_text,
            "parse_mode": "HTML",
        });

        let response = self
            .http
            .post(self.api_url("editMessageText"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram editMessageText")?;

        let payload: TelegramApiResponse<bool> = response
            .json()
            .await
            .context("failed to parse Telegram editMessageText response")?;

        payload.into_result("editMessageText").map(|_| ())
    }

    /// Delete a message.
    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<()> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });

        let response = self
            .http
            .post(self.api_url("deleteMessage"))
            .json(&body)
            .send()
            .await
            .context("failed to call Telegram deleteMessage")?;

        let payload: TelegramApiResponse<bool> = response
            .json()
            .await
            .context("failed to parse Telegram deleteMessage response")?;

        payload.into_result("deleteMessage").map(|_| ())
    }

    /// Set bot command menu.
    pub async fn set_my_commands(&self, commands: Vec<(String, String)>) -> Result<()> {
        let body = serde_json::json!({
            "commands": commands.into_iter().map(|(cmd, desc)| {
                serde_json::json!({
                    "command": cmd,
                    "description": desc
                })
            }).collect::<Vec<_>>()
        });

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

        payload.into_result("setMyCommands").map(|_| ())
    }

    /// Set reaction to a message.
    pub async fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        reaction: &str,
    ) -> Result<()> {
        let body = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [serde_json::json!({ "type": "emoji", "emoji": reaction })]
        });

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

        payload.into_result("setMessageReaction").map(|_| ())
    }
}
