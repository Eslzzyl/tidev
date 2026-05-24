use std::collections::HashSet;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use tidev_types::TodoItem;

use crate::tooling::{FileReadTracker, SkillCatalog};
use tidev_storage::SessionStore;
use tidev_session::session::{ToolCall, ToolExecutionResult};

use super::ToolDefinition;
use super::builtin;

pub trait ToolArgs: for<'de> Deserialize<'de> + Serialize {
    fn schema() -> Value;
}

/// Decode tool arguments with an enhanced error message that includes the
/// JSON Schema field descriptions so the model can self-correct.
pub(crate) fn decode_tool_args<Args: ToolArgs>(tool_name: &str, arguments: Value) -> Result<Args> {
    let schema = Args::schema();
    serde_json::from_value::<Args>(arguments).map_err(|e| {
        let expected = describe_schema(&schema);
        anyhow::anyhow!(
            "failed to decode arguments for tool '{}': {}\nExpected fields: {}",
            tool_name,
            e,
            expected
        )
    })
}

/// Convert a JSON Schema object to a human-readable field description string.
/// Example output: `path: string, old_text: string, new_text: string, replace_all (optional): boolean`
fn describe_schema(schema: &Value) -> String {
    let properties = match schema.get("properties").and_then(|v| v.as_object()) {
        Some(props) => props,
        None => return String::new(),
    };

    let required: HashSet<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut fields: Vec<String> = Vec::with_capacity(properties.len());
    for (name, prop_schema) in properties {
        let field_type = prop_schema
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("any");
        let optional = if required.contains(name.as_str()) {
            String::new()
        } else {
            " (optional)".to_string()
        };
        fields.push(format!("{}{}: {}", name, optional, field_type));
    }
    fields.join(", ")
}

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
        let mut schema = <$item_ty as ToolArgs>::schema();
        if let Value::Object(ref mut object_schema) = schema {
            object_schema.insert("description".to_string(), Value::String($desc.to_string()));
        }
        (schema, true)
    }};
    (optional_object($item_ty:ty, $desc:literal)) => {{
        let mut schema = <$item_ty as ToolArgs>::schema();
        if let Value::Object(ref mut object_schema) = schema {
            object_schema.insert("description".to_string(), Value::String($desc.to_string()));
        }
        (schema, false)
    }};
}

