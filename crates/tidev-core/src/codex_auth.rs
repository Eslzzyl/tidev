use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, RwLock as StdRwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tidev_config::auth::{AuthStore, CodexOAuthCredential};
use tidev_config::paths::ConfigPaths;
use tidev_llm::{RequestAuth, RequestAuthResolver};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, mpsc::UnboundedSender};
use url::Url;
use uuid::Uuid;

const DEFAULT_ISSUER: &str = "https://auth.openai.com";
const DEFAULT_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEFAULT_ORIGINATOR: &str = "tidev";
const CALLBACK_PATH: &str = "/auth/callback";
const REFRESH_SKEW_MS: u64 = 5 * 60 * 1000;
const DEVICE_TIMEOUT_SECS: u64 = 900;

#[derive(Clone, Debug)]
pub struct CodexAuthEndpoints {
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub device_usercode: String,
    pub device_token: String,
    pub device_verification: String,
}

impl Default for CodexAuthEndpoints {
    fn default() -> Self {
        Self {
            issuer: DEFAULT_ISSUER.to_string(),
            authorize: format!("{DEFAULT_ISSUER}/oauth/authorize"),
            token: format!("{DEFAULT_ISSUER}/oauth/token"),
            device_usercode: format!("{DEFAULT_ISSUER}/api/accounts/deviceauth/usercode"),
            device_token: format!("{DEFAULT_ISSUER}/api/accounts/deviceauth/token"),
            device_verification: format!("{DEFAULT_ISSUER}/codex/device"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodexLoginMode {
    Browser,
    DeviceCode,
}

#[derive(Clone, Debug)]
pub enum CodexAuthEvent {
    AuthorizationUrl(String),
    DeviceCode {
        user_code: String,
        verification_uri: String,
        expires_in_secs: u64,
    },
    Progress(String),
}

#[derive(Clone)]
pub struct CodexAuthService {
    auth: Arc<StdRwLock<AuthStore>>,
    paths: ConfigPaths,
    http: Client,
    endpoints: CodexAuthEndpoints,
    refresh_locks: Arc<StdMutex<HashMap<String, Arc<Mutex<()>>>>>,
    recent_refresh_ms: Arc<StdMutex<HashMap<String, u64>>>,
}

impl fmt::Debug for CodexAuthService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexAuthService")
            .field("paths", &self.paths)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl CodexAuthService {
    pub fn new(auth: Arc<StdRwLock<AuthStore>>, paths: ConfigPaths) -> Result<Self> {
        install_rustls_provider();
        let http = Client::builder()
            .build()
            .context("failed to construct Codex OAuth HTTP client")?;
        Ok(Self::with_client(
            auth,
            paths,
            http,
            CodexAuthEndpoints::default(),
        ))
    }

    pub fn with_client(
        auth: Arc<StdRwLock<AuthStore>>,
        paths: ConfigPaths,
        http: Client,
        endpoints: CodexAuthEndpoints,
    ) -> Self {
        Self {
            auth,
            paths,
            http,
            endpoints,
            refresh_locks: Arc::new(StdMutex::new(HashMap::new())),
            recent_refresh_ms: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub async fn login(
        &self,
        provider_id: &str,
        mode: CodexLoginMode,
        events: UnboundedSender<CodexAuthEvent>,
    ) -> Result<()> {
        if provider_id.trim().is_empty() {
            bail!("provider id cannot be empty");
        }
        let token = match mode {
            CodexLoginMode::Browser => self.login_browser(&events).await?,
            CodexLoginMode::DeviceCode => self.login_device_code(&events).await?,
        };
        let credential = self.credential_from_token_response(token, None)?;
        self.store_credential(provider_id, credential)?;
        let _ = events.send(CodexAuthEvent::Progress(
            "Codex OAuth login complete".into(),
        ));
        Ok(())
    }

    async fn login_browser(
        &self,
        events: &UnboundedSender<CodexAuthEvent>,
    ) -> Result<TokenResponse> {
        let (listener, port) = bind_callback_listener().await?;
        let redirect_uri = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let verifier = random_urlsafe(64);
        let state = random_urlsafe(32);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let client_id = client_id();
        let authorize = build_browser_authorization_url(
            &self.endpoints.authorize,
            &client_id,
            &redirect_uri,
            &challenge,
            &state,
        )?;
        let _ = events.send(CodexAuthEvent::AuthorizationUrl(authorize.to_string()));
        let query = receive_callback(listener, &state).await?;
        if let Some(error) = query.get("error") {
            bail!("Codex OAuth authorization failed: {error}");
        }
        let code = query
            .get("code")
            .context("Codex OAuth callback did not contain a code")?;
        let response = self
            .http
            .post(&self.endpoints.token)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client_id.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", redirect_uri.as_str()),
                ("code_verifier", verifier.as_str()),
            ])
            .send()
            .await
            .context("Codex OAuth token exchange failed")?;
        parse_token_response(response).await
    }

    async fn login_device_code(
        &self,
        events: &UnboundedSender<CodexAuthEvent>,
    ) -> Result<TokenResponse> {
        let client_id = client_id();
        let response = self
            .http
            .post(&self.endpoints.device_usercode)
            .json(&serde_json::json!({"client_id": client_id}))
            .send()
            .await
            .context("Codex device-code request failed")?;
        let response = response
            .error_for_status()
            .context("Codex device-code request was rejected")?;
        let start: DeviceCodeStart = response
            .json()
            .await
            .context("invalid Codex device-code response")?;
        let _ = events.send(CodexAuthEvent::DeviceCode {
            user_code: start.user_code.clone(),
            verification_uri: self.endpoints.device_verification.clone(),
            expires_in_secs: start.expires_in.unwrap_or(DEVICE_TIMEOUT_SECS),
        });

        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_secs(start.expires_in.unwrap_or(DEVICE_TIMEOUT_SECS));
        let mut interval = std::time::Duration::from_secs(start.interval.unwrap_or(5));
        loop {
            if tokio::time::Instant::now() >= deadline {
                bail!("Codex device-code login timed out");
            }
            let response = self
                .http
                .post(&self.endpoints.device_token)
                .json(&serde_json::json!({
                    "device_auth_id": &start.device_auth_id,
                    "user_code": &start.user_code,
                }))
                .send()
                .await
                .context("Codex device-code polling failed")?;
            if response.status().is_success() {
                let authorization: DeviceCodeAuthorizationResponse = response
                    .json()
                    .await
                    .context("invalid Codex device-code authorization response")?;
                let redirect_uri = format!(
                    "{}/deviceauth/callback",
                    self.endpoints.issuer.trim_end_matches('/')
                );
                let response = self
                    .http
                    .post(&self.endpoints.token)
                    .form(&[
                        ("grant_type", "authorization_code"),
                        ("client_id", client_id.as_str()),
                        ("code", authorization.authorization_code.as_str()),
                        ("redirect_uri", redirect_uri.as_str()),
                        ("code_verifier", authorization.code_verifier.as_str()),
                    ])
                    .send()
                    .await
                    .context("Codex device-code token exchange failed")?;
                return parse_token_response(response).await;
            }
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let code = device_error_code(&body).unwrap_or_default();
            if status.as_u16() == 403
                || status.as_u16() == 404
                || matches!(
                    code.as_str(),
                    "authorization_pending" | "device_authorization_pending"
                )
            {
                tokio::time::sleep(interval).await;
                continue;
            }
            if code == "slow_down" {
                interval += std::time::Duration::from_secs(5);
                tokio::time::sleep(interval).await;
                continue;
            }
            bail!("Codex device-code polling failed: HTTP {}", status);
        }
    }

    fn credential_from_token_response(
        &self,
        response: TokenResponse,
        previous: Option<&CodexOAuthCredential>,
    ) -> Result<CodexOAuthCredential> {
        let access_token = response.access_token;
        let id_token = response
            .id_token
            .or_else(|| previous.and_then(|value| value.id_token.clone()));
        let account_id = account_id_from_token(&access_token)
            .or_else(|| id_token.as_deref().and_then(account_id_from_token))
            .or_else(|| previous.map(|value| value.account_id.clone()))
            .filter(|value| !value.trim().is_empty())
            .context("Codex OAuth token did not contain chatgpt_account_id")?;
        let expires_at_ms = response
            .expires_in
            .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1000)))
            .or_else(|| expiry_from_token(&access_token))
            .or_else(|| previous.map(|value| value.expires_at_ms))
            .context("Codex OAuth token did not contain an expiry")?;
        let refresh_token = response
            .refresh_token
            .or_else(|| previous.map(|value| value.refresh_token.clone()))
            .filter(|value| !value.trim().is_empty())
            .context("Codex OAuth token did not contain a refresh token")?;
        let is_fedramp = fedramp_from_token(&access_token)
            .or_else(|| id_token.as_deref().and_then(fedramp_from_token))
            .or_else(|| previous.and_then(|value| value.is_fedramp));
        Ok(CodexOAuthCredential {
            access_token,
            refresh_token,
            expires_at_ms,
            account_id,
            id_token,
            is_fedramp,
        })
    }

    fn store_credential(&self, provider_id: &str, credential: CodexOAuthCredential) -> Result<()> {
        let mut auth = self
            .auth
            .write()
            .map_err(|_| anyhow::anyhow!("auth store lock poisoned"))?;
        auth.set_codex_oauth(provider_id, credential);
        auth.save(&self.paths)
    }

    async fn resolve_inner(&self, provider_id: &str, force_refresh: bool) -> Result<RequestAuth> {
        let current = self.load_credential(provider_id)?;
        if !force_refresh && !current.is_expiring(now_ms(), REFRESH_SKEW_MS) {
            return Ok(request_auth(&current));
        }

        let lock = {
            let mut locks = self
                .refresh_locks
                .lock()
                .map_err(|_| anyhow::anyhow!("Codex refresh lock poisoned"))?;
            locks
                .entry(provider_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        let current = self.load_credential(provider_id)?;
        let recently_refreshed = force_refresh
            && self
                .recent_refresh_ms
                .lock()
                .ok()
                .and_then(|values| values.get(provider_id).copied())
                .is_some_and(|timestamp| now_ms().saturating_sub(timestamp) < 30_000);
        if (!force_refresh && !current.is_expiring(now_ms(), REFRESH_SKEW_MS)) || recently_refreshed
        {
            return Ok(request_auth(&current));
        }

        let client_id = client_id();
        let refresh_token = current.refresh_token.clone();
        let response = self
            .http
            .post(&self.endpoints.token)
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id.as_str()),
                ("refresh_token", refresh_token.as_str()),
            ])
            .send()
            .await
            .context("Codex OAuth token refresh failed")?;
        if response.status().as_u16() == 400 || response.status().as_u16() == 401 {
            self.clear_credential(provider_id)?;
            bail!("Codex OAuth refresh token is invalid; login again");
        }
        let token = parse_token_response(response).await?;
        let credential = self.credential_from_token_response(token, Some(&current))?;
        self.store_credential(provider_id, credential.clone())?;
        if let Ok(mut values) = self.recent_refresh_ms.lock() {
            values.insert(provider_id.to_string(), now_ms());
        }
        Ok(request_auth(&credential))
    }

