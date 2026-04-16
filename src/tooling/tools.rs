use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use uuid::Uuid;

use crate::{
    session::{ToolCall, ToolExecutionResult},
    storage::SessionStore,
};
use crate::tooling::SkillCatalog;

use super::ToolDefinition;
use super::builtin;

pub trait ToolArgs: for<'de> Deserialize<'de> + Serialize {
    fn schema() -> Value;
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
        path: string("Path to read relative to the workspace root"),
        offset: optional_integer("1-indexed line number to start reading from"),
        limit: optional_integer("Maximum number of lines to read"),
    }
}

tool_args! {
    pub struct WriteArgs {
        path: string("Path to write relative to the workspace root"),
        content: string("File contents to write"),
    }
}

tool_args! {
    pub struct EditArgs {
        path: string("Path to edit relative to the workspace root"),
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
    pub struct ListArgs {
        path: optional_string("Directory path relative to the workspace root"),
    }
}

tool_args! {
    pub struct GlobArgs {
        pattern: string("Glob pattern to match against workspace-relative paths"),
        path: optional_string("Directory path to search relative to the workspace root"),
    }
}

tool_args! {
    pub struct GrepArgs {
        pattern: string("Regular expression to search for in file contents"),
        path: optional_string("Directory path to search relative to the workspace root"),
        include: optional_string("File glob to include in the search"),
    }
}

tool_args! {
    pub struct BashArgs {
        command: string("Shell command to execute from the workspace root"),
    }
}

tool_args! {
    pub struct TaskArgs {
        description: string("Short title for the task"),
        prompt: string("Task prompt to give the subagent"),
        subagent_type: optional_string("Optional subagent type, such as general or review"),
    }
}

tool_args! {
    pub struct TodoItem {
        content: string("Brief description of the task"),
        status: string("Current status of the task: pending, in_progress, completed, cancelled"),
        priority: string("Priority level of the task: high, medium, low"),
    }
}

tool_args! {
    pub struct TodoWriteArgs {
        todos: array(TodoItem, "The updated todo list"),
    }
}

tool_args! {
    pub struct SkillArgs {
        name: string("Skill name to load"),
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
    }
}

tool_args! {
    pub struct WebFetchArgs {
        url: string("The URL to fetch"),
        format: optional_string("Output format: text, markdown, or html"),
        timeout: optional_integer("Timeout in seconds (max 120)"),
    }
}

pub(super) fn parse_arguments<Args>(tool_name: &str, arguments: Value) -> Result<Args>
where
    Args: ToolArgs,
{
    serde_json::from_value(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", tool_name))
}

pub(super) fn tool_definitions(skill_description: String) -> Vec<ToolDefinition> {
    builtin::definitions(skill_description)
}

pub fn execute_shell_tool_call(
    workspace_root: &Path,
    call: &ToolCall,
    max_output_bytes: usize,
    cancelled: Arc<AtomicBool>,
) -> Result<String> {
    builtin::exec::execute_tool_call_with_cancel(workspace_root, call, max_output_bytes, cancelled)
}

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

fn truncate_to_limit(mut value: String, max_bytes: usize) -> String {
    truncate_in_place(&mut value, max_bytes);
    value
}

pub(super) fn execute_tool_call(
    workspace_root: &Path,
    skills: &SkillCatalog,
    store: &SessionStore,
    session_id: Uuid,
    call: &ToolCall,
    max_output_bytes: usize,
) -> Result<ToolExecutionResult> {
    let output = builtin::execute_tool_call(
        workspace_root,
        skills,
        store,
        session_id,
        call,
        max_output_bytes,
    )?;

    Ok(ToolExecutionResult::new(truncate_to_limit(
        output,
        max_output_bytes,
    )))
}
