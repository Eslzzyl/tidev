//! Configuration loading for tidev.
//!
//! This crate provides [`AppConfig`] (the main configuration struct),
//! [`ConfigPaths`] for directory discovery, [`AuthStore`] for credential
//! storage, and associated types for provider/model configuration.

pub mod auth;
pub mod mcp;
pub mod paths;
pub mod provider;
pub mod reasoning;
pub mod theme;
pub mod types;

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use crate::auth::AuthStore;
use crate::auth::{ActiveModel, ModelSummary};
pub use crate::mcp::McpConfig;
pub use crate::mcp::McpServerConfig;
use crate::paths::ConfigPaths;
use crate::provider::{ProviderConfig, ProviderSource};

/// Bundled provider presets directory embedded at compile time.
static PRESETS_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/../../presets");

// ---------------------------------------------------------------------------
// Public re-exports
// ---------------------------------------------------------------------------

pub use crate::reasoning::ThinkingLevelType;
pub use crate::reasoning::ThinkingMatcher;
pub use crate::theme::{ThemeCatalog, ThemeColor, ThemeDefinition};
pub use crate::types::ApiType;

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
    #[serde(default)]
    pub unix_shell: Option<String>,
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
// SubagentConfig
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubagentConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self { enabled: true }
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

fn default_log_enabled() -> bool {
    true
}
fn default_log_level() -> String {
    "INFO".to_string()
}
fn default_max_size_mb() -> u32 {
    10
}
fn default_max_files() -> u32 {
    5
}
fn default_max_request_files() -> usize {
    100
}
fn default_max_response_files() -> usize {
    100
}

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

/// What to do with a user message submitted while the session's agent loop
/// is busy (the model is still executing a turn).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SendWhileBusy {
    /// Wait until the current turn (all requests and tool calls) finishes,
    /// then send the message as the start of a new turn.
    #[default]
    Queue,
    /// Persist the message immediately and insert it into the running turn
    /// at the next request boundary, without interrupting the in-flight
    /// model stream.
    Steer,
}

impl SendWhileBusy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Steer => "steer",
        }
    }
}

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
    #[serde(default)]
    pub collapse_thinking: bool,
    /// Collapse edit/write/apply_patch diffs to per-file +N/-M summaries by
    /// default. Click a tool card to toggle the fold state.
    #[serde(default)]
    pub collapse_diffs: bool,
    /// How to handle a user message submitted while the session's agent
    /// loop is busy. See [`SendWhileBusy`].
    #[serde(default)]
    pub send_while_busy: SendWhileBusy,
    #[serde(default = "default_right_sidebar_visible")]
    pub right_sidebar_visible: bool,
}

fn default_scroll_speed() -> f32 {
    3.0
}
fn default_tab_width() -> usize {
    4
}
fn default_right_sidebar_visible() -> bool {
    true
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            sidebar_width: 40,
            welcome_width: 90,
            max_input_lines: 6,
            scroll_speed: 3.0,
            external_editor: None,
            tab_width: 4,
            collapse_thinking: false,
            collapse_diffs: false,
            send_while_busy: SendWhileBusy::Queue,
            right_sidebar_visible: default_right_sidebar_visible(),
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
    #[serde(default = "default_provider")]
    pub default_provider: String,
    #[serde(default = "default_model")]
    pub default_model: String,
    #[serde(default)]
    pub default_thinking_level: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub logging: LogConfig,
    #[serde(
        default,
        deserialize_with = "deserialize_provider_configs",
        serialize_with = "serialize_provider_configs"
    )]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
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
    #[serde(skip)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub snapshot: SnapshotConfig,
    #[serde(default)]
    pub subagent: SubagentConfig,
    #[serde(skip)]
    pub bundled_providers: BTreeMap<String, ProviderConfig>,
    /// Effective provider catalog after applying user overrides.
    #[serde(skip)]
    effective_providers: BTreeMap<String, ProviderConfig>,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

/// A partial provider definition used when reading user configuration.
///
/// `ProviderConfig` represents a fully resolved provider and therefore has
/// required metadata fields. User configuration, however, may only specify a
/// model under a bundled provider. This intermediate type preserves that
/// partial form until the bundled catalog is available for merging.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ProviderConfigOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    api_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    models: BTreeMap<String, crate::provider::ModelConfig>,
}

