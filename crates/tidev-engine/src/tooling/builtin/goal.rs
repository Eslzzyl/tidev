use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::tooling::tools::{GetGoalArgs, UpdateGoalArgs, decode_tool_args};
use crate::tooling::{ToolDefinition, ToolPermission};
use tidev_storage::SessionStore;
use tidev_types::{Goal, GoalStatus};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::new::<GetGoalArgs>(
            "get_goal",
            "Get the current active goal for this session, including status and resource usage.",
            ToolPermission::Session,
        ),
        ToolDefinition::new::<UpdateGoalArgs>(
            "update_goal",
            "Mark the current goal as complete. Call this only when you have verified every requirement against the current state.",
            ToolPermission::Session,
        ),
    ]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    store: &SessionStore,
    session_id: uuid::Uuid,
    tool_name: &str,
    arguments: Value,
) -> Result<String> {
    match tool_name {
        "get_goal" => execute_get_goal(store, session_id),
        "update_goal" => execute_update_goal(store, session_id, arguments),
        other => anyhow::bail!("unsupported goal tool '{}'", other),
    }
}

fn execute_get_goal(store: &SessionStore, session_id: uuid::Uuid) -> Result<String> {
    let goal: Option<Goal> = store.get_goal(session_id)?;
    serde_json::to_string_pretty(&serde_json::json!({
        "goal": goal,
    }))
    .context("failed to serialize goal")
}

fn execute_update_goal(
    store: &SessionStore,
    session_id: uuid::Uuid,
    arguments: Value,
) -> Result<String> {
    let args = decode_tool_args::<UpdateGoalArgs>("update_goal", arguments)?;

    if args.status != "complete" {
        anyhow::bail!("only status='complete' is supported, got '{}'", args.status);
    }

    // Verify a goal actually exists and is Active before allowing completion.
    let goal = store
        .get_goal(session_id)?
        .ok_or_else(|| anyhow::anyhow!("no active goal found for this session"))?;

    if goal.status != GoalStatus::Active {
        anyhow::bail!(
            "goal is not active (current status: {:?}); cannot mark complete",
            goal.status
        );
    }

    store.update_goal_status(session_id, GoalStatus::Complete)?;

    serde_json::to_string_pretty(&serde_json::json!({
        "status": "complete",
        "reasoning": args.reasoning,
        "objective": goal.objective,
    }))
    .context("failed to serialize completion result")
}
