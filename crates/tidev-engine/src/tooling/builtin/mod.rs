use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::mpsc::UnboundedSender;

use super::tools::{QuestionArgs, SkillArgs};
use super::{SkillCatalog, ToolDefinition, ToolPermission, canonical_tool_name};
use tidev_types::prompts::SessionMode;

use crate::config::AuthStore;
use crate::config::WebSearchConfig;
use tidev_session::session::BackendEvent;
use tidev_session::session::ToolCall;
use tidev_storage::SessionStore;

/// Context for executing a tool call — groups all shared configuration
/// that is independent of the specific tool being invoked.
pub struct ToolContext<'a> {
    pub workspace_root: &'a Path,
    pub config_dir: &'a Path,
    pub skills: &'a SkillCatalog,
    pub store: &'a SessionStore,
    pub session_id: uuid::Uuid,
    pub max_output_bytes: usize,
    pub rtk_enabled: bool,
    pub mode: SessionMode,
    pub allow_outside: bool,
    pub sensitive_file_approved: bool,
    pub web_search_config: &'a WebSearchConfig,
    pub auth_store: &'a AuthStore,
    pub event_tx: Option<UnboundedSender<BackendEvent>>,
}

pub mod apply_patch;
pub mod exec;
pub mod file;
pub mod search;
pub mod sensitive;
pub mod sudo;
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
    ctx: &ToolContext<'_>,
    call: &ToolCall,
) -> Result<tidev_session::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            file::execute_tool_call(
                ctx.workspace_root,
                ctx.config_dir,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.allow_outside,
                ctx.sensitive_file_approved,
            )?
        }
        Some("glob") | Some("grep") => {
            let output = search::execute_tool_call(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.allow_outside,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("bash") => {
            let result = exec::execute_tool_call(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.rtk_enabled,
                ctx.session_id,
                None,
            )?;
            tidev_session::session::ToolExecutionResult::new(result.output)
                .with_rtk_rewritten(result.rtk_rewritten)
        }
        Some("task") => {
            let output = task::execute_tool_call(
                ctx.workspace_root,
                ctx.store,
                ctx.session_id,
                &call.name,
                arguments,
                ctx.mode,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("todowrite") => {
            let output = todo::execute_tool_call(
                ctx.workspace_root,
                ctx.store,
                ctx.session_id,
                &call.name,
                arguments,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let output = ctx.skills.render_skill(&args.name)?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.web_search_config,
                ctx.auth_store,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    };

    Ok(result)
}

/// Execute a tool call with optional streaming output events.
///
/// When `event_tx` is `Some`, the bash tool will emit [`BackendEvent::ShellOutput`]
/// events as output is produced. Other tools ignore the sender and execute normally.
pub fn execute_tool_call_streaming(
    ctx: &ToolContext<'_>,
    call: &ToolCall,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<tidev_session::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            file::execute_tool_call(
                ctx.workspace_root,
                ctx.config_dir,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.allow_outside,
                ctx.sensitive_file_approved,
            )?
        }
        Some("glob") | Some("grep") => {
            let output = search::execute_tool_call(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.allow_outside,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("bash") => {
            let result = exec::execute_tool_call_with_cancel(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.rtk_enabled,
                cancelled.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                ctx.session_id,
                ctx.event_tx.clone(),
            )?;
            tidev_session::session::ToolExecutionResult::new(result.output)
                .with_rtk_rewritten(result.rtk_rewritten)
        }
        Some("task") => {
            let output = task::execute_tool_call(
                ctx.workspace_root,
                ctx.store,
                ctx.session_id,
                &call.name,
                arguments,
                ctx.mode,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("todowrite") => {
            let output = todo::execute_tool_call(
                ctx.workspace_root,
                ctx.store,
                ctx.session_id,
                &call.name,
                arguments,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let output = ctx.skills.render_skill(&args.name)?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                ctx.web_search_config,
                ctx.auth_store,
            )?;
            tidev_session::session::ToolExecutionResult::new(output)
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    };

    Ok(result)
}

/// Kill any remaining tracked child processes. Called during program exit
/// to prevent orphaned bash subprocesses.
pub use exec::{kill_all_children, kill_process_group};
