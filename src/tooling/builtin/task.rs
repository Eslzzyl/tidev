use anyhow::{Result, bail, ensure};
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
         designer (UI/UX), fixer (implementation).",
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
    let arguments: Value = serde_json::from_str(&call.arguments).map_err(|e| {
        anyhow::anyhow!("failed to parse arguments for tool '{}': {}", call.name, e)
    })?;
    let args = serde_json::from_value::<TaskArgs>(arguments).map_err(|e| {
        anyhow::anyhow!("failed to decode arguments for tool '{}': {}", call.name, e)
    })?;

    let description = args.description.trim();
    let prompt = args.prompt.trim();

    ensure!(!description.is_empty(), "task description cannot be empty");
    ensure!(!prompt.is_empty(), "task prompt cannot be empty");

    let subagent_type_str = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow::anyhow!(
            "subagent_type is required: specify one of explorer, librarian, oracle, designer, fixer"
        ))?;

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
