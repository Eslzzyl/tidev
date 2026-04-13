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
    EditProvider {
        provider_id: String,
        step: EditProviderStep,
        model_step: Option<EditModelStep>,
        draft: EditProviderDraft,
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
}

impl EditModelStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ModelId => "Model id",
            Self::ModelDisplayName => "Model display name",
            Self::ContextWindow => "Context window",
            Self::MaxOutputTokens => "Max output tokens",
            Self::Temperature => "Temperature",
        }
    }

    pub fn placeholder(self) -> &'static str {
        match self {
            Self::ModelId => "model id",
            Self::ModelDisplayName => "model display name",
            Self::ContextWindow => "128000",
            Self::MaxOutputTokens => "32768",
            Self::Temperature => "0.7",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::ModelId => "The exact model id the provider expects.",
            Self::ModelDisplayName => "Shown in the TUI and session metadata.",
            Self::ContextWindow => "Total token budget for the model context.",
            Self::MaxOutputTokens => "Maximum tokens the model may generate per turn.",
            Self::Temperature => "Usually 0.0 to 1.0 for deterministic coding help.",
        }
    }

    pub fn next(self, editing_existing: bool) -> Option<Self> {
        match self {
            Self::ModelId if editing_existing => Some(Self::ModelDisplayName),
            Self::ModelId => Some(Self::ModelDisplayName),
            Self::ModelDisplayName => Some(Self::ContextWindow),
            Self::ContextWindow => Some(Self::MaxOutputTokens),
            Self::MaxOutputTokens => Some(Self::Temperature),
            Self::Temperature => None,
        }
    }

    pub fn is_secret(self) -> bool {
        false
    }
}

#[derive(Clone, Debug)]
pub struct NewModelDraft {
    pub model_id: String,
    pub model_display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
}

impl Default for NewModelDraft {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            model_display_name: String::new(),
            context_window: 128_000,
            max_output_tokens: 32_768,
            temperature: 0.7,
        }
    }
}

impl NewModelDraft {
    pub fn from_model(model_id: impl Into<String>, model: &ModelConfig) -> Self {
        Self {
            model_id: model_id.into(),
            model_display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
        }
    }

    fn current_value(&self, step: NewProviderStep) -> String {
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

    pub fn current_value_for_edit(&self, step: EditModelStep) -> String {
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

    fn apply_step(&mut self, step: NewProviderStep, input: &str) -> Result<()> {
        let value = input.trim();

        match step {
            NewProviderStep::ModelId => {
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

    pub fn apply_edit_step(&mut self, step: EditModelStep, input: &str) -> Result<()> {
        let value = input.trim();

        match step {
            EditModelStep::ModelId => {
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

    fn into_model_config(self) -> (String, ModelConfig) {
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
            },
        )
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

    pub fn apply_step(&mut self, step: EditProviderStep, input: &str) -> Result<()> {
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

    pub fn begin_edit_model(&mut self, model_id: &str) -> Result<()> {
        let model = self
            .models
            .get(model_id)
            .with_context(|| format!("unknown model '{model_id}'"))?;

        self.model = NewModelDraft::from_model(model_id.to_string(), model);
        self.editing_model_id = Some(model_id.to_string());
        Ok(())
    }

    pub fn finish_current_model(&mut self) -> Result<()> {
        let editing_model_id = self.editing_model_id.clone();
        let (model_id, model_config) = self.model.clone().into_model_config();
        let model_id = editing_model_id.unwrap_or(model_id);

        if model_id.trim().is_empty() {
            bail!("model id cannot be empty");
        }

        if self.editing_model_id.is_none() && self.models.contains_key(&model_id) {
            bail!("model '{model_id}' already exists");
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

    pub fn request_delete_selected_model(&mut self) -> Result<String> {
        let model_id = self
            .selected_model_id()
            .with_context(|| "no model selected")?;
        self.pending_delete_model_id = Some(model_id.clone());
        Ok(model_id)
    }

    pub fn confirm_delete_selected_model(&mut self) -> Result<String> {
        let Some(model_id) = self.pending_delete_model_id.take() else {
            bail!("no pending model deletion");
        };

        if self.models.len() <= 1 {
            bail!("at least one model must remain");
        }

        self.models.remove(&model_id);

        if self.selected_model_index >= self.models.len() {
            self.selected_model_index = self.models.len().saturating_sub(1);
        }

        Ok(model_id)
    }

    pub fn into_provider_config(self) -> Result<(ProviderConfig, String)> {
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
            bail!("API key cannot be empty");
        }

        Ok((
            ProviderConfig {
                display_name,
                base_url: self.base_url,
                models: self.models,
            },
            api_key,
        ))
    }
}

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

    pub fn apply_step(&mut self, step: NewProviderStep, input: &str) -> Result<()> {
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

    pub fn finish_current_model(&mut self) -> Result<()> {
        let (model_id, model_config) = self.model.clone().into_model_config();

        if model_id.trim().is_empty() {
            bail!("model id cannot be empty");
        }

        if self.models.contains_key(&model_id) {
            bail!("model '{model_id}' already exists");
        }

        self.models.insert(model_id, model_config);
        self.model = NewModelDraft::default();
        Ok(())
    }

    pub fn into_provider_config(self) -> Result<(String, ProviderConfig, String)> {
        if self.provider_id.trim().is_empty() {
            bail!("provider id cannot be empty");
        }

        if self.models.is_empty() {
            bail!("at least one model must be configured");
        }

        let provider_id = self.provider_id;
        let display_name = if self.display_name.is_empty() {
            provider_id.clone()
        } else {
            self.display_name
        };

        Ok((
            provider_id,
            ProviderConfig {
                display_name,
                base_url: self.base_url,
                models: self.models,
            },
            self.api_key,
        ))
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
