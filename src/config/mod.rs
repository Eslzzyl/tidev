mod auth;
pub mod logging;
pub mod mcp;
mod paths;
mod provider;
pub mod reasoning;
pub mod sandbox;
mod tmp;
mod ui;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::prompts::{SessionMode, default_system_prompt, gateway_system_prompt};
use crate::theme::ThemeName;
use crate::tooling::ToolPermission;

use self::reasoning::{ThinkingLevelType, ThinkingMatcher};

pub use auth::{
    ActiveModel, AuthStore, ModelSummary,
    ProviderAuth, WebAuth,
};
pub use logging::LogConfig;
pub use mcp::{McpConfig, McpServerConfig};
pub use paths::ConfigPaths;
pub use provider::{ApiType, ModelConfig, ProviderConfig, ProviderSource};
pub use tmp::TmpConfig;
pub use ui::UiConfig;

pub use self::sandbox::SandboxConfig;

/// Per-model overrides for memory operations.
/// `None` = inherit from the session's active model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// Optional override for consolidation/reflection model.
    #[serde(default)]
    pub consolidation_model: Option<String>,
    /// Per-role thinking level overrides, keyed by role name (e.g. "consolidation").
    /// Format matches `ThinkingLevelType::to_string()` (e.g. "deepseek:High").
    #[serde(default)]
    pub thinking_levels: BTreeMap<String, String>,
    /// Whether to inject comprehensive memory context into the conversation.
    /// When true, memory context (observations, summaries, facts, procedures,
    /// slots, graph, insights) is injected into the first user message only.
    /// When false (default), memory is stored but never auto-injected.
    /// Like agentmemory's AGENTMEMORY_INJECT_CONTEXT.
    #[serde(default)]
    pub inject_context: bool,
    /// Whether to search and inject memory context relevant to the file
    /// being operated on, before each file tool call (read/write/edit/grep/glob).
    /// Like agentmemory's pre-tool-use enrich hook.
    #[serde(default)]
    pub enrich_tools: bool,
    /// Token budget for the first-turn context injection.
    #[serde(default = "default_context_token_budget")]
    pub context_token_budget: usize,
}

fn default_context_token_budget() -> usize {
    2000
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            consolidation_model: None,
            thinking_levels: BTreeMap::new(),
            inject_context: false,
            enrich_tools: false,
            context_token_budget: 2000,
        }
    }
}

// ---------------------------------------------------------------------------
// Web search configuration
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebSearchConfig {
    /// Default search provider (exa, brave, google, tavily).
    #[serde(default = "default_websearch_provider")]
    pub default_provider: String,
    /// Per-provider configuration overrides.
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
    /// Optional custom endpoint URL for the provider
    /// (e.g., a self-hosted Exa instance).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

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
    pub logging: LogConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "mcp::McpConfig::is_empty")]
    pub mcp: McpConfig,
    #[serde(default)]
    pub permissions: PermissionConfig,
    #[serde(default)]
    pub notifications: NotificationConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub rtk: RtkConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub tmp: TmpConfig,
    #[serde(default)]
    pub hooks: crate::hooks::HooksConfig,
    #[serde(default)]
    pub websearch: WebSearchConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
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
            logging: LogConfig::default(),
            providers: BTreeMap::new(),
            instructions: Vec::new(),
            skills: Vec::new(),
            mcp: McpConfig::default(),
            permissions: PermissionConfig::default(),
            notifications: NotificationConfig::default(),
            gateway: GatewayConfig::default(),
            rtk: RtkConfig::default(),
            agent: AgentConfig::default(),
            sandbox: SandboxConfig::default(),
            tmp: TmpConfig::default(),
            hooks: crate::hooks::HooksConfig::default(),
            websearch: WebSearchConfig::default(),
            memory: MemoryConfig::default(),
            bundled_providers: bundled_provider_catalog().unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GatewayConfig {
    #[serde(default)]
    pub telegram: TelegramGatewayConfig,
    #[serde(default)]
    pub qq: QQGatewayConfig,
    /// Default provider for gateway mode (falls back to global default if empty).
    #[serde(default)]
    pub default_provider: String,
    /// Default model for gateway mode (falls back to global default if empty).
    #[serde(default)]
    pub default_model: String,
    /// Enable session persistence for gateway mode.
    /// When enabled, sessions are persisted to SQLite and restored on restart.
    #[serde(default = "default_gateway_session_persistence")]
    pub session_persistence: bool,
}

