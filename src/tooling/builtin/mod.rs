use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};
use tokio::sync::mpsc::UnboundedSender;

use super::tools::{MemoryArgs, QuestionArgs, SkillArgs};
use super::{SkillCatalog, ToolDefinition, ToolPermission, canonical_tool_name};
use crate::config::AuthStore;
use crate::config::WebSearchConfig;
use crate::session::BackendEvent;
use crate::{prompts::SessionMode, session::ToolCall, storage::SessionStore};

pub mod exec;
pub mod file;
pub mod memory;
pub mod search;
pub mod sensitive;
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
    definitions.push(ToolDefinition::new::<MemoryArgs>(
        "memory",
        "Store, search, and manage workspace memories and slots. Operations: remember, search, list, read, forget, observations. Slots: slot_list, slot_get, slot_set, slot_append, slot_delete. Eviction: evict. Use to remember user preferences, project decisions, architecture decisions, and other important context.",
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
    memory_store: &Arc<crate::memory::MemoryStore>,
    mode: SessionMode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    sandbox_policy: Option<crate::sandbox::SandboxPolicy>,
    web_search_config: &WebSearchConfig,
    auth_store: &AuthStore,
) -> Result<crate::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            file::execute_tool_call(
                workspace_root,
                config_dir,
                &call.name,
                arguments,
                max_output_bytes,
                allow_outside,
                sensitive_file_approved,
            )?
        }
        Some("glob") | Some("grep") => {
            let output = search::execute_tool_call(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                allow_outside,
            )?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("bash") => {
            let result = exec::execute_tool_call(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                rtk_enabled,
                sandbox_policy,
                session_id,
                None, // event_tx — caller who needs streaming uses the dedicated streaming path
            )?;
            crate::session::ToolExecutionResult::new(result.output)
                .with_rtk_rewritten(result.rtk_rewritten)
                .with_sandbox(result.sandboxed, result.sandbox_type)
                .with_sandbox_denied(result.sandbox_denied)
        }
        Some("task") => {
            let output = task::execute_tool_call(
                workspace_root,
                store,
                session_id,
                &call.name,
                arguments,
                mode,
            )?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("todowrite") => {
            let output =
                todo::execute_tool_call(workspace_root, store, session_id, &call.name, arguments)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let output = skills.render_skill(&args.name)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("memory") => {
            let result = crate::tooling::builtin::memory::execute_tool_call(
                workspace_root,
                memory_store,
                call,
                arguments,
            )?;
            crate::session::ToolExecutionResult::new(result)
        }
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                web_search_config,
                auth_store,
            )?;
            crate::session::ToolExecutionResult::new(output)
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
    workspace_root: &Path,
    config_dir: &Path,
    skills: &SkillCatalog,
    store: &SessionStore,
    session_id: uuid::Uuid,
    call: &ToolCall,
    max_output_bytes: usize,
    rtk_enabled: bool,
    memory_store: &Arc<crate::memory::MemoryStore>,
    mode: SessionMode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    event_tx: Option<UnboundedSender<BackendEvent>>,
    sandbox_policy: Option<crate::sandbox::SandboxPolicy>,
    web_search_config: &WebSearchConfig,
    auth_store: &AuthStore,
    cancelled: Option<Arc<AtomicBool>>,
) -> Result<crate::session::ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            file::execute_tool_call(
                workspace_root,
                config_dir,
                &call.name,
                arguments,
                max_output_bytes,
                allow_outside,
                sensitive_file_approved,
            )?
        }
        Some("glob") | Some("grep") => {
            let output = search::execute_tool_call(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                allow_outside,
            )?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("bash") => {
            let result = exec::execute_tool_call_with_cancel(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                rtk_enabled,
                cancelled.unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                sandbox_policy,
                session_id,
                event_tx, // pass the sender through for streaming
            )?;
            crate::session::ToolExecutionResult::new(result.output)
                .with_rtk_rewritten(result.rtk_rewritten)
                .with_sandbox(result.sandboxed, result.sandbox_type)
                .with_sandbox_denied(result.sandbox_denied)
        }
        Some("task") => {
            let output = task::execute_tool_call(
                workspace_root,
                store,
                session_id,
                &call.name,
                arguments,
                mode,
            )?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("todowrite") => {
            let output =
                todo::execute_tool_call(workspace_root, store, session_id, &call.name, arguments)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("skill") => {
            let args = super::tools::parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let output = skills.render_skill(&args.name)?;
            crate::session::ToolExecutionResult::new(output)
        }
        Some("memory") => {
            let result = crate::tooling::builtin::memory::execute_tool_call(
                workspace_root,
                memory_store,
                call,
                arguments,
            )?;
            crate::session::ToolExecutionResult::new(result)
        }
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call(
                workspace_root,
                &call.name,
                arguments,
                max_output_bytes,
                web_search_config,
                auth_store,
            )?;
            crate::session::ToolExecutionResult::new(output)
        }
        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    };

    Ok(result)
}

/// Kill any remaining tracked child processes. Called during program exit
/// to prevent orphaned bash subprocesses.
pub use exec::kill_all_children;
