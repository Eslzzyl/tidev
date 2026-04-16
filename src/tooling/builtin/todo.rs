use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

use crate::storage::SessionStore;
use crate::tooling::tools::TodoWriteArgs;
use crate::tooling::{ToolDefinition, ToolPermission};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TodoWriteArgs>(
        "todowrite",
        "Update the session todo list",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    store: &SessionStore,
    session_id: uuid::Uuid,
    call: &crate::session::ToolCall,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;
    let args = serde_json::from_value::<TodoWriteArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;
    validate_todos(&args.todos)?;
    store.replace_todos(session_id, &args.todos)?;
    let todos = store.load_todos(session_id)?;
    serde_json::to_string_pretty(&todos).context("failed to serialize todo list")
}

fn validate_todos(todos: &[crate::tooling::tools::TodoItem]) -> Result<()> {
    for (index, todo) in todos.iter().enumerate() {
        if todo.content.trim().is_empty() {
            bail!("todo item {} has empty content", index + 1);
        }

        if !matches!(
            todo.status.as_str(),
            "pending" | "in_progress" | "completed" | "cancelled"
        ) {
            bail!(
                "todo item {} has invalid status '{}',",
                index + 1,
                todo.status
            );
        }

        if !matches!(todo.priority.as_str(), "high" | "medium" | "low") {
            bail!(
                "todo item {} has invalid priority '{}',",
                index + 1,
                todo.priority
            );
        }
    }

    Ok(())
}