fn default_gateway_session_persistence() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RtkConfig {
    /// Enable RTK (Rust Token Killer) to compress command outputs and save tokens.
    /// RTK must be installed on the system for this to work.
    /// When RTK is not installed, this setting is ignored.
    #[serde(default = "default_rtk_enabled")]
    pub enabled: bool,
    /// Whether RTK is installed on the system (runtime detection, not persisted).
    #[serde(skip)]
    pub installed: bool,
}

impl Default for RtkConfig {
    fn default() -> Self {
        Self {
            enabled: default_rtk_enabled(),
            installed: false,
        }
    }
}

/// Configuration for the multi-agent subsystem.
///
/// Controls delegation depth, per-agent session limits, and default sub-agent model.
/// Example `config.toml` entry:
///
/// ```toml
/// [agent]
/// enabled = true
/// default_subagent_model = "gpt-4o-mini"
/// max_depth = 3
/// max_sessions_per_agent = 2
///
/// [agent.models]
/// explorer = "gpt-4o-mini"
/// oracle = "gpt-4o"
/// fixer = "claude-3-5-sonnet"
/// ```
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Enable the multi-agent delegation system.
    #[serde(default = "default_agent_enabled")]
    pub enabled: bool,
    /// Default model ID for sub-agents when no per-agent override is set.
    /// If empty, sub-agents inherit the parent session's model.
    #[serde(default)]
    pub default_subagent_model: String,
    /// Default provider for sub-agents.
    #[serde(default)]
    pub default_subagent_provider: String,
    /// Maximum delegation chain depth (3 = orchestrator -> sub -> sub).
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// Maximum concurrent sub-agent tasks per parent session.
    #[serde(default = "default_max_sessions_per_agent")]
    pub max_sessions_per_agent: usize,
    /// Per-agent model overrides, keyed by agent type name.
    /// E.g. `explorer = "gpt-4o-mini"` or `explorer = "openai/gpt-4o-mini"`.
    /// Format: `"model_id"` or `"provider/model_id"`.
    #[serde(default)]
    pub models: BTreeMap<String, String>,
    /// Per-agent thinking level overrides, keyed by agent type name.
    /// E.g. `explorer = "deepseek:High"` or `fixer = "qwen:On"`.
    /// Format matches `ThinkingLevelType::to_string()` (e.g. "deepseek:High", "qwen:On").
    /// When set, this overrides the auto-detected thinking level for the agent's model.
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

impl AgentConfig {
    /// Given an agent type name (e.g. "explorer"), return the configured model override
    /// from `[agent.models]`, if any.
    ///
    /// Format can be `"model_id"` or `"provider/model_id"`.
    pub fn model_for(&self, agent_type: &str) -> Option<&str> {
        self.models.get(agent_type).map(|s| s.as_str())
    }

