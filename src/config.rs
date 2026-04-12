use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use crate::prompts::{default_system_prompt, resolve_system_prompt};
use crate::theme::ThemeName;

#[derive(Clone, Debug)]
pub struct ConfigPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub auth_file: PathBuf,
    pub database_file: PathBuf,
}

impl ConfigPaths {
    pub fn discover() -> Result<Self> {
        let home_dir = dirs::home_dir().context("unable to determine the home directory")?;

        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| home_dir.join(".config"))
            .join("tidev");
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| home_dir.join(".local/share"))
            .join("tidev");

        Ok(Self {
            config_file: config_dir.join("config.toml"),
            auth_file: data_dir.join("auth.json"),
            database_file: data_dir.join("sessions.sqlite3"),
            config_dir,
            data_dir,
        })
    }

    pub fn ensure_directories(&self) -> Result<()> {
        fs::create_dir_all(&self.config_dir).with_context(|| {
            format!(
                "failed to create config directory {}",
                self.config_dir.display()
            )
        })?;
        fs::create_dir_all(&self.data_dir).with_context(|| {
            format!(
                "failed to create data directory {}",
                self.data_dir.display()
            )
        })?;
        Ok(())
    }

    pub fn default_config_path(&self) -> &Path {
        &self.config_file
    }

    pub fn default_auth_path(&self) -> &Path {
        &self.auth_file
    }

    pub fn default_database_path(&self) -> &Path {
        &self.database_file
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_provider: String,
    pub default_model: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        let mut models = BTreeMap::new();
        models.insert(
            "gpt-4o-mini".to_string(),
            ModelConfig {
                display_name: "GPT-4o mini".to_string(),
                context_window: 128_000,
                max_output_tokens: 2_048,
                temperature: 0.7,
                system_prompt: None,
                system_prompt_preset: Some("tidev_default".to_string()),
                supports_streaming: true,
            },
        );

        let mut providers = BTreeMap::new();
        providers.insert(
            "openai".to_string(),
            ProviderConfig {
                display_name: "OpenAI Compatible".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                api_key_env: Some("OPENAI_API_KEY".to_string()),
                models,
            },
        );

        Self {
            default_provider: "openai".to_string(),
            default_model: "gpt-4o-mini".to_string(),
            theme: ThemeName::Dark.as_str().to_string(),
            ui: UiConfig::default(),
            providers,
        }
    }
}

impl AppConfig {
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        if !paths.config_file.exists() {
            let example = Self::example_toml();
            fs::write(&paths.config_file, example)
                .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
            return toml::from_str(example).with_context(|| {
                format!("failed to parse generated {}", paths.config_file.display())
            });
        }

        let contents = fs::read_to_string(&paths.config_file)
            .with_context(|| format!("failed to read {}", paths.config_file.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.config_file.display()))?;
        Ok(config)
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(&paths.config_file, contents)
            .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
        Ok(())
    }

    pub fn example_toml() -> &'static str {
        r#"# TiDev configuration
# `theme` can be `dark` or `light`.
theme = "dark"
default_provider = "openai"
default_model = "gpt-4o-mini"

[ui]
sidebar_width = 30
welcome_width = 72
max_input_lines = 6

