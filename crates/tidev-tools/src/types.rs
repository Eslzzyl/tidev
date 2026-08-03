//! Tool type definitions for the tidev tool system.
//!
//! This module defines the core types for the tool system:
//! - [`TodoItem`]: a task/todo item within a session
//! - [`ToolDefinition`]: metadata describing a tool (name, description, parameters, permissions)
//! - [`ToolOrigin`]: whether a tool is local or comes from MCP
//! - [`ToolPermission`]: permission level required by a tool (hardcoded per mode)
//! - [`ToolArgs`] trait + `tool_args!` macro: parameter schema definition

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// TodoItem
// ---------------------------------------------------------------------------

/// A task/todo item within a session.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TodoItem {
    pub content: String,
    pub status: String,
}

// ---------------------------------------------------------------------------
// ToolPermission
// ---------------------------------------------------------------------------

/// The permission level required to use a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Returns whether this permission level is allowed in a read-only session.
    ///
    /// Hardcoded: Plan mode only allows read-only and non-destructive tools;
    /// Build mode allows everything.
    pub fn allowed_in_read_only(&self, read_only: bool) -> bool {
        if read_only {
            matches!(
                self,
                Self::Read | Self::Search | Self::Execute | Self::Session
            )
        } else {
            true
        }
    }
}

// ---------------------------------------------------------------------------
// ToolOrigin
// ---------------------------------------------------------------------------

/// Identifies where a tool originates from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolOrigin {
    /// A built-in tool implemented by tidev itself.
    Local,
    /// A tool exposed by an MCP server.
    Mcp {
        server_name: String,
        tool_name: String,
    },
}

impl ToolOrigin {
    /// If this tool is backed by an MCP server, return the target reference.
    pub fn as_mcp(&self) -> Option<(&str, &str)> {
        match self {
            Self::Local => None,
            Self::Mcp {
                server_name,
                tool_name,
            } => Some((server_name, tool_name)),
        }
    }
}

// ---------------------------------------------------------------------------
// ToolDefinition
// ---------------------------------------------------------------------------

/// Metadata describing a tool that the LLM can call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub display_name: String,
    pub description: String,
    /// JSON Schema object describing the tool's parameters.
    pub parameters: Value,
    pub permission: ToolPermission,
    pub origin: ToolOrigin,
}

impl ToolDefinition {
    /// Create a new built-in tool definition.
    ///
    /// The `Args` type parameter determines the parameter schema via [`ToolArgs`].
    pub fn new<Args: ToolArgs>(
        name: &'static str,
        description: impl Into<String>,
        permission: ToolPermission,
    ) -> Self {
        Self {
            name: name.to_string(),
            display_name: name.to_string(),
            description: description.into(),
            parameters: Args::schema(),
            permission,
            origin: ToolOrigin::Local,
        }
    }

    /// Construct an MCP tool name in the `mcp__{server}__{tool}` format.
    pub fn mcp_name(server_name: &str, tool_name: &str) -> String {
        let mut name = String::from("mcp__");
        name.push_str(&sanitize_mcp_name(server_name));
        name.push_str("__");
        name.push_str(&sanitize_mcp_name(tool_name));
        name
    }

    /// Create a new MCP-backed tool definition.
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

    /// If this tool is backed by an MCP server, return the target info.
    pub fn mcp_target(&self) -> Option<(&str, &str)> {
        self.origin.as_mcp()
    }
}

