use anyhow::{Result, bail, ensure};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

use tidev_types::agent_type::AgentType;
use tidev_types::tools::{TaskArgs, ToolDefinition, ToolPermission};
use tidev_types::prompts::SessionMode;
use crate::builtin::utils::decode_tool_args;
use crate::todo_persistence::TodoPersistence;

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TaskArgs>(
        "task",
        "Run a subagent task. Use `subagent_type` to delegate to a specialist: \
         explorer (code search), librarian (docs), oracle (strategy), \
         designer (UI/UX), fixer (implementation).",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    _store: &dyn TodoPersistence,
    _session_id: Uuid,
    tool_name: &str,
    arguments: Value,
    mode: SessionMode,
) -> Result<String> {
    let args = decode_tool_args::<TaskArgs>(tool_name, arguments)?;

    let description = args.description.trim();
    let prompt = args.prompt.trim();

    ensure!(!description.is_empty(), "task description cannot be empty");
    ensure!(!prompt.is_empty(), "task prompt cannot be empty");

    let subagent_type_str = args.subagent_type.trim();
    ensure!(
        !subagent_type_str.is_empty(),
        "subagent_type is required: specify one of explorer, librarian, oracle, designer, fixer"
    );

    let agent_type = AgentType::parse(subagent_type_str)
        .ok_or_else(|| anyhow::anyhow!(
            "unknown subagent type '{subagent_type_str}': expected one of explorer, librarian, oracle, designer, fixer"
        ))?;

    // In plan mode, reject delegation to fixer subagents (they perform writes)
    if mode == SessionMode::Plan && agent_type == AgentType::Fixer {
        bail!(
            "Task delegation to fixer subagent rejected: Plan mode is read-only and does not allow write operations. \
            You may delegate to read-only subagents (explorer, librarian, oracle, designer) in plan mode. \
            Switch to build mode to use the fixer subagent."
        );
    }

    Ok(format!(
        "Started {agent_type} subagent task '{description}'",
        agent_type = agent_type.display_name()
    ))
}
