//! tidev's MCP integration layer.
//!
//! Connection management and result formatting live in `tidev-agent`. This
//! module maps tidev configuration and permission metadata onto that generic
//! client while retaining the public API used by the TUI and runtime.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tidev_agent::{McpRegistry, McpServerSpec, McpToolInfo};
use tidev_config::mcp::McpServerConfig;
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_tools::types::{ToolDefinition, ToolPermission};

use crate::mode::Mode;

pub use tidev_agent::{McpConnectionStatus, McpServerSummary};

/// Product-facing MCP manager.
#[derive(Clone, Debug)]
pub struct McpManager {
    registry: McpRegistry,
    workspace_root: PathBuf,
    configs: Arc<Mutex<BTreeMap<String, McpServerConfig>>>,
}

impl McpManager {
    /// Create a manager from tidev configuration.
    pub fn new(workspace_root: PathBuf, servers: BTreeMap<String, McpServerConfig>) -> Self {
        let specs = servers
            .iter()
            .map(|(name, config)| (name.clone(), to_agent_spec(&workspace_root, config)))
            .collect();
        Self {
            registry: McpRegistry::new(specs),
            workspace_root,
            configs: Arc::new(Mutex::new(servers)),
        }
    }

    pub fn has_connecting(&self) -> bool {
        self.registry.has_connecting()
    }

    pub async fn wait_until_ready(&self, timeout: std::time::Duration) -> Result<()> {
        self.registry.wait_until_ready(timeout).await
    }

    pub async fn refresh_all(&self) -> Result<()> {
        self.registry.refresh_all().await
    }

    pub async fn refresh_server(&self, name: &str) -> Result<()> {
        self.registry.refresh_server(name).await
    }

    pub async fn upsert_server(&self, name: String, config: McpServerConfig) -> Result<()> {
        let spec = to_agent_spec(&self.workspace_root, &config);
        self.configs.lock().unwrap().insert(name.clone(), config);
        self.registry.upsert_server(name, spec).await
    }

    pub async fn remove_server(&self, name: &str) -> Result<()> {
        self.registry.remove_server(name).await?;
        self.configs.lock().unwrap().remove(name);
        Ok(())
    }

    pub fn server_config(&self, name: &str) -> Option<McpServerConfig> {
        self.configs.lock().unwrap().get(name).cloned()
    }

    pub fn has_server(&self, name: &str) -> bool {
        self.registry.has_server(name)
    }

    pub async fn disconnect_server(&self, name: &str) -> Result<()> {
        self.registry.disconnect_server(name).await
    }

    pub async fn toggle_server(&self, name: &str) -> Result<()> {
        self.registry.toggle_server(name).await
    }

    pub fn summaries(&self) -> Vec<McpServerSummary> {
        self.registry.summaries()
    }

    pub fn available_definitions(&self, mode: Mode) -> Vec<ToolDefinition> {
        self.registry
            .all_tools()
            .into_iter()
            .filter(|tool| mode != Mode::Plan || tool.read_only)
            .map(|tool| to_host_definition(&tool))
            .collect()
    }

    pub fn all_definitions(&self) -> Vec<ToolDefinition> {
        self.registry
            .all_tools()
            .into_iter()
            .map(|tool| to_host_definition(&tool))
            .collect()
    }

    pub fn definition_for(&self, tool_name: &str) -> Option<ToolDefinition> {
        self.registry
            .tool_info_for(tool_name)
            .map(|tool| to_host_definition(&tool))
    }

    pub fn can_execute(&self, tool_name: &str, mode: Mode) -> bool {
        self.definition_for(tool_name).is_some_and(|definition| {
            definition
                .permission
                .allowed_in_read_only(mode == Mode::Plan)
        })
    }

    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolExecutionResult> {
        self.registry.execute_call(call).await
    }

    /// Access the generic registry for core-owned adapters.
    #[cfg(test)]
    pub(crate) fn agent_registry(&self) -> &McpRegistry {
        &self.registry
    }
}

fn to_agent_spec(workspace_root: &Path, config: &McpServerConfig) -> McpServerSpec {
    match config {
        McpServerConfig::Stdio {
            command,
            args,
            cwd,
            env,
            disabled,
        } => McpServerSpec::Stdio {
            command: command.clone(),
            args: args.clone(),
            cwd: cwd
                .as_deref()
                .map(|cwd| resolve_workspace_path(workspace_root, cwd)),
            env: env.clone(),
            disabled: *disabled,
        },
        McpServerConfig::Http {
            url,
            headers,
            disabled,
        } => McpServerSpec::Http {
            url: url.clone(),
            headers: headers.clone(),
            disabled: *disabled,
        },
        McpServerConfig::Sse {
            url,
            headers,
            disabled,
        } => McpServerSpec::Sse {
            url: url.clone(),
            headers: headers.clone(),
            disabled: *disabled,
        },
    }
}

fn resolve_workspace_path(workspace_root: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    }
}

fn to_host_definition(tool: &McpToolInfo) -> ToolDefinition {
    let permission = match tool.tool_name.as_str() {
        "websearch" => ToolPermission::Search,
        "webfetch" => ToolPermission::Read,
        _ if tool.read_only => ToolPermission::Read,
        _ => ToolPermission::Execute,
    };
    ToolDefinition::mcp(
        tool.definition.name.clone(),
        tool.definition.display_name.clone(),
        tool.definition.description.clone(),
        tool.definition.parameters.clone(),
        permission,
        tool.server_name.clone(),
        tool.tool_name.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio_config() -> McpServerConfig {
        McpServerConfig::Stdio {
            command: "node".into(),
            args: vec!["server.js".into()],
            cwd: None,
            env: BTreeMap::new(),
            disabled: false,
        }
    }

    #[test]
    fn manager_preserves_config_and_summary_order() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([
                ("b".to_string(), stdio_config()),
                ("a".to_string(), stdio_config()),
            ]),
        );
        assert_eq!(manager.summaries()[0].name, "a");
        assert_eq!(
            manager.summaries()[1].status,
            McpConnectionStatus::Disconnected
        );
        assert_eq!(manager.server_config("a").unwrap().kind_label(), "stdio");
    }

    #[test]
    fn relative_stdio_cwd_is_resolved_for_agent() {
        let manager = McpManager::new(
            PathBuf::from("/workspace"),
            BTreeMap::from([("srv".to_string(), stdio_config())]),
        );
        let spec = manager.agent_registry().server_spec("srv").unwrap();
        assert!(matches!(spec, McpServerSpec::Stdio { .. }));
    }

    #[test]
    fn http_headers_round_trip_to_agent_spec() {
        let config = McpServerConfig::Http {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::from([("Authorization".into(), "Bearer token".into())]),
            disabled: true,
        };
        let spec = to_agent_spec(Path::new("/workspace"), &config);

        assert!(matches!(
            spec,
            McpServerSpec::Http { url, headers, disabled }
                if url == "https://example.com/mcp"
                    && headers.get("Authorization") == Some(&"Bearer token".to_string())
                    && disabled
        ));
    }
}
