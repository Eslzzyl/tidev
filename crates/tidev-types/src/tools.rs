//! Tool type definitions shared across the tidev workspace.
//!
//! This module defines the core types for the tool system:
//! - [`ToolDefinition`]: metadata describing a tool (name, description, parameters, permissions)
//! - [`ToolOrigin`]: whether a tool is local or comes from MCP
//! - [`ToolPermission`]: permission level required by a tool
//! - [`ToolArgs`] trait + `tool_args!` macro: parameter schema definition
//! - [`FileReadStamp`]: records a file read for edit-before-read enforcement
//! - [`canonical_tool_name()`]: normalizes tool name aliases

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
// FileReadStamp
// ---------------------------------------------------------------------------

/// Records that a file was read at a point in time, along with its metadata.
///
/// Used to enforce the rule that a file must be read before editing, and
/// detects if the file has been modified since it was last read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadStamp {
    pub read_at: chrono::DateTime<chrono::Utc>,
    pub mtime: Option<i64>,
    pub size: Option<i64>,
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
    /// Returns true if this permission should prompt for user confirmation.
    pub fn needs_confirmation(&self) -> bool {
        matches!(self, Self::Write | Self::Edit | Self::Execute)
    }
}

// ---------------------------------------------------------------------------
// PermissionSettings / PermissionConfig
// ---------------------------------------------------------------------------

/// A set of permission booleans for a single mode (Plan / Build).
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

/// Maps each [`SessionMode`](crate::prompts::SessionMode) to its permission set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default)]
    pub plan: PermissionSettings,
    #[serde(default)]
    pub build: PermissionSettings,
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

impl PermissionConfig {
    pub fn is_allowed(
        &self,
        mode: crate::prompts::SessionMode,
        permission: ToolPermission,
    ) -> bool {
        match mode {
            crate::prompts::SessionMode::Plan => self.plan.is_allowed(permission),
            crate::prompts::SessionMode::Build => self.build.is_allowed(permission),
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
}

impl ToolOrigin {
    pub fn permission_key(&self, name: &str) -> String {
        match self {
            Self::Local => name.to_string(),
        }
    }

    pub fn permission_label(&self, display_name: &str, _name: &str) -> String {
        match self {
            Self::Local => display_name.to_string(),
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

    pub fn needs_confirmation(&self) -> bool {
        self.permission.needs_confirmation()
    }

    pub fn permission_key(&self) -> String {
        self.origin.permission_key(&self.name)
    }

    pub fn permission_label(&self) -> String {
        self.origin.permission_label(&self.display_name, &self.name)
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
        (Value::Object(schema), true)
    }};
    (optional_array($item_ty:ty, $desc:literal)) => {{
        let mut schema = serde_json::Map::new();
        schema.insert("type".to_string(), Value::String("array".to_string()));
        schema.insert("description".to_string(), Value::String($desc.to_string()));
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

        impl $crate::tools::ToolArgs for $name {
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
    pub struct BashArgs {
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
        subagent_type: string("Subagent type: explorer, librarian, oracle, designer, fixer"),
        task_id: optional_string("Resume a previous task by session ID (UUID)"),
    }
}

tool_args! {
    /// Update the todo list.
    pub struct TodoWriteArgs {
        todos: array(crate::tools::TodoItem, "The updated todo list"),
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
        options: array(crate::tools::QuestionOption, "Available choices"),
        multiple: optional_boolean("Allow selecting multiple choices"),
        custom: optional_boolean("Allow typing a custom answer"),
    }
}

tool_args! {
    /// Ask the user questions during execution.
    pub struct QuestionArgs {
        questions: array(crate::tools::QuestionInfo, "Questions to ask"),
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
// canonical_tool_name
// ---------------------------------------------------------------------------

/// Map a tool name (possibly an alias) to its canonical form.
pub fn canonical_tool_name(tool_name: &str) -> Option<&'static str> {
    match tool_name {
        "read" | "read_file" => Some("read"),
        "write" | "write_file" => Some("write"),
        "edit" => Some("edit"),
        "glob" => Some("glob"),
        "grep" => Some("grep"),
        "bash" | "shell" => Some("bash"),
        "task" => Some("task"),
        "question" => Some("question"),
        "todowrite" | "todo" => Some("todowrite"),
        "skill" => Some("skill"),
        "websearch" => Some("websearch"),
        "webfetch" => Some("webfetch"),
        "apply_patch" => Some("apply_patch"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_tool_name() {
        assert_eq!(canonical_tool_name("read"), Some("read"));
        assert_eq!(canonical_tool_name("read_file"), Some("read"));
        assert_eq!(canonical_tool_name("write_file"), Some("write"));
        assert_eq!(canonical_tool_name("shell"), Some("bash"));
        assert_eq!(canonical_tool_name("todo"), Some("todowrite"));
        assert_eq!(canonical_tool_name("unknown"), None);
    }

    #[test]
    fn test_permission_config_default() {
        let config = PermissionConfig::default();
        assert!(config.is_allowed(crate::prompts::SessionMode::Plan, ToolPermission::Read));
        assert!(!config.is_allowed(crate::prompts::SessionMode::Plan, ToolPermission::Write));
        assert!(config.is_allowed(crate::prompts::SessionMode::Build, ToolPermission::Write));
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
}
