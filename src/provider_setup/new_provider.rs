use crate::config::ModelConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewProviderStep {
    ProviderId,
    DisplayName,
    BaseUrl,
    ApiKey,
    ModelId,
    ModelDisplayName,
    ContextWindow,
    MaxOutputTokens,
    Temperature,
    AddAnotherModel,
}

impl NewProviderStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::ProviderId => "Provider id",
            Self::DisplayName => "Display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
            Self::AddAnotherModel => "Add another model",
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::ProviderId => Some(Self::DisplayName),
            Self::DisplayName => Some(Self::BaseUrl),
            Self::BaseUrl => Some(Self::ApiKey),
            Self::ApiKey => Some(Self::ModelId),
            Self::ModelId => Some(Self::ModelDisplayName),
            Self::ModelDisplayName => Some(Self::ContextWindow),
            Self::ContextWindow => Some(Self::MaxOutputTokens),
            Self::MaxOutputTokens => Some(Self::Temperature),
            Self::Temperature => Some(Self::AddAnotherModel),
            Self::AddAnotherModel => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ProviderId => "Provider id",
            Self::DisplayName => "Provider display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
            Self::AddAnotherModel => "Add another model",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::ProviderId => "provider id",
            Self::DisplayName => "provider display name",
            Self::BaseUrl => "https://api.openai.com/v1",
            Self::ApiKey => "Paste the API key",
            Self::ModelId => "model id",
            Self::ModelDisplayName => "model display name",
            Self::ContextWindow => "128000",
            Self::MaxOutputTokens => "32768",
            Self::Temperature => "0.7",
            Self::AddAnotherModel => "y or n",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::ProviderId => "Use lowercase letters, numbers, '-', or '_' only.",
            Self::DisplayName => "Shown in the TUI and session metadata.",
            Self::BaseUrl => "Use an OpenAI-compatible chat completions endpoint.",
            Self::ApiKey => "Stored in auth.json only.",
            Self::ModelId => "The exact model id the provider expects.",
            Self::ModelDisplayName => "Shown in the TUI and session metadata.",
            Self::ContextWindow => "Total token budget for the model context.",
            Self::MaxOutputTokens => "Maximum tokens the model may generate per turn.",
            Self::Temperature => "Usually 0.0 to 1.0 for deterministic coding help.",
            Self::AddAnotherModel => "Press y to add another model, or Enter/n to finish.",
        }
    }

    pub fn is_secret(self) -> bool {
        matches!(self, Self::ApiKey)
    }
}

#[derive(Clone, Debug)]
pub struct NewModelDraft {
    pub model_id: String,
    pub request_model_id: String,
    pub model_display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
}

impl Default for NewModelDraft {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            request_model_id: String::new(),
            model_display_name: String::new(),
            context_window: 128_000,
            max_output_tokens: 32_768,
            temperature: 0.7,
        }
    }
}

impl NewModelDraft {
    pub fn from_model(model_id: impl Into<String>, model: &ModelConfig) -> Self {
        let model_id = model_id.into();
        Self {
            model_id: model_id.clone(),
            request_model_id: model.request_model_id.clone().unwrap_or(model_id),
            model_display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
        }
    }

    pub fn current_value(&self, step: NewProviderStep) -> String {
        match step {
            NewProviderStep::ModelId => self.model_id.clone(),
            NewProviderStep::ModelDisplayName => {
                if self.model_display_name.is_empty() {
                    self.model_id.clone()
                } else {
                    self.model_display_name.clone()
                }
            }
            NewProviderStep::ContextWindow => self.context_window.to_string(),
            NewProviderStep::MaxOutputTokens => self.max_output_tokens.to_string(),
            NewProviderStep::Temperature => self.temperature.to_string(),
            _ => String::new(),
        }
    }

    pub fn current_value_for_edit(&self, step: super::EditModelStep) -> String {
        use super::EditModelStep;
        match step {
            EditModelStep::ModelId => self.model_id.clone(),
            EditModelStep::ModelDisplayName => {
                if self.model_display_name.is_empty() {
                    self.model_id.clone()
                } else {
                    self.model_display_name.clone()
                }
            }
            EditModelStep::ContextWindow => self.context_window.to_string(),
            EditModelStep::MaxOutputTokens => self.max_output_tokens.to_string(),
            EditModelStep::Temperature => self.temperature.to_string(),
        }
    }

