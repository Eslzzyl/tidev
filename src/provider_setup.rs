use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;

use crate::config::{ModelConfig, ProviderConfig};

#[derive(Clone, Debug)]
pub enum ConnectDialog {
    ProviderPicker {
        selected: usize,
    },
    ApiKey {
        provider_id: String,
    },
    NewProvider {
        step: NewProviderStep,
        draft: NewProviderDraft,
    },
}

impl ConnectDialog {
    pub fn provider_picker() -> Self {
        Self::ProviderPicker { selected: 0 }
    }

    pub fn api_key(provider_id: impl Into<String>) -> Self {
        Self::ApiKey {
            provider_id: provider_id.into(),
        }
    }

    pub fn new_provider() -> Self {
        Self::NewProvider {
            step: NewProviderStep::ProviderId,
            draft: NewProviderDraft::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewProviderStep {
    ProviderId,
    DisplayName,
    BaseUrl,
    ApiKeyEnv,
    ModelId,
    ModelDisplayName,
    ContextWindow,
    MaxOutputTokens,
    Temperature,
}

impl NewProviderStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::ProviderId => "Provider id",
            Self::DisplayName => "Display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKeyEnv => "API key env",
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::ProviderId => Some(Self::DisplayName),
            Self::DisplayName => Some(Self::BaseUrl),
            Self::BaseUrl => Some(Self::ApiKeyEnv),
            Self::ApiKeyEnv => Some(Self::ModelId),
            Self::ModelId => Some(Self::ModelDisplayName),
            Self::ModelDisplayName => Some(Self::ContextWindow),
            Self::ContextWindow => Some(Self::MaxOutputTokens),
            Self::MaxOutputTokens => Some(Self::Temperature),
            Self::Temperature => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderId => "Provider id",
            Self::DisplayName => "Provider display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKeyEnv => "API key env var",
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::ProviderId => "local-openai",
            Self::DisplayName => "Local OpenAI",
            Self::BaseUrl => "https://api.openai.com/v1",
            Self::ApiKeyEnv => "OPENAI_API_KEY or none",
            Self::ModelId => "gpt-4o-mini",
            Self::ModelDisplayName => "GPT-4o mini",
            Self::ContextWindow => "128000",
            Self::MaxOutputTokens => "2048",
            Self::Temperature => "0.7",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::ProviderId => "Use lowercase letters, numbers, '-', or '_' only.",
            Self::DisplayName => "Shown in the TUI and session metadata.",
            Self::BaseUrl => "Use an OpenAI-compatible chat completions endpoint.",
            Self::ApiKeyEnv => "Type 'none' to skip env lookup and use auth.json only.",
            Self::ModelId => "The exact model id the provider expects.",
            Self::ModelDisplayName => "A friendly name for the model in the UI.",
            Self::ContextWindow => "Total token budget for the model context.",
            Self::MaxOutputTokens => "Maximum tokens the model may generate per turn.",
            Self::Temperature => "Usually 0.0 to 1.0 for deterministic coding help.",
        }
    }
}

#[derive(Clone, Debug)]
pub struct NewProviderDraft {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    pub api_key_env: Option<String>,
    pub model_id: String,
    pub model_display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
}

impl Default for NewProviderDraft {
    fn default() -> Self {
        Self {
            provider_id: "local-openai".to_string(),
            display_name: "Local OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key_env: Some("OPENAI_API_KEY".to_string()),
            model_id: "gpt-4o-mini".to_string(),
            model_display_name: "GPT-4o mini".to_string(),
            context_window: 128_000,
            max_output_tokens: 2_048,
            temperature: 0.7,
        }
    }
}

