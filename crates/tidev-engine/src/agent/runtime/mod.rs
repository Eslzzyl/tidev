//! Shared agent runtime — orchestrates the LLM ↔ tool execution loop.
//!
//! Both the TUI and web frontends use this same runtime so that tool
//! definitions, system-prompt composition, message preprocessing, and the
//! core streaming loop are defined in a single place.
//!
//! Consumers provide an [`UnboundedSender<BackendEvent>`] to receive
//! real-time events (text deltas, tool calls, tool results, …) and call
//! [`AgentRuntime::run_agent_loop`] which drives the full turn loop:
//!
//! ```text
//!  load messages  →  compose system prompt  →  stream LLM
//!       ↑                                        |
//!       |                              tool calls? ──no──→ done
//!       |                                        |
//!       └──── persist results ←── execute tools ←┘
//! ```

mod agent_loop;
mod persistence;
mod subagent;
pub mod types;

// ── Re-exports ─────────────────────────────────────────────────────────────
pub use types::*;

// ── Imports ────────────────────────────────────────────────────────────────
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::Result;
use tokio::sync::Mutex;

use tidev_llm::LlmClient;
use tidev_session::system_info::SystemInfo;
use tidev_storage::SessionStore;

use crate::config::{AppConfig, AuthStore, ConfigPaths};
use crate::tooling::ToolRegistry;
use crate::tooling::builtin::utils::canonicalize_display;

// ── AgentRuntime ───────────────────────────────────────────────────────────

/// The shared agent runtime that drives the LLM ↔ tool execution loop.
///
/// This struct owns all the resources needed to run an agent session:
/// configuration, LLM client, tool registry, session store, and hooks.
/// Frontends (TUI, web, gateway) clone [`AgentRuntime`] and call
/// [`run_agent_loop`](AgentRuntime::run_agent_loop) to process turns.
#[derive(Clone)]
pub struct AgentRuntime {
    pub workspace_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_paths: ConfigPaths,
    pub config: AppConfig,
    pub auth: AuthStore,
    pub store: Arc<Mutex<SessionStore>>,
    pub llm_client: LlmClient,
    pub tools: ToolRegistry,
    /// Instruction file paths/URLs from config (e.g. `config.instructions`).
    pub instructions: Vec<String>,
    /// Cache for instruction file contents to avoid re-reading.
    pub instruction_content_cache: HashMap<String, String>,
    /// Queue of user messages received while the agent loop is running.
    /// After each turn completes, the loop processes the next message
    /// automatically.  Frontends push through [`queue_user_message`].
    pub queued_messages: Arc<StdMutex<VecDeque<QueuedUserMessage>>>,
    /// When `false` (default), tools that need user confirmation are
    /// rejected with an error instead of executed.  When `true`, all
    /// tools are executed without interactive confirmation.
    ///
    /// The TUI sets this to `true` because it handles interactive
    /// permission dialogs itself via the [`PendingToolApproval`] channel.
    /// Web and gateway frontends typically leave this `false` as a
    /// safe default: tools that require approval are simply rejected.
    pub auto_approve_permissions: bool,
    /// Hook engine for PostToolUse hooks (formatting, etc.)
    pub hooks: crate::hooks::HookEngine,
}

// ── Small utility methods ──────────────────────────────────────────────────

impl AgentRuntime {
    /// Enqueue a user message for processing after the current turn ends.
    ///
    /// This is the shared "type-ahead" mechanism — when a frontend receives
    /// a user message while `run_agent_loop` is still processing, it can
    /// call this method and the loop will pick it up automatically.
    ///
    /// Returns `true` if the message was queued (the loop is running).
    /// Returns `false` if the queue is not being consumed (no loop active);
    /// the frontend should start a new loop manually.
    pub fn queue_user_message(&self, msg: QueuedUserMessage) -> bool {
        let mut queue = self.queued_messages.lock().unwrap();
        let was_empty = queue.is_empty();
        queue.push_back(msg);
        // If the queue already had items, the loop is definitely running.
        // If it was empty, the caller needs to verify a loop is active.
        !was_empty
    }

