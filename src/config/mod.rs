mod auth;
pub mod mcp;
mod paths;
mod provider;
mod ui;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::prompts::default_system_prompt;
use crate::theme::ThemeName;

pub use auth::{ActiveModel, AuthStore, ModelSummary, ProviderAuth};
pub use mcp::{McpConfig, McpServerConfig};
pub use paths::ConfigPaths;
pub use provider::{ApiType, ModelConfig, ProviderConfig, ProviderSource};
pub use ui::UiConfig;

const BUNDLED_PRESETS_TOML: &str = include_str!("../../presets.toml");

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
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "mcp::McpConfig::is_empty")]
    pub mcp: McpConfig,
    #[serde(skip)]
    pub bundled_providers: BTreeMap<String, ProviderConfig>,
}

fn default_theme() -> String {
    ThemeName::Dark.as_str().to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_provider: "openai".to_string(),
            default_model: "gpt-4o-mini".to_string(),
            theme: ThemeName::Dark.as_str().to_string(),
            ui: UiConfig::default(),
            providers: BTreeMap::new(),
            instructions: Vec::new(),
            skills: Vec::new(),
            mcp: McpConfig::default(),
            bundled_providers: bundled_provider_catalog().unwrap_or_default(),
        }
    }
}

impl AppConfig {
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        if !paths.config_file.exists() {
            let example = Self::example_toml();
            std::fs::write(&paths.config_file, example)
                .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
            let config: Self = toml::from_str(example).with_context(|| {
                format!("failed to parse generated {}", paths.config_file.display())
            })?;
            return config.attach_bundled_providers();
        }

        let contents = std::fs::read_to_string(&paths.config_file)
            .with_context(|| format!("failed to read {}", paths.config_file.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.config_file.display()))?;
        config.attach_bundled_providers()
    }

    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(&paths.config_file, contents)
            .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
        Ok(())
    }

    pub fn example_toml() -> &'static str {
        r#"# TiDev configuration
# Bundled provider presets ship with the binary and do not need to be copied here.
# Add your own providers below if you want custom endpoints.
# `theme` can be one of: dark, light, nord, one-dark, catppuccin, solarized, orng, github, material.
theme = "dark"
default_provider = "openai"
default_model = "gpt-4o-mini"

# Optional custom instruction files or glob patterns to include in the system prompt.
# Example: instructions = ["docs/style.md", "packages/*/AGENTS.md"]
instructions = []

# Optional additional skill sources. Each entry can be a local path or an HTTP(S) URL to a SKILL.md file.
# Example: skills = ["https://example.com/skills/git-release/SKILL.md"]
skills = []

# MCP servers can be declared here. Supported transports: stdio, streamable HTTP, and SSE.
# [mcp.servers.my_server]
# kind = "stdio"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
#
# [mcp.servers.remote]
# kind = "http"
# url = "https://example.com/mcp"
#
# [mcp.servers.events]
# kind = "sse"
# url = "https://example.com/sse"
#
# [mcp.servers.webtools]
# kind = "stdio"
# command = "webtools-mcp"
#
# # During development, you can also launch it through Cargo:
# # command = "cargo"
# # args = ["run", "--quiet", "--bin", "webtools-mcp", "--"]

[mcp]

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
            supports_images: model.supports_images,
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
            if let Some(provider) = self.provider(&provider_id)
                && provider.models.contains_key(query)
            {
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
                api_type: None,
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
