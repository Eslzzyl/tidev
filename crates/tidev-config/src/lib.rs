//! Configuration loading for tidev.
//!
//! This crate provides [`AppConfig`] (the main configuration struct),
//! [`ConfigPaths`] for directory discovery, [`AuthStore`] for credential
//! storage, and associated types for provider/model configuration.

pub mod auth;
pub mod paths;
pub mod provider;
pub mod reasoning;
pub mod types;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::auth::{ActiveModel, AuthStore, ModelSummary};
use crate::paths::ConfigPaths;
use crate::provider::{ProviderConfig, ProviderSource};
use tidev_types::tools::PermissionConfig;

/// Bundled provider presets compiled into the binary.
const BUNDLED_PRESETS_TOML: &str = include_str!("../../../presets.toml");

// ---------------------------------------------------------------------------
// Public re-exports
// ---------------------------------------------------------------------------

pub use crate::types::ApiType;
pub use crate::reasoning::ThinkingMatcher;

// ---------------------------------------------------------------------------
// WebSearchConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebSearchConfig {
    #[serde(default = "default_websearch_provider")]
    pub default_provider: String,
    #[serde(default)]
    pub providers: BTreeMap<String, WebSearchProviderConfig>,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            default_provider: default_websearch_provider(),
            providers: BTreeMap::new(),
        }
    }
}

fn default_websearch_provider() -> String {
    "exa".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebSearchProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

// ---------------------------------------------------------------------------
// AgentConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_agent_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub default_subagent_model: String,
    #[serde(default)]
    pub default_subagent_provider: String,
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    #[serde(default = "default_max_sessions_per_agent")]
    pub max_sessions_per_agent: usize,
    #[serde(default)]
    pub models: BTreeMap<String, String>,
    #[serde(default)]
    pub thinking_levels: BTreeMap<String, String>,
}

fn default_agent_enabled() -> bool {
    true
}
fn default_max_depth() -> usize {
    3
}
fn default_max_sessions_per_agent() -> usize {
    5
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_subagent_model: String::new(),
            default_subagent_provider: String::new(),
            max_depth: 3,
            max_sessions_per_agent: 5,
            models: BTreeMap::new(),
            thinking_levels: BTreeMap::new(),
        }
    }
}

impl AgentConfig {
    pub fn model_for(&self, agent_type: &str) -> Option<&str> {
        self.models.get(agent_type).map(|s| s.as_str())
    }
    pub fn default_model(&self) -> Option<&str> {
        let m = self.default_subagent_model.trim();
        if m.is_empty() { None } else { Some(m) }
    }
}

// ---------------------------------------------------------------------------
// ShellConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ShellConfig {
    #[serde(default)]
    pub windows_shell: Option<String>,
}

// ---------------------------------------------------------------------------
// AccessControlConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccessControlConfig {
    #[serde(default)]
    pub allow_sensitive_file_access: bool,
    #[serde(default)]
    pub allow_outside_workspace_access: bool,
}

// ---------------------------------------------------------------------------
// NotificationConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub condition: String,
}

fn default_true() -> bool {
    true
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            method: "auto".to_string(),
            condition: "unfocused".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// SnapshotConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub ignore_globs: Vec<String>,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
    #[serde(default)]
    pub max_files: usize,
    #[serde(default = "default_track_timeout_ms")]
    pub track_timeout_ms: u64,
    #[serde(default = "default_stat_concurrency")]
    pub stat_concurrency: usize,
}

fn default_max_file_size() -> u64 {
    2 * 1024 * 1024
}
fn default_track_timeout_ms() -> u64 {
    30_000
}
fn default_stat_concurrency() -> usize {
    8
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ignore_globs: Vec::new(),
            max_file_size: default_max_file_size(),
            max_files: 0,
            track_timeout_ms: default_track_timeout_ms(),
            stat_concurrency: default_stat_concurrency(),
        }
    }
}

// ---------------------------------------------------------------------------
// TmpConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TmpConfig {
    #[serde(default)]
    pub auto_cleanup: bool,
    #[serde(default = "default_max_age_hours")]
    pub max_age_hours: u64,
}

fn default_max_age_hours() -> u64 {
    24
}

impl Default for TmpConfig {
    fn default() -> Self {
        Self {
            auto_cleanup: false,
            max_age_hours: 24,
        }
    }
}

// ---------------------------------------------------------------------------
// McpConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

impl McpConfig {
    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpServerConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
    },
    Sse {
        url: String,
    },
}

impl McpServerConfig {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
            Self::Sse { .. } => "sse",
        }
    }
}

// ---------------------------------------------------------------------------
// LogConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_enabled")]
    pub enabled: bool,
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
    #[serde(default = "default_max_files")]
    pub max_files: u32,
    #[serde(default)]
    pub console: bool,
    #[serde(default)]
    pub save_request_body: bool,
    #[serde(default = "default_max_request_files")]
    pub max_request_files: usize,
    #[serde(default)]
    pub save_response_body: bool,
    #[serde(default = "default_max_response_files")]
    pub max_response_files: usize,
}

