use anyhow::Result;
use serde_json::Value;
use std::any::Any;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::builtin::utils::parse_arguments;
use crate::types::{QuestionArgs, SkillArgs, ToolDefinition, ToolPermission};
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_utils::tool_name::canonical_tool_name;

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
    pub request_id: u64,
    pub max_output_bytes: usize,
    pub read_only: bool,
    pub allow_outside: bool,
    pub sensitive_file_approved: bool,
    pub web_search_config: &'a tidev_config::WebSearchConfig,
    pub auth_store: &'a tidev_config::AuthStore,
    pub event_tx: Option<UnboundedSender<ShellOutput>>,
    pub instruction_sources: Option<Arc<Mutex<Vec<String>>>>,
}

/// Streaming output emitted by the shell tool before the host converts it to
/// its product-facing event type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellOutput {
    pub session_id: uuid::Uuid,
    pub request_id: u64,
    pub tool_call_id: String,
    pub content: String,
    pub finished: bool,
    pub exit_code: Option<i32>,
}

pub mod apply_patch;
pub mod classify;
pub mod exec;
pub mod file;
pub mod search;
pub mod sensitive;
pub mod sudo;
pub mod task;
pub mod todo;
pub mod utils;
pub mod web;

pub fn definitions() -> Vec<ToolDefinition> {
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
        crate::skills::SKILL_TOOL_DESCRIPTION,
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
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    })
    .await;

    match result {
        Ok(Ok(Ok(output))) => ToolExecutionResult::new(output),
        Ok(Ok(Err(e))) => ToolExecutionResult::new(format!("Error: {e:#}")),
        Ok(Err(panic)) => {
            ToolExecutionResult::new(format!("Error: tool panicked: {}", panic_msg(panic)))
        }
        Err(join_err) => ToolExecutionResult::new(format!("Error: tool aborted: {join_err}")),
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
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
    })
    .await;

    match result {
        Ok(Ok(Ok(r))) => r,
        Ok(Ok(Err(e))) => ToolExecutionResult::new(format!("Error: {e:#}")),
        Ok(Err(panic)) => {
            ToolExecutionResult::new(format!("Error: tool panicked: {}", panic_msg(panic)))
        }
        Err(join_err) => ToolExecutionResult::new(format!("Error: tool aborted: {join_err}")),
    }
}

fn normalize_skill_path(path: Option<String>) -> Option<String> {
    path.filter(|path| !path.is_empty())
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
/// Shell emits [`ShellOutput`] events when `event_tx` is `Some`.
/// Other tools execute in [`tokio::task::spawn_blocking`] to avoid blocking the
/// async runtime. The `cancel` token is used for cooperative cancellation of
/// the shell tool.
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
            let instruction_sources = ctx.instruction_sources.clone();
            safe_spawn_blocking_result(move || {
                file::execute_tool_call(
                    &workspace_root,
                    &config_dir,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    allow_outside,
                    sensitive_file_approved,
                    instruction_sources,
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

        // ── Shell (async, panics caught via tokio::spawn) ───────────────
        Some("shell") => {
            let workspace_root = ctx.workspace_root.to_path_buf();
            let call_name = call.name.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let read_only = ctx.read_only;
            let cancel = cancel.clone();
            let session_id = ctx.session_id;
            let request_id = ctx.request_id;
            let event_tx = ctx.event_tx.clone();
            let tool_call_id = call.id.clone();
            match tokio::task::spawn(async move {
                exec::execute_tool_call_with_cancel_async(
                    &workspace_root,
                    &call_name,
                    arguments,
                    max_output_bytes,
                    &cancel,
                    read_only,
                    session_id,
                    request_id,
                    event_tx,
                    &tool_call_id,
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
            safe_spawn_blocking_str(move || {
                task::execute_tool_call(&workspace_root, &*store, session_id, &call_name, arguments)
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
                todo::execute_tool_call(&workspace_root, &*store, session_id, &call_name, arguments)
            })
            .await
        }

        // ── Skill access (list / load / read skill files) ───────────────
        Some("skill") => {
            let args = match parse_arguments::<SkillArgs>(&call.name, arguments) {
                Ok(a) => a,
                Err(e) => {
                    return ToolExecutionResult::new(format!("Error: {e:#}"));
                }
            };
            let skills = ctx.skills.clone();
            let max_output_bytes = ctx.max_output_bytes;
            let SkillArgs {
                name,
                path,
                offset,
                limit,
            } = args;
            // Some tool adapters encode an omitted optional string as an
            // empty string. Treat that representation the same as an absent
            // path so `skill` can load a named skill without a document path.
            let path = normalize_skill_path(path);
            safe_spawn_blocking_str(move || match (name.as_deref(), path.as_deref()) {
                (None, None) => skills.list_skills(
                    offset.unwrap_or(1).max(1) as usize,
                    limit.unwrap_or(crate::skills::DEFAULT_SKILL_PAGE_SIZE as i64) as usize,
                ),
                (None, Some(_)) => Err(anyhow::anyhow!(
                    "skill: a path requires a skill name to read from"
                )),
                (Some(name), None) => skills.render_skill(name),
                (Some(name), Some(path)) => skills.read_skill_file(name, path, max_output_bytes),
            })
            .await
        }

        // ── Web search / fetch (async, runs inline for abort support) ──
        Some("websearch") | Some("webfetch") => {
            let call_name = call.name.clone();
            let web_search_config = ctx.web_search_config.clone();
            let auth_store = ctx.auth_store.clone();
            // Run inline (no tokio::task::spawn) so that when the caller
            // aborts this task via JoinSet, the HTTP future is dropped
            // directly, closing the connection immediately.
            match web::execute_tool_call_async(
                &call_name,
                arguments,
                &web_search_config,
                &auth_store,
            )
            .await
            {
                Ok(output) => ToolExecutionResult::new(output),
                Err(e) => ToolExecutionResult::new(format!("Error: {e:#}")),
            }
        }

        None => ToolExecutionResult::new(format!("Error: unknown tool '{}'", call.name)),
        Some(other) => ToolExecutionResult::new(format!("Error: unsupported tool '{}'", other)),
    }
}

/// Kill any remaining tracked child processes. Called during program exit
/// to prevent orphaned shell subprocesses.
pub use exec::{kill_all_children, kill_process_group};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_skill_path_is_treated_as_absent() {
        let args = SkillArgs {
            name: Some("git-workflow".to_string()),
            path: Some(String::new()),
            offset: None,
            limit: None,
        };
        let SkillArgs { path, .. } = args;
        assert_eq!(normalize_skill_path(path), None);
        assert_eq!(
            normalize_skill_path(Some("docs/guide.md".to_string())),
            Some("docs/guide.md".to_string())
        );
    }
}
