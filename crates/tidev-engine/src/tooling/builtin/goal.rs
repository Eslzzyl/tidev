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
            "Only use this when you are working toward an explicit, \
             previously-assigned goal and need to check its status. \
             Do NOT call this to 'see if there is a goal' or to proactively \
             check what to work on — it returns null if no goal is set. \
             Returns the active goal object (objective, status, tokens_used, \
             time_used_seconds) or null if no goal exists.",
            ToolPermission::Session,
        ),
        ToolDefinition::new::<UpdateGoalArgs>(
            "update_goal",
            "Mark the current active goal as complete. \
             Only call when a goal exists and is active, and you have verified \
             every requirement is met. \
             Errors with 'no active goal found' if no goal exists in this session. \
             Arguments: status (must be 'complete'), reasoning (explain \
             verification evidence for each requirement).",
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
    match goal {
        Some(g) => serde_json::to_string_pretty(&serde_json::json!({"goal": g}))
            .context("failed to serialize goal"),
        None => serde_json::to_string_pretty(&serde_json::json!({
            "goal": null,
            "message": "No active goal exists for this session. \
                        The get_goal tool is only meaningful when a goal \
                        has been explicitly assigned. If you were not \
                        told to work toward a goal, do not call this tool."
        }))
        .context("failed to serialize response"),
    }
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
