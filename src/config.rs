use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::prompts::default_system_prompt;
use crate::theme::ThemeName;

const BUNDLED_PRESETS_TOML: &str = include_str!("../presets.toml");

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

        let config_dir = home_dir.join(".config").join("tidev");
        let data_dir = home_dir.join(".local/share").join("tidev");

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
    #[serde(skip)]
    pub bundled_providers: BTreeMap<String, ProviderConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSource {
    User,
    Bundled,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_provider: "openai".to_string(),
            default_model: "gpt-4o-mini".to_string(),
            theme: ThemeName::Dark.as_str().to_string(),
            ui: UiConfig::default(),
            providers: BTreeMap::new(),
            bundled_providers: bundled_provider_catalog().unwrap_or_default(),
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
            let config: Self = toml::from_str(example).with_context(|| {
                format!("failed to parse generated {}", paths.config_file.display())
            })?;
            return config.attach_bundled_providers();
        }

        let contents = fs::read_to_string(&paths.config_file)
            .with_context(|| format!("failed to read {}", paths.config_file.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.config_file.display()))?;
        config.attach_bundled_providers()
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
# Bundled provider presets ship with the binary and do not need to be copied here.
# Add your own providers below if you want custom endpoints.
# `theme` can be dark or light.
theme = "dark"
default_provider = "openai"
default_model = "gpt-4o-mini"

[ui]
sidebar_width = 30
welcome_width = 72
max_input_lines = 6
"#
    }

    fn attach_bundled_providers(mut self) -> Result<Self> {
        self.bundled_providers = bundled_provider_catalog()?;
        Ok(self)
    }

    fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
        self.providers
            .get(provider_id)
            .or_else(|| self.bundled_providers.get(provider_id))
    }

    pub fn provider_source(&self, provider_id: &str) -> Option<ProviderSource> {
        if self.providers.contains_key(provider_id) {
            Some(ProviderSource::User)
        } else if self.bundled_providers.contains_key(provider_id) {
            Some(ProviderSource::Bundled)
        } else {
            None
        }
    }

    pub fn provider_exists(&self, provider_id: &str) -> bool {
        self.provider_source(provider_id).is_some()
    }

    pub fn available_models(&self) -> Vec<ModelSummary> {
        let mut models = Vec::new();

        for provider_id in self.provider_ids() {
            let Some(provider) = self.provider(&provider_id) else {
                continue;
            };

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

    pub fn connected_models(&self, auth: &AuthStore) -> Vec<ModelSummary> {
        self.available_models()
            .into_iter()
            .filter(|summary| auth.api_key(&summary.provider_id).is_some())
            .collect()
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
            .provider(provider_id)
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
            .provider(provider_id)
            .with_context(|| format!("unknown provider '{provider_id}'"))?;
        let model = provider
            .models
            .get(model_id)
            .with_context(|| format!("unknown model '{model_id}' for provider '{provider_id}'"))?;

        let api_key = self.resolve_api_key(auth, provider_id);
        let api_type = provider
            .api_type
            .as_deref()
            .map(ApiType::parse)
            .unwrap_or_default();

        Ok(ActiveModel {
            provider_id: provider_id.to_string(),
            provider_display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            api_type,
            model_id: model_id.to_string(),
            display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
            system_prompt: model
                .system_prompt
                .clone()
                .filter(|prompt| !prompt.trim().is_empty())
                .unwrap_or_else(default_system_prompt),
            api_key,
        })
    }

    pub fn default_model_summary(&self) -> Result<ModelSummary> {
        let provider = self
            .provider(&self.default_provider)
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

        for provider_id in self.provider_ids() {
            if let Some(provider) = self.provider(&provider_id) {
                if provider.models.contains_key(query) {
                    matches.push((provider_id.clone(), query.to_string()));
                }
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

    fn resolve_api_key(&self, auth: &AuthStore, provider_id: &str) -> Option<String> {
        auth.providers
            .get(provider_id)
            .and_then(|entry| entry.api_key.clone())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn provider_ids(&self) -> Vec<String> {
        let mut provider_ids = BTreeSet::new();
        for provider_id in self.providers.keys().chain(self.bundled_providers.keys()) {
            provider_ids.insert(provider_id.clone());
        }

        provider_ids.into_iter().collect()
    }

    pub fn provider_display_name(&self, provider_id: &str) -> Option<&str> {
        self.provider(provider_id)
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    #[default]
    OpenAi,
    Anthropic,
}

impl ApiType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "anthropic" => Self::Anthropic,
            _ => Self::OpenAi,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_type: Option<String>,
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
    pub api_type: ApiType,
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
        match self.api_type {
            ApiType::Anthropic => {
                format!("{}/v1/messages", self.base_url.trim_end_matches('/'))
            }
            ApiType::OpenAi => {
                format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
            }
        }
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

#[derive(Clone, Debug, Deserialize)]
struct BundledProviderCatalog {
    #[serde(default)]
    providers: BTreeMap<String, ProviderConfig>,
}

fn bundled_provider_catalog() -> Result<BTreeMap<String, ProviderConfig>> {
    let catalog: BundledProviderCatalog =
        toml::from_str(BUNDLED_PRESETS_TOML).context("failed to parse bundled provider catalog")?;
    Ok(catalog.providers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_provider_catalog_loads() {
        let catalog = bundled_provider_catalog().expect("bundled catalog should parse");
        assert!(catalog.contains_key("deepseek"));
    }

    #[test]
    fn app_config_uses_bundled_provider_ids() {
        let config = AppConfig::default();
        assert!(config.provider_ids().contains(&"deepseek".to_string()));
        assert_eq!(
            config.provider_source("deepseek"),
            Some(ProviderSource::Bundled)
        );
    }

    #[test]
    fn user_provider_overrides_bundled_preset() {
        let mut config = AppConfig::default();
        config.providers.insert(
            "deepseek".to_string(),
            ProviderConfig {
                display_name: "Custom DeepSeek".to_string(),
                base_url: "https://example.com/v1".to_string(),
                models: BTreeMap::new(),
            },
        );

        assert_eq!(
            config.provider_source("deepseek"),
            Some(ProviderSource::User)
        );
        assert_eq!(
            config.provider_display_name("deepseek"),
            Some("Custom DeepSeek")
        );
        assert_eq!(
            config
                .provider_ids()
                .iter()
                .filter(|id| *id == "deepseek")
                .count(),
            1
        );
    }
}