    pub fn apply_step(&mut self, step: NewProviderStep, input: &str) -> anyhow::Result<()> {
        use super::{non_empty, normalize_identifier, parse_temperature, parse_usize};
        let value = input.trim();

        match step {
            NewProviderStep::ModelId => {
                self.request_model_id = value.to_string();
                self.model_id = normalize_identifier(value, "model id")?;
                self.model_display_name = self.model_id.clone();
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
            _ => {}
        }

        Ok(())
    }

    pub fn apply_edit_step(
        &mut self,
        step: super::EditModelStep,
        input: &str,
    ) -> anyhow::Result<()> {
        use super::EditModelStep;
        use super::{non_empty, normalize_identifier, parse_temperature, parse_usize};
        let value = input.trim();

        match step {
            EditModelStep::ModelId => {
                self.request_model_id = value.to_string();
                self.model_id = normalize_identifier(value, "model id")?;
                self.model_display_name = self.model_id.clone();
            }
            EditModelStep::ModelDisplayName => {
                self.model_display_name = non_empty(value, "model display name")?.to_string();
            }
            EditModelStep::ContextWindow => {
                self.context_window = parse_usize(value, "context window")?;
            }
            EditModelStep::MaxOutputTokens => {
                self.max_output_tokens = parse_usize(value, "max output tokens")?;
            }
            EditModelStep::Temperature => {
                self.temperature = parse_temperature(value)?;
            }
        }

        Ok(())
    }

    pub fn into_model_config(self) -> (String, ModelConfig) {
        let model_id = self.model_id;

        (
            model_id.clone(),
            ModelConfig {
                display_name: self.model_display_name,
                context_window: self.context_window,
                max_output_tokens: self.max_output_tokens,
                temperature: self.temperature,
                system_prompt: None,
                supports_streaming: true,
                supports_images: false,
                extra_body: None,
                request_model_id: Some(self.request_model_id),
            },
        )
    }
}

use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct NewProviderDraft {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: NewModelDraft,
    pub models: BTreeMap<String, ModelConfig>,
}

impl Default for NewProviderDraft {
    fn default() -> Self {
        Self {
            provider_id: String::new(),
            display_name: String::new(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_key: String::new(),
            model: NewModelDraft::default(),
            models: BTreeMap::new(),
        }
    }
}

impl NewProviderDraft {
    pub fn current_value(&self, step: NewProviderStep) -> String {
        match step {
            NewProviderStep::ProviderId => self.provider_id.clone(),
            NewProviderStep::DisplayName => {
                if self.display_name.is_empty() {
                    self.provider_id.clone()
                } else {
                    self.display_name.clone()
                }
            }
            NewProviderStep::BaseUrl => self.base_url.clone(),
            NewProviderStep::ApiKey => String::new(),
            NewProviderStep::ModelId
            | NewProviderStep::ModelDisplayName
            | NewProviderStep::ContextWindow
            | NewProviderStep::MaxOutputTokens
            | NewProviderStep::Temperature => self.model.current_value(step),
            NewProviderStep::AddAnotherModel => String::new(),
        }
    }

    pub fn apply_step(&mut self, step: NewProviderStep, input: &str) -> anyhow::Result<()> {
        use super::{non_empty, normalize_base_url, normalize_identifier};
        let value = input.trim();

        match step {
            NewProviderStep::ProviderId => {
                self.provider_id = normalize_identifier(value, "provider id")?;
                self.display_name = self.provider_id.clone();
            }
            NewProviderStep::DisplayName => {
                self.display_name = non_empty(value, "provider display name")?.to_string();
            }
            NewProviderStep::BaseUrl => {
                self.base_url = normalize_base_url(value)?;
            }
            NewProviderStep::ApiKey => {
                self.api_key = non_empty(value, "API key")?.to_string();
            }
            NewProviderStep::ModelId
            | NewProviderStep::ModelDisplayName
            | NewProviderStep::ContextWindow
            | NewProviderStep::MaxOutputTokens
            | NewProviderStep::Temperature => self.model.apply_step(step, value)?,
            NewProviderStep::AddAnotherModel => {}
        }

        Ok(())
    }

    pub fn finish_current_model(&mut self) -> anyhow::Result<()> {
        let (model_id, model_config) = self.model.clone().into_model_config();

        if model_id.trim().is_empty() {
            anyhow::bail!("model id cannot be empty");
        }

        if self.models.contains_key(&model_id) {
            anyhow::bail!("model '{model_id}' already exists");
        }

        self.models.insert(model_id, model_config);
        self.model = NewModelDraft::default();
        Ok(())
    }

    pub fn into_provider_config(
        self,
    ) -> anyhow::Result<(String, crate::config::ProviderConfig, String)> {
        if self.provider_id.trim().is_empty() {
            anyhow::bail!("provider id cannot be empty");
        }

        if self.models.is_empty() {
            anyhow::bail!("at least one model must be configured");
        }

        let provider_id = self.provider_id;
        let display_name = if self.display_name.is_empty() {
            provider_id.clone()
        } else {
            self.display_name
        };

        Ok((
            provider_id,
            crate::config::ProviderConfig {
                display_name,
                base_url: self.base_url,
                api_type: None,
                models: self.models,
            },
            self.api_key,
        ))
    }
}