/// Sanitize a string for use in `mcp__{sanitized}__{sanitized}` names.
fn sanitize_mcp_name(value: &str) -> String {
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

// ---------------------------------------------------------------------------
// ToolArgs trait
// ---------------------------------------------------------------------------

/// A type that can produce its own JSON Schema for use as a tool's parameter
/// definition, and can be deserialized from the corresponding JSON.
pub trait ToolArgs: for<'de> Deserialize<'de> + Serialize {
    /// Returns a JSON Schema object describing the tool's parameters.
    fn schema() -> Value;
}

// ---------------------------------------------------------------------------
// Tool field type macros
// ---------------------------------------------------------------------------

macro_rules! tool_field_type {
    (string($desc:literal)) => {
        String
    };
    (optional_string($desc:literal)) => {
        Option<String>
    };
    (boolean($desc:literal)) => {
        bool
    };
    (optional_boolean($desc:literal)) => {
        Option<bool>
    };
    (integer($desc:literal)) => {
        i64
    };
    (optional_integer($desc:literal)) => {
        Option<i64>
    };
    (array($item_ty:ty, $desc:literal)) => {
        Vec<$item_ty>
    };
    (optional_array($item_ty:ty, $desc:literal)) => {
        Option<Vec<$item_ty>>
    };
    (object($item_ty:ty, $desc:literal)) => {
        $item_ty
    };
    (optional_object($item_ty:ty, $desc:literal)) => {
        Option<$item_ty>
    };
}

macro_rules! tool_field_schema {
    (string($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("string".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_string($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("string".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (boolean($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("boolean".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_boolean($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("boolean".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (integer($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("integer".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_integer($desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("integer".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
    (array($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("array".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        schema.insert("items".to_string(), <$item_ty as ToolArgs>::schema());
        (Value::Object(schema), true)
    }};
    (optional_array($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("array".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        schema.insert("items".to_string(), <$item_ty as ToolArgs>::schema());
        (Value::Object(schema), false)
    }};
    (object($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), true)
    }};
    (optional_object($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
        (Value::Object(schema), false)
    }};
}

/// Generate a struct with a [`ToolArgs`] implementation from a concise DSL.
///
/// # Example
///
/// ```ignore
/// tool_args! {
///     pub struct ReadArgs {
///         file_path: string("Path to read"),
///         offset: optional_integer("Line number to start from"),
///     }
/// }
/// ```
#[macro_export]
macro_rules! tool_args {
    (
        $(#[$struct_meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $(#[$field_meta:meta])*
                $field:ident : $kind:ident ( $($kind_args:tt)+ )
            ),* $(,)?
        }
    ) => {
        $(#[$struct_meta])*
        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
        #[serde(deny_unknown_fields)]
        $vis struct $name {
            $(
                $(#[$field_meta])*
                pub $field: tool_field_type!($kind($($kind_args)+)),
            )*
        }

        impl $crate::types::ToolArgs for $name {
            fn schema() -> Value {
                let mut properties = serde_json::Map::new();
                let mut required = Vec::new();

                $(
                    let (field_schema, is_required) = tool_field_schema!($kind($($kind_args)+));
                    properties.insert(stringify!($field).to_string(), field_schema);
                    if is_required {
                        required.push(Value::String(stringify!($field).to_string()));
                    }
                )*

                let mut schema = serde_json::Map::new();
                schema.insert("type".to_string(), Value::String("object".to_string()));
                schema.insert("properties".to_string(), Value::Object(properties));
                schema.insert("additionalProperties".to_string(), Value::Bool(false));
                if !required.is_empty() {
                    schema.insert("required".to_string(), Value::Array(required));
                }

                Value::Object(schema)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Tool argument structs
// ---------------------------------------------------------------------------

tool_args! {
    /// Read a file or directory.
    pub struct ReadArgs {
        file_path: string("Path to read (relative to workspace root, or absolute)"),
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to read"),
    }
}

tool_args! {
    /// Write a text file.
    pub struct WriteArgs {
        file_path: string("Path to write (relative to workspace root, or absolute)"),
        content: string("File contents to write"),
    }
}

tool_args! {
    /// Edit a file by replacing text.
    pub struct EditArgs {
        file_path: string("Path to edit (relative to workspace root, or absolute)"),
        #[serde(alias = "old_string")]
        old_text: string("Text to replace; must match exactly"),
        #[serde(alias = "new_string")]
        new_text: string("Replacement text"),
        replace_all: optional_boolean("Replace all matches instead of only the first"),
    }
}

tool_args! {
    /// Apply a codex-format patch to one or more files.
    pub struct ApplyPatchArgs {
        patch_text: string(r#"The full patch text that describes all changes to be made. Must use the codex patch format:

*** Begin Patch
[one or more file operations]
*** End Patch

File operations:
  *** Add File: <path>    — create a new file
  *** Delete File: <path> — delete a file
  *** Update File: <path> — modify a file (with @@ hunks)

Within hunks, use ' ' (context), '-' (delete), '+' (insert) prefixes."#),
    }
}

tool_args! {
    /// Find files matching a glob pattern.
    pub struct GlobArgs {
        pattern: string("Glob pattern to match against workspace-relative paths"),
        path: optional_string("Directory path to search (relative to workspace root, or absolute)"),
    }
}

tool_args! {
    /// Search files with a regular expression.
    pub struct GrepArgs {
        pattern: string("Regular expression to search for in file contents"),
        path: optional_string("Directory path to search (relative to workspace root, or absolute)"),
        include: optional_string("File glob to include in the search"),
    }
}

tool_args! {
    /// Run a shell command.
    pub struct ShellArgs {
        command: string("Shell command to execute from the workspace root"),
        description: optional_string("Clear, concise description of what this command does"),
        timeout: optional_integer("Timeout in milliseconds (default: 120000, max: 600000)"),
    }
}

tool_args! {
    /// Delegate a subtask to a sub-agent.
    pub struct TaskArgs {
        description: string("Short title for the task"),
        prompt: string("Task prompt to give the subagent"),
        subagent_type: string("Subagent type: explorer, librarian, oracle, fixer"),
        task_id: optional_string("Resume a previous task by session ID (UUID)"),
    }
}

tool_args! {
    /// Update the todo list.
    pub struct TodoWriteArgs {
        todos: array(crate::types::TodoItem, "The updated todo list"),
    }
}

impl ToolArgs for TodoItem {
    fn schema() -> Value {
        let mut properties = serde_json::Map::new();
        properties.insert(
            "content".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Brief description of the task",
            }),
        );
        properties.insert(
            "status".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "Current status of the task: pending, in_progress, completed",
            }),
        );
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("object".to_string()));
        schema.insert("properties".to_string(), Value::Object(properties));
        schema.insert("additionalProperties".to_string(), Value::Bool(false));
        Value::Object(schema)
    }
}

tool_args! {
    /// Load a skill by name.
    pub struct SkillArgs {
        name: string("Skill name to load"),
    }
}

tool_args! {
    /// An option within a question.
    pub struct QuestionOption {
        label: string("Display text for the option"),
        description: optional_string("Optional explanation of the option"),
    }
}

tool_args! {
    /// A single question to ask the user.
    pub struct QuestionInfo {
        question: string("Complete question"),
        header: string("Short label for the question"),
        options: array(crate::types::QuestionOption, "Available choices"),
        multiple: optional_boolean("Allow selecting multiple choices"),
        custom: optional_boolean("Allow typing a custom answer"),
    }
}

tool_args! {
    /// Ask the user questions during execution.
    pub struct QuestionArgs {
        questions: array(crate::types::QuestionInfo, "Questions to ask"),
    }
}

tool_args! {
    /// Search the web.
    pub struct WebSearchArgs {
        query: string("Web search query"),
        num_results: optional_integer("Number of search results to return"),
        search_type: optional_string("Search type: auto, fast, or deep"),
        offset: optional_integer("Number of search results to skip from the start (0-indexed)"),
    }
}

tool_args! {
    /// Fetch a web page.
    pub struct WebFetchArgs {
        url: string("The URL to fetch"),
        format: optional_string("Output format: text, markdown, or html"),
        timeout: optional_integer("Timeout in seconds (max 120)"),
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to return"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowed_in_read_only() {
        assert!(ToolPermission::Read.allowed_in_read_only(true));
        assert!(ToolPermission::Search.allowed_in_read_only(true));
        assert!(ToolPermission::Execute.allowed_in_read_only(true));
        assert!(ToolPermission::Session.allowed_in_read_only(true));
        assert!(!ToolPermission::Write.allowed_in_read_only(true));
        assert!(!ToolPermission::Edit.allowed_in_read_only(true));

        assert!(ToolPermission::Read.allowed_in_read_only(false));
        assert!(ToolPermission::Write.allowed_in_read_only(false));
        assert!(ToolPermission::Edit.allowed_in_read_only(false));
        assert!(ToolPermission::Execute.allowed_in_read_only(false));
    }

    #[test]
    fn test_read_args_schema() {
        let schema = ReadArgs::schema();
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(props.contains_key("file_path"));
        assert!(props.contains_key("offset"));
        assert!(props.contains_key("limit"));
        let required = schema.get("required").and_then(|v| v.as_array()).unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0], "file_path");
    }

    #[test]
    fn test_write_args_schema() {
        let schema = WriteArgs::schema();
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .unwrap();
        assert!(props.contains_key("file_path"));
        assert!(props.contains_key("content"));
    }

    // ── MCP type tests ────────────────────────────────────────────────

    #[test]
    fn test_tool_origin_mcp_as_mcp() {
        let mcp_origin = ToolOrigin::Mcp {
            server_name: "srv".into(),
            tool_name: "tool".into(),
        };
        let local_origin = ToolOrigin::Local;

        assert_eq!(mcp_origin.as_mcp(), Some(("srv", "tool")));
        assert_eq!(local_origin.as_mcp(), None);
    }

    #[test]
    fn test_tool_definition_mcp_name() {
        assert_eq!(
            ToolDefinition::mcp_name("my-server", "do-thing"),
            "mcp__my-server__do-thing"
        );
    }

    #[test]
    fn test_tool_definition_mcp_name_sanitize() {
        assert_eq!(ToolDefinition::mcp_name("a b!c", "x@y"), "mcp__a_b_c__x_y");
    }

    #[test]
    fn test_tool_definition_mcp_name_empty() {
        // Everything sanitized away falls back to "mcp"
        let name = ToolDefinition::mcp_name("!!!", "@@@");
        assert_eq!(name, "mcp__mcp__mcp");
    }

    #[test]
    fn test_tool_definition_mcp_constructor() {
        let params = serde_json::json!({"type": "object"});
        let def = ToolDefinition::mcp(
            "mcp__srv__tool".into(),
            "Srv Tool".into(),
            "Does something".into(),
            params.clone(),
            ToolPermission::Execute,
            "srv".into(),
            "tool".into(),
        );

        assert_eq!(def.name, "mcp__srv__tool");
        assert_eq!(def.display_name, "Srv Tool");
        assert_eq!(def.description, "Does something");
        assert_eq!(def.parameters, params);
        assert_eq!(def.permission, ToolPermission::Execute);
        assert_eq!(def.mcp_target(), Some(("srv", "tool")));
    }

    #[test]
    fn test_tool_definition_mcp_target_local() {
        let def = ToolDefinition::new::<ReadArgs>("read", "Read a file", ToolPermission::Read);
        assert_eq!(def.mcp_target(), None);
    }

    #[test]
    fn test_tool_definition_mcp_target_mcp() {
        let def = ToolDefinition::mcp(
            "mcp__s__t".into(),
            "display".into(),
            "desc".into(),
            serde_json::json!({}),
            ToolPermission::Execute,
            "s".into(),
            "t".into(),
        );
        assert_eq!(def.mcp_target(), Some(("s", "t")));
    }
}