    /// Compose the static system prompt — called exactly once per session lifetime.
    ///
    /// Content: base prompt + environment info.
    /// Result is persisted to the session DB record and never changes.
    pub fn compose_static_system_prompt(&self, base_prompt: &str) -> String {
        let base_prompt = base_prompt.trim();
        let system_info = SystemInfo::detect();
        let working_dir = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let is_git = tidev_session::system_info::is_git_repo(&self.workspace_root);

        let mut prompt = String::new();
        if !base_prompt.is_empty() {
            prompt.push_str(base_prompt);
        }
        prompt.push_str("\n\nHere is some useful information about the environment:\n<env>\n  ");
        prompt.push_str(&format!("Working directory: {}\n  ", working_dir));
        prompt.push_str(&format!(
            "Workspace root folder: {}\n  ",
            self.workspace_root.display()
        ));
        prompt.push_str(&format!(
            "Is directory a git repo: {}\n  ",
            if is_git { "yes" } else { "no" }
        ));
        prompt.push_str(&system_info.format_env());
        // Inform the model which shell the bash tool uses (Windows only).
        #[cfg(windows)]
        {
            let shell = crate::shell::get();
            prompt.push_str(&format!("Shell: {}\n  ", shell.display_name));
        }
        prompt.push_str("\n</env>");
        prompt
    }

    /// Inject instruction files into the last user message if they
    /// haven't been injected yet in this session.
    ///
    /// Uses `session_instruction_sources` in the database to track which
    /// files have already been injected, so the same file is never injected
    /// twice — even across process restarts.
    async fn inject_new_instructions(
        &mut self,
        session_id: uuid::Uuid,
        last_user_msg: &mut tidev_session::session::Message,
    ) -> Result<bool> {
        let (instruction_prompt, sources, new_cache) =
            crate::instructions::system_prompt_and_sources_with_cache(
                &self.workspace_root,
                &self.config_dir,
                &self.instructions,
                &self.instruction_content_cache,
            )
            .unwrap_or_default();
        self.instruction_content_cache = new_cache;

        if instruction_prompt.is_empty() || sources.is_empty() {
            return Ok(false);
        }

        // Load already-injected sources from database (persists across restarts).
        let already_injected_raw = {
            let store = self.store.lock().await;
            store.load_instruction_sources(session_id)?
        };

        // Normalize already-injected paths to canonical absolute form so they
        // match the format used by system_prompt_and_sources_with_cache().
        // DB may contain relative paths (saved by TUI's update_loaded_instruction_sources)
        // or absolute paths (saved by a previous run of this function).
        // Normalizing both to canonical absolutes ensures correct comparison.
        let already_injected: Vec<String> = already_injected_raw
            .iter()
            .map(|s| {
                if s.starts_with("http://") || s.starts_with("https://") {
                    return s.clone();
                }
                let path = if std::path::Path::new(s).is_absolute() {
                    std::path::PathBuf::from(s)
                } else {
                    self.workspace_root.join(s)
                };
                canonicalize_display(&path).display().to_string()
            })
            .collect();

        // Find sources that haven't been injected yet.
        let new_sources: Vec<&String> = sources
            .iter()
            .filter(|s| !already_injected.contains(s))
            .collect();

        if new_sources.is_empty() {
            return Ok(false);
        }

        // Build <system-reminder> content for the new instruction files.
        let mut sections = Vec::new();
        for source in &new_sources {
            if let Some(content) = self.instruction_content_cache.get(*source) {
                sections.push(format!("Instructions from: {}\n{}", source, content));
            }
        }

        if sections.is_empty() {
            return Ok(false);
        }

        let injection = format!(
            "<system-reminder>\n{}\n</system-reminder>",
            sections.join("\n\n")
        );
        last_user_msg.content = format!("{}\n\n{}", injection, last_user_msg.content);

        // Persist: update message content + record sources as injected.
        let store = self.store.lock().await;
        store.update_message_content(last_user_msg.id, &last_user_msg.content)?;
        for source in &new_sources {
            store.append_instruction_source(session_id, source)?;
        }
        drop(store);

        log::info!(
            "injected {} new instruction file(s) into user message {}",
            new_sources.len(),
            last_user_msg.id
        );

        Ok(true)
    }

