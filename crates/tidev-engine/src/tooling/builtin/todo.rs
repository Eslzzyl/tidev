use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

use tidev_storage::SessionStore;
use crate::tooling::tools::{TodoWriteArgs, decode_tool_args};
use crate::tooling::{ToolDefinition, ToolPermission};
use tidev_types::TodoItem;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TodoWriteArgs>(
        "todowrite",
        "Create and manage a structured task list for the current session. Use proactively to track progress on complex multi-step tasks.\n\
        \n\
        When to use:\n\
        - Complex tasks with 3+ distinct steps\n\
        - User explicitly requests todo tracking\n\
        - After receiving new instructions — immediately capture requirements\n\
        - Before starting a task — mark it as in_progress first\n\
        \n\
        When NOT to use:\n\
        - Single straightforward task (do it directly)\n\
        - Purely conversational requests\n\
        \n\
        Task states: pending (not started), in_progress (working on), completed (done)\n\
        Keep exactly ONE task as in_progress at a time.\n\
        When all tasks are completed the list is automatically cleared.",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    store: &SessionStore,
    session_id: uuid::Uuid,
    tool_name: &str,
    arguments: Value,
) -> Result<String> {
    let args = decode_tool_args::<TodoWriteArgs>(tool_name, arguments)?;
    validate_todos(&args.todos)?;

    // Load old todos for comparison in the return value
    let old_todos = store.load_todos(session_id)?;

    // Auto-clear: when all items are completed, clear the list
    let all_done = args.todos.iter().all(|t| t.status == "completed");
    let todos_to_store: Vec<TodoItem> = if all_done {
        Vec::new()
    } else {
        args.todos.clone()
    };

    store.replace_todos(session_id, &todos_to_store)?;

    serde_json::to_string_pretty(&serde_json::json!({
        "oldTodos": old_todos,
        "newTodos": args.todos,
    }))
    .context("failed to serialize todo list")
}

fn validate_todos(todos: &[TodoItem]) -> Result<()> {
    for (index, todo) in todos.iter().enumerate() {
        if todo.content.trim().is_empty() {
            bail!("todo item {} has empty content", index + 1);
        }

        if !matches!(
            todo.status.as_str(),
            "pending" | "in_progress" | "completed"
        ) {
            bail!(
                "todo item {} has invalid status '{}',",
                index + 1,
                todo.status
            );
        }
    }

    Ok(())
}
