//! Discord REST API client for sending, editing, and deleting messages.

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::json;

/// Maximum allowed Discord message length.
pub const DISCORD_MAX_MESSAGE_LENGTH: usize = 2000;

/// Discord REST API client.
pub struct DiscordClient {
    client: Client,
    bot_token: String,
}

impl DiscordClient {
    /// Create a new Discord client with the given bot token.
    pub fn new(bot_token: String) -> Self {
        Self {
            client: Client::new(),
            bot_token,
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!("https://discord.com/api/v10{}", path)
    }

    fn auth_header(&self) -> String {
        format!("Bot {}", self.bot_token)
    }

    /// Send a text message to a channel. Returns the message ID.
    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<String> {
        let url = self.api_url(&format!("/channels/{channel_id}/messages"));
        let body = json!({ "content": content });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("Discord send_message failed ({status}): {text}");
        }

        let msg: serde_json::Value = resp.json().await?;
        msg.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("Discord send_message response missing 'id'")
    }

    /// Edit an existing message (used for draft updates).
    pub async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<()> {
        let url = self.api_url(&format!(
            "/channels/{channel_id}/messages/{message_id}"
        ));
        let body = json!({ "content": content });

        let resp = self
            .client
            .patch(&url)
            .header("Authorization", self.auth_header())
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 429 {
            // Rate-limited; skip the edit (cosmetic-only)
            return Ok(());
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("Discord edit_message failed ({status}): {text}");
        }
        Ok(())
    }

    /// Delete a message.
    pub async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let url = self.api_url(&format!(
            "/channels/{channel_id}/messages/{message_id}"
        ));

        let resp = self
            .client
            .delete(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        let status = resp.status();
        if status.as_u16() == 429 {
            return Ok(());
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("Discord delete_message failed ({status}): {text}");
        }
        Ok(())
    }

    /// Trigger a typing indicator in a channel.
    pub async fn trigger_typing(&self, channel_id: &str) -> Result<()> {
        let url = self.api_url(&format!("/channels/{channel_id}/typing"));

        let resp = self
            .client
            .post(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            // Typing indicator is best-effort
            let text = resp.text().await.unwrap_or_default();
            crate::log_warn!("Discord trigger_typing failed: {text}");
        }
        Ok(())
    }

    /// Add a reaction emoji to a message.
    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded: String = urlencoding::encode(emoji).into_owned();
        let url = self.api_url(&format!(
            "/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"
        ));

        let resp = self
            .client
            .put(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        if !resp.status().is_success() {
            // Reactions are best-effort
            let text = resp.text().await.unwrap_or_default();
            crate::log_warn!("Discord add_reaction failed: {text}");
        }
        Ok(())
    }

    /// Get the Gateway WebSocket URL from Discord.
    pub async fn get_gateway_url(&self) -> Result<String> {
        let resp = self
            .client
            .get(self.api_url("/gateway/bot"))
            .header("Authorization", self.auth_header())
            .send()
            .await?;

        let gw: super::types::BotGatewayResponse = resp.json().await?;
        Ok(gw.url)
    }
}