    /// Inject memory context into the first user message of a session
    /// (only when no assistant message exists yet).
    async fn inject_first_turn_memory(
        &self,
        session_id: uuid::Uuid,
        last_user_msg: &mut tidev_session::session::Message,
        has_assistant: bool,
    ) -> Result<bool> {
        if has_assistant || !self.config.memory.enabled || !self.config.memory.inject_context {
            return Ok(false);
        }

        let ws = self.workspace_root.display().to_string();
        let memory_store = self.tools.memory_store();
        let mut sections: Vec<String> = Vec::new();

        macro_rules! timed_memory_op {
            ($label:expr, $body:expr) => {{
                let _start = std::time::Instant::now();
                let _result = $body;
                let _elapsed = _start.elapsed();
                log::debug!("inject_first_turn_memory: {} took {:?}", $label, _elapsed);
                if _elapsed > std::time::Duration::from_millis(500) {
                    log::warn!(
                        "inject_first_turn_memory: {} took {:?} (slow)",
                        $label,
                        _elapsed
                    );
                }
                _result
            }};
        }

        // ── Session summaries (other sessions) ──────────────────────────
        if let Ok(summaries) = timed_memory_op!(
            "load_other_session_summaries",
            memory_store.load_other_session_summaries(&session_id, 5)
        ) && !summaries.is_empty()
        {
            sections.push(Self::format_session_summaries(&summaries));
        }

        // ── Consolidated knowledge (cross-session facts) ────────────────
        if let Ok(facts) = timed_memory_op!(
            "load_consolidated_facts",
            memory_store.load_consolidated_facts(&ws, 5)
        ) && !facts.is_empty()
        {
            let mut block = "## Consolidated Project Knowledge\n".to_string();
            for fact in &facts {
                block.push_str(&format!(
                    "- {} (confidence: {:.1})\n",
                    fact.content, fact.strength
                ));
            }
            sections.push(block);
        }

        // ── Consolidated procedures ─────────────────────────────────────
        if let Ok(procs) = timed_memory_op!(
            "load_consolidated_procedures",
            memory_store.load_consolidated_procedures(&ws, 3)
        ) && !procs.is_empty()
        {
            let mut block = "## Reusable Procedures\n".to_string();
            for proc in &procs {
                block.push_str(&format!("- **{}**: {}\n", proc.title, proc.content));
            }
            sections.push(block);
        }

        // ── Memory slots ────────────────────────────────────────────────
        if let Ok(slot_content) =
            timed_memory_op!("render_pinned_slots", memory_store.render_pinned_slots(&ws))
            && !slot_content.is_empty()
        {
            sections.push(slot_content);
        }

        // ── Compose final injection ─────────────────────────────────────
        if sections.is_empty() {
            return Ok(false);
        }

        let injection = format!(
            "\n\n<system-reminder>\n{}\n</system-reminder>",
            sections.join("\n\n")
        );
        last_user_msg.content.push_str(&injection);

        // Persist the updated message
        {
            let store = self.store.lock().await;
            store.update_message_content(last_user_msg.id, &last_user_msg.content)?;
        }

        log::info!(
            "injected {} memory section(s) into first user message of session {}",
            sections.len(),
            session_id
        );

        Ok(true)
    }

    /// Format session summaries into a Markdown block.
    fn format_session_summaries(summaries: &[crate::memory::SessionSummary]) -> String {
        let mut parts = vec!["## Related Session Summaries".to_string()];
        for s in summaries {
            let title = s.title.as_deref().unwrap_or("(untitled)");
            let narrative = s.narrative.as_deref().unwrap_or("");
            let decisions_str = if s.key_decisions.is_empty() {
                String::new()
            } else {
                format!("\n    Decisions: {}", s.key_decisions.join(", "))
            };
            let files_str = if s.files_modified.is_empty() {
                String::new()
            } else {
                format!("\n    Files: {}", s.files_modified.join(", "))
            };
            parts.push(format!(
                "- **{}**: {}{}{}",
                title, narrative, decisions_str, files_str,
            ));
        }
        parts.join("\n")
    }

    /// Get all available tool definitions (built-in + MCP).
    pub fn tool_definitions(&self) -> Vec<crate::tooling::ToolDefinition> {
        self.tools.all_definitions()
    }
}

/// Check whether a tool name corresponds to a file operation.
///
/// Matches agentmemory's PreToolUse matcher `"Edit|Write|Read|Glob|Grep"`
/// plus tidev-specific tools (apply_patch).
fn is_file_operation(tool_name: &str) -> bool {
    matches!(
        crate::tooling::canonical_tool_name(tool_name),
        Some("read" | "write" | "edit" | "apply_patch" | "grep" | "glob")
    )
}

// ── Tests ──────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests;
