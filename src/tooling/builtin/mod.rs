use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;

use super::tools::{QuestionArgs, SkillArgs};
use super::{SkillCatalog, ToolDefinition, ToolPermission, canonical_tool_name};
use crate::{session::ToolCall, storage::SessionStore};

pub mod exec;
pub mod file;
pub mod search;
pub mod task;
pub mod todo;
pub mod utils;
pub mod web;

pub fn definitions(skill_description: String) -> Vec<ToolDefinition> {
    let mut definitions = Vec::new();
    definitions.extend(file::definitions());
    definitions.extend(search::definitions());
    definitions.extend(exec::definitions());
    definitions.extend(task::definitions());
    definitions.extend(todo::definitions());
    definitions.extend(web::definitions());
    definitions.push(ToolDefinition::new::<QuestionArgs>(
        "question",
        "Ask the user questions during execution",
        ToolPermission::Session,
    ));
    definitions.push(ToolDefinition::new::<SkillArgs>(
        "skill",
        skill_description,
        ToolPermission::Session,
    ));
    definitions
}

pub fn execute_tool_call(
    workspace_root: &Path,
    config_dir: &Path,
    skills: &SkillCatalog,
    store: &SessionStore,
    session_id: uuid::Uuid,
    call: &ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
) -> Result<crate::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") | Some("list") => {
            file::execute_tool_call(workspace_root, config_dir, call, max_output_bytes)?
        }
        Some("glob") | Some("grep") => {
            let output = search::execute_tool_call(workspace_root, call, max_output_bytes)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("bash") => {
            let result =
                exec::execute_tool_call(workspace_root, call, max_output_bytes, rtk_enabled)?;
            crate::session::ToolExecutionResult::new(result.output)
                .with_rtk_rewritten(result.rtk_rewritten)
        }
        Some("task") => {
            let output = task::execute_tool_call(workspace_root, store, session_id, call)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("todowrite") => {
            let output = todo::execute_tool_call(workspace_root, store, session_id, call)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let output = skills.render_skill(&args.name)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call(workspace_root, call, max_output_bytes)?;
            crate::session::ToolExecutionResult::new(output)
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    };

    Ok(result)
}
