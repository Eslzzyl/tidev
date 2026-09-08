use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::types::ApiType;

/// Validate a provider-specific HTTP User-Agent value.
pub fn validate_user_agent(value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        anyhow::bail!("user_agent cannot be empty");
    }
    if !value.is_ascii() || value.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        anyhow::bail!("user_agent must contain only visible ASCII characters");
    }
    Ok(())
}

/// Validate an HTTP header name using the RFC token character set.
pub fn validate_header_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty() || name.trim() != name {
        anyhow::bail!("header name cannot be empty or contain surrounding whitespace");
    }
    if !name.bytes().all(is_header_name_byte) {
        anyhow::bail!("header name must contain only valid HTTP token characters");
    }
    Ok(())
}

/// Validate a provider-configured HTTP header value.
pub fn validate_header_value(value: &str) -> anyhow::Result<()> {
    if !value.is_ascii() || value.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
        anyhow::bail!("header value must contain only visible ASCII characters");
    }
    Ok(())
}

/// Validate static and session-derived provider request headers.
pub fn validate_headers(
    headers: &BTreeMap<String, String>,
    session_header: Option<&str>,
) -> anyhow::Result<()> {
    let mut names = Vec::with_capacity(headers.len());
    for (name, value) in headers {
        validate_header_name(name)
            .map_err(|error| anyhow::anyhow!("invalid header name '{name}': {error}"))?;
        validate_header_value(value)
            .map_err(|error| anyhow::anyhow!("invalid value for header '{name}': {error}"))?;
        if is_reserved_header_name(name) {
            anyhow::bail!("header '{name}' is managed by the provider protocol");
        }
        if names
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(name))
        {
            anyhow::bail!("duplicate header names are not allowed: '{name}'");
        }
        names.push(name.clone());
    }

    if let Some(name) = session_header {
        validate_header_name(name)
            .map_err(|error| anyhow::anyhow!("invalid session_header '{name}': {error}"))?;
        if is_reserved_header_name(name) {
            anyhow::bail!("session_header '{name}' is managed by the provider protocol");
        }
    }
    Ok(())
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
        )
}

fn is_reserved_header_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization"
            | "content-length"
            | "content-type"
            | "connection"
            | "host"
            | "transfer-encoding"
            | "user-agent"
            | "x-api-key"
            | "x-goog-api-key"
            | "anthropic-version"
            | "anthropic-beta"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSource {
    User,
    Bundled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_type: Option<String>,
    /// Optional HTTP User-Agent override for all models under this provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    /// Static HTTP headers applied to every request for this provider.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Header name that receives the current conversation UUID on each request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_header: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
}

impl ProviderConfig {
    /// Resolve the effective [`ApiType`] for a given model using cascade
    /// precedence: model-level → provider-level → default.
    pub fn resolve_api_type(&self, model: &ModelConfig) -> ApiType {
        model
            .api_type
            .as_deref()
            .or(self.api_type.as_deref())
            .map(ApiType::parse)
            .unwrap_or_default()
    }

    /// Resolve the effective base URL for a given model using cascade
    /// precedence: model-level → provider-level.
    pub fn resolve_base_url(&self, model: &ModelConfig) -> String {
        model
            .base_url
            .clone()
            .or_else(|| Some(self.base_url.clone()))
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    /// Per-model API type override.
    #[serde(default)]
    pub api_type: Option<String>,
    /// Per-model base URL override.
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_images: bool,
    /// Whether the model supports multiple tool calls in one response.
    #[serde(default = "default_true")]
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
    #[serde(default)]
    pub request_model_id: Option<String>,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn model(api_type: Option<String>, base_url: Option<String>) -> ModelConfig {
        ModelConfig {
            display_name: "test".into(),
            context_window: 100_000,
            max_output_tokens: 8_000,
            api_type,
            base_url,
            temperature: None,
            system_prompt: None,
            supports_streaming: true,
            supports_images: false,
            supports_parallel_tool_calls: true,
            extra_body: None,
            request_model_id: None,
        }
    }

    fn provider(api_type: Option<String>, base_url: String) -> ProviderConfig {
        ProviderConfig {
            display_name: "Test".into(),
            base_url,
            api_type,
            user_agent: None,
            headers: BTreeMap::new(),
            session_header: None,
            models: BTreeMap::new(),
        }
    }

    // ── resolve_api_type ────────────────────────────────────────────────

    #[test]
    fn resolve_api_type_model_overrides_provider() {
        let provider = provider(Some("anthropic".into()), "https://api.anthropic.com".into());
        let model = model(Some("openai_chat_completions".into()), None);
        assert_eq!(
            provider.resolve_api_type(&model),
            ApiType::OpenAiChatCompletions
        );
    }

    #[test]
    fn resolve_api_type_falls_back_to_provider() {
        let provider = provider(Some("anthropic".into()), "https://api.anthropic.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_api_type(&model), ApiType::Anthropic);
    }

    #[test]
    fn resolve_api_type_default_when_both_none() {
        let provider = provider(None, "https://api.test.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_api_type(&model), ApiType::default());
    }

    // ── resolve_base_url ────────────────────────────────────────────────

    #[test]
    fn resolve_base_url_model_overrides_provider() {
        let provider = provider(None, "https://api.default.com".into());
        let model = model(None, Some("https://api.custom.com".into()));
        assert_eq!(provider.resolve_base_url(&model), "https://api.custom.com");
    }

    #[test]
    fn resolve_base_url_falls_back_to_provider() {
        let provider = provider(None, "https://api.default.com".into());
        let model = model(None, None);
        assert_eq!(provider.resolve_base_url(&model), "https://api.default.com");
    }

    #[test]
    fn validate_headers_accepts_custom_and_session_headers() {
        let headers = BTreeMap::from([(String::from("x-tenant-id"), String::from("team-a"))]);
        validate_headers(&headers, Some("x-opencode-session"))
            .expect("custom provider headers should be accepted");
    }

    #[test]
    fn validate_headers_rejects_protocol_owned_headers() {
        let headers = BTreeMap::from([(String::from("Authorization"), String::from("secret"))]);
        let error = validate_headers(&headers, None).expect_err("Authorization is provider-owned");
        assert!(
            error
                .to_string()
                .contains("managed by the provider protocol")
        );
    }

    #[test]
    fn validate_headers_rejects_invalid_session_header_name() {
        let error = validate_headers(&BTreeMap::new(), Some("x invalid"))
            .expect_err("invalid session header names should be rejected");
        assert!(error.to_string().contains("invalid session_header"));
    }
}