    fn load_credential(&self, provider_id: &str) -> Result<CodexOAuthCredential> {
        let auth = self
            .auth
            .read()
            .map_err(|_| anyhow::anyhow!("auth store lock poisoned"))?;
        auth.codex_oauth(provider_id)
            .cloned()
            .with_context(|| format!("Codex OAuth is not configured for provider '{provider_id}'"))
    }
    fn clear_credential(&self, provider_id: &str) -> Result<()> {
        let mut auth = self
            .auth
            .write()
            .map_err(|_| anyhow::anyhow!("auth store lock poisoned"))?;
        auth.remove_codex_oauth(provider_id);
        auth.save(&self.paths)
    }
}

impl RequestAuthResolver for CodexAuthService {
    fn resolve<'a>(
        &'a self,
        provider_id: &'a str,
        force_refresh: bool,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<RequestAuth>> + Send + 'a>> {
        Box::pin(self.resolve_inner(provider_id, force_refresh))
    }
}

fn request_auth(credential: &CodexOAuthCredential) -> RequestAuth {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert(
        "ChatGPT-Account-ID".to_string(),
        credential.account_id.clone(),
    );
    if credential.is_fedramp == Some(true) {
        headers.insert("X-OpenAI-Fedramp".to_string(), "true".to_string());
    }
    RequestAuth {
        bearer_token: credential.access_token.clone(),
        headers,
    }
}