fn default_log_enabled() -> bool { true }
fn default_log_level() -> String { "INFO".to_string() }
fn default_max_size_mb() -> u32 { 10 }
fn default_max_files() -> u32 { 5 }
fn default_max_request_files() -> usize { 100 }
fn default_max_response_files() -> usize { 100 }

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            level: "INFO".to_string(),
            max_size_mb: 10,
            max_files: 5,
            console: false,
            save_request_body: false,
            max_request_files: 100,
            save_response_body: false,
            max_response_files: 100,
        }
    }
}

// ---------------------------------------------------------------------------
// UiConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiConfig {
    pub sidebar_width: u16,
    pub welcome_width: u16,
    pub max_input_lines: u16,
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: f32,
    #[serde(default)]
    pub external_editor: Option<String>,
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,
}

fn default_scroll_speed() -> f32 { 3.0 }
fn default_tab_width() -> usize { 4 }

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 40,
            welcome_width: 90,
            max_input_lines: 6,
            scroll_speed: 3.0,
            external_editor: None,
            tab_width: 4,
        }
    }
}

// ---------------------------------------------------------------------------
// AppConfig
// ---------------------------------------------------------------------------

/// The main tidev application configuration.
///
/// Loaded from `~/.config/tidev/config.toml` with optional
/// project-level overlay (`.tidev/config.toml` in the workspace root).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_provider: String,
    pub default_model: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub logging: LogConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "McpConfig::is_empty")]
    pub mcp: McpConfig,
    #[serde(default)]
    pub permissions: PermissionConfig,
    #[serde(default)]
    pub access_control: AccessControlConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub shell: ShellConfig,
    #[serde(default)]
    pub tmp: TmpConfig,
    #[serde(default)]
    pub websearch: WebSearchConfig,
    #[serde(default)]
    pub snapshot: SnapshotConfig,
    #[serde(skip)]
    pub bundled_providers: BTreeMap<String, ProviderConfig>,
}

fn default_theme() -> String {
    "dark".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            default_provider: "openai".to_string(),
            default_model: "gpt-4o-mini".to_string(),
            theme: "dark".to_string(),
            ui: UiConfig::default(),
            logging: LogConfig::default(),
            providers: BTreeMap::new(),
            instructions: Vec::new(),
            skills: Vec::new(),
            mcp: McpConfig::default(),
            permissions: PermissionConfig::default(),
            access_control: AccessControlConfig::default(),
            notifications: NotificationConfig::default(),
            agent: AgentConfig::default(),
            shell: ShellConfig::default(),
            tmp: TmpConfig::default(),
            websearch: WebSearchConfig::default(),
            snapshot: SnapshotConfig::default(),
            bundled_providers: bundled_provider_catalog().unwrap_or_default(),
        }
    }
}

impl AppConfig {
    // ── Loading ──────────────────────────────────────────────────────

    /// Load config from the default paths, merging global and project config.
    pub fn load(paths: &ConfigPaths) -> Result<Self> {
        let mut config = Self::load_global(paths)?;
        config.bundled_providers = bundled_provider_catalog()?;
        Ok(config)
    }

    /// Load the global config file only.
    fn load_global(paths: &ConfigPaths) -> Result<Self> {
        let config: Self = if paths.config_file.exists() {
            let contents = std::fs::read_to_string(&paths.config_file)
                .with_context(|| format!("failed to read {}", paths.config_file.display()))?;
            toml::from_str(&contents)
                .with_context(|| format!("failed to parse {}", paths.config_file.display()))?
        } else {
            // Create default config file
            let config = Self::default();
            config.save(paths)?;
            config
        };
        Ok(config)
    }

    /// Load config and overlay with project-level `.tidev/config.toml`.
    pub fn load_with_overlay(paths: &ConfigPaths, workspace_root: &std::path::Path) -> Result<Self> {
        let mut config = Self::load_global(paths)?;

        // Apply project-level overlay
        let project_config_path = workspace_root.join(".tidev").join("config.toml");
        if project_config_path.exists() {
            let contents = std::fs::read_to_string(&project_config_path)
                .with_context(|| format!("failed to read {}", project_config_path.display()))?;
            let overlay: AppConfig = toml::from_str(&contents)
                .with_context(|| format!("failed to parse {}", project_config_path.display()))?;
            config.merge(overlay, &contents);
        }

        config.bundled_providers = bundled_provider_catalog()?;
        Ok(config)
    }

    // ── Merging ──────────────────────────────────────────────────────

