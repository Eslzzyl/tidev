//! Lark/Feishu REST API client for authentication and message operations.

use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

use super::types::{
    BotInfoResp, SendMessageResp, TenantAccessTokenResp, WsEndpointResp,
};

/// Lark/Feishu REST API client.
pub struct LarkClient {
    client: Client,
    app_id: String,
    app_secret: String,
    use_feishu: bool,
    token_cache: Arc<RwLock<Option<(String, Instant)>>>,
}

impl LarkClient {
    pub fn new(app_id: String, app_secret: String, use_feishu: bool) -> Self {
        Self {
            client: Client::new(),
            app_id,
            app_secret,
            use_feishu,
            token_cache: Arc::new(RwLock::new(None)),
        }
    }

    fn api_base(&self) -> &str {
        if self.use_feishu {
            "https://open.feishu.cn/open-apis"
        } else {
            "https://open.larksuite.com/open-apis"
        }
    }

    fn ws_base(&self) -> String {
        format!("https://{}", if self.use_feishu {
            "open.feishu.cn"
        } else {
            "open.larksuite.com"
        })
    }

    /// Get a cached or fresh tenant_access_token.
    pub async fn get_tenant_access_token(&self) -> Result<String> {
        {
            let cache = self.token_cache.read().await;
            if let Some((token, expiry)) = &*cache
                && Instant::now() < *expiry
            {
                return Ok(token.clone());
            }
        }

        let mut cache = self.token_cache.write().await;
        // Double-check after acquiring write lock
        if let Some((token, expiry)) = &*cache
            && Instant::now() < *expiry
        {
            return Ok(token.clone());
        }

        let resp = self
            .client
            .post(format!("{}/auth/v3/tenant_access_token/internal", self.api_base()))
            .json(&json!({
                "app_id": self.app_id,
                "app_secret": self.app_secret,
            }))
            .send()
            .await?
            .json::<TenantAccessTokenResp>()
            .await?;

        if resp.code != 0 {
            bail!(
                "tenant_access_token failed: code={} msg={}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        let token = resp
            .tenant_access_token
            .context("tenant_access_token missing in response")?;
        let expire = resp.expire.unwrap_or(7200);
        let expiry = Instant::now() + Duration::from_secs(expire.saturating_sub(60));

        *cache = Some((token.clone(), expiry));
        Ok(token)
    }

    /// Get the WebSocket endpoint URL for event listening.
    pub async fn get_ws_endpoint(&self) -> Result<(String, super::types::WsClientConfig)> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .client
            .post(format!("{}/callback/ws/endpoint", self.ws_base()))
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "AppID": self.app_id,
                "AppSecret": self.app_secret,
            }))
            .send()
            .await?
            .json::<WsEndpointResp>()
            .await?;

        if resp.code != 0 {
            bail!(
                "WS endpoint failed: code={} msg={}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        let ep = resp.data.context("WS endpoint: empty data")?;
        Ok((ep.url, ep.client_config.unwrap_or_default()))
    }

    /// Get bot info (to resolve our own open_id).
    pub async fn get_bot_info(&self) -> Result<String> {
        let token = self.get_tenant_access_token().await?;
        let resp = self
            .client
            .get(format!("{}/bot/v3/info", self.api_base()))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json::<BotInfoResp>()
            .await?;

        if resp.code != 0 {
            bail!(
                "bot info failed: code={} msg={}",
                resp.code,
                resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        resp.bot
            .and_then(|b| {
                let oid = b.open_id;
                if oid.is_empty() { None } else { Some(oid) }
            })
            .context("bot open_id missing or empty")
    }

    /// Send a text message to a chat. Returns the message_id.
    pub async fn send_text_message(&self, chat_id: &str, text: &str) -> Result<String> {
        let token = self.get_tenant_access_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type=chat_id", self.api_base());

        let body = json!({
            "receive_id": chat_id,
            "msg_type": "text",
            "content": json!({ "text": text }).to_string(),
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await?;

        let _status = resp.status();
        let send_resp: SendMessageResp = resp.json().await?;
        if send_resp.code != 0 {
            bail!(
                "send_text_message failed: code={} msg={}",
                send_resp.code,
                send_resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        let data = send_resp.data.context("send_text_message: missing data")?;
        let message_id = data.message_id;
        if message_id.is_empty() {
            bail!("send_text_message: empty message_id");
        }
        Ok(message_id)
    }

    /// Send an interactive card message (for long/complex responses).
    pub async fn send_card_message(&self, chat_id: &str, markdown: &str) -> Result<String> {
        let token = self.get_tenant_access_token().await?;
        let url = format!("{}/im/v1/messages?receive_id_type=chat_id", self.api_base());

        let card_content = json!({
            "config": { "wide_screen_mode": true },
            "header": {
                "title": { "tag": "plain_text", "content": "Tidev" },
                "template": "blue"
            },
            "elements": [
                { "tag": "markdown", "content": markdown }
            ]
        });

        let body = json!({
            "receive_id": chat_id,
            "msg_type": "interactive",
            "content": card_content.to_string(),
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await?;

        let send_resp: SendMessageResp = resp.json().await?;
        if send_resp.code != 0 {
            bail!(
                "send_card_message failed: code={} msg={}",
                send_resp.code,
                send_resp.msg.as_deref().unwrap_or("(none)")
            );
        }

        let data = send_resp.data.context("send_card_message: missing data")?;
        let message_id = data.message_id;
        if message_id.is_empty() {
            bail!("send_card_message: empty message_id");
        }
        Ok(message_id)
    }

    /// Add a reaction emoji to a message (best-effort).
    pub async fn add_reaction(&self, message_id: &str, emoji_type: &str) -> Result<()> {
        let token = match self.get_tenant_access_token().await {
            Ok(t) => t,
            Err(e) => {
                crate::log_warn!("Lark add_reaction: token refresh failed: {e}");
                return Ok(());
            }
        };

        let url = format!(
            "{}/im/v1/messages/{message_id}/reactions",
            self.api_base()
        );

        let body = json!({
            "reaction_type": { "emoji_type": emoji_type }
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json; charset=utf-8")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            crate::log_warn!("Lark add_reaction failed: {text}");
        }
        Ok(())
    }
}
