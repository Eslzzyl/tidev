//! Telegram data types for API requests and responses.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Telegram API response wrapper.
#[derive(Debug, Deserialize)]
pub struct TelegramApiResponse<T> {
    pub ok: bool,
    pub result: Option<T>,
    pub description: Option<String>,
    pub error_code: Option<u32>,
}

impl<T> TelegramApiResponse<T> {
    /// Extract result or generate error from failed response.
    pub fn into_result(self, operation: &str) -> Result<T, anyhow::Error> {
        if self.ok {
            self.result
                .context("missing result field in Telegram API response")
        } else {
            let msg = self
                .description
                .unwrap_or_else(|| "unknown error".to_string());
            anyhow::bail!("telegram API {} failed: code={:?}, description={}", operation, self.error_code, msg)
        }
    }
}

/// Incoming update from Telegram getUpdates.
#[derive(Debug, Deserialize, Clone)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

/// Incoming message from Telegram.
#[derive(Debug, Deserialize, Clone)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub date: i64,
    pub text: Option<String>,
    pub caption: Option<String>,
    pub chat: TelegramChat,
    pub from: Option<TelegramUser>,
    pub reply_to_message: Option<Box<TelegramMessage>>,
    pub message_thread_id: Option<i64>,
}

/// User from Telegram.
#[derive(Debug, Deserialize, Clone)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
}

/// Chat entity from Telegram.
#[derive(Debug, Deserialize, Clone)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: Option<String>,
    pub username: Option<String>,
}

/// Sent message from Telegram sendMessage.
#[derive(Debug, Deserialize, Clone)]
pub struct TelegramSentMessage {
    pub message_id: i64,
    pub date: i64,
    pub chat: TelegramChat,
}

/// Request body for sendMessage API.
#[derive(Debug, Serialize)]
pub struct SendMessageRequest<'a> {
    pub chat_id: i64,
    pub text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
}

/// Request body for editMessageText API.
#[derive(Debug, Serialize)]
pub struct EditMessageRequest<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub text: &'a str,
    pub parse_mode: String,
}