    /// Return the default sub-agent model string, if configured.
    pub fn default_model(&self) -> Option<&str> {
        let m = self.default_subagent_model.trim();
        if m.is_empty() { None } else { Some(m) }
    }
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

fn default_rtk_enabled() -> bool {
    true
}

/// Check if RTK is installed on the system.
pub fn check_rtk_installed() -> bool {
    std::process::Command::new("rtk")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelegramGatewayConfig {
    /// Enable Telegram polling gateway mode.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed Telegram user/chat identifiers.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Long-poll timeout in seconds passed to getUpdates.
    #[serde(default = "default_telegram_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}

fn default_telegram_poll_timeout_secs() -> u64 {
    30
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct QQGatewayConfig {
    /// Enable QQ Channel gateway mode.
    #[serde(default)]
    pub enabled: bool,
    /// Allowed QQ user/channel identifiers.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Whether to use sandbox environment.
    #[serde(default)]
    pub sandbox: bool,
}

impl Default for TelegramGatewayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowlist: Vec::new(),
            poll_timeout_secs: default_telegram_poll_timeout_secs(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationConfig {
    /// Enable notifications (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Notification method: "auto", "osc9", or "bel" (default: "auto")
    #[serde(default)]
    pub method: String,
    /// When to notify: "unfocused" or "always" (default: "unfocused")
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

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PermissionSettings {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub search: bool,
    #[serde(default)]
    pub write: bool,
    #[serde(default)]
    pub edit: bool,
    #[serde(default)]
    pub execute: bool,
    #[serde(default)]
    pub session: bool,
}

impl PermissionSettings {
    pub fn is_allowed(&self, permission: ToolPermission) -> bool {
        match permission {
            ToolPermission::Read => self.read,
            ToolPermission::Search => self.search,
            ToolPermission::Write => self.write,
            ToolPermission::Edit => self.edit,
            ToolPermission::Execute => self.execute,
            ToolPermission::Session => self.session,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default)]
    pub plan: PermissionSettings,
    #[serde(default)]
    pub build: PermissionSettings,
}

impl PermissionConfig {
    pub fn is_allowed(&self, mode: SessionMode, permission: ToolPermission) -> bool {
        match mode {
            SessionMode::Plan => self.plan.is_allowed(permission),
            SessionMode::Build => self.build.is_allowed(permission),
        }
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            plan: PermissionSettings {
                read: true,
                search: true,
                write: false,
                edit: false,
                execute: true,
                session: true,
            },
            build: PermissionSettings {
                read: true,
                search: true,
                write: true,
                edit: true,
                execute: true,
                session: true,
            },
        }
    }
}

impl AppConfig {
    pub fn load_or_create(paths: &ConfigPaths) -> Result<Self> {
        paths.ensure_directories()?;

        let rtk_installed = check_rtk_installed();

        if !paths.config_file.exists() {
            let example = Self::example_toml();
            std::fs::write(&paths.config_file, example)
                .with_context(|| format!("failed to write {}", paths.config_file.display()))?;
            let mut config: Self = toml::from_str(example).with_context(|| {
                format!("failed to parse generated {}", paths.config_file.display())
            })?;
            config.rtk.installed = rtk_installed;
            // If RTK is not installed, disable it by default
            if !rtk_installed {
                config.rtk.enabled = false;
            }
            return config.attach_bundled_providers();
        }

        let contents = std::fs::read_to_string(&paths.config_file)
            .with_context(|| format!("failed to read {}", paths.config_file.display()))?;
        let mut config: Self = toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", paths.config_file.display()))?;
        config.rtk.installed = rtk_installed;
        // If RTK is not installed, force disabled
        if !rtk_installed {
            config.rtk.enabled = false;
        }
        config.attach_bundled_providers()
    }

    /// Load the global config (`~/.config/tidev/config.toml`), then merge
    /// in a project-local override from `<workspace_root>/.tidev/config.toml`
    /// if it exists.
    ///
    /// Merge rules:
    /// - Scalar fields (strings, bools, numbers): project value wins.
    /// - Map fields (providers, mcp servers): project entries override global.
    /// - List fields (hooks, instructions, skills): **appended** — both global
    ///   and project entries are active, with project entries running last.
    /// - `hooks.disable_all_hooks`: project wins (project can opt out of all hooks).
    /// - Sub-config sections (`[ui]`, `[logging]`, etc.) are replaced only
    ///   when the project config explicitly contains that section.
    pub fn load_with_project_overlay(paths: &ConfigPaths, workspace_root: &Path) -> Result<Self> {
        let mut config = Self::load_or_create(paths)?;

        let project_config_path = workspace_root.join(".tidev/config.toml");
        if project_config_path.exists() {
            let project_toml = std::fs::read_to_string(&project_config_path)
                .with_context(|| format!("failed to read {}", project_config_path.display()))?;
            let keys = top_level_toml_keys(&project_toml);
            let project_config: Self = toml::from_str(&project_toml)
                .with_context(|| format!("failed to parse {}", project_config_path.display()))?;
            config.merge_overlay(project_config, &keys);
        }

        Ok(config)
    }

    /// Merge `overlay` into `self`, with `overlay` values taking priority.
    ///
    /// This is a **shallow merge** at the field level:
    /// - Scalar fields are replaced.
    /// - `BTreeMap` fields are extended (overlay entries override).
    /// - `Vec` fields are appended (both sets active).
    ///
    /// `keys_in_toml` lists the top-level TOML keys that were explicitly
    /// present in the overlay source file.  Sub-configs not in this list
    /// are left untouched (so a project config that only sets `[hooks]`
    /// won't accidentally zero out `[ui]`, etc.).
    pub(crate) fn merge_overlay(
        &mut self,
        overlay: AppConfig,
        keys_in_toml: &std::collections::BTreeSet<String>,
    ) {
        // Helper: check whether a TOML key was explicitly present.
        let has = |key: &str| keys_in_toml.contains(key);

        // ── Scalars: project wins ──────────────────────────────────────
        if has("default_provider") && !overlay.default_provider.is_empty() {
            self.default_provider = overlay.default_provider;
        }
        if has("default_model") && !overlay.default_model.is_empty() {
            self.default_model = overlay.default_model;
        }
        if has("theme") {
            self.theme = overlay.theme;
        }

        // ── Maps: project entries override ─────────────────────────────
        if has("providers") {
            self.providers.extend(overlay.providers);
        }
        // mcp is a sub-table with nested `servers` map; only merge if the
        // overlay explicitly contained an `[mcp]` section.
        if has("mcp") {
            self.mcp.servers.extend(overlay.mcp.servers);
        }

        // ── Lists: appended ────────────────────────────────────────────
        if has("instructions") {
            self.instructions.extend(overlay.instructions);
        }
        if has("skills") {
            self.skills.extend(overlay.skills);
        }

        // ── Sub-configs: full replacement when section is present ─────
        if has("ui") {
            self.ui = overlay.ui;
        }
        if has("logging") {
            self.logging = overlay.logging;
        }
        if has("permissions") {
            self.permissions = overlay.permissions;
        }
        if has("notifications") {
            self.notifications = overlay.notifications;
        }
        if has("gateway") {
            self.gateway = overlay.gateway;
        }
        if has("rtk") {
            self.rtk = overlay.rtk;
        }
        if has("agent") {
            self.agent = overlay.agent;
        }
        if has("sandbox") {
            self.sandbox = overlay.sandbox;
        }
        if has("tmp") {
            self.tmp = overlay.tmp;
        }

        // ── Hooks: append (project hooks run after global hooks) ─────
        if has("hooks") {
            if overlay.hooks.disable_all_hooks {
                self.hooks.disable_all_hooks = true;
            }
            self.hooks.post_tool_use.extend(overlay.hooks.post_tool_use);
        }
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

# Optional logging configuration.
# Set enabled = true to enable file logging.
# level can be: DEBUG, INFO, WARN, ERROR
# max_size_mb: max log file size before rotation (default: 10)
# max_files: number of rotated log files to keep (default: 5)
#[logging]
#enabled = false
#level = "INFO"
#max_size_mb = 10
#max_files = 5

# RTK (Rust Token Killer) configuration.
# When enabled, command outputs are compressed to save tokens.
# RTK must be installed on your system for this to work.
# If RTK is not installed, this setting is ignored.
#[rtk]
#enabled = true

# Optional permission settings by mode.
# By default plan mode allows read/search/session/execute (shell, but only for read-only commands) and build mode allows all permissions.
#permissions = { plan = { read = true, search = true, session = true, execute = true, write = false, edit = false }, build = { read = true, search = true, session = true, write = true, edit = true, execute = true } }

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

[mcp]

[ui]
sidebar_width = 30
welcome_width = 72
max_input_lines = 6
# Scroll speed multiplier (default: 3)
# scroll_speed = 3

[notifications]
# Enable notifications (default: true)
# enabled = true
# Notification method: "auto", "osc9", or "bel" (default: "auto")
# method = "auto"
# When to notify: "unfocused" or "always" (default: "unfocused")
# condition = "unfocused"

# Optional [sandbox] configuration for shell command sandboxing.
# When enabled, shell commands are restricted by the OS sandbox
# (Seatbelt on macOS, Bubblewrap/Landlock on Linux).
# mode can be: "workspace-write" (default) or "danger-full-access"
#[sandbox]
#mode = "workspace-write"

# Optional [tmp] configuration for managing temporary files.
# When auto_cleanup is enabled, tidev will remove its own temp files
# on startup that are older than max_age_hours.
#[tmp]
#auto_cleanup = false
#max_age_hours = 24

[gateway.telegram]
enabled = false
# allowlist can contain Telegram user IDs or chat IDs as strings.
allowlist = []
poll_timeout_secs = 30

# Web search provider configuration.
[websearch]
# Default search provider: exa, brave, google, tavily
default_provider = "exa"
"#
    }

    fn attach_bundled_providers(mut self) -> Result<Self> {
        self.bundled_providers = bundled_provider_catalog()?;
        Ok(self)
    }

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

    pub fn resolve_active_model_for_gateway(&self, auth: &AuthStore) -> Result<ActiveModel> {
        let provider_id = if !self.gateway.default_provider.is_empty() {
            &self.gateway.default_provider
        } else {
            &self.default_provider
        };
        let model_id = if !self.gateway.default_model.is_empty() {
            &self.gateway.default_model
        } else {
            &self.default_model
        };
        let mut model = self.resolve_model_by_ids(auth, provider_id, model_id)?;
        model.system_prompt = gateway_system_prompt();
        Ok(model)
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

        // Apply agent-specific thinking_level override if configured
        if let Some(tl_str) = self.agent.thinking_levels.get(agent_type) {
            model.thinking_level = ThinkingLevelType::from_string(tl_str);
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

    /// Return the configured memory model label for a role, if any.
    /// Roles: "consolidation".
    pub fn memory_model_label(&self, role: &str) -> Option<&str> {
        match role {
            "consolidation" => self.memory.consolidation_model.as_deref(),
            _ => None,
        }
    }

    /// Return a human-readable label for a memory model role.
    pub fn memory_model_display(&self, role: &str) -> String {
        self.memory_model_label(role)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "<inherit>".to_string())
    }

    /// Set the memory model override for a role and persist to config.
    pub fn set_memory_model(
        &mut self,
        paths: &ConfigPaths,
        role: &str,
        model_str: &str,
    ) -> Result<()> {
        if role == "consolidation" {
            if model_str.is_empty() {
                self.memory.consolidation_model = None;
                self.memory.thinking_levels.remove(role);
            } else {
                self.memory.consolidation_model = Some(model_str.to_string());
                // Clear thinking level when model changes
                self.memory.thinking_levels.remove(role);
            }
        }
        self.save(paths)
    }

    /// Set both the memory model override and thinking level for a role.
    /// Pass empty `thinking_level` to clear the override.
    pub fn set_memory_model_and_thinking(
        &mut self,
        paths: &ConfigPaths,
        role: &str,
        model_str: &str,
        thinking_level: &str,
    ) -> Result<()> {
        if role == "consolidation" {
            if model_str.is_empty() {
                self.memory.consolidation_model = None;
                self.memory.thinking_levels.remove(role);
            } else {
                self.memory.consolidation_model = Some(model_str.to_string());
                if thinking_level.is_empty() {
                    self.memory.thinking_levels.remove(role);
                } else {
                    self.memory.thinking_levels.insert(role.to_string(), thinking_level.to_string());
                }
            }
        }
        self.save(paths)
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

        let request_model_id = model
            .request_model_id
            .clone()
            .unwrap_or_else(|| model_id.to_string());

        // Determine thinking level with cascade fallback:
        // 1. Try request_model_id first (if present)
        // 2. Then try display_name (if request_model_id is None)
        // 3. Finally try model_id (if both above are None)
        let thinking_level = if let Some(ref rid) = model.request_model_id {
            ThinkingMatcher::match_for_model(rid)
        } else {
            let display_name = model.display_name.clone();
            if display_name.is_empty() {
                ThinkingMatcher::match_for_model(model_id)
            } else {
                ThinkingMatcher::match_for_model(&display_name)
            }
        };

        Ok(ActiveModel {
            provider_id: provider_id.to_string(),
            provider_display_name: provider.display_name.clone(),
            base_url: provider.base_url.clone(),
            api_type,
            model_id: model_id.to_string(),
            request_model_id,
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
            extra_body: model.extra_body.clone(),
            thinking_level,
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

/// Extract the top-level key names from raw TOML text.
///
/// This is used during config merging to determine which sections the
/// project config explicitly sets, so we don't accidentally overwrite
/// global config sections with default values.
pub(crate) fn top_level_toml_keys(toml_str: &str) -> std::collections::BTreeSet<String> {
    use std::collections::BTreeSet;

    let mut keys = BTreeSet::new();
    for line in toml_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Table header: [section] or [section.nested]
        // Array-of-tables header: [[array_name]]
        if trimmed.starts_with('[') {
            // Count opening brackets to distinguish [[ from [
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

        // Key-value at top level: key = value
        if let Some(eq_pos) = trimmed.find('=') {
            let before_eq = trimmed[..eq_pos].trim();
            // Skip quoted keys and inline tables
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

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
    fn connected_models_only_includes_providers_with_api_keys() {
        let mut config = AppConfig::default();
        config.providers.insert(
            "provider_one".to_string(),
            ProviderConfig {
                display_name: "Provider One".to_string(),
                base_url: "https://api.provider.one".to_string(),
                api_type: None,
                models: BTreeMap::from([(
                    "model-a".to_string(),
                    ModelConfig {
                        display_name: "Model A".to_string(),
                        context_window: 1024,
                        max_output_tokens: 1024,
                        temperature: Some(0.7),
                        system_prompt: None,
                        supports_streaming: true,
                        supports_images: false,
                        extra_body: None,
                        request_model_id: None,
                    },
                )]),
            },
        );

        config.providers.insert(
            "provider_two".to_string(),
            ProviderConfig {
                display_name: "Provider Two".to_string(),
                base_url: "https://api.provider.two".to_string(),
                api_type: None,
                models: BTreeMap::from([(
                    "model-b".to_string(),
                    ModelConfig {
                        display_name: "Model B".to_string(),
                        context_window: 1024,
                        max_output_tokens: 1024,
                        temperature: Some(0.7),
                        system_prompt: None,
                        supports_streaming: true,
                        supports_images: false,
                        extra_body: None,
                        request_model_id: None,
                    },
                )]),
            },
        );

        let mut auth = AuthStore::default();
        auth.set_api_key("provider_one", "sk-test-key");

        let connected = config.connected_models(&auth);

        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].provider_id, "provider_one");
        assert_eq!(connected[0].model_id, "model-a");
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

    #[test]
    fn default_permission_config_matches_mode_expectations() {
        let permissions = PermissionConfig::default();

        assert!(permissions.is_allowed(SessionMode::Plan, ToolPermission::Read));
        assert!(permissions.is_allowed(SessionMode::Plan, ToolPermission::Search));
        assert!(permissions.is_allowed(SessionMode::Plan, ToolPermission::Session));
        assert!(permissions.is_allowed(SessionMode::Plan, ToolPermission::Execute));
        assert!(!permissions.is_allowed(SessionMode::Plan, ToolPermission::Write));
        assert!(!permissions.is_allowed(SessionMode::Plan, ToolPermission::Edit));

        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Read));
        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Search));
        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Session));
        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Write));
        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Edit));
        assert!(permissions.is_allowed(SessionMode::Build, ToolPermission::Execute));
    }

    #[test]
    fn gateway_mode_uses_separate_system_prompt() {
        use crate::prompts::{default_system_prompt, gateway_system_prompt};

        let auth = AuthStore::default();
        let mut config = AppConfig::default();
        // Use bundled providers to avoid "unknown provider" error
        config.providers = config.bundled_providers.clone();
        config.default_provider = "deepseek".to_string();
        config.default_model = "deepseek-v4-flash".to_string();

        let tui_model = config.resolve_active_model(&auth).unwrap();
        let gateway_model = config.resolve_active_model_for_gateway(&auth).unwrap();

        // Verify gateway uses a different prompt than tui mode
        assert_eq!(
            gateway_model.system_prompt,
            gateway_system_prompt(),
            "gateway model should use gateway_system_prompt"
        );
        assert_ne!(
            gateway_model.system_prompt, tui_model.system_prompt,
            "gateway model should have different system_prompt from tui model"
        );

        // Verify tui mode still uses default prompt
        assert_eq!(
            tui_model.system_prompt,
            default_system_prompt(),
            "tui model should use default_system_prompt"
        );
    }
}
