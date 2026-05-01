use anyhow::{Result, bail};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

use crate::agent::AgentType;
use crate::prompts::SessionMode;
use crate::storage::SessionStore;
use crate::tooling::tools::TaskArgs;
use crate::tooling::{ToolDefinition, ToolPermission};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TaskArgs>(
        "task",
        "Run a subagent task. Use `subagent_type` to delegate to a specialist: \
         explorer (code search), librarian (docs), oracle (strategy), \
         designer (UI/UX), fixer (implementation). Default: general.",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    _store: &SessionStore,
    _session_id: Uuid,
    call: &crate::session::ToolCall,
    mode: SessionMode,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .map_err(|e| anyhow::anyhow!("failed to parse arguments for tool '{}': {}", call.name, e))?;
    let args = serde_json::from_value::<TaskArgs>(arguments)
        .map_err(|e| anyhow::anyhow!("failed to decode arguments for tool '{}': {}", call.name, e))?;

    let description = args.description.trim();
    let prompt = args.prompt.trim();
    let subagent_type_str = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("general");

    if description.is_empty() {
        bail!("task description cannot be empty");
    }
    if prompt.is_empty() {
        bail!("task prompt cannot be empty");
    }

    let agent_type = AgentType::parse(subagent_type_str).unwrap_or(AgentType::General);

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