fn client_id() -> String {
    std::env::var("CODEX_APP_SERVER_LOGIN_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.into())
}

fn build_browser_authorization_url(
    authorize_endpoint: &str,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Result<Url> {
    let mut authorize =
        Url::parse(authorize_endpoint).context("invalid Codex authorize endpoint")?;
    authorize.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        (
            "scope",
            "openid profile email offline_access api.connectors.read api.connectors.invoke",
        ),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", DEFAULT_ORIGINATOR),
    ]);
    Ok(authorize)
}

fn device_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error").cloned())
        .and_then(|error| {
            error.as_str().map(str::to_string).or_else(|| {
                error
                    .get("code")
                    .and_then(|value| value.as_str())
                    .map(str::to_string)
            })
        })
}

fn random_urlsafe(bytes: usize) -> String {
    let mut value = Vec::with_capacity(bytes);
    while value.len() < bytes {
        value.extend_from_slice(Uuid::new_v4().as_bytes());
    }
    URL_SAFE_NO_PAD.encode(&value[..bytes])
}

async fn bind_callback_listener() -> Result<(TcpListener, u16)> {
    for port in [1455, 1457] {
        match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => return Ok((listener, port)),
            Err(_) => continue,
        }
    }
    bail!("unable to bind Codex OAuth callback ports 1455 or 1457")
}