[providers.openai]
display_name = "OpenAI Compatible"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.models.gpt-4o-mini]
display_name = "GPT-4o mini"
context_window = 128000
max_output_tokens = 2048
temperature = 0.7
supports_streaming = true
# Use either `system_prompt` for custom text or `system_prompt_preset` for a built-in template.
system_prompt_preset = "tidev_default"
"#
    }

    pub fn available_models(&self) -> Vec<ModelSummary> {
        let mut models = Vec::new();

        for (provider_id, provider) in &self.providers {
            for (model_id, model) in &provider.models {
                models.push(ModelSummary {
                    provider_id: provider_id.clone(),
                    provider_display_name: provider.display_name.clone(),
                    model_id: model_id.clone(),
                    model_display_name: model.display_name.clone(),
                    base_url: provider.base_url.clone(),
                    context_window: model.context_window,
                    max_output_tokens: model.max_output_tokens,
                });
            }
        }

        models
    }

    pub fn resolve_active_model(&self, auth: &AuthStore) -> Result<ActiveModel> {
        self.resolve_model(auth, None)
    }

    pub fn resolve_provider_default_model(
        &self,
        auth: &AuthStore,
        provider_id: &str,
    ) -> Result<ActiveModel> {
        let provider = self
            .providers
            .get(provider_id)
            .with_context(|| format!("unknown provider '{provider_id}'"))?;
        let model_id = provider
            .models
            .keys()
            .next()
            .cloned()
            .with_context(|| format!("provider '{provider_id}' has no configured models"))?;

        self.resolve_model_by_ids(auth, provider_id, &model_id)
    }

    pub fn resolve_model(&self, auth: &AuthStore, query: Option<&str>) -> Result<ActiveModel> {
        let (provider_id, model_id) = match query.map(str::trim).filter(|value| !value.is_empty()) {
            Some(query) => self.resolve_model_key(query)?,
            None => (self.default_provider.clone(), self.default_model.clone()),
        };

        self.resolve_model_by_ids(auth, &provider_id, &model_id)
    }

    pub fn resolve_model_by_ids(
        &self,
        auth: &AuthStore,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ActiveModel> {
        let provider = self
            .providers
            .get(provider_id)
            .with_context(|| format!("unknown provider '{provider_id}'"))?;
        let model = provider
            .models
            .get(model_id)
            .with_context(|| format!("unknown model '{model_id}' for provider '{provider_id}'"))?;

        let api_key = self.resolve_api_key(auth, provider_id, provider);

        Ok(ActiveModel {
            provider_id: provider_id.to_string(),
            provider_display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            model_id: model_id.to_string(),
            display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
            system_prompt: model
                .system_prompt
                .clone()
                .filter(|prompt| !prompt.trim().is_empty())
                .or_else(|| resolve_system_prompt(model.system_prompt_preset.as_deref()))
                .unwrap_or_else(default_system_prompt),
            api_key,
        })
    }

    pub fn default_model_summary(&self) -> Result<ModelSummary> {
        let provider = self
            .providers
            .get(&self.default_provider)
            .with_context(|| format!("unknown default provider '{}'", self.default_provider))?;
        let model = provider
            .models
            .get(&self.default_model)
            .with_context(|| format!("unknown default model '{}'", self.default_model))?;

        Ok(ModelSummary {
            provider_id: self.default_provider.clone(),
            provider_display_name: provider.display_name.clone(),
            model_id: self.default_model.clone(),
            model_display_name: model.display_name.clone(),
            base_url: provider.base_url.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
        })
    }

    fn resolve_model_key(&self, query: &str) -> Result<(String, String)> {
        if let Some((provider, model)) = query.split_once(':').or_else(|| query.split_once('/')) {
            let provider = provider.trim();
            let model = model.trim();

            if provider.is_empty() || model.is_empty() {
                bail!("model selector '{query}' must be in provider:model or provider/model form");
            }

            return Ok((provider.to_string(), model.to_string()));
        }

        let mut matches = Vec::new();

        for (provider_id, provider) in &self.providers {
            if provider.models.contains_key(query) {
                matches.push((provider_id.clone(), query.to_string()));
            }
        }

        match matches.len() {
            0 => bail!("unknown model '{query}'"),
            1 => Ok(matches.remove(0)),
            _ => {
                let choices = matches
                    .into_iter()
                    .map(|(provider_id, model_id)| format!("{provider_id}:{model_id}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!("model '{query}' is ambiguous; use one of: {choices}");
            }
        }
    }

    fn resolve_api_key(
        &self,
        auth: &AuthStore,
        provider_id: &str,
        provider: &ProviderConfig,
    ) -> Option<String> {
        if let Some(env_key) = provider
            .api_key_env
            .as_ref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.trim().is_empty())
        {
            return Some(env_key);
        }

        auth.providers
            .get(provider_id)
            .and_then(|entry| entry.api_key.clone())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    pub fn provider_display_name(&self, provider_id: &str) -> Option<&str> {
        self.providers
            .get(provider_id)
            .map(|provider| provider.display_name.as_str())
    }

    pub fn set_theme(&mut self, theme: ThemeName) {
        self.theme = theme.as_str().to_string();
    }

    pub fn theme_name(&self) -> ThemeName {
        ThemeName::parse(&self.theme).unwrap_or(ThemeName::Dark)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub sidebar_width: u16,
    pub welcome_width: u16,
    pub max_input_lines: u16,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 30,
            welcome_width: 72,
            max_input_lines: 6,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub system_prompt_preset: Option<String>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
}

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    ThemeName::Dark.as_str().to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuthStore {
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderAuth>,
}

impl AuthStore {
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        if !paths.auth_file.exists() {
            let auth = Self::default();
            auth.save(paths)?;
            return Ok(auth);
        }

        let contents = fs::read_to_string(&paths.auth_file)
            .with_context(|| format!("failed to read {}", paths.auth_file.display()))?;
        let auth: Self = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.auth_file.display()))?;
        Ok(auth)
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents =
            serde_json::to_string_pretty(self).context("failed to serialize auth store")?;
        fs::write(&paths.auth_file, contents)
            .with_context(|| format!("failed to write {}", paths.auth_file.display()))?;
        Ok(())
    }

    pub fn set_api_key(&mut self, provider_id: impl Into<String>, api_key: impl Into<String>) {
        let provider_id = provider_id.into();
        let api_key = api_key.into();
        self.providers.entry(provider_id).or_default().api_key = Some(api_key);
    }

    pub fn api_key(&self, provider_id: &str) -> Option<&str> {
        self.providers
            .get(provider_id)
            .and_then(|provider| provider.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderAuth {
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ActiveModel {
    pub provider_id: String,
    pub provider_display_name: String,
    pub base_url: String,
    pub model_id: String,
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
    pub system_prompt: String,
    pub api_key: Option<String>,
}

impl ActiveModel {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }

    pub fn api_key_present(&self) -> bool {
        self.api_key
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    }

    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

#[derive(Clone, Debug)]
pub struct ModelSummary {
    pub provider_id: String,
    pub provider_display_name: String,
    pub model_id: String,
    pub model_display_name: String,
    pub base_url: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
}

impl ModelSummary {
    pub fn label(&self) -> String {
        format!("{}/{}", self.provider_id, self.model_id)
    }
}
