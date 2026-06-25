use serde_json::Value;

/// Tool definition for MCP-provided tools.
#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub parameters: Value,
    pub permission: tidev_types::types::ToolPermission,
    /// The MCP server name (extracted from the tool name).
    pub server_name: String,
    /// The original remote tool name (before MCP prefix).
    pub remote_tool_name: String,
}

impl ToolDefinition {
    pub fn mcp(server_name: &str, remote_tool_name: &str, description: String, parameters: Value) -> Self {
        Self {
            name: Self::mcp_name(server_name, remote_tool_name),
            display_name: format!("[MCP:{}] {}", server_name, remote_tool_name),
            description,
            parameters,
            permission: tidev_types::types::ToolPermission::Read,
            server_name: server_name.to_string(),
            remote_tool_name: remote_tool_name.to_string(),
        }
    }

    pub fn mcp_name(server_name: &str, tool_name: &str) -> String {
        format!("mcp__{}__{}", server_name, tool_name)
    }

    pub fn is_mcp(tool_name: &str) -> bool {
        tool_name.starts_with("mcp__")
    }

    /// Parse an MCP-prefixed tool name back into (server_name, tool_name).
    pub fn parse_mcp_name(tool_name: &str) -> Option<(&str, &str)> {
        let name = tool_name.strip_prefix("mcp__")?;
        let (server, tool) = name.split_once("__")?;
        Some((server, tool))
    }

    /// Return the (server_name, remote_tool_name) for this MCP tool.
    pub fn mcp_target(&self) -> Option<(String, String)> {
        Some((self.server_name.clone(), self.remote_tool_name.clone()))
    }

    pub fn permission_key(&self) -> String {
        format!("mcp:{}:{}", self.server_name, self.remote_tool_name)
    }

    pub fn permission_label(&self) -> String {
        format!("{} / {}", self.server_name, self.remote_tool_name)
    }
}