    /// Merge another config into this one (project overlay into global).
    fn merge(&mut self, overlay: AppConfig, overlay_toml: &str) {
        let has = |key: &str| top_level_toml_keys(overlay_toml).contains(key);

        // Scalar fields: replaced when present
        if has("theme") { self.theme = overlay.theme; }
        if has("default_provider") { self.default_provider = overlay.default_provider; }
        if has("default_model") { self.default_model = overlay.default_model; }

        // MCP servers: extend (project servers add to global)
        if has("mcp") {
            self.mcp.servers.extend(overlay.mcp.servers);
        }

        // Lists: append
        if has("instructions") { self.instructions.extend(overlay.instructions); }
        if has("skills") { self.skills.extend(overlay.skills); }

        // Sub-configs: full replacement when section is present
        if has("ui") { self.ui = overlay.ui; }
        if has("logging") { self.logging = overlay.logging; }
        if has("permissions") { self.permissions = overlay.permissions; }
        if has("access_control") { self.access_control = overlay.access_control; }
        if has("notifications") { self.notifications = overlay.notifications; }
        if has("agent") { self.agent = overlay.agent; }
        if has("shell") { self.shell = overlay.shell; }
        if has("tmp") { self.tmp = overlay.tmp; }
    }

    // ── Saving ───────────────────────────────────────────────────────

    /// Save the config to the default config file.
    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(&paths.config_file, contents)
            .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
        Ok(())
    }

    // ── Provider helpers ─────────────────────────────────────────────

    pub fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
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

    pub fn provider_ids(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        for id in self.providers.keys() {
            ids.insert(id.clone());
        }
        for id in self.bundled_providers.keys() {
            ids.insert(id.clone());
        }
        ids.into_iter().collect()
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

    // ── Model resolution ───────────────────────────────────────────

    fn resolve_api_key(&self, auth: &AuthStore, provider_id: &str) -> Option<String> {
        auth.api_key(provider_id).map(|s| s.to_string())
    }

    pub fn resolve_model(
        &self,
        auth: &AuthStore,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> Result<ActiveModel> {
        let pid = provider_id.unwrap_or(&self.default_provider);
        let mid = model_id.unwrap_or(&self.default_model);
        self.resolve_model_by_ids(auth, pid, mid)
    }

    pub fn resolve_active_model(&self, auth: &AuthStore) -> Result<ActiveModel> {
        self.resolve_model(auth, None, None)
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
        let api_type = provider.resolve_api_type(model);
        let base_url = provider.resolve_base_url(model);

        let request_model_id = model
            .request_model_id
            .clone()
            .unwrap_or_else(|| model_id.to_string());

        let thinking_level = if let Some(ref rid) = model.request_model_id {
            ThinkingMatcher::match_for_model(rid)
        } else {
            ThinkingMatcher::match_for_model(model_id)
        };

        Ok(ActiveModel {
            provider_id: provider_id.to_string(),
            provider_display_name: provider.display_name.clone(),
            base_url,
            api_type,
            model_id: model_id.to_string(),
            request_model_id,
            display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
            supports_images: model.supports_images,
            system_prompt: String::new(),
            api_key,
            extra_body: model.extra_body.clone(),
            thinking_level,
        })
    }

    // ── Theme ──────────────────────────────────────────────────────

    pub fn set_theme(&mut self, theme: &str) {
        self.theme = theme.to_string();
    }
}

// ---------------------------------------------------------------------------
// Bundled provider catalog
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// TOML key extraction (for config merging)
// ---------------------------------------------------------------------------

/// Extract the top-level key names from raw TOML text.
///
/// Used during config merging to determine which sections the project config
/// explicitly sets, so we don't accidentally overwrite global config sections
/// with default values.
pub fn top_level_toml_keys(toml_str: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();

    for line in toml_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.starts_with('[') {
            let open_count = trimmed.chars().take_while(|c| *c == '[').count();
            let close_count = trimmed.chars().rev().take_while(|c| *c == ']').count();
            if open_count > 0 && open_count == close_count && open_count <= 2 {
                let key = trimmed[open_count..trimmed.len() - close_count]
                    .split('.')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
            continue;
        }

        if let Some(eq_pos) = trimmed.find('=') {
            let before_eq = trimmed[..eq_pos].trim();
            if !before_eq.starts_with('"') && !before_eq.starts_with('{') {
                let key = before_eq.to_string();
                if !key.is_empty() {
                    keys.insert(key);
                }
            }
        }
    }

    keys
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    fn resolve_model_returns_active_model() {
        let config = AppConfig::default();
        let auth = AuthStore::default();
        let model = config.resolve_model_by_ids(&auth, "deepseek", "deepseek-v4-flash");
        assert!(model.is_ok(), "resolve_model should succeed: {:?}", model.err());
        let model = model.unwrap();
        assert_eq!(model.provider_id, "deepseek");
        assert_eq!(model.model_id, "deepseek-v4-flash");
        assert_eq!(model.api_type, ApiType::OpenAiChatCompletions);
    }

    #[test]
    fn resolve_unknown_provider_fails() {
        let config = AppConfig::default();
        let auth = AuthStore::default();
        let result = config.resolve_model_by_ids(&auth, "nonexistent", "model");
        assert!(result.is_err());
    }

    #[test]
    fn top_level_keys_detects_sections() {
        let toml = r#"
theme = "dark"
[ui]
tab_width = 4
[agent]
enabled = true
"#;
        let keys = top_level_toml_keys(toml);
        assert!(keys.contains("theme"));
        assert!(keys.contains("ui"));
        assert!(keys.contains("agent"));
        assert!(!keys.contains("logging"));
    }
}
