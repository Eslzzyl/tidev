use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths::ConfigPaths;
use crate::reasoning::ThinkingLevelType;
use crate::types::ApiType;

// ---------------------------------------------------------------------------
// AuthStore
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderAuth>,
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelAuth>,
    #[serde(default)]
    pub web: WebAuth,
}

impl AuthStore {
    /// Load from auth file, creating a default one if it doesn't exist.
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        if !paths.auth_file.exists() {
            let auth = Self::default();
            auth.save(paths)?;
            return Ok(auth);
        }

        let contents = std::fs::read_to_string(&paths.auth_file)
            .with_context(|| format!("failed to read {}", paths.auth_file.display()))?;
        let auth: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.auth_file.display()))?;
        Ok(auth)
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents =
            serde_json::to_string_pretty(self).context("failed to serialize auth store")?;
        let parent = paths
            .auth_file
            .parent()
            .context("auth file has no parent directory")?;
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_path = parent.join(format!(
            ".{}.tmp-{}-{}",
            paths
                .auth_file
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("auth.json"),
            std::process::id(),
            nonce
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        let result = (|| -> Result<()> {
            file.write_all(contents.as_bytes())
                .with_context(|| format!("failed to write {}", temp_path.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync {}", temp_path.display()))?;
            std::fs::rename(&temp_path, &paths.auth_file)
                .with_context(|| format!("failed to replace {}", paths.auth_file.display()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp_path);
        }
        result
    }

    pub fn set_api_key(&mut self, provider_id: impl Into<String>, api_key: impl Into<String>) {
        let provider_id = provider_id.into();
        let api_key = api_key.into();
        let auth = self.providers.entry(provider_id).or_default();
        auth.api_key = Some(api_key);
        auth.codex_oauth = None;
    }

    pub fn set_codex_oauth(
        &mut self,
        provider_id: impl Into<String>,
        credential: CodexOAuthCredential,
    ) {
        let auth = self.providers.entry(provider_id.into()).or_default();
        auth.api_key = None;
        auth.codex_oauth = Some(credential);
    }

    pub fn codex_oauth(&self, provider_id: &str) -> Option<&CodexOAuthCredential> {
        self.providers
            .get(provider_id)
            .and_then(|provider| provider.codex_oauth.as_ref())
    }

    pub fn is_connected_for(&self, provider_id: &str, api_type: ApiType) -> bool {
        match api_type {
            ApiType::OpenAiCodexResponses => self.codex_oauth(provider_id).is_some(),
            _ => self.api_key(provider_id).is_some(),
        }
    }

    pub fn remove_credentials(&mut self, provider_id: &str) -> bool {
        let Some(auth) = self.providers.get_mut(provider_id) else {
            return false;
        };
        let removed = auth.api_key.take().is_some() || auth.codex_oauth.take().is_some();
        if auth.api_key.is_none() && auth.codex_oauth.is_none() {
            self.providers.remove(provider_id);
        }
        removed
    }

    /// Remove an OAuth credential for a single provider.
    pub fn remove_codex_oauth(&mut self, provider_id: &str) -> bool {
        if let Some(auth) = self.providers.get_mut(provider_id)
            && auth.codex_oauth.take().is_some()
        {
            if auth.api_key.is_none() {
                self.providers.remove(provider_id);
            }
            return true;
        }
        false
    }

    pub fn api_key(&self, provider_id: &str) -> Option<&str> {
        self.providers
            .get(provider_id)
            .and_then(|provider| provider.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    /// Remove the API key for a single provider, effectively disconnecting it.
    /// Returns `true` if a key was actually removed.
    pub fn remove_api_key(&mut self, provider_id: &str) -> bool {
        if let Some(auth) = self.providers.get_mut(provider_id)
            && auth.api_key.take().is_some()
        {
            return true;
        }
        false
    }

    /// Remove all provider entries whose ID does not appear in `known_ids`.
    pub fn prune_orphan_providers(&mut self, known_ids: &[String]) -> usize {
        let before = self.providers.len();
        self.providers.retain(|id, _| known_ids.contains(id));
        before - self.providers.len()
    }
}

// ---------------------------------------------------------------------------
// ProviderAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ProviderAuth {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_oauth: Option<CodexOAuthCredential>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexOAuthCredential {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at_ms: u64,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_fedramp: Option<bool>,
}

impl CodexOAuthCredential {
    pub fn is_expiring(&self, now_ms: u64, skew_ms: u64) -> bool {
        self.expires_at_ms <= now_ms.saturating_add(skew_ms)
    }
}

impl fmt::Debug for CodexOAuthCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexOAuthCredential")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &"<redacted>")
            .field("expires_at_ms", &self.expires_at_ms)
            .field("account_id", &self.account_id)
            .field("id_token", &self.id_token.as_ref().map(|_| "<redacted>"))
            .field("is_fedramp", &self.is_fedramp)
            .finish()
    }
}

impl fmt::Debug for ProviderAuth {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderAuth")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("codex_oauth", &self.codex_oauth)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ChannelAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChannelAuth {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// WebAuth
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WebAuth {
    /// Optional token for web UI authentication (Bearer token)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// API keys for web search providers, keyed by provider name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub search_api_keys: BTreeMap<String, String>,
    /// Google Custom Search Engine ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_cx: Option<String>,
}

// ---------------------------------------------------------------------------
// ActiveModel
// ---------------------------------------------------------------------------

/// A fully resolved model configuration ready for use by the LLM layer.
#[derive(Clone, Debug)]
pub struct ActiveModel {
    pub provider_id: String,
    pub provider_display_name: String,
    pub base_url: String,
    /// Optional provider-specific HTTP User-Agent override.
    pub user_agent: Option<String>,
    /// Static HTTP headers applied to every request for this provider.
    pub headers: std::collections::BTreeMap<String, String>,
    /// Header name that receives the current conversation UUID on each request.
    pub session_header: Option<String>,
    pub api_type: ApiType,
    pub model_id: String,
    pub request_model_id: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: Option<f32>,
    pub supports_images: bool,
    pub supports_parallel_tool_calls: bool,
    pub system_prompt: String,
    pub api_key: Option<String>,
    pub extra_body: Option<serde_json::Value>,
    pub thinking_level: ThinkingLevelType,
}

impl ActiveModel {
    /// Build the API endpoint URL for this model.
    pub fn endpoint(&self) -> String {
        match self.api_type {
            ApiType::Anthropic => {
                format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
            }
            ApiType::OpenAiChatCompletions => {
                format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
            }
            ApiType::OpenAiResponses => {
                let base = self.base_url.trim_end_matches('/');
                if base.ends_with("/v1/responses") {
                    base.to_string()
                } else {
                    format!("{base}/v1/responses")
                }
            }
            ApiType::OpenAiCodexResponses => {
                let base = self.base_url.trim_end_matches('/');
                if base.ends_with("/responses") {
                    base.to_string()
                } else if base.ends_with("/codex") {
                    format!("{base}/responses")
                } else {
                    format!("{base}/codex/responses")
                }
            }
            ApiType::GoogleGemini => {
                format!(
                    "{}/models/{}:generateContent",
                    self.base_url.trim_end_matches('/'),
                    self.request_model_id
                )
            }
        }
    }

    /// Gemini streaming endpoint.
    pub fn gemini_stream_endpoint(&self) -> String {
        format!(
            "{}/models/{}:streamGenerateContent?alt=sse",
            self.base_url.trim_end_matches('/'),
            self.request_model_id
        )
    }

    /// Merged extra_body with thinking level configuration.
    pub fn merged_extra_body(&self) -> Option<serde_json::Value> {
        self.merged_extra_body_with_thinking(self.thinking_level.clone())
    }

    pub fn merged_extra_body_with_thinking(
        &self,
        thinking_level: ThinkingLevelType,
    ) -> Option<serde_json::Value> {
        let thinking_extra = thinking_level.extra_body();

        match (&self.extra_body, thinking_extra) {
            (Some(base), Some(extra)) => {
                let mut merged = base.as_object().cloned().unwrap_or_default();
                if let Some(obj) = extra.as_object() {
                    merged.extend(obj.clone());
                }
                Some(serde_json::Value::Object(merged))
            }
            (Some(base), None) => Some(base.clone()),
            (None, Some(extra)) => Some(extra),
            (None, None) => None,
        }
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_display_name, self.display_name)
    }

    /// Determine whether this model should receive `apply_patch` instead of `write`/`edit`.
    ///
    /// GPT models (gpt-4o, gpt-4o-mini, gpt-4.1, gpt-5, gpt-6, etc.) get `apply_patch`.
    /// All other models (Claude, DeepSeek, Gemini, GPT-4, any OSS model) get `write`/`edit`.
    pub fn use_apply_patch(&self) -> bool {
        let id = self.request_model_id.to_ascii_lowercase();
        if !id.starts_with("gpt-") || id.contains("oss") {
            return false;
        }
        // gpt-4 and gpt-4-turbo/gpt-4-32k: no apply_patch
        // gpt-4o, gpt-4.1, gpt-5, etc.: apply_patch
        let after_prefix = &id[4..];
        after_prefix != "4" && !after_prefix.starts_with("4-")
    }

    /// Whether this is a GPT model eligible for fast mode.
    pub fn is_gpt(&self) -> bool {
        let id = self.request_model_id.trim().to_ascii_lowercase();
        id.starts_with("gpt-") && !id.contains("oss")
    }

    pub fn api_key_present(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }

    /// Thinking configuration for OpenAI Responses API.
    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        self.thinking_level.thinking_config()
    }
}

// ---------------------------------------------------------------------------
// ModelSummary
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ModelSummary {
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub request_model_id: String,
    pub model_display_name: String,
    pub base_url: String,
    pub api_type: ApiType,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub supports_images: bool,
}

impl ModelSummary {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }

    pub fn is_gpt(&self) -> bool {
        let id = self.request_model_id.trim().to_ascii_lowercase();
        id.starts_with("gpt-") && !id.contains("oss")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_model(model_id: &str) -> ActiveModel {
        ActiveModel {
            provider_id: "test".into(),
            provider_display_name: "Test".into(),
            base_url: "https://test.com".into(),
            user_agent: None,
            headers: std::collections::BTreeMap::new(),
            session_header: None,
            api_type: ApiType::OpenAiChatCompletions,
            model_id: model_id.into(),
            request_model_id: model_id.into(),
            display_name: model_id.into(),
            context_window: 128000,
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: ThinkingLevelType::None,
        }
    }

    #[test]
    fn gpt_4o_uses_apply_patch() {
        assert!(make_model("gpt-4o").use_apply_patch());
        assert!(make_model("gpt-4o-mini").use_apply_patch());
        assert!(make_model("gpt-4.1").use_apply_patch());
        assert!(make_model("gpt-5").use_apply_patch());
    }

    #[test]
    fn gpt_4_does_not_use_apply_patch() {
        assert!(!make_model("gpt-4").use_apply_patch());
        assert!(!make_model("gpt-4-turbo").use_apply_patch());
        assert!(!make_model("gpt-4-32k").use_apply_patch());
    }

    #[test]
    fn oss_models_do_not_use_apply_patch() {
        assert!(!make_model("gpt-4o-oss").use_apply_patch());
        assert!(!make_model("gpt-4o-oss-instruct").use_apply_patch());
    }

    #[test]
    fn request_model_id_controls_apply_patch_selection() {
        let mut model = make_model("custom-astra");
        model.request_model_id = "gpt-6-astra".into();
        assert!(model.use_apply_patch());

        model.request_model_id = "gpt-4".into();
        assert!(!model.use_apply_patch());
    }

    #[test]
    fn non_gpt_models_do_not_use_apply_patch() {
        assert!(!make_model("claude-3-5-sonnet").use_apply_patch());
        assert!(!make_model("deepseek-v4-flash").use_apply_patch());
        assert!(!make_model("gemini-2.5-flash").use_apply_patch());
    }

    #[test]
    fn gpt_models_are_eligible_for_fast_mode() {
        assert!(make_model("gpt-5").is_gpt());
        assert!(make_model("GPT-4o-mini").is_gpt());
        assert!(!make_model("gpt-oss").is_gpt());
        assert!(!make_model("claude-3-5-sonnet").is_gpt());
    }
    #[test]
    fn codex_oauth_credentials_are_backward_compatible_and_redacted() {
        let mut auth: AuthStore =
            serde_json::from_str(r#"{"providers":{"openai":{"api_key":"legacy-key"}}}"#).unwrap();
        assert_eq!(auth.api_key("openai"), Some("legacy-key"));
        assert!(!auth.is_connected_for("openai", ApiType::OpenAiCodexResponses));

        auth.set_codex_oauth(
            "openai",
            CodexOAuthCredential {
                access_token: "access-secret".into(),
                refresh_token: "refresh-secret".into(),
                expires_at_ms: 2_000,
                account_id: "account-123".into(),
                id_token: Some("id-secret".into()),
                is_fedramp: Some(false),
            },
        );
        assert!(auth.api_key("openai").is_none());
        assert!(auth.is_connected_for("openai", ApiType::OpenAiCodexResponses));
        let debug = format!("{:?}", auth.providers["openai"]);
        assert!(!debug.contains("access-secret"));
        assert!(!debug.contains("refresh-secret"));
        assert!(!debug.contains("id-secret"));
        assert!(debug.contains("account-123"));
    }

    #[test]
    fn auth_save_round_trips_oauth_credentials_atomically() {
        let root = std::env::temp_dir().join(format!(
            "tidev-auth-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let paths = ConfigPaths {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            config_file: root.join("config/config.toml"),
            mcp_file: root.join("config/mcp.json"),
            auth_file: root.join("data/auth.json"),
            database_file: root.join("data/sessions.sqlite3"),
        };
        let mut auth = AuthStore::default();
        auth.set_codex_oauth(
            "openai-codex",
            CodexOAuthCredential {
                access_token: "access".into(),
                refresh_token: "refresh".into(),
                expires_at_ms: 123,
                account_id: "account".into(),
                id_token: None,
                is_fedramp: None,
            },
        );
        auth.save(&paths).unwrap();
        let loaded = AuthStore::load_or_create(&paths).unwrap();
        assert_eq!(
            loaded.codex_oauth("openai-codex").unwrap().account_id,
            "account"
        );
        assert!(!std::fs::read_dir(&paths.data_dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        }));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&paths.auth_file)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
