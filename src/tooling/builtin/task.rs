use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

use crate::session::{Message, MessageRole};
use crate::storage::SessionStore;
use crate::tooling::tools::TaskArgs;
use crate::tooling::{ToolDefinition, ToolPermission};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TaskArgs>(
        "task",
        "Run a subagent task",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    workspace_root: &Path,
    store: &SessionStore,
    session_id: Uuid,
    call: &crate::session::ToolCall,
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}", call.name))?;
    let args = serde_json::from_value::<TaskArgs>(arguments)
        .with_context(|| format!("failed to decode arguments for tool '{}'", call.name))?;

    let parent_session = store
        .load_session_record(session_id)?
        .context("parent session not found")?;
    let description = args.description.trim();
    let prompt = args.prompt.trim();
    let subagent_type = args
        .subagent_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("general");

    if description.is_empty() {
        bail!("task description cannot be empty");
    }
    if prompt.is_empty() {
        bail!("task prompt cannot be empty");
    }

    let child_session_id = Uuid::new_v4();
    let child_title = format!("Task: {description}");
    store.create_session_with_parent(
        child_session_id,
        parent_session.session_id,
        workspace_root,
        &parent_session.provider_id,
        &parent_session.provider_display_name,
        &parent_session.model_id,
        &parent_session.model_display_name,
        &child_title,
    )?;

    store.copy_tool_permissions(parent_session.session_id, child_session_id)?;

    let bootstrap_message = Message::new(
        MessageRole::System,
        format!(
            "You are a {subagent_type} assistant. Work on the task and keep the response concise."
        ),
    );
    store.append_message(child_session_id, &bootstrap_message)?;

    let user_message = Message::new(MessageRole::User, prompt.to_string());
    store.append_message(child_session_id, &user_message)?;

    Ok(format!(
        "Started {subagent_type} subagent task '{description}'"
    ))
}
