mod registry;
mod schema;
mod tools;

use serde_json::Value;

pub use registry::ToolRegistry;
pub use schema::ToolArgs;
pub use tools::TodoItem;

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
    pub fn is_allowed_in(self, mode: SessionMode) -> bool {
        match mode {
            SessionMode::Plan => matches!(self, Self::Read | Self::Search | Self::Session),
            SessionMode::Build => true,
        }
    }

    pub fn needs_confirmation(self) -> bool {
        matches!(
            self,
            Self::Write | Self::Edit | Self::Execute | Self::Session
        )
    }
}

#[derive(Clone, Debug)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
    pub permission: ToolPermission,
}

impl ToolDefinition {
    pub fn new<Args>(
        name: &'static str,
        description: &'static str,
        permission: ToolPermission,
    ) -> Self
    where
        Args: ToolArgs,
    {
        Self {
            name,
            description,
            parameters: Args::schema(),
            permission,
        }
    }

    pub fn needs_confirmation(&self) -> bool {
        self.permission.needs_confirmation()
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
        "todowrite" | "todo" => Some("todowrite"),
        _ => None,
    }
}

pub use tools::{
    execute_shell_tool_call, list_dir, read_file, run_shell, run_shell_with_cancel, write_file,
};