async fn receive_callback(
    listener: TcpListener,
    expected_state: &str,
) -> Result<HashMap<String, String>> {
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(900), listener.accept())
            .await
            .context("timed out waiting for Codex OAuth callback")??;
    let mut buffer = vec![0_u8; 16 * 1024];
    let length = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..length]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|line| line.split_whitespace().next())
        .context("invalid Codex OAuth callback request")?;
    let url = Url::parse(&format!("http://127.0.0.1{target}"))?;
    if url.path() != CALLBACK_PATH {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await;
        bail!("invalid Codex OAuth callback path")
    }
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if query.get("state").map(String::as_str) != Some(expected_state) {
        let _ = stream
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
        bail!("Codex OAuth callback state mismatch")
    }
    let body = b"Codex login completed. You can close this window.";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    Ok(query)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeStart {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_u64")]
    interval: Option<u64>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_optional_u64")]
    expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DeviceCodeAuthorizationResponse {
    authorization_code: String,
    code_verifier: String,
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Number(u64),
        String(String),
    }

    match Option::<Value>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Value::Number(value)) => Ok(Some(value)),
        Some(Value::String(value)) => value
            .trim()
            .parse()
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

async fn parse_token_response(response: reqwest::Response) -> Result<TokenResponse> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Codex OAuth token endpoint returned HTTP {status}: {body}");
    }
    response
        .json()
        .await
        .context("invalid Codex OAuth token response")
}

