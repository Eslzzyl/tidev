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
mod utils;
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
) -> Result<String> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let output = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") | Some("list") => {
            file::execute_tool_call(workspace_root, config_dir, call, max_output_bytes)
        }
        Some("glob") | Some("grep") => {
            search::execute_tool_call(workspace_root, call, max_output_bytes)
        }
        Some("bash") => exec::execute_tool_call(workspace_root, call, max_output_bytes),
        Some("task") => task::execute_tool_call(workspace_root, store, session_id, call),
        Some("todowrite") => todo::execute_tool_call(workspace_root, store, session_id, call),
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            skills.render_skill(&args.name)
        }
        Some("websearch") | Some("webfetch") => {
            web::execute_tool_call(workspace_root, call, max_output_bytes)
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    }?;

    Ok(output)
}