impl ProviderConfigOverride {
    fn into_partial_provider(self) -> ProviderConfig {
        ProviderConfig {
            display_name: self.display_name.unwrap_or_default(),
            base_url: self.base_url.unwrap_or_default(),
            api_type: self.api_type,
            user_agent: self.user_agent,
            models: self.models,
        }
    }

    fn from_provider(provider: &ProviderConfig) -> Self {
        Self {
            display_name: (!provider.display_name.is_empty())
                .then(|| provider.display_name.clone()),
            base_url: (!provider.base_url.is_empty()).then(|| provider.base_url.clone()),
            api_type: provider.api_type.clone(),
            user_agent: provider.user_agent.clone(),
            models: provider.models.clone(),
        }
    }
}

fn deserialize_provider_configs<'de, D>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, ProviderConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    let overrides = BTreeMap::<String, ProviderConfigOverride>::deserialize(deserializer)?;
    Ok(overrides
        .into_iter()
        .map(|(provider_id, provider)| (provider_id, provider.into_partial_provider()))
        .collect())
}

fn serialize_provider_configs<S>(
    providers: &BTreeMap<String, ProviderConfig>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let overrides = providers
        .iter()
        .map(|(provider_id, provider)| {
            (
                provider_id.clone(),
                ProviderConfigOverride::from_provider(provider),
            )
        })
        .collect::<BTreeMap<_, _>>();
    overrides.serialize(serializer)
}

impl Default for AppConfig {
    fn default() -> Self {
        let bundled_providers = bundled_provider_catalog().unwrap_or_default();
        Self {
            default_provider: default_provider(),
            default_model: default_model(),
            default_thinking_level: String::new(),
            theme: "dark".to_string(),
            ui: UiConfig::default(),
            logging: LogConfig::default(),
            providers: BTreeMap::new(),
            instructions: Vec::new(),
            skills: Vec::new(),
            access_control: AccessControlConfig::default(),
            notifications: NotificationConfig::default(),
            agent: AgentConfig::default(),
            shell: ShellConfig::default(),
            tmp: TmpConfig::default(),
            websearch: WebSearchConfig::default(),
            mcp: McpConfig::default(),
            snapshot: SnapshotConfig::default(),
            subagent: SubagentConfig::default(),
            effective_providers: bundled_providers.clone(),
            bundled_providers,
        }
    }
}

impl AppConfig {
    // ── Loading ──────────────────────────────────────────────────────

    /// Load config from the default paths, merging global and project config.
    pub fn load(paths: &ConfigPaths) -> Result<Self> {
        let mut config = Self::load_global(paths)?;
        config.bundled_providers = bundled_provider_catalog()?;
        config.rebuild_effective_providers()?;
        Ok(config)
    }