impl NewProviderDraft {
    pub fn current_value(&self, step: NewProviderStep) -> String {
        match step {
            NewProviderStep::ProviderId => self.provider_id.clone(),
            NewProviderStep::DisplayName => self.display_name.clone(),
            NewProviderStep::BaseUrl => self.base_url.clone(),
            NewProviderStep::ApiKeyEnv => self
                .api_key_env
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            NewProviderStep::ModelId => self.model_id.clone(),
            NewProviderStep::ModelDisplayName => self.model_display_name.clone(),
            NewProviderStep::ContextWindow => self.context_window.to_string(),
            NewProviderStep::MaxOutputTokens => self.max_output_tokens.to_string(),
            NewProviderStep::Temperature => self.temperature.to_string(),
        }
    }

    pub fn apply_step(&mut self, step: NewProviderStep, input: &str) -> Result<()> {
        let value = input.trim();

        match step {
            NewProviderStep::ProviderId => {
                self.provider_id = normalize_identifier(value, "provider id")?;
            }
            NewProviderStep::DisplayName => {
                self.display_name = non_empty(value, "provider display name")?.to_string();
            }
            NewProviderStep::BaseUrl => {
                self.base_url = normalize_base_url(value)?;
            }
            NewProviderStep::ApiKeyEnv => {
                self.api_key_env = normalize_optional_env(value)?;
            }
            NewProviderStep::ModelId => {
                self.model_id = normalize_identifier(value, "model id")?;
            }
            NewProviderStep::ModelDisplayName => {
                self.model_display_name = non_empty(value, "model display name")?.to_string();
            }
            NewProviderStep::ContextWindow => {
                self.context_window = parse_usize(value, "context window")?;
            }
            NewProviderStep::MaxOutputTokens => {
                self.max_output_tokens = parse_usize(value, "max output tokens")?;
            }
            NewProviderStep::Temperature => {
                self.temperature = parse_temperature(value)?;
            }
        }

        Ok(())
    }

    pub fn into_provider_config(self) -> (String, ProviderConfig) {
        let provider_id = self.provider_id;
        let mut models = BTreeMap::new();
        models.insert(
            self.model_id.clone(),
            ModelConfig {
                display_name: self.model_display_name,
                context_window: self.context_window,
                max_output_tokens: self.max_output_tokens,
                temperature: self.temperature,
                system_prompt: None,
                supports_streaming: true,
            },
        );

        (
            provider_id,
            ProviderConfig {
                display_name: self.display_name,
                base_url: self.base_url,
                api_key_env: self.api_key_env,
                models,
            },
        )
    }
}

fn non_empty<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }

    Ok(value)
}

fn normalize_identifier(value: &str, label: &str) -> Result<String> {
    let normalized = value
        .trim()
        .to_ascii_lowercase()
        .replace(' ', "-")
        .replace('.', "-");

    if normalized.is_empty() {
        bail!("{label} cannot be empty");
    }

    if normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_'))
    {
        Ok(normalized)
    } else {
        bail!("{label} may only contain lowercase letters, numbers, '-' or '_'");
    }
}

fn normalize_base_url(value: &str) -> Result<String> {
    let value = value.trim().trim_end_matches('/');

    if value.is_empty() {
        bail!("base URL cannot be empty");
    }

    if !(value.starts_with("http://") || value.starts_with("https://")) {
        bail!("base URL must start with http:// or https://");
    }

    Ok(value.to_string())
}

fn normalize_optional_env(value: &str) -> Result<Option<String>> {
    let value = value.trim();

    if value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "none" | "off" | "null" | "-"
        )
    {
        return Ok(None);
    }

    let normalized = value
        .to_ascii_uppercase()
        .replace('-', "_")
        .replace(' ', "_");

    if normalized
        .chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
    {
        Ok(Some(normalized))
    } else {
        bail!("API key env var may only contain uppercase letters, numbers, and '_'");
    }
}

fn parse_usize(value: &str, label: &str) -> Result<usize> {
    value
        .parse::<usize>()
        .with_context(|| format!("{label} must be a positive integer"))
}

fn parse_temperature(value: &str) -> Result<f32> {
    let temperature = value
        .parse::<f32>()
        .with_context(|| "temperature must be a number")?;

    if !(0.0..=2.0).contains(&temperature) {
        bail!("temperature should usually be between 0.0 and 2.0");
    }

    Ok(temperature)
}