fn claims(token: &str) -> Option<serde_json::Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn account_id_from_token(token: &str) -> Option<String> {
    let claims = claims(token)?;
    claims
        .get("https://api.openai.com/auth.chatgpt_account_id")
        .or_else(|| claims.get("chatgpt_account_id"))
        .or_else(|| {
            claims
                .get("https://api.openai.com/auth")
                .and_then(|auth| auth.get("chatgpt_account_id"))
        })
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn expiry_from_token(token: &str) -> Option<u64> {
    claims(token)?.get("exp")?.as_u64()?.checked_mul(1000)
}

fn fedramp_from_token(token: &str) -> Option<bool> {
    let claims = claims(token)?;
    claims
        .get("fedramp")
        .or_else(|| claims.get("https://api.openai.com/auth.is_fedramp"))
        .and_then(|value| value.as_bool())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn install_rustls_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(claims: serde_json::Value) -> String {
        format!(
            "header.{}.signature",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
        )
    }

    #[test]
    fn extracts_account_id_from_supported_claim_shapes() {
        assert_eq!(
            account_id_from_token(&token(serde_json::json!({
                "https://api.openai.com/auth.chatgpt_account_id": "account-a"
            }))),
            Some("account-a".into())
        );
        assert_eq!(
            account_id_from_token(&token(serde_json::json!({
                "chatgpt_account_id": "account-b"
            }))),
            Some("account-b".into())
        );
        assert_eq!(
            account_id_from_token(&token(serde_json::json!({
                "https://api.openai.com/auth": {"chatgpt_account_id": "account-c"}
            }))),
            Some("account-c".into())
        );
    }

    #[test]
    fn extracts_expiry_and_fedramp_claims() {
        let value = token(serde_json::json!({
            "exp": 1_700_000_000_u64,
            "fedramp": true
        }));
        assert_eq!(expiry_from_token(&value), Some(1_700_000_000_000));
        assert_eq!(fedramp_from_token(&value), Some(true));
    }

    #[test]
    fn browser_authorization_url_encodes_redirect_uri_once() {
        let redirect_uri = "http://127.0.0.1:1455/auth/callback";
        let url = build_browser_authorization_url(
            "https://auth.openai.com/oauth/authorize",
            DEFAULT_CLIENT_ID,
            redirect_uri,
            "challenge",
            "state",
        )
        .unwrap();

        assert_eq!(
            url.query_pairs()
                .find_map(|(key, value)| (key == "redirect_uri").then(|| value.into_owned())),
            Some(redirect_uri.to_string())
        );
        assert!(
            url.as_str()
                .contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback")
        );
        assert!(!url.as_str().contains("redirect_uri=http%253A"));
    }

    #[test]
    fn device_code_numeric_fields_accept_strings_and_numbers() {
        let numeric: DeviceCodeStart = serde_json::from_value(serde_json::json!({
            "device_auth_id": "device-id",
            "user_code": "ABCD-1234",
            "interval": 5,
            "expires_in": 900
        }))
        .unwrap();
        assert_eq!(numeric.interval, Some(5));
        assert_eq!(numeric.expires_in, Some(900));

        let strings: DeviceCodeStart = serde_json::from_value(serde_json::json!({
            "device_auth_id": "device-id",
            "user_code": "ABCD-1234",
            "interval": "5",
            "expires_in": "900"
        }))
        .unwrap();
        assert_eq!(strings.interval, Some(5));
        assert_eq!(strings.expires_in, Some(900));
    }

    #[test]
    fn device_error_code_accepts_string_and_object_shapes() {
        assert_eq!(
            device_error_code(r#"{"error":"slow_down"}"#),
            Some("slow_down".into())
        );
        assert_eq!(
            device_error_code(r#"{"error":{"code":"deviceauth_authorization_pending"}}"#),
            Some("deviceauth_authorization_pending".into())
        );
    }

    #[tokio::test]
    async fn refresh_is_singleflight_and_updates_persisted_auth() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let refreshed_access_token = token(serde_json::json!({
            "https://api.openai.com/auth.chatgpt_account_id": "account-new",
            "exp": 1_900_000_000_u64
        }));
        let response_body = format!(
            r#"{{"access_token":"{refreshed_access_token}","refresh_token":"new-refresh","expires_in":3600}}"#
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes())
                .await
                .unwrap();
        });

        let root = std::env::temp_dir().join(format!(
            "tidev-codex-refresh-test-{}-{}",
            std::process::id(),
            now_ms()
        ));
        let paths = ConfigPaths {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            config_file: root.join("config/config.toml"),
            mcp_file: root.join("config/mcp.json"),
            auth_file: root.join("data/auth.json"),
            database_file: root.join("data/sessions.sqlite3"),
        };
        let mut initial_auth = AuthStore::default();
        initial_auth.set_codex_oauth(
            "openai-codex",
            CodexOAuthCredential {
                access_token: "old-access".into(),
                refresh_token: "old-refresh".into(),
                expires_at_ms: 0,
                account_id: "account-old".into(),
                id_token: None,
                is_fedramp: None,
            },
        );
        initial_auth.save(&paths).unwrap();
        let auth = Arc::new(StdRwLock::new(initial_auth));
        let endpoints = CodexAuthEndpoints {
            issuer: format!("http://{address}"),
            authorize: format!("http://{address}/authorize"),
            token: format!("http://{address}/token"),
            device_usercode: format!("http://{address}/device/usercode"),
            device_token: format!("http://{address}/device/token"),
            device_verification: format!("http://{address}/device"),
        };
        install_rustls_provider();
        let service = CodexAuthService::with_client(
            auth,
            paths.clone(),
            Client::builder().build().unwrap(),
            endpoints,
        );

        let first = service.clone();
        let second = service.clone();
        let (first, second) = tokio::join!(
            async move { first.resolve("openai-codex", false).await.unwrap() },
            async move { second.resolve("openai-codex", false).await.unwrap() }
        );
        assert_eq!(first.bearer_token, refreshed_access_token);
        assert_eq!(second.bearer_token, refreshed_access_token);
        assert_eq!(
            first.headers.get("ChatGPT-Account-ID"),
            Some(&"account-new".to_string())
        );
        server.await.unwrap();

        let stored = AuthStore::load_or_create(&paths).unwrap();
        assert_eq!(
            stored.codex_oauth("openai-codex").unwrap().refresh_token,
            "new-refresh"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn default_codex_endpoints_are_complete() {
        let endpoints = CodexAuthEndpoints::default();
        assert_eq!(endpoints.issuer, "https://auth.openai.com");
        assert!(endpoints.authorize.ends_with("/oauth/authorize"));
        assert!(endpoints.token.ends_with("/oauth/token"));
        assert!(endpoints.device_usercode.ends_with("/deviceauth/usercode"));
        assert!(endpoints.device_token.ends_with("/deviceauth/token"));
        assert!(endpoints.device_verification.ends_with("/codex/device"));
    }
}