    /// Load the global config file only.
    fn load_global(paths: &ConfigPaths) -> Result<Self> {
        let mut config: Self = if paths.config_file.exists() {
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
        config.mcp = McpConfig::load(paths)?;
        Ok(config)
    }

    /// Load config and overlay with project-level `.tidev/config.toml`.
    pub fn load_with_overlay(
        paths: &ConfigPaths,
        workspace_root: &std::path::Path,
    ) -> Result<Self> {
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

        // Overlay workspace-level MCP config (.tidev/mcp.json)
        config.mcp = McpConfig::load_with_workspace(paths, workspace_root)?;

        config.bundled_providers = bundled_provider_catalog()?;
        config.rebuild_effective_providers()?;
        Ok(config)
    }

    // ── Merging ──────────────────────────────────────────────────────

    /// Merge another config into this one (project overlay into global).
    fn merge(&mut self, overlay: AppConfig, overlay_toml: &str) {
        let has = |key: &str| top_level_toml_keys(overlay_toml).contains(key);

        // Scalar fields: replaced when present
        if has("theme") {
            self.theme = overlay.theme;
        }
        if has("default_provider") {
            self.default_provider = overlay.default_provider;
        }
        if has("default_model") {
            self.default_model = overlay.default_model;
        }
        if has("default_thinking_level") {
            self.default_thinking_level = overlay.default_thinking_level;
        }

        // Providers and models use overlay semantics: provider fields replace
        // only fields explicitly present in the overlay, while model entries
        // with the same key replace the previous model entirely.
        if has("providers") {
            for (provider_id, provider) in &overlay.providers {
                let existing = self
                    .providers
                    .entry(provider_id.clone())
                    .or_insert_with(empty_provider_config);
                apply_provider_override(existing, provider);
            }
        }

        // Lists: append
        if has("instructions") {
            self.instructions.extend(overlay.instructions);
        }
        if has("skills") {
            self.skills.extend(overlay.skills);
        }

        // Sub-configs: full replacement when section is present
        if has("ui") {
            self.ui = overlay.ui;
        }
        if has("logging") {
            self.logging = overlay.logging;
        }
        if has("access_control") {
            self.access_control = overlay.access_control;
        }
        if has("notifications") {
            self.notifications = overlay.notifications;
        }
        if has("agent") {
            self.agent = overlay.agent;
        }
        if has("shell") {
            self.shell = overlay.shell;
        }
        if has("tmp") {
            self.tmp = overlay.tmp;
        }
        if has("subagent") {
            self.subagent = overlay.subagent;
        }
    }

    /// Rebuild the effective provider catalog from bundled presets and user
    /// provider overrides.
    fn rebuild_effective_providers(&mut self) -> Result<()> {
        let mut effective = self.bundled_providers.clone();

        for (provider_id, user_provider) in &self.providers {
            let merged =
                merge_provider_config(effective.get(provider_id), user_provider, provider_id)?;
            effective.insert(provider_id.clone(), merged);
        }

        self.effective_providers = effective;
        Ok(())
    }

    // ── Saving ───────────────────────────────────────────────────────

    /// Save the config to the default config file and save MCP config to mcp.json.
    pub fn save(&self, paths: &ConfigPaths) -> Result<()> {
        paths.ensure_directories()?;
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        std::fs::write(&paths.config_file, contents)
            .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
        self.mcp.save(paths)?;
        Ok(())
    }

    // ── Provider helpers ─────────────────────────────────────────────

    /// Add or replace a user-defined provider and rebuild the effective catalog.
    pub fn set_user_provider(
        &mut self,
        provider_id: String,
        provider: ProviderConfig,
    ) -> Result<()> {
        let previous = self.providers.insert(provider_id.clone(), provider);
        if let Err(error) = self.rebuild_effective_providers() {
            match previous {
                Some(provider) => {
                    self.providers.insert(provider_id, provider);
                }
                None => {
                    self.providers.remove(&provider_id);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Remove a user-defined provider and rebuild the effective catalog.
    pub fn remove_user_provider(&mut self, provider_id: &str) -> Result<Option<ProviderConfig>> {
        let removed = self.providers.remove(provider_id);
        if let Err(error) = self.rebuild_effective_providers() {
            if let Some(provider) = removed.clone() {
                self.providers.insert(provider_id.to_owned(), provider);
            }
            return Err(error);
        }
        Ok(removed)
    }

    pub fn provider(&self, provider_id: &str) -> Option<&ProviderConfig> {
        self.effective_providers
            .get(provider_id)
            .or_else(|| self.providers.get(provider_id))
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

    pub fn provider_display_name(&self, provider_id: &str) -> Option<&str> {
        self.provider(provider_id)
            .map(|provider| provider.display_name.as_str())
    }

    pub fn provider_exists(&self, provider_id: &str) -> bool {
        self.provider_source(provider_id).is_some()
    }

    pub fn provider_ids(&self) -> Vec<String> {
        let mut ids: BTreeSet<String> = BTreeSet::new();
        ids.extend(self.effective_providers.keys().cloned());
        ids.extend(self.providers.keys().cloned());
        ids.extend(self.bundled_providers.keys().cloned());
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
                    request_model_id: model
                        .request_model_id
                        .clone()
                        .unwrap_or_else(|| model_id.clone()),
                    model_display_name: model.display_name.clone(),
                    base_url: provider.base_url.clone(),
                    context_window: model.context_window,
                    max_output_tokens: model.max_output_tokens,
                    supports_images: model.supports_images,
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
        let mut model = self.resolve_model(auth, None, None)?;
        if !self.default_thinking_level.is_empty() && model.thinking_level.is_supported() {
            model.thinking_level = ThinkingMatcher::coerce_saved(
                &self.default_thinking_level,
                &model.request_model_id,
            );
        }
        Ok(model)
    }

    /// Resolve a model for a given provider using its first configured model.
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

        // Determine thinking level with cascade fallback:
        // 1. Try request_model_id first (if present)
        // 2. Then try display_name (if request_model_id is None)
        // 3. Finally try model_id (if display_name is empty)
        let thinking_level = if let Some(ref rid) = model.request_model_id {
            ThinkingMatcher::match_for_model(rid)
        } else if !model.display_name.is_empty() {
            ThinkingMatcher::match_for_model(&model.display_name)
        } else {
            ThinkingMatcher::match_for_model(model_id)
        };

        Ok(ActiveModel {
            provider_id: provider_id.to_string(),
            provider_display_name: provider.display_name.clone(),
            base_url,
            user_agent: provider.user_agent.clone(),
            api_type,
            model_id: model_id.to_string(),
            request_model_id,
            display_name: model.display_name.clone(),
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            temperature: model.temperature,
            supports_images: model.supports_images,
            supports_parallel_tool_calls: model.supports_parallel_tool_calls,
            system_prompt: String::new(),
            api_key,
            extra_body: model.extra_body.clone(),
            thinking_level,
        })
    }

    // ── Agent model overrides ──────────────────────────────────────

    /// Resolve an ActiveModel for a sub-agent type, checking the `[agent.models]` config.
    ///
    /// If the agent type has a configured model string, it is resolved.
    /// If the model string is in `"provider/model_id"` format, the provider prefix is used;
    /// otherwise, the default provider is assumed.
    ///
    /// Returns `None` when no override is configured (caller should fall back to parent model).
    pub fn resolve_agent_active_model(
        &self,
        auth: &AuthStore,
        agent_type: &str,
    ) -> Result<Option<ActiveModel>> {
        let Some(model_str) = self
            .agent
            .model_for(agent_type)
            .or_else(|| self.agent.default_model())
        else {
            return Ok(None);
        };

        let (provider_id, model_id) = if let Some(slash_pos) = model_str.find('/') {
            let provider = &model_str[..slash_pos];
            let model = &model_str[slash_pos + 1..];
            (provider.to_string(), model.to_string())
        } else {
            // Use the default provider
            (self.default_provider.clone(), model_str.to_string())
        };

        let mut model = self.resolve_model_by_ids(auth, &provider_id, &model_id)?;

        // Apply agent-specific thinking_level override if configured.
        // Only override when the model actually supports thinking (auto-detected
        // level is not None); otherwise a stale override from a previous model
        // would force invalid thinking parameters onto the API request.
        if let Some(tl_str) = self.agent.thinking_levels.get(agent_type)
            && model.thinking_level.is_supported()
        {
            // Coerce so an override saved under another family (e.g.
            // "qwen:on" before Qwen3.8 levels) falls back to the model default.
            model.thinking_level = ThinkingMatcher::coerce_saved(tl_str, &model.request_model_id);
        }

        Ok(Some(model))
    }

    /// Set the model override for a specific agent type and persist to config.
    /// `model_str` should be in `"provider/model_id"` format.
    pub fn set_agent_model(
        &mut self,
        paths: &ConfigPaths,
        agent_type: &str,
        model_str: &str,
    ) -> Result<()> {
        if model_str.is_empty() {
            self.agent.models.remove(agent_type);
        } else {
            self.agent
                .models
                .insert(agent_type.to_string(), model_str.to_string());
        }
        self.save(paths)
    }

    /// Set both the model override and thinking level for an agent type.
    /// `model_str` in `"provider/model_id"` format, `thinking_level` in
    /// `ThinkingLevelType::to_string()` format (e.g. "deepseek:High").
    /// Pass empty `thinking_level` to clear the override.
    pub fn set_agent_model_and_thinking(
        &mut self,
        paths: &ConfigPaths,
        agent_type: &str,
        model_str: &str,
        thinking_level: &str,
    ) -> Result<()> {
        if model_str.is_empty() {
            self.agent.models.remove(agent_type);
        } else {
            self.agent
                .models
                .insert(agent_type.to_string(), model_str.to_string());
        }
        if thinking_level.is_empty() {
            self.agent.thinking_levels.remove(agent_type);
        } else {
            self.agent
                .thinking_levels
                .insert(agent_type.to_string(), thinking_level.to_string());
        }
        self.save(paths)
    }

    /// Return the configured model label for an agent type, if any.
    /// Format: `"provider/model_id"` or `None` (inherit).
    pub fn agent_model_label(&self, agent_type: &str) -> Option<&str> {
        self.agent.models.get(agent_type).map(|s| s.as_str())
    }

    /// Return a human-readable label for an agent type's current model,
    /// or the string `"<inherit>"` if none is configured.
    pub fn agent_model_display(&self, agent_type: &str) -> String {
        self.agent_model_label(agent_type)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<inherit>".to_string())
    }

    // ── Theme ──────────────────────────────────────────────────────

    pub fn set_theme(&mut self, theme: &str) {
        self.theme = theme.to_string();
    }
}

fn empty_provider_config() -> ProviderConfig {
    ProviderConfig {
        display_name: String::new(),
        base_url: String::new(),
        api_type: None,
        user_agent: None,
        models: BTreeMap::new(),
    }
}

/// Apply one partial provider definition over another partial definition.
fn apply_provider_override(base: &mut ProviderConfig, override_config: &ProviderConfig) {
    if !override_config.display_name.is_empty() {
        base.display_name.clone_from(&override_config.display_name);
    }
    if !override_config.base_url.is_empty() {
        base.base_url.clone_from(&override_config.base_url);
    }
    if override_config.api_type.is_some() {
        base.api_type.clone_from(&override_config.api_type);
    }
    if override_config.user_agent.is_some() {
        base.user_agent.clone_from(&override_config.user_agent);
    }
    base.models.extend(override_config.models.clone());
}

/// Resolve a user provider override against an optional bundled provider.
fn merge_provider_config(
    bundled: Option<&ProviderConfig>,
    user: &ProviderConfig,
    provider_id: &str,
) -> Result<ProviderConfig> {
    let display_name = if user.display_name.is_empty() {
        bundled
            .map(|provider| provider.display_name.clone())
            .unwrap_or_default()
    } else {
        user.display_name.clone()
    };
    if display_name.trim().is_empty() {
        bail!("provider '{provider_id}' requires 'display_name'");
    }

    let base_url = if user.base_url.is_empty() {
        bundled
            .map(|provider| provider.base_url.clone())
            .unwrap_or_default()
    } else {
        user.base_url.clone()
    };
    if base_url.trim().is_empty() {
        bail!("provider '{provider_id}' requires 'base_url'");
    }

    if let Some(user_agent) = user.user_agent.as_deref() {
        crate::provider::validate_user_agent(user_agent)
            .with_context(|| format!("invalid user_agent for provider '{provider_id}'"))?;
    }

    let mut models = bundled
        .map(|provider| provider.models.clone())
        .unwrap_or_default();
    models.extend(user.models.clone());

    Ok(ProviderConfig {
        display_name,
        base_url,
        api_type: user
            .api_type
            .clone()
            .or_else(|| bundled.and_then(|provider| provider.api_type.clone())),
        user_agent: user
            .user_agent
            .clone()
            .or_else(|| bundled.and_then(|provider| provider.user_agent.clone())),
        models,
    })
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
    let mut providers = BTreeMap::new();
    let mut files: Vec<_> = PRESETS_DIR.files().collect();
    files.sort_by_key(|f| f.path());
    for file in files {
        let ext = file.path().extension();
        if ext.is_none_or(|e| e != "toml") {
            continue;
        }
        let content = file.contents_utf8().context("non-utf8 preset file")?;
        let catalog: BundledProviderCatalog =
            toml::from_str(content).context("failed to parse bundled provider catalog")?;
        providers.extend(catalog.providers);
    }
    Ok(providers)
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
        assert!(
            model.is_ok(),
            "resolve_model should succeed: {:?}",
            model.err()
        );
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
    fn user_models_extend_bundled_provider() {
        let mut config: AppConfig = toml::from_str(
            r#"
default_provider = "deepseek"
default_model = "custom-model"

[providers.deepseek.models.custom-model]
request_model_id = "deepseek-v4-custom"
display_name = "DeepSeek Custom"
context_window = 1048576
max_output_tokens = 262144
"#,
        )
        .expect("partial provider override should parse");
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");
        config
            .rebuild_effective_providers()
            .expect("provider override should resolve");

        let provider = config.provider("deepseek").expect("provider should exist");
        assert!(provider.models.contains_key("custom-model"));
        assert!(provider.models.contains_key("deepseek-v4-pro"));
        assert_eq!(
            provider.models["custom-model"].request_model_id.as_deref(),
            Some("deepseek-v4-custom")
        );
    }

    #[test]
    fn user_model_replaces_bundled_model_with_same_key() {
        let mut config: AppConfig = toml::from_str(
            r#"
[providers.deepseek.models.deepseek-v4-pro]
request_model_id = "deepseek-v4-pro-custom"
display_name = "Custom DeepSeek V4 Pro"
context_window = 500000
max_output_tokens = 100000
"#,
        )
        .expect("partial provider override should parse");
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");
        config
            .rebuild_effective_providers()
            .expect("provider override should resolve");

        let provider = config.provider("deepseek").expect("provider should exist");
        let model = provider
            .models
            .get("deepseek-v4-pro")
            .expect("overridden model should exist");
        assert_eq!(model.display_name, "Custom DeepSeek V4 Pro");
        assert_eq!(model.context_window, 500000);
        assert_eq!(
            model.request_model_id.as_deref(),
            Some("deepseek-v4-pro-custom")
        );
        assert!(provider.models.contains_key("deepseek-v4-flash"));
    }

    #[test]
    fn provider_fields_override_bundled_provider_without_removing_models() {
        let mut config: AppConfig = toml::from_str(
            r#"
[providers.deepseek]
display_name = "DeepSeek Mirror"
base_url = "https://mirror.example.com/v1"
user_agent = "mirror-client/1.0"
"#,
        )
        .expect("provider override should parse");
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");
        config
            .rebuild_effective_providers()
            .expect("provider override should resolve");

        let provider = config.provider("deepseek").expect("provider should exist");
        assert_eq!(provider.display_name, "DeepSeek Mirror");
        assert_eq!(provider.base_url, "https://mirror.example.com/v1");
        assert_eq!(provider.user_agent.as_deref(), Some("mirror-client/1.0"));
        assert!(provider.models.contains_key("deepseek-v4-pro"));

        let model = config
            .resolve_model_by_ids(&AuthStore::default(), "deepseek", "deepseek-v4-flash")
            .expect("overridden provider model should resolve");
        assert_eq!(model.user_agent.as_deref(), Some("mirror-client/1.0"));
    }

    #[test]
    fn invalid_provider_user_agent_is_rejected() {
        let mut config: AppConfig = toml::from_str(
            r#"
[providers.custom]
display_name = "Custom"
base_url = "https://example.com/v1"
user_agent = "invalid\nuser-agent"

[providers.custom.models.model]
display_name = "Model"
context_window = 100000
max_output_tokens = 10000
"#,
        )
        .expect("provider override should parse");
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");

        let error = config
            .rebuild_effective_providers()
            .expect_err("invalid provider User-Agent should be rejected");
        assert!(error.to_string().contains("invalid user_agent"));
    }

    #[test]
    fn serializing_partial_provider_override_does_not_expand_bundled_models() {
        let config: AppConfig = toml::from_str(
            r#"
[providers.deepseek.models.custom-model]
request_model_id = "deepseek-v4-custom"
display_name = "DeepSeek Custom"
context_window = 1048576
max_output_tokens = 262144
"#,
        )
        .expect("partial provider override should parse");
        let serialized = toml::to_string(&config).expect("config should serialize");
        assert!(serialized.contains("custom-model"));
        assert!(!serialized.contains("deepseek-v4-pro"));

        let reparsed: AppConfig = toml::from_str(&serialized).expect("config should round-trip");
        assert!(
            reparsed
                .providers
                .get("deepseek")
                .expect("provider should exist")
                .models
                .contains_key("custom-model")
        );
    }

    #[test]
    fn project_provider_overrides_merge_with_global_provider_overrides() {
        let mut config: AppConfig = toml::from_str(
            r#"
[providers.deepseek.models.global-model]
request_model_id = "deepseek-global"
display_name = "Global Model"
context_window = 100000
max_output_tokens = 10000
"#,
        )
        .expect("global provider override should parse");
        let overlay_toml = r#"
[providers.deepseek.models.project-model]
request_model_id = "deepseek-project"
display_name = "Project Model"
context_window = 200000
max_output_tokens = 20000
"#;
        let overlay: AppConfig =
            toml::from_str(overlay_toml).expect("project provider override should parse");
        config.merge(overlay, overlay_toml);
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");
        config
            .rebuild_effective_providers()
            .expect("provider override should resolve");

        let provider = config.provider("deepseek").expect("provider should exist");
        assert!(provider.models.contains_key("global-model"));
        assert!(provider.models.contains_key("project-model"));
        assert!(provider.models.contains_key("deepseek-v4-pro"));
    }

    #[test]
    fn new_provider_still_requires_provider_metadata() {
        let mut config: AppConfig = toml::from_str(
            r#"
[providers.custom.models.model]
request_model_id = "model"
display_name = "Model"
context_window = 100000
max_output_tokens = 10000
"#,
        )
        .expect("provider model should parse before resolution");
        config.bundled_providers = bundled_provider_catalog().expect("bundled catalog");
        let error = config
            .rebuild_effective_providers()
            .expect_err("new providers must define metadata");
        assert!(error.to_string().contains("display_name"));
    }

    #[test]
    fn setting_and_removing_user_provider_updates_effective_catalog() {
        let mut config = AppConfig::default();
        let provider = ProviderConfig {
            display_name: "Custom".to_owned(),
            base_url: "https://custom.example.com/v1".to_owned(),
            api_type: None,
            user_agent: None,
            models: BTreeMap::from([(
                "custom-model".to_owned(),
                crate::provider::ModelConfig {
                    display_name: "Custom Model".to_owned(),
                    context_window: 128_000,
                    max_output_tokens: 16_384,
                    api_type: None,
                    base_url: None,
                    temperature: Some(0.7),
                    system_prompt: None,
                    supports_streaming: true,
                    supports_images: false,
                    supports_parallel_tool_calls: true,
                    extra_body: None,
                    request_model_id: None,
                },
            )]),
        };

        config
            .set_user_provider("custom".to_owned(), provider)
            .expect("custom provider should be added");
        assert!(config.provider_ids().contains(&"custom".to_owned()));
        assert_eq!(config.provider("custom").unwrap().display_name, "Custom");

        config
            .remove_user_provider("custom")
            .expect("custom provider should be removed");
        assert!(!config.provider_ids().contains(&"custom".to_owned()));
        assert!(config.provider("custom").is_none());
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

    #[test]
    fn send_while_busy_defaults_to_queue() {
        let config = AppConfig::default();
        assert_eq!(config.ui.send_while_busy, SendWhileBusy::Queue);
        assert_eq!(config.ui.send_while_busy.as_str(), "queue");
    }

    #[test]
    fn send_while_busy_parses_lowercase_values() {
        let base = r#"
default_provider = "openai"
default_model = "gpt-4o-mini"
[ui]
sidebar_width = 40
welcome_width = 90
max_input_lines = 6
"#;
        let config: AppConfig = toml::from_str(&format!("{base}send_while_busy = \"steer\"\n"))
            .expect("config with steer should parse");
        assert_eq!(config.ui.send_while_busy, SendWhileBusy::Steer);

        let config: AppConfig = toml::from_str(&format!("{base}send_while_busy = \"queue\"\n"))
            .expect("config with queue should parse");
        assert_eq!(config.ui.send_while_busy, SendWhileBusy::Queue);
    }

    #[test]
    fn send_while_busy_serializes_round_trip() {
        let config = AppConfig::default();
        let serialized = toml::to_string(&config).expect("config should serialize");
        let parsed: AppConfig = toml::from_str(&serialized).expect("config should round-trip");
        assert_eq!(parsed.ui.send_while_busy, SendWhileBusy::Queue);
    }

    #[test]
    fn default_thinking_level_round_trip_and_overlay() {
        let toml_str = r#"
default_provider = "deepseek"
default_model = "deepseek-reasoner"
default_thinking_level = "deepseek:High"
"#;
        let config: AppConfig = toml::from_str(toml_str).expect("config should parse");
        assert_eq!(config.default_thinking_level, "deepseek:High");
    }
}