macro_rules! tool_args {
    (
        $(#[$struct_meta:meta])*
        $vis:vis struct $name:ident {
            $(
                $field:ident : $kind:ident ( $($kind_args:tt)+ )
            ),* $(,)?
        }
    ) => {
        $(#[$struct_meta])*
        #[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
        #[serde(deny_unknown_fields)]
        $vis struct $name {
            $(
                pub $field: tool_field_type!($kind($($kind_args)+)),
            )*
        }

        impl ToolArgs for $name {
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

tool_args! {
    pub struct ReadArgs {
        file_path: string("Path to read (relative to workspace root, or absolute)"),
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to read"),
    }
}

tool_args! {
    pub struct WriteArgs {
        file_path: string("Path to write (relative to workspace root, or absolute)"),
        content: string("File contents to write"),
    }
}

tool_args! {
    pub struct EditArgs {
        file_path: string("Path to edit (relative to workspace root, or absolute)"),
        old_text: string("Text to replace; must match exactly"),
        new_text: string("Replacement text"),
        replace_all: optional_boolean("Replace all matches instead of only the first"),
    }
}

tool_args! {
    pub struct ApplyPatchArgs {
        patch_text: string("The full patch text that describes all changes to be made"),
    }
}

tool_args! {
    pub struct GlobArgs {
        pattern: string("Glob pattern to match against workspace-relative paths"),
        path: optional_string("Directory path to search (relative to workspace root, or absolute)"),
    }
}

tool_args! {
    pub struct GrepArgs {
        pattern: string("Regular expression to search for in file contents"),
        path: optional_string("Directory path to search (relative to workspace root, or absolute)"),
        include: optional_string("File glob to include in the search"),
    }
}

tool_args! {
    pub struct BashArgs {
        command: string("Shell command to execute from the workspace root"),
        description: optional_string("Clear, concise description of what this command does"),
        timeout: optional_integer("Timeout in milliseconds (default: 120000, max: 600000)"),
    }
}

tool_args! {
    pub struct TaskArgs {
        description: string("Short title for the task"),
        prompt: string("Task prompt to give the subagent"),
        subagent_type: string("Subagent type: explorer, librarian, oracle, designer, fixer"),
        task_id: optional_string("Resume a previous task by session ID (UUID)"),
    }
}

tool_args! {
    pub struct TodoWriteArgs {
        todos: array(TodoItem, "The updated todo list"),
    }
}

// TodoItem is defined in tidev-types; we implement ToolArgs here
// since the trait is crate-local to tidev-engine.
impl ToolArgs for tidev_types::TodoItem {
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
    pub struct SkillArgs {
        name: string("Skill name to load"),
    }
}

tool_args! {
    pub struct MemoryArgs {
        operation: string("Operation: remember, search, list, read, forget, observations. Slots: slot_list, slot_get, slot_set, slot_append, slot_delete. Eviction: evict."),
        memory_type: optional_string("Memory type: user, project, feedback, reference (optional, defaults to fact)"),
        title: optional_string("Memory title (optional, defaults to first 80 chars of content)"),
        content: optional_string("Memory content in markdown (required for remember)"),
        tags: optional_string("Comma-separated tags or JSON array of tags (optional)"),
        query: optional_string("Search query (required for search)"),
        memory_id: optional_string("Memory UUID (required for read, forget)"),
    }
}

tool_args! {
    pub struct QuestionOption {
        label: string("Display text for the option"),
        description: optional_string("Optional explanation of the option"),
    }
}

tool_args! {
    pub struct QuestionInfo {
        question: string("Complete question"),
        header: string("Short label for the question"),
        options: array(QuestionOption, "Available choices"),
        multiple: optional_boolean("Allow selecting multiple choices"),
        custom: optional_boolean("Allow typing a custom answer"),
    }
}

tool_args! {
    pub struct QuestionArgs {
        questions: array(QuestionInfo, "Questions to ask"),
    }
}

tool_args! {
    pub struct WebSearchArgs {
        query: string("Web search query"),
        num_results: optional_integer("Number of search results to return"),
        search_type: optional_string("Search type: auto, fast, or deep"),
        offset: optional_integer("Number of search results to skip from the start (0-indexed)"),
    }
}

tool_args! {
    pub struct WebFetchArgs {
        url: string("The URL to fetch"),
        format: optional_string("Output format: text, markdown, or html"),
        timeout: optional_integer("Timeout in seconds (max 120)"),
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to return"),
    }
}

pub(super) fn parse_arguments<Args>(tool_name: &str, arguments: Value) -> Result<Args>
where
    Args: ToolArgs,
{
    decode_tool_args::<Args>(tool_name, arguments)
}

pub(super) fn tool_definitions(skill_description: String) -> Vec<ToolDefinition> {
    builtin::definitions(skill_description)
}

#[allow(clippy::too_many_arguments)]
pub fn execute_shell_tool_call(
    workspace_root: &Path,
    call: &ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
    sandbox_policy: Option<crate::sandbox::SandboxPolicy>,
    cancelled: Arc<AtomicBool>,
    session_id: Uuid,
    event_tx: Option<UnboundedSender<tidev_session::session::BackendEvent>>,
) -> Result<tidev_session::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let result = builtin::exec::execute_tool_call_with_cancel(
        workspace_root,
        &call.name,
        arguments,
        max_output_bytes,
        rtk_enabled,
        cancelled,
        sandbox_policy,
        session_id,
        event_tx,
    )?;
    Ok(tidev_session::session::ToolExecutionResult::new(result.output)
        .with_rtk_rewritten(result.rtk_rewritten)
        .with_sandbox(result.sandboxed, result.sandbox_type)
        .with_sandbox_denied(result.sandbox_denied))
}

#[allow(dead_code)]
fn truncate_in_place(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value.truncate(end);
    value.push_str("\n[truncated]");
}

#[allow(dead_code)]
fn truncate_to_limit(mut value: String, max_bytes: usize) -> String {
    truncate_in_place(&mut value, max_bytes);
    value
}

#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_tool_call(
    workspace_root: &Path,
    config_dir: &Path,
    skills: &SkillCatalog,
    file_read_tracker: &Arc<FileReadTracker>,
    store: &SessionStore,
    session_id: Uuid,
    call: &ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
    memory_store: &Arc<crate::memory::MemoryStore>,
    mode: tidev_types::prompts::SessionMode,
) -> Result<ToolExecutionResult> {
    // Pre-execution checks for file read tracking
    let tool_name = crate::tooling::canonical_tool_name(&call.name);

    // Check if we need to track a read or verify prior read
    match tool_name {
        Some("read") => {
            // After read executes successfully, we will record it
        }
        Some("edit") | Some("write") | Some("apply_patch") => {
            // Extract file path and check if file exists (only check existing files)
            if let Some(file_path) = extract_file_path_for_check(call, tool_name.unwrap())
                && let Ok(path) = resolve_workspace_path_safe(workspace_root, &file_path)
                && path.exists()
            {
                // Check if file was read and not modified
                if let Err(e) = file_read_tracker.check_read(session_id, &path) {
                    return Err(anyhow::anyhow!("{}", e));
                }
            }
        }
        _ => {}
    }

    let mut result = builtin::execute_tool_call(
        &builtin::ToolContext {
            workspace_root,
            config_dir,
            skills,
            store,
            session_id,
            max_output_bytes,
            rtk_enabled,
            memory_store,
            mode,
            allow_outside: false,
            sensitive_file_approved: false,
            sandbox_policy: None,
            web_search_config: &crate::config::WebSearchConfig::default(),
            auth_store: &crate::config::AuthStore::default(),
            event_tx: None,
        },
        call,
    )?;

    // Post-execution: record file reads
    if let Some("read") = tool_name
        && let Some(file_path) = extract_file_path_for_check(call, "read")
        && let Ok(absolute_path) = resolve_workspace_path_safe(workspace_root, &file_path)
    {
        // Record the read (ignore errors - non-critical)
        let _ = file_read_tracker.record_read(store, session_id, &absolute_path);
    }

    let original_output_len = result.output.len();
    result.output = truncate_to_limit(result.output, max_output_bytes);

    // If output was truncated, mark it in metadata
    if result.output.len() < original_output_len {
        result.metadata.truncated = Some(true);
    }

    Ok(result)
}

/// Extract file path from tool arguments for read/edit/write operations
#[allow(dead_code)]
fn extract_file_path_for_check(call: &ToolCall, tool_name: &str) -> Option<String> {
    let arguments: Value = serde_json::from_str(&call.arguments).ok()?;

    match tool_name {
        "read" | "edit" | "write" => arguments
            .get("path")
            .and_then(|v| v.as_str())
            .map(String::from),
        "apply_patch" => {
            // apply_patch doesn't have a path field in args, it parses from patch_text
            // For now, we skip apply_patch check as it would require parsing the patch
            None
        }
        _ => None,
    }
}

/// Resolve workspace path without failing (returns Err on failure instead of panicking)
#[allow(dead_code)]
fn resolve_workspace_path_safe(
    workspace_root: &Path,
    candidate: &str,
) -> Result<std::path::PathBuf, ()> {
    use std::path::Component;

    let candidate_path = std::path::Path::new(candidate);
    let mut resolved = if candidate_path.is_absolute() {
        std::path::PathBuf::new()
    } else {
        workspace_root.to_path_buf()
    };

    for component in candidate_path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
        }
    }

    if !resolved.starts_with(workspace_root) {
        return Err(());
    }

    Ok(resolved)
}

/// Extract the file path from a unified diff patch string.
/// Returns the file path if found, or None if parsing fails.
pub fn extract_file_path_from_patch(patch: &str) -> Option<String> {
    // Look for the "+++ " line which indicates the new file path
    for line in patch.lines() {
        if let Some(stripped) = line.strip_prefix("+++ ") {
            // Remove any timestamp after the path (format: "+++ b/path/to/file\t2024-01-01...")
            let path = stripped.split('\t').next().unwrap_or(stripped);
            // Remove the "b/" prefix if present (standard git diff format)
            let path = if let Some(p) = path.strip_prefix("b/") {
                p
            } else {
                path
            };
            return Some(path.to_string());
        }
    }
    None
}
