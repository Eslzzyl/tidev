use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::hooks::config::{HooksConfig, PostToolUseHookConfig};
use crate::hooks::matcher::matches_tool;
use crate::hooks::runner::run_hook_command;
use crate::memory::{HookPayload, HookType, MemoryStore};
use crate::session::{ToolCall, ToolExecutionResult};

/// Max chars for pre-tool enrich context, matching agentmemory's MAX_CONTEXT_LENGTH.
const ENRICH_MAX_CHARS: usize = 4000;

/// Outcome of running hooks for a single tool call.
#[derive(Clone, Debug, Default)]
pub struct PostToolUseHookOutcome {
    /// Per-hook outcomes, in execution order.
    pub hooks: Vec<SingleHookOutcome>,
    /// Whether any hook was matched and executed.
    pub any_hook_ran: bool,
}

#[derive(Clone, Debug)]
pub struct SingleHookOutcome {
    /// The hook command that was executed.
    pub command: String,
    /// Whether the hook succeeded.
    pub success: bool,
    /// Hook output (stdout on success, error on failure).
    pub output: String,
    /// Display name for this hook.
    pub display_name: String,
    /// Template arguments used for this invocation.
    pub filepath: Option<String>,
}

impl PostToolUseHookOutcome {
    /// Human-readable summary of all hook outcomes, to be appended to
    /// the tool result output shown to the model.
    pub fn format_for_result(&self) -> Option<String> {
        if !self.any_hook_ran {
            return None;
        }

        let mut lines: Vec<String> = Vec::new();
        for hook in &self.hooks {
            let action = if hook.success { "Ran" } else { "Failed" };
            let details = if hook.success {
                if hook.output.is_empty() {
                    String::new()
                } else {
                    format!(": {}", hook.output)
                }
            } else {
                format!(": {}", hook.output)
            };
            lines.push(format!(
                "[{action} {hook_command}]{details}",
                hook_command = hook.command
            ));
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }
}

/// The hook engine — loads configuration, matches hooks to tool calls,
/// and executes them.
#[derive(Clone)]
pub struct HookEngine {
    config: HooksConfig,
    workspace_root: PathBuf,
    memory_store: Option<Arc<MemoryStore>>,
}

impl HookEngine {
    pub fn new(config: HooksConfig, workspace_root: PathBuf) -> Self {
        Self {
            config,
            workspace_root,
            memory_store: None,
        }
    }

    /// Attach a MemoryStore for automatic observation capture.
    pub fn with_memory_store(mut self, store: Arc<MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Update the config at runtime (e.g., after config reload).
    pub fn update_config(&mut self, config: HooksConfig) {
        self.config = config;
    }

    pub fn config(&self) -> &HooksConfig {
        &self.config
    }

    /// Run matching PostToolUse hooks after a tool has executed.
    ///
    /// If a hook's formatter changes the file on disk, callers should
    /// re-read the file and regenerate the diff. This method returns
    /// per-hook outcomes for that purpose.
    pub async fn on_post_tool_use(
        &self,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
        session_id: Option<uuid::Uuid>,
    ) -> PostToolUseHookOutcome {
        if self.config.disable_all_hooks {
            return PostToolUseHookOutcome::default();
        }

        let mut outcomes = Vec::new();
        let mut any_hook_ran = false;

        for hook in &self.config.post_tool_use {
            if !self.matches(hook, tool_call, result) {
                continue;
            }

            // Resolve template variables
            let filepath: &str = result.metadata.filepath.as_deref().unwrap_or_default();

            let filepath_abs = if filepath.is_empty() {
                String::new()
            } else {
                // Try to resolve relative filepath to absolute for the command
                let candidate = self.workspace_root.join(filepath);
                if candidate.exists() {
                    candidate.to_string_lossy().to_string()
                } else {
                    // Might already be absolute
                    filepath.to_string()
                }
            };

            let command = hook
                .command
                .replace("{filepath}", &filepath_abs)
                .replace("{workspace_root}", &self.workspace_root.to_string_lossy())
                .replace("{tool_name}", &tool_call.name);

            let cwd = hook
                .cwd
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.workspace_root.clone());

            // Run the hook
            let outcome = run_hook_command(&command, &cwd, hook.timeout_sec).await;

            any_hook_ran = true;
            outcomes.push(SingleHookOutcome {
                command,
                success: outcome.success,
                output: outcome.output,
                display_name: hook.display_name().to_string(),
                filepath: result.metadata.filepath.clone(),
            });
        }

        // ── Memory observation capture ────────────────────────────────
        if let (Some(store), Some(sid)) = (&self.memory_store, session_id) {
            let payload = HookPayload {
                session_id: sid,
                hook_type: HookType::PostToolUse,
                timestamp: chrono::Utc::now(),
                tool_name: Some(tool_call.name.clone()),
                tool_input: Some(tool_call.arguments.clone()),
                tool_output: Some(result.output.clone()),
                user_prompt: None,
                assistant_response: None,
            };
            let _ = store.observe(&payload);
        }

        PostToolUseHookOutcome {
            hooks: outcomes,
            any_hook_ran,
        }
    }

    /// Record a memory observation before tool execution.
    pub fn on_pre_tool_use(&self, tool_call: &ToolCall, session_id: Option<uuid::Uuid>) {
        if let (Some(store), Some(sid)) = (&self.memory_store, session_id) {
            let payload = HookPayload {
                session_id: sid,
                hook_type: HookType::PreToolUse,
                timestamp: chrono::Utc::now(),
                tool_name: Some(tool_call.name.clone()),
                tool_input: Some(tool_call.arguments.clone()),
                tool_output: None,
                user_prompt: None,
                assistant_response: None,
            };
            let _ = store.observe(&payload);
        }
    }

