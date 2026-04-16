use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;

use crate::config::McpServerConfig;
use crate::mcp::McpServerSummary;

use super::App;

#[derive(Clone, Debug)]
pub struct McpPanelState {
    pub selected_index: usize,
    pub(crate) editor: Option<McpServerEditorState>,
}

impl McpPanelState {
    pub fn new() -> Self {
        Self {
            selected_index: 0,
            editor: None,
        }
    }

    pub fn move_selection(&mut self, items: &[McpPanelItem], delta: isize) {
        if items.is_empty() {
            self.selected_index = 0;
            return;
        }

        let len = items.len() as isize;
        let current = self.selected_index.min(items.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    pub fn selected_item<'a>(&self, items: &'a [McpPanelItem]) -> Option<&'a McpPanelItem> {
        items.get(self.selected_index)
    }

    pub fn begin_create_editor(&mut self, previous_query: String) {
        self.editor = Some(McpServerEditorState::new_create(previous_query));
    }

    pub fn begin_edit_editor(
        &mut self,
        previous_query: String,
        original_name: String,
        config: &McpServerConfig,
    ) {
        self.editor = Some(McpServerEditorState::new_edit(
            previous_query,
            original_name,
            config,
        ));
    }

    pub fn clear_editor(&mut self) -> Option<String> {
        self.editor.take().map(|editor| editor.previous_query)
    }
}

impl Default for McpPanelState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct McpPanelItem {
    pub summary: McpServerSummary,
}

impl McpPanelItem {
    pub fn is_selectable(&self) -> bool {
        true
    }
}

impl App {
    pub(crate) fn open_mcp_panel(&mut self, initial_query: String) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.composer.clear();
        self.composer
            .set_placeholder("Search MCP servers by name or transport");
        self.composer.set_text(initial_query);

