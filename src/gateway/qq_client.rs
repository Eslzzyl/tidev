use anyhow::{Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

#[derive(Debug, Serialize, Deserialize)]
pub struct QQAccessToken {
    pub access_token: String,
    pub expires_in: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QQGatewayResponse {
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct QQClient {
    client: Client,
    app_id: String,
    app_secret: String,
    sandbox: bool,
    token_cache: Arc<RwLock<Option<(String, Instant)>>>,
}

impl QQClient {
    pub fn new(app_id: String, app_secret: String, sandbox: bool) -> Self {
        Self {
            client: Client::new(),
            app_id,
            app_secret,
            sandbox,
            token_cache: Arc::new(RwLock::new(None)),
        }
    }

    pub fn base_url(&self) -> &str {
        if self.sandbox {
            "https://sandbox.api.sgroup.qq.com"
        } else {
            "https://api.sgroup.qq.com"
        }
    }

    pub async fn get_access_token(&self) -> Result<String> {
        {
            let cache = self.token_cache.read().await;
            if let Some((token, expiry)) = &*cache
                && Instant::now() < *expiry {
                    return Ok(token.clone());
                }
        }

        let mut cache = self.token_cache.write().await;
        // Double check after acquiring write lock
        if let Some((token, expiry)) = &*cache
            && Instant::now() < *expiry {
                return Ok(token.clone());
            }

        let resp = self
            .client
            .post("https://bots.qq.com/app/getAppAccessToken")
            .json(&serde_json::json!({
                "appId": self.app_id,
                "clientSecret": self.app_secret,
            }))
            .send()
            .await?
            .json::<QQAccessToken>()
            .await?;

        let token = resp.access_token;
        let expires_in: u64 = resp.expires_in.parse().unwrap_or(7200);
        // Buffering the expiry by 60 seconds
        let expiry = Instant::now() + Duration::from_secs(expires_in.saturating_sub(60));

        *cache = Some((token.clone(), expiry));
        Ok(token)
    }

    pub async fn get_gateway_url(&self) -> Result<String> {
        let token = self.get_access_token().await?;
        let resp = self
            .client
            .get(format!("{}/gateway", self.base_url()))
            .header("Authorization", format!("QQBot {}", token))
            .send()
            .await?
            .json::<QQGatewayResponse>()
            .await?;

        Ok(resp.url)
    }

    pub async fn send_message(
        &self,
        channel_id: &str,
        content: &str,
        msg_id: Option<&str>,
    ) -> Result<()> {
        let token = self.get_access_token().await?;
        let url = format!("{}/channels/{}/messages", self.base_url(), channel_id);

        let mut body = serde_json::json!({
            "content": content,
        });

        if let Some(mid) = msg_id {
            body.as_object_mut()
                .unwrap()
                .insert("msg_id".to_string(), mid.into());
        }

        let resp = self
            .client
            .post(url)
            .header("Authorization", format!("QQBot {}", token))
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            bail!("QQ send_message failed: {}", error_text);
        }

        Ok(())
    }
}
