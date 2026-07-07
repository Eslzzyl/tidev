use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use tidev_types::message::BackendEvent;
use tidev_types::message::ToolCall;
use tidev_types::message::ToolExecutionResult;
use tidev_types::prompts::SessionMode;
use tidev_types::tools::{QuestionArgs, SkillArgs, ToolDefinition, ToolPermission, canonical_tool_name};
use crate::builtin::utils::parse_arguments;

use crate::skills::SkillCatalog;
use crate::todo_persistence::TodoPersistence;

/// Context for executing a tool call — groups all shared configuration
/// that is independent of the specific tool being invoked.
pub struct ToolContext<'a> {
    pub workspace_root: &'a Path,
    pub config_dir: &'a Path,
    pub skills: &'a SkillCatalog,
    pub todo: Arc<dyn TodoPersistence + Send + Sync>,
    pub session_id: uuid::Uuid,
    pub max_output_bytes: usize,
    pub mode: SessionMode,
    pub allow_outside: bool,
    pub sensitive_file_approved: bool,
    pub web_search_config: &'a tidev_config::WebSearchConfig,
    pub auth_store: &'a tidev_config::AuthStore,
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

/// Execute a tool call with streaming output support.
///
/// Bash emits [`BackendEvent::ShellOutput`] events when `event_tx` is `Some`.
/// Other tools execute in [`tokio::task::spawn_blocking`] to avoid blocking the
/// async runtime. The `cancel` token is used for cooperative cancellation of
/// the bash tool.
pub async fn execute_tool_call(
    ctx: &ToolContext<'_>,
    call: &ToolCall,
    cancel: &CancellationToken,
) -> Result<ToolExecutionResult> {
    let arguments: Value = serde_json::from_str(&call.arguments)
        .with_context(|| format!("failed to parse arguments for tool '{}'", call.name))?;

    let result = match canonical_tool_name(&call.name) {
        // ── File operations ────────────────────────────────────────────
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let config_dir = ctx.config_dir.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let allow_outside = ctx.allow_outside;
            let sensitive_file_approved = ctx.sensitive_file_approved;
            tokio::task::spawn_blocking(move || {
                file::execute_tool_call(
                    &workspace_root,
                    &config_dir,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    allow_outside,
                    sensitive_file_approved,
                )
            })
            .await??
        }

        // ── Search operations ──────────────────────────────────────────
        Some("glob") | Some("grep") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let allow_outside = ctx.allow_outside;
            let output = tokio::task::spawn_blocking(move || {
                search::execute_tool_call(
                    &workspace_root,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    allow_outside,
                )
            })
            .await??;
            ToolExecutionResult::new(output)
        }

        // ── Bash (truly async) ─────────────────────────────────────────
        Some("bash") => {
            let result = exec::execute_tool_call_with_cancel_async(
                ctx.workspace_root,
                &call.name,
                arguments,
                ctx.max_output_bytes,
                cancel,
                ctx.session_id,
                ctx.event_tx.clone(),
            )
            .await?;
            ToolExecutionResult::new(result.output)
        }

        // ── Sub-agent task ─────────────────────────────────────────────
        Some("task") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let store = ctx.todo.clone();
            let session_id = ctx.session_id;
            let call_name = call.name.clone();
            let mode = ctx.mode;
            let output = tokio::task::spawn_blocking(move || {
                task::execute_tool_call(
                    &workspace_root,
                    &*store,
                    session_id,
                    &call_name,
                    arguments,
                    mode,
                )
            })
            .await??;
            ToolExecutionResult::new(output)
        }

        // ── Todo persistence ───────────────────────────────────────────
        Some("todowrite") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let store = ctx.todo.clone();
            let session_id = ctx.session_id;
            let call_name = call.name.clone();
            let output = tokio::task::spawn_blocking(move || {
                todo::execute_tool_call(
                    &workspace_root,
                    &*store,
                    session_id,
                    &call_name,
                    arguments,
                )
            })
            .await??;
            ToolExecutionResult::new(output)
        }

        // ── Skill rendering ────────────────────────────────────────────
        Some("skill") => {
            let args = parse_arguments::<SkillArgs>(&call.name, arguments)?;
            let skill_name = args.name.clone();
            let skills = ctx.skills.clone();
            let output = tokio::task::spawn_blocking(move || {
                skills.render_skill(&skill_name)
            })
            .await??;
            ToolExecutionResult::new(output)
        }

        // ── Web search / fetch ─────────────────────────────────────────
        Some("websearch") | Some("webfetch") => {
            let output = web::execute_tool_call_async(
                &call.name,
                arguments,
                ctx.web_search_config,
                ctx.auth_store,
            )
            .await?;
            ToolExecutionResult::new(output)
        }

        None => bail!("unknown tool '{}'", call.name),
        Some(other) => bail!("unsupported tool '{}'", other),
    };

    Ok(result)
}

/// Kill any remaining tracked child processes. Called during program exit
/// to prevent orphaned bash subprocesses.
pub use exec::{kill_all_children, kill_process_group};