        let mut panel = McpPanelState::new();
        let items = self.mcp_panel_items();
        panel.selected_index = first_selectable_index(&items).unwrap_or(0);
        self.mcp_panel = Some(panel);
    }

    pub(crate) fn close_mcp_panel(&mut self) {
        if self.mcp_panel.take().is_some() {
            self.composer.clear();
            self.composer
                .set_placeholder("Ask TiDev about your code, task, or question...");
        }
    }

    pub(crate) fn reset_mcp_panel_selection(&mut self) {
        let items = self.mcp_panel_items();
        if let Some(panel) = &mut self.mcp_panel {
            panel.selected_index = first_selectable_index(&items).unwrap_or(0);
        }
    }

    pub(crate) fn handle_mcp_panel_key(
        &mut self,
        key: KeyEvent,
        runtime: &tokio::runtime::Runtime,
    ) -> Result<()> {
        let Some(mut panel) = self.mcp_panel.clone() else {
            return Ok(());
        };

        if let Some(mut editor) = panel.editor.clone() {
            let input_before = self.composer.text().to_string();

            match key.code {
                KeyCode::Esc => {
                    let previous_query = editor.previous_query.clone();
                    panel.clear_editor();
                    self.mcp_panel = Some(panel);
                    self.composer.clear();
                    self.composer.set_text(previous_query);
                    self.composer
                        .set_placeholder("Search MCP servers by name or transport");
                    self.reset_mcp_panel_selection();
                }
                KeyCode::Enter | KeyCode::Tab => match editor.apply_input(self.composer.text()) {
                    Ok(Some(result)) => match self.apply_mcp_server_draft(runtime, result) {
                        Ok(()) => {
                            let previous_query = editor.previous_query.clone();
                            panel.clear_editor();
                            self.mcp_panel = Some(panel);
                            self.composer.clear();
                            self.composer.set_text(previous_query);
                            self.composer
                                .set_placeholder("Search MCP servers by name or transport");
                            self.reset_mcp_panel_selection();
                        }
                        Err(error) => {
                            self.last_notice = Some(error.to_string());
                            panel.editor = Some(editor);
                            self.mcp_panel = Some(panel);
                            self.composer.set_text(input_before);
                        }
                    },
                    Ok(None) => {
                        let next_input = editor.current_input();
                        let next_placeholder = editor.placeholder();
                        panel.editor = Some(editor);
                        self.mcp_panel = Some(panel);
                        self.composer.clear();
                        self.composer.set_text(next_input);
                        self.composer.set_placeholder(next_placeholder);
                    }
                    Err(error) => {
                        self.last_notice = Some(error.to_string());
                        panel.editor = Some(editor);
                        self.mcp_panel = Some(panel);
                        self.composer.set_text(input_before);
                    }
                },
                _ => {
                    let _ = self.composer.handle_key_with_history(key, false);
                    panel.editor = Some(editor);
                    self.mcp_panel = Some(panel);
                }
            }

            return Ok(());
        }

        let items = self.mcp_panel_items();

        match key.code {
            KeyCode::Up => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Down => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Enter => {
                if let Some(selected) = panel.selected_item(&items) {
                    let name = selected.summary.name.clone();
                    let result = match selected.summary.status {
                        crate::mcp::McpConnectionStatus::Connected
                        | crate::mcp::McpConnectionStatus::Connecting => {
                            runtime.block_on(self.tools.disconnect_mcp_server(&name))
                        }
                        _ => runtime.block_on(self.tools.toggle_mcp_server(&name)),
                    };

                    match result {
                        Ok(()) => {
                            self.last_notice = Some(format!("Updated MCP server '{name}'"));
                        }
                        Err(error) => {
                            self.last_notice = Some(error.to_string());
                        }
                    }
                }
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                if let Some(selected) = panel.selected_item(&items) {
                    let name = selected.summary.name.clone();
                    if let Err(error) = runtime.block_on(self.tools.refresh_mcp_server(&name)) {
                        self.last_notice = Some(error.to_string());
                    } else {
                        self.last_notice = Some(format!("Refreshed MCP server '{name}'"));
                    }
                }
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let previous_query = self.composer.text().to_string();
                self.open_new_mcp_server_editor(previous_query);
            }
            KeyCode::Char('e') | KeyCode::Char('E') => {
                if let Some(selected) = panel.selected_item(&items) {
                    let previous_query = self.composer.text().to_string();
                    self.open_existing_mcp_server_editor(
                        previous_query,
                        selected.summary.name.clone(),
                    )?;
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                if let Some(selected) = panel.selected_item(&items) {
                    let name = selected.summary.name.clone();
                    self.remove_mcp_server_from_editor(runtime, &name)?;
                }
            }
            KeyCode::Esc => {
                self.close_mcp_panel();
            }
            KeyCode::Tab => {}
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, -1);
                self.mcp_panel = Some(next_panel);
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                let mut next_panel = panel;
                next_panel.move_selection(&items, 1);
                self.mcp_panel = Some(next_panel);
            }
            _ => {
                let previous_query = self.composer.text().to_string();
                let _ = self.composer.handle_key_with_history(key, false);
                if self.composer.text() != previous_query {
                    self.reset_mcp_panel_selection();
                }
            }
        }

        Ok(())
    }

    pub(crate) fn mcp_panel_items(&self) -> Vec<McpPanelItem> {
        let query = self.composer.text().trim().to_ascii_lowercase();
        self.tools
            .mcp_summaries()
            .into_iter()
            .filter(|summary| mcp_panel_matches_query(&query, summary))
            .map(|summary| McpPanelItem { summary })
            .collect()
    }

    pub(crate) fn open_new_mcp_server_editor(&mut self, previous_query: String) {
        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();

        let mut panel = self.mcp_panel.clone().unwrap_or_default();
        panel.begin_create_editor(previous_query);
        self.mcp_panel = Some(panel);
        self.composer.clear();
        if let Some(editor) = self
            .mcp_panel
            .as_ref()
            .and_then(|panel| panel.editor.as_ref())
        {
            self.composer.set_text(editor.current_input());
            self.composer.set_placeholder(editor.placeholder());
        }
    }

    pub(crate) fn open_existing_mcp_server_editor(
        &mut self,
        previous_query: String,
        server_name: String,
    ) -> Result<()> {
        let Some(config) = self.config.mcp.servers.get(&server_name).cloned() else {
            self.last_notice = Some(format!("MCP server '{server_name}' does not exist"));
            return Ok(());
        };

        self.command_palette.clear();
        self.connect_dialog = None;
        self.theme_panel = None;
        self.model_panel = None;
        self.session_panel = None;
        self.at_mention.clear();
        self.draft_attachments.clear();

        let mut panel = self.mcp_panel.clone().unwrap_or_default();
        panel.begin_edit_editor(previous_query, server_name, &config);
        self.mcp_panel = Some(panel);
        self.composer.clear();
        if let Some(editor) = self
            .mcp_panel
            .as_ref()
            .and_then(|panel| panel.editor.as_ref())
        {
            self.composer.set_text(editor.current_input());
            self.composer.set_placeholder(editor.placeholder());
        }

        Ok(())
    }

    pub(crate) fn remove_mcp_server_from_editor(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        name: &str,
    ) -> Result<()> {
        if self.config.mcp.servers.remove(name).is_none() {
            self.last_notice = Some(format!("MCP server '{name}' does not exist"));
            return Ok(());
        }

        self.config.save(&self.paths)?;

        if self.tools.mcp_manager().has_server(name) {
            runtime.block_on(self.tools.mcp_manager().remove_server(name))?;
        }

        self.last_notice = Some(format!("Removed MCP server '{name}'"));
        self.reset_mcp_panel_selection();
        Ok(())
    }

    pub(crate) fn apply_mcp_server_draft(
        &mut self,
        runtime: &tokio::runtime::Runtime,
        result: McpServerDraftResult,
    ) -> Result<()> {
        let McpServerDraftResult {
            original_name,
            name,
            config,
        } = result;

        if original_name.as_deref() != Some(name.as_str())
            && self.config.mcp.servers.contains_key(&name)
        {
            bail!("MCP server '{name}' already exists");
        }

        let mut previous_name = None;
        if let Some(original_name) = original_name.clone()
            && original_name != name
        {
            previous_name = Some(original_name);
            self.config
                .mcp
                .servers
                .remove(previous_name.as_ref().expect("name set"));
        }

        self.config.mcp.servers.insert(name.clone(), config.clone());
        self.config.save(&self.paths)?;

        if let Some(previous_name) = previous_name
            && self.tools.mcp_manager().has_server(&previous_name) {
                runtime.block_on(self.tools.mcp_manager().remove_server(&previous_name))?;
            }

        runtime.block_on(self.tools.mcp_manager().upsert_server(name.clone(), config))?;

        self.last_notice = Some(format!("Saved MCP server '{name}'"));
        self.reset_mcp_panel_selection();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpServerKind {
    Stdio,
    Http,
    Sse,
}

impl McpServerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Sse => "sse",
        }
    }

    pub fn from_input(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stdio" => Ok(Self::Stdio),
            "http" | "streamable_http" | "streamable-http" => Ok(Self::Http),
            "sse" => Ok(Self::Sse),
            other => bail!("unknown MCP transport '{other}'"),
        }
    }

    pub fn is_stdio(self) -> bool {
        matches!(self, Self::Stdio)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum McpEditorStep {
    Name,
    Kind,
    Endpoint,
    Args,
    Cwd,
    Env,
}

impl McpEditorStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "Server name",
            Self::Kind => "Transport kind",
            Self::Endpoint => "Command or URL",
            Self::Args => "Arguments",
            Self::Cwd => "Working directory",
            Self::Env => "Environment variables",
        }
    }

    pub fn placeholder(self, kind: McpServerKind) -> String {
        match self {
            Self::Name => "server name".to_string(),
            Self::Kind => "stdio, http, or sse".to_string(),
            Self::Endpoint => {
                if kind.is_stdio() {
                    "npx".to_string()
                } else {
                    "https://example.com/mcp".to_string()
                }
            }
            Self::Args => "space-separated arguments, e.g. -y package-name".to_string(),
            Self::Cwd => "optional working directory".to_string(),
            Self::Env => "KEY=VALUE per line".to_string(),
        }
    }

    pub fn help(self, kind: McpServerKind) -> String {
        match self {
            Self::Name => "Use lowercase letters, numbers, '-', or '_' only.".to_string(),
            Self::Kind => {
                "Choose stdio for a local process or http/sse for a remote endpoint.".to_string()
            }
            Self::Endpoint => {
                if kind.is_stdio() {
                    "Command used to launch the MCP server process.".to_string()
                } else {
                    "Remote MCP URL. Use https:// for hosted servers.".to_string()
                }
            }
            Self::Args => "Use shell-style quoting if you need spaces in an argument.".to_string(),
            Self::Cwd => "Leave blank to use the workspace root.".to_string(),
            Self::Env => {
                "Leave blank if the server does not need custom environment variables.".to_string()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct McpServerDraft {
    pub name: String,
    pub kind: McpServerKind,
    pub endpoint: String,
    pub args: String,
    pub cwd: String,
    pub env: String,
}

impl McpServerDraft {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            kind: McpServerKind::Stdio,
            endpoint: String::new(),
            args: String::new(),
            cwd: String::new(),
            env: String::new(),
        }
    }

    pub fn from_config(name: impl Into<String>, config: &McpServerConfig) -> Self {
        match config {
            McpServerConfig::Stdio {
                command,
                args,
                cwd,
                env,
            } => Self {
                name: name.into(),
                kind: McpServerKind::Stdio,
                endpoint: command.clone(),
                args: args.join(" "),
                cwd: cwd.clone().unwrap_or_default(),
                env: format_env_lines(env),
            },
            McpServerConfig::Http { url } => Self {
                name: name.into(),
                kind: McpServerKind::Http,
                endpoint: url.clone(),
                args: String::new(),
                cwd: String::new(),
                env: String::new(),
            },
            McpServerConfig::Sse { url } => Self {
                name: name.into(),
                kind: McpServerKind::Sse,
                endpoint: url.clone(),
                args: String::new(),
                cwd: String::new(),
                env: String::new(),
            },
        }
    }

    pub fn current_value(&self, step: McpEditorStep) -> String {
        match step {
            McpEditorStep::Name => self.name.clone(),
            McpEditorStep::Kind => self.kind.label().to_string(),
            McpEditorStep::Endpoint => self.endpoint.clone(),
            McpEditorStep::Args => self.args.clone(),
            McpEditorStep::Cwd => self.cwd.clone(),
            McpEditorStep::Env => self.env.clone(),
        }
    }

    pub fn apply_step(&mut self, step: McpEditorStep, input: &str) -> Result<()> {
        let value = input.trim();

        match step {
            McpEditorStep::Name => {
                self.name = normalize_identifier(value, "MCP server name")?;
            }
            McpEditorStep::Kind => {
                self.kind = McpServerKind::from_input(value)?;
            }
            McpEditorStep::Endpoint => {
                self.endpoint = if self.kind.is_stdio() {
                    non_empty(value, "command")?.to_string()
                } else {
                    normalize_remote_url(value)?
                };
            }
            McpEditorStep::Args => {
                self.args = value.to_string();
            }
            McpEditorStep::Cwd => {
                self.cwd = value.to_string();
            }
            McpEditorStep::Env => {
                self.env = value.to_string();
            }
        }

        Ok(())
    }

    pub fn into_config(self) -> Result<McpServerConfig> {
        match self.kind {
            McpServerKind::Stdio => Ok(McpServerConfig::Stdio {
                command: non_empty(&self.endpoint, "command")?.to_string(),
                args: parse_args(&self.args)?,
                cwd: optional_string(self.cwd),
                env: parse_env_map(&self.env)?,
            }),
            McpServerKind::Http => Ok(McpServerConfig::Http {
                url: normalize_remote_url(&self.endpoint)?,
            }),
            McpServerKind::Sse => Ok(McpServerConfig::Sse {
                url: normalize_remote_url(&self.endpoint)?,
            }),
        }
    }

    pub fn summary_text(&self) -> String {
        let mut lines = vec![
            format!(
                "name: {}",
                if self.name.is_empty() {
                    "<empty>"
                } else {
                    self.name.as_str()
                }
            ),
            format!("transport: {}", self.kind.label()),
            format!(
                "endpoint: {}",
                if self.endpoint.is_empty() {
                    "<empty>"
                } else {
                    self.endpoint.as_str()
                }
            ),
        ];

        if self.kind.is_stdio() {
            let env_count = if self.env.is_empty() {
                "0".to_string()
            } else {
                self.env.lines().count().to_string()
            };

            lines.push(format!(
                "args: {}",
                if self.args.is_empty() {
                    "<none>"
                } else {
                    self.args.as_str()
                }
            ));
            lines.push(format!(
                "cwd: {}",
                if self.cwd.is_empty() {
                    "<workspace root>"
                } else {
                    self.cwd.as_str()
                }
            ));
            lines.push(format!("env lines: {env_count}"));
        }

        lines.join("\n")
    }
}

#[derive(Clone, Debug)]
pub(crate) struct McpServerEditorState {
    pub previous_query: String,
    pub original_name: Option<String>,
    pub step: McpEditorStep,
    pub draft: McpServerDraft,
}

impl McpServerEditorState {
    pub fn new_create(previous_query: String) -> Self {
        Self {
            previous_query,
            original_name: None,
            step: McpEditorStep::Name,
            draft: McpServerDraft::new(),
        }
    }

    pub fn new_edit(
        previous_query: String,
        original_name: String,
        config: &McpServerConfig,
    ) -> Self {
        Self {
            previous_query,
            original_name: Some(original_name.clone()),
            step: McpEditorStep::Name,
            draft: McpServerDraft::from_config(original_name, config),
        }
    }

    pub fn title(&self) -> String {
        match &self.original_name {
            Some(name) => format!("Edit MCP server '{name}'"),
            None => "Add MCP server".to_string(),
        }
    }

    pub fn current_input(&self) -> String {
        self.draft.current_value(self.step)
    }

    pub fn placeholder(&self) -> String {
        self.step.placeholder(self.draft.kind)
    }

    pub fn help(&self) -> String {
        self.step.help(self.draft.kind)
    }

    pub fn step_label(&self) -> &'static str {
        self.step.label()
    }

    pub fn apply_input(&mut self, input: &str) -> Result<Option<McpServerDraftResult>> {
        self.draft.apply_step(self.step, input)?;

        if let Some(next_step) = self.next_step() {
            self.step = next_step;
            return Ok(None);
        }

        let result = McpServerDraftResult {
            original_name: self.original_name.clone(),
            name: self.draft.name.clone(),
            config: self.draft.clone().into_config()?,
        };

        Ok(Some(result))
    }

    fn next_step(&self) -> Option<McpEditorStep> {
        match self.step {
            McpEditorStep::Name => Some(McpEditorStep::Kind),
            McpEditorStep::Kind => Some(McpEditorStep::Endpoint),
            McpEditorStep::Endpoint if self.draft.kind.is_stdio() => Some(McpEditorStep::Args),
            McpEditorStep::Endpoint => None,
            McpEditorStep::Args => Some(McpEditorStep::Cwd),
            McpEditorStep::Cwd => Some(McpEditorStep::Env),
            McpEditorStep::Env => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct McpServerDraftResult {
    pub original_name: Option<String>,
    pub name: String,
    pub config: McpServerConfig,
}

fn non_empty<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    if value.is_empty() {
        bail!("{label} cannot be empty");
    }

    Ok(value)
}

fn normalize_identifier(value: &str, label: &str) -> Result<String> {
    let normalized = value.trim().to_ascii_lowercase().replace([' ', '.'], "-");

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

fn normalize_remote_url(value: &str) -> Result<String> {
    let value = value.trim();

    if value.is_empty() {
        bail!("URL cannot be empty");
    }

    if !(value.starts_with("http://") || value.starts_with("https://")) {
        bail!("URL must start with http:// or https://");
    }

    Ok(value.to_string())
}

fn parse_args(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Vec::new());
    }

    shlex::split(value).with_context(|| "failed to parse arguments")
}

fn parse_env_map(value: &str) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();

    for (line_number, line) in value.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            bail!("environment line {} must use KEY=VALUE", line_number + 1);
        };

        let key = key.trim();
        let raw_value = raw_value.trim();

        if key.is_empty() {
            bail!("environment line {} is missing a key", line_number + 1);
        }

        env.insert(key.to_string(), raw_value.to_string());
    }

    Ok(env)
}

fn optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn format_env_lines(env: &BTreeMap<String, String>) -> String {
    env.iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_selectable_index(items: &[McpPanelItem]) -> Option<usize> {
    items.iter().position(McpPanelItem::is_selectable)
}

fn mcp_panel_matches_query(query: &str, summary: &McpServerSummary) -> bool {
    if query.is_empty() {
        return true;
    }

    let name = summary.name.to_ascii_lowercase();
    let kind = summary.kind.to_ascii_lowercase();
    let status = summary.status_text().to_ascii_lowercase();
    let tool_count = summary.tool_count.to_string();

    name.contains(query)
        || kind.contains(query)
        || status.contains(query)
        || tool_count.contains(query)
}