    /// Search memory for observations relevant to the file being operated on.
    ///
    /// Like agentmemory's `mem::enrich` (pre-tool-use enrich hook).
    /// Returns a `<system-reminder>` block with relevant context, or `None`.
    /// Capped at [`ENRICH_MAX_CHARS`] characters.
    pub async fn on_pre_tool_use_enrich(
        &self,
        tool_call: &ToolCall,
        session_id: Option<uuid::Uuid>,
    ) -> Option<String> {
        let store = self.memory_store.as_ref()?;
        let _sid = session_id?;

        let workspace_root = self.workspace_root.to_string_lossy().to_string();

        // Extract file paths from tool arguments
        let file_paths = extract_tool_file_paths(tool_call);
        if file_paths.is_empty() {
            return None;
        }

        // Build search query from file names and search
        let search_query = file_paths
            .iter()
            .filter_map(|p| {
                Path::new(p)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.to_string())
            })
            .collect::<Vec<_>>()
            .join(" ");

        if search_query.trim().is_empty() {
            return None;
        }

        let entries = store
            .search(&workspace_root, &search_query)
            .ok()?;

        if entries.is_empty() {
            return None;
        }

        let mut parts: Vec<String> = Vec::new();
        for entry in entries.iter().take(8) {
            let type_str = entry.memory_type.as_str();
            let title = &entry.title;
            let content = &entry.content;
            parts.push(format!("- [{}] {}: {}", type_str, title, content));
        }

        let body = parts.join("\n");
        if body.is_empty() {
            return None;
        }

        let wrapped = format!(
            "<agentmemory-relevant-context>\n{}\n</agentmemory-relevant-context>",
            body
        );

        // Cap at ENRICH_MAX_CHARS
        let truncated = if wrapped.len() > ENRICH_MAX_CHARS {
            let mut t = wrapped[..ENRICH_MAX_CHARS].to_string();
            t.push_str("\n... (truncated)");
            t
        } else {
            wrapped
        };

        Some(truncated)
    }

    /// Record a memory observation after tool failure.
    pub fn on_post_tool_failure(
        &self,
        tool_call: &ToolCall,
        error: &str,
        session_id: Option<uuid::Uuid>,
    ) {
        if let (Some(store), Some(sid)) = (&self.memory_store, session_id) {
            let payload = HookPayload {
                session_id: sid,
                hook_type: HookType::PostToolFailure,
                timestamp: chrono::Utc::now(),
                tool_name: Some(tool_call.name.clone()),
                tool_input: Some(tool_call.arguments.clone()),
                tool_output: Some(error.to_string()),
                user_prompt: None,
                assistant_response: None,
            };
            let _ = store.observe(&payload);
        }
    }

    /// Record a generic memory observation (for session_end, subagent, etc.)
    pub fn record_observation(
        &self,
        hook_type: HookType,
        tool_name: Option<String>,
        tool_input: Option<String>,
        tool_output: Option<String>,
        session_id: Option<uuid::Uuid>,
    ) {
        if let (Some(store), Some(sid)) = (&self.memory_store, session_id) {
            let payload = HookPayload {
                session_id: sid,
                hook_type,
                timestamp: chrono::Utc::now(),
                tool_name,
                tool_input,
                tool_output,
                user_prompt: None,
                assistant_response: None,
            };
            let _ = store.observe(&payload);
        }
    }
}

/// Extract file paths from a tool call's JSON arguments.
///
/// Mirrors agentmemory's pre-tool-use enrich file-extraction logic:
/// Grep uses "path"/"file", other file tools use "file_path"/"path".
fn extract_tool_file_paths(tool_call: &ToolCall) -> Vec<String> {
    let args: serde_json::Value = match serde_json::from_str(&tool_call.arguments) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let canonical =
        crate::tooling::canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);

    let keys: &[&str] = if canonical == "grep" {
        &["path", "file"]
    } else {
        &["file_path", "path"]
    };

    let mut paths: Vec<String> = Vec::new();
    if let Some(obj) = args.as_object() {
        for key in keys {
            if let Some(val) = obj.get(*key) {
                match val {
                    serde_json::Value::String(s) if !s.is_empty() => {
                        paths.push(s.clone());
                    }
                    serde_json::Value::Array(arr) => {
                        for item in arr {
                            if let serde_json::Value::String(s) = item {
                                if !s.is_empty() {
                                    paths.push(s.clone());
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    paths
}

impl HookEngine {
    /// Check whether a hook matches the given tool call + result.
    fn matches(
        &self,
        hook: &PostToolUseHookConfig,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
    ) -> bool {
        // 1. Check matcher pattern against tool name (use canonical name)
        let canonical =
            crate::tooling::canonical_tool_name(&tool_call.name).unwrap_or(&tool_call.name);
        if !matches_tool(&hook.matcher, canonical) {
            return false;
        }

        // 2. If extensions are specified, check the file extension
        if !hook.extensions.is_empty() {
            match &result.metadata.filepath {
                Some(fp) => {
                    let ext = Path::new(fp)
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| format!(".{e}"))
                        .unwrap_or_default();
                    if !hook.extensions.contains(&ext) {
                        return false;
                    }
                }
                None => return false, // extensions specified but no filepath
            }
        }

        true
    }
}
