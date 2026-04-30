pub mod builtin;
mod file_read_tracker;
mod registry;
mod skills;
mod tools;

use serde_json::Value;

pub use file_read_tracker::FileReadStamp;
pub use file_read_tracker::FileReadTracker;
pub use registry::ToolRegistry;
pub use skills::SkillCatalog;
pub use tools::TodoItem;
pub use tools::ToolArgs;
pub(crate) use tools::{QuestionArgs, QuestionInfo, SkillArgs, TaskArgs};

use crate::config::PermissionConfig;
use crate::prompts::SessionMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPermission {
    Read,
    Search,
    Write,
    Edit,
    Execute,
    Session,
}

impl ToolPermission {
    pub fn is_allowed_in(self, mode: SessionMode, permission_config: &PermissionConfig) -> bool {
        permission_config.is_allowed(mode, self)
    }

    pub fn needs_confirmation(self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolOrigin {
    Local,
    Mcp {
        server_name: String,
        tool_name: String,
    },
}

impl ToolOrigin {
    pub fn permission_key(&self, name: &str) -> String {
        match self {
            Self::Local => name.to_string(),
            Self::Mcp {
                server_name,
                tool_name,
            } => format!("mcp:{server_name}:{tool_name}"),
        }
    }

    pub fn permission_label(&self, display_name: &str, _name: &str) -> String {
        match self {
            Self::Local => display_name.to_string(),
            Self::Mcp {
                server_name,
                tool_name,
            } => format!("{server_name} / {tool_name} ({display_name})"),
        }
    }

    pub fn as_mcp(&self) -> Option<(&str, &str)> {
        match self {
            Self::Local => None,
            Self::Mcp {
                server_name,
                tool_name,
            } => Some((server_name.as_str(), tool_name.as_str())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub parameters: Value,
    pub permission: ToolPermission,
    pub origin: ToolOrigin,
}

impl ToolDefinition {
    pub fn new<Args>(
        name: &'static str,
        description: impl Into<String>,
        permission: ToolPermission,
    ) -> Self
    where
        Args: ToolArgs,
    {
        Self {
            name: name.to_string(),
            display_name: name.to_string(),
            description: description.into(),
            parameters: Args::schema(),
            permission,
            origin: ToolOrigin::Local,
        }
    }

    pub fn mcp_name(server_name: &str, tool_name: &str) -> String {
        let mut name = String::from("mcp__");
        name.push_str(&sanitize_name(server_name));
        name.push_str("__");
        name.push_str(&sanitize_name(tool_name));
        name
    }

    pub fn mcp(
        name: String,
        display_name: String,
        description: String,
        parameters: Value,
        permission: ToolPermission,
        server_name: String,
        tool_name: String,
    ) -> Self {
        Self {
            name,
            display_name,
            description,
            parameters,
            permission,
            origin: ToolOrigin::Mcp {
                server_name,
                tool_name,
            },
        }
    }

    pub fn needs_confirmation(&self) -> bool {
        self.permission.needs_confirmation()
    }

    pub fn permission_key(&self) -> String {
        self.origin.permission_key(&self.name)
    }

    pub fn permission_label(&self) -> String {
        self.origin.permission_label(&self.display_name, &self.name)
    }

    pub fn mcp_target(&self) -> Option<(&str, &str)> {
        self.origin.as_mcp()
    }
}

pub(crate) fn canonical_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "read" | "read_file" => Some("read"),
        "write" | "write_file" => Some("write"),
        "edit" => Some("edit"),
        "list" | "list_dir" => Some("list"),
        "glob" => Some("glob"),
        "grep" => Some("grep"),
        "bash" | "shell" => Some("bash"),
        "task" => Some("task"),
        "question" => Some("question"),
        "todowrite" | "todo" => Some("todowrite"),
        "skill" => Some("skill"),
        "memory" => Some("memory"),
        "websearch" => Some("websearch"),
        "webfetch" => Some("webfetch"),
        "apply_patch" => Some("apply_patch"),
        _ => None,
    }
}

fn sanitize_name(value: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;

    for ch in value.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, '-' | '_') {
            Some(ch)
        } else {
            None
        };

        match mapped {
            Some(ch) => {
                sanitized.push(ch);
                last_was_separator = false;
            }
            None if !last_was_separator => {
                sanitized.push('_');
                last_was_separator = true;
            }
            None => {}
        }
    }

    if sanitized.trim_matches('_').is_empty() {
        "mcp".to_string()
    } else {
        sanitized.trim_matches('_').to_string()
    }
}

pub use tools::execute_shell_tool_call;
