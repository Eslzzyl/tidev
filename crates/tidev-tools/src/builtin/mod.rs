use anyhow::Result;
use serde_json::Value;
use std::any::Any;
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

/// Run a blocking operation that returns `String`, with panic catching.
///
/// Panics inside the closure are caught via `catch_unwind`. Both
/// `JoinError` from `spawn_blocking` and `Result::Err` from the closure
/// are converted into a `ToolExecutionResult` with an error message so
/// the agent loop never sees an error.
async fn safe_spawn_blocking_str<F>(f: F) -> ToolExecutionResult
where
    F: FnOnce() -> Result<String> + Send + 'static,
{
    let result = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f()))
    })
    .await;

    match result {
        Ok(Ok(Ok(output))) => ToolExecutionResult::new(output),
        Ok(Ok(Err(e))) => ToolExecutionResult::new(format!("Error: {e:#}")),
        Ok(Err(panic)) => {
            ToolExecutionResult::new(format!("Error: tool panicked: {}", panic_msg(panic)))
        }
        Err(join_err) => {
            ToolExecutionResult::new(format!("Error: tool aborted: {join_err}"))
        }
    }
}

/// Run a blocking operation that returns `ToolExecutionResult`, with panic
/// catching — same as [`safe_spawn_blocking_str`] but for tools that already
/// construct a `ToolExecutionResult` internally (e.g. file ops).
async fn safe_spawn_blocking_result<F>(f: F) -> ToolExecutionResult
where
    F: FnOnce() -> Result<ToolExecutionResult> + Send + 'static,
{
    let result = tokio::task::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f()))
    })
    .await;

    match result {
        Ok(Ok(Ok(r))) => r,
        Ok(Ok(Err(e))) => ToolExecutionResult::new(format!("Error: {e:#}")),
        Ok(Err(panic)) => {
            ToolExecutionResult::new(format!("Error: tool panicked: {}", panic_msg(panic)))
        }
        Err(join_err) => {
            ToolExecutionResult::new(format!("Error: tool aborted: {join_err}"))
        }
    }
}

fn panic_msg(panic: Box<dyn Any + Send>) -> String {
    if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = panic.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "unknown".to_string()
    }
}

/// Execute a tool call with streaming output support.
///
/// Bash emits [`BackendEvent::ShellOutput`] events when `event_tx` is `Some`.
/// Other tools execute in [`tokio::task::spawn_blocking`] to avoid blocking the
/// async runtime. The `cancel` token is used for cooperative cancellation of
/// the bash tool.
///
/// This function never returns an error — every failure (parse error, tool
/// error, panic) is converted into a `ToolExecutionResult` with an error
/// message so the agent loop can continue and the model can react.
pub async fn execute_tool_call(
    ctx: &ToolContext<'_>,
    call: &ToolCall,
    cancel: &CancellationToken,
) -> ToolExecutionResult {
    let arguments: Value = match serde_json::from_str(&call.arguments) {
        Ok(v) => v,
        Err(e) => {
            return ToolExecutionResult::new(format!(
                "Error: failed to parse arguments for tool '{}': {}",
                call.name, e
            ));
        }
    };

    match canonical_tool_name(&call.name) {
        // ── File operations ────────────────────────────────────────────
        Some("read") | Some("write") | Some("edit") | Some("apply_patch") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let config_dir = ctx.config_dir.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let allow_outside = ctx.allow_outside;
            let sensitive_file_approved = ctx.sensitive_file_approved;
            safe_spawn_blocking_result(move || {
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
            .await
        }

        // ── Search operations ──────────────────────────────────────────
        Some("glob") | Some("grep") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let allow_outside = ctx.allow_outside;
            safe_spawn_blocking_str(move || {
                search::execute_tool_call(
                    &workspace_root,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    allow_outside,
                )
            })
            .await
        }

        // ── Bash (async, panics caught via tokio::spawn) ───────────────
        Some("bash") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let cancel = cancel.clone();
            let session_id = ctx.session_id;
            let event_tx = ctx.event_tx.clone();
            match tokio::task::spawn(async move {
                exec::execute_tool_call_with_cancel_async(
                    &workspace_root,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    &cancel,
                    session_id,
                    event_tx,
                )
                .await
                .map(|r| r.output)
            })
            .await
            {
                Ok(Ok(output)) => ToolExecutionResult::new(output),
                Ok(Err(e)) => ToolExecutionResult::new(format!("Error: {e:#}")),
                Err(join_err) => {
                    ToolExecutionResult::new(format!("Error: tool panicked: {join_err}"))
                }
            }
        }

        // ── Sub-agent task ─────────────────────────────────────────────
        Some("task") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let store = ctx.todo.clone();
            let session_id = ctx.session_id;
            let call_name = call.name.clone();
            let mode = ctx.mode;
            safe_spawn_blocking_str(move || {
                task::execute_tool_call(
                    &workspace_root,
                    &*store,
                    session_id,
                    &call_name,
                    arguments,
                    mode,
                )
            })
            .await
        }

        // ── Todo persistence ───────────────────────────────────────────
        Some("todowrite") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let store = ctx.todo.clone();
            let session_id = ctx.session_id;
            let call_name = call.name.clone();
            safe_spawn_blocking_str(move || {
                todo::execute_tool_call(
                    &workspace_root,
                    &*store,
                    session_id,
                    &call_name,
                    arguments,
                )
            })
            .await
        }

        // ── Skill rendering ────────────────────────────────────────────
        Some("skill") => {
            let args = match parse_arguments::<SkillArgs>(&call.name, arguments) {
                Ok(a) => a,
                Err(e) => {
                    return ToolExecutionResult::new(format!("Error: {e:#}"));
                }
            };
            let skill_name = args.name.clone();
            let skills = ctx.skills.clone();
            safe_spawn_blocking_str(move || skills.render_skill(&skill_name)).await
        }

        // ── Web search / fetch (async, panics caught via tokio::spawn) ─
        Some("websearch") | Some("webfetch") => {
            let call_name = call.name.clone();
            let web_search_config = ctx.web_search_config.clone();
            let auth_store = ctx.auth_store.clone();
            match tokio::task::spawn(async move {
                web::execute_tool_call_async(
                    &call_name,
                    arguments,
                    &web_search_config,
                    &auth_store,
                )
                .await
            })
            .await
            {
                Ok(Ok(output)) => ToolExecutionResult::new(output),
                Ok(Err(e)) => ToolExecutionResult::new(format!("Error: {e:#}")),
                Err(join_err) => {
                    ToolExecutionResult::new(format!("Error: tool panicked: {join_err}"))
                }
            }
        }

        None => ToolExecutionResult::new(format!("Error: unknown tool '{}'", call.name)),
        Some(other) => ToolExecutionResult::new(format!("Error: unsupported tool '{}'", other)),
    }
}

/// Kill any remaining tracked child processes. Called during program exit
/// to prevent orphaned bash subprocesses.
pub use exec::{kill_all_children, kill_process_group};
