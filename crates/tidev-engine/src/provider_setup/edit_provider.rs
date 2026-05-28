use std::collections::BTreeMap;

use crate::config::{ModelConfig, ProviderConfig};

use super::NewModelDraft;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditProviderStep {
    DisplayName,
    BaseUrl,
    ApiKey,
    ModelList,
    ConfirmDeleteModel,
}

impl EditProviderStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::DisplayName => "Display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
            Self::ModelList => "Models",
            Self::ConfirmDeleteModel => "Delete model",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::DisplayName => "Provider display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
            Self::ModelList => "Models",
            Self::ConfirmDeleteModel => "Delete model",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::DisplayName => "provider display name",
            Self::BaseUrl => "https://api.openai.com/v1",
            Self::ApiKey => "Leave blank to keep the current key",
            Self::ModelList => "Use Enter / n / d / s",
            Self::ConfirmDeleteModel => "y or n",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::DisplayName => "Shown in the TUI and session metadata.",
            Self::BaseUrl => "Use an OpenAI-compatible chat completions endpoint.",
            Self::ApiKey => "Leave blank to keep the existing key in auth.json.",
            Self::ModelList => "Enter edits, n adds, d deletes, s saves.",
            Self::ConfirmDeleteModel => "Press y to delete, or n / Esc to keep it.",
        }
    }

    pub fn next(self) -> Option<Self> {
        match self {
            Self::DisplayName => Some(Self::BaseUrl),
            Self::BaseUrl => Some(Self::ApiKey),
            Self::ApiKey => Some(Self::ModelList),
            Self::ModelList | Self::ConfirmDeleteModel => None,
        }
    }

    pub fn is_secret(self) -> bool {
        matches!(self, Self::ApiKey)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditModelStep {
    ModelId,
    ModelDisplayName,
    ContextWindow,
    MaxOutputTokens,
    Temperature,
    ModelApiType,
    ModelBaseUrl,
}

impl EditModelStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
            Self::ModelApiType => "API type",
            Self::ModelBaseUrl => "Base URL",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
            Self::ModelApiType => "API type",
            Self::ModelBaseUrl => "Base URL",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::ModelId => "model id",
            Self::ModelDisplayName => "model display name",
            Self::ContextWindow => "128000",
            Self::MaxOutputTokens => "32768",
            Self::Temperature => "0.7",
            Self::ModelApiType => "openai_chat_completions | anthropic | openai_responses | google_gemini",
            Self::ModelBaseUrl => "https://api.openai.com/v1",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::ModelId => "The exact model id the provider expects.",
            Self::ModelDisplayName => "Shown in the TUI and session metadata.",
            Self::ContextWindow => "Total token budget for the model context.",
            Self::MaxOutputTokens => "Maximum tokens the model may generate per turn.",
            Self::Temperature => "Usually 0.0 to 1.0 for deterministic coding help.",
            Self::ModelApiType => "Leave blank to inherit from provider. Override per-model if needed.",
            Self::ModelBaseUrl => "Leave blank to inherit from provider. Override per-model if needed.",
        }
    }

    /// Next step in the model editing wizard.
    /// `editing` is true when modifying an existing model (skips `ModelId`).
    pub fn next(self, _editing: bool) -> Option<Self> {
        match self {
            Self::ModelId => Some(Self::ModelDisplayName),
            Self::ModelDisplayName => Some(Self::ContextWindow),
            Self::ContextWindow => Some(Self::MaxOutputTokens),
            Self::MaxOutputTokens => Some(Self::Temperature),
            Self::Temperature => Some(Self::ModelApiType),
            Self::ModelApiType => Some(Self::ModelBaseUrl),
            Self::ModelBaseUrl => None,
        }
    }

    pub fn is_secret(self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
pub struct EditProviderDraft {
    pub display_name: String,
    pub base_url: String,
    pub api_key: String,
    pub existing_api_key: Option<String>,
    pub models: BTreeMap<String, ModelConfig>,
    pub selected_model_index: usize,
    pub model: NewModelDraft,
    pub editing_model_id: Option<String>,
    pub pending_delete_model_id: Option<String>,
}

impl EditProviderDraft {
    pub fn from_provider(provider: &ProviderConfig, api_key: Option<String>) -> Self {
        Self {
            display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            api_key: String::new(),
            existing_api_key: api_key,
            models: provider.models.clone(),
            selected_model_index: 0,
            model: NewModelDraft::default(),
            editing_model_id: None,
            pending_delete_model_id: None,
        }
    }

    pub fn current_value(&self, step: EditProviderStep) -> String {
        match step {
            EditProviderStep::DisplayName => self.display_name.clone(),
            EditProviderStep::BaseUrl => self.base_url.clone(),
            EditProviderStep::ApiKey => String::new(),
            EditProviderStep::ModelList | EditProviderStep::ConfirmDeleteModel => String::new(),
        }
    }

    pub fn apply_step(&mut self, step: EditProviderStep, input: &str) -> anyhow::Result<()> {
        use super::{non_empty, normalize_base_url};
        let value = input.trim();

        match step {
            EditProviderStep::DisplayName => {
                self.display_name = non_empty(value, "provider display name")?.to_string();
            }
            EditProviderStep::BaseUrl => {
                self.base_url = normalize_base_url(value)?;
            }
            EditProviderStep::ApiKey => {
                if !value.is_empty() {
                    self.api_key = value.to_string();
                }
            }
            EditProviderStep::ModelList | EditProviderStep::ConfirmDeleteModel => {}
        }

        Ok(())
    }

    pub fn selected_model_id(&self) -> Option<String> {
        self.models.keys().nth(self.selected_model_index).cloned()
    }

    pub fn selected_model_config(&self) -> Option<(String, ModelConfig)> {
        self.selected_model_id().and_then(|model_id| {
            self.models
                .get(&model_id)
                .cloned()
                .map(|model| (model_id, model))
        })
    }

    pub fn move_selection_up(&mut self) {
        let count = self.models.len();
        if count == 0 {
            self.selected_model_index = 0;
        } else if self.selected_model_index == 0 {
            self.selected_model_index = count.saturating_sub(1);
        } else {
            self.selected_model_index -= 1;
        }
    }

    pub fn move_selection_down(&mut self) {
        let count = self.models.len();
        if count == 0 {
            self.selected_model_index = 0;
        } else {
            self.selected_model_index = (self.selected_model_index + 1) % count;
        }
    }

    pub fn begin_new_model(&mut self) {
        self.model = NewModelDraft::default();
        self.editing_model_id = None;
    }

    pub fn begin_edit_model(&mut self, model_id: &str) -> anyhow::Result<()> {
        let model = self
            .models
            .get(model_id)
            .ok_or_else(|| anyhow::anyhow!("unknown model '{model_id}'"))?;

        self.model = NewModelDraft::from_model(model_id.to_string(), model);
        self.editing_model_id = Some(model_id.to_string());
        Ok(())
    }

    pub fn finish_current_model(&mut self) -> anyhow::Result<()> {
        let editing_model_id = self.editing_model_id.clone();
        let (model_id, model_config) = self.model.clone().into_model_config();
        let model_id = editing_model_id.unwrap_or(model_id);

        if model_id.trim().is_empty() {
            anyhow::bail!("model id cannot be empty");
        }

        if self.editing_model_id.is_none() && self.models.contains_key(&model_id) {
            anyhow::bail!("model '{model_id}' already exists");
        }

        self.models.insert(model_id.clone(), model_config);
        self.selected_model_index = self
            .models
            .keys()
            .position(|candidate| candidate == &model_id)
            .unwrap_or(0);
        self.model = NewModelDraft::default();
        self.editing_model_id = None;
        Ok(())
    }

    pub fn request_delete_selected_model(&mut self) -> anyhow::Result<String> {
        let model_id = self
            .selected_model_id()
            .ok_or_else(|| anyhow::anyhow!("no model selected"))?;
        self.pending_delete_model_id = Some(model_id.clone());
        Ok(model_id)
    }

    pub fn confirm_delete_selected_model(&mut self) -> anyhow::Result<String> {
        let Some(model_id) = self.pending_delete_model_id.take() else {
            anyhow::bail!("no pending model deletion");
        };

        if self.models.len() <= 1 {
            anyhow::bail!("at least one model must remain");
        }

        self.models.remove(&model_id);

        if self.selected_model_index >= self.models.len() {
            self.selected_model_index = self.models.len().saturating_sub(1);
        }

        Ok(model_id)
    }

    pub fn into_provider_config(self) -> anyhow::Result<(ProviderConfig, String)> {
        let display_name = if self.display_name.is_empty() {
            "Unnamed provider".to_string()
        } else {
            self.display_name
        };

        let api_key = if self.api_key.trim().is_empty() {
            self.existing_api_key.unwrap_or_default()
        } else {
            self.api_key
        };

        if api_key.trim().is_empty() {
            anyhow::bail!("API key cannot be empty");
        }

        Ok((
            ProviderConfig {
                display_name,
                base_url: self.base_url,
                api_type: None,
                models: self.models,
            },
            api_key,
        ))
    }
}
