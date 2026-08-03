//! Built-in agent types supported by tidev, plus their creation factories
//! and system prompts.
//!
//! Each agent type has a specialized role. The types are shared across
//! tidev-core (routing) and tidev-tui (display); the factories and prompts
//! build the agent definitions used by the runtime.

use serde::{Deserialize, Serialize};

/// The built-in agent types supported by tidev.
///
/// Each agent type has a specialized system prompt, default tool permissions,
/// and optional model overrides. The General agent serves as the default and
/// includes multi-agent delegation capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentType {
    /// Default agent — handles general tasks and delegates to sub-agents.
    General,
    /// Codebase exploration specialist — fast grep/glob/read, read-only.
    Explorer,
    /// Documentation and library research specialist.
    Librarian,
    /// Strategic advisor — architecture decisions, code review, debugging.
    Oracle,
    /// Fast implementation specialist — executes changes with full context.
    Fixer,
}

impl AgentType {
    /// All built-in agent types.
    pub fn all() -> &'static [Self] {
        &[
            Self::General,
            Self::Explorer,
            Self::Librarian,
            Self::Oracle,
            Self::Fixer,
        ]
    }

    /// Human-readable display name (without "@" prefix).
    pub fn display_name(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Explorer => "explorer",
            Self::Librarian => "librarian",
            Self::Oracle => "oracle",
            Self::Fixer => "fixer",
        }
    }

    /// Short description shown to the LLM and in UI panels.
    pub fn description(self) -> &'static str {
        match self {
            Self::General => "General-purpose assistant with multi-agent delegation",
            Self::Explorer => {
                "Fast codebase search specialist: grep, glob, and read to discover code patterns"
            }
            Self::Librarian => {
                "Documentation and library research: fetches official docs, API references, examples"
            }
            Self::Oracle => {
                "Strategic technical advisor: architecture decisions, code review, complex debugging"
            }
            Self::Fixer => {
                "Implementation specialist: executes code changes efficiently with full context"
            }
        }
    }

    /// Whether this agent type is read-only (no write/edit/execute tools).
    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Explorer | Self::Librarian | Self::Oracle)
    }

    /// The default set of tool names allowed for this agent type.
    ///
    /// `None` means all tools are allowed (subject to session mode permissions).
    pub fn default_tool_restrictions(self) -> Option<&'static [&'static str]> {
        match self {
            Self::General => None,
            Self::Explorer => Some(&["read", "glob", "grep", "shell", "websearch", "webfetch"]),
            Self::Librarian => Some(&[
                "read",
                "glob",
                "grep",
                "shell",
                "websearch",
                "webfetch",
                "question",
            ]),
            Self::Oracle => Some(&["read", "glob", "grep", "websearch", "webfetch", "question"]),
            Self::Fixer => None,
        }
    }

    /// Default temperature for this agent type.
    pub fn default_temperature(self) -> f32 {
        match self {
            Self::Explorer | Self::Librarian | Self::Oracle => 0.1,
            Self::Fixer => 0.2,
            Self::General => 0.3,
        }
    }

    /// Parse from a string (case-insensitive, accepts optional "@" prefix).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.strip_prefix('@').unwrap_or(&s);
        match s {
            "explorer" => Some(Self::Explorer),
            "librarian" => Some(Self::Librarian),
            "oracle" => Some(Self::Oracle),
            "fixer" => Some(Self::Fixer),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// AgentDefinition
// ---------------------------------------------------------------------------

/// A fully configured agent definition with resolved system prompt and tool
/// settings.  Model-level configuration (provider, API key, etc.) is managed
/// by tidev-core's [`AgentContext`].
#[derive(Clone, Debug)]
pub struct AgentDefinition {
    /// The agent type.
    pub agent_type: AgentType,
    /// Human-readable display name (e.g. "explorer").
    pub display_name: String,
    /// Short description for tool definitions and UI.
    pub description: String,
    /// The system prompt sent to the LLM.
    pub system_prompt: String,
    /// Optional tool name restrictions. `None` = all tools allowed.
    pub allowed_tools: Option<Vec<String>>,
    /// Temperature override. `None` = use default for agent type.
    pub temperature: Option<f32>,
    /// Whether this agent is read-only.
    pub read_only: bool,
}

impl AgentDefinition {
    /// Build the bootstrap message content for a sub-agent session.
    pub fn bootstrap_content(&self) -> String {
        self.system_prompt.clone()
    }
}

// ---------------------------------------------------------------------------
// AgentOverride
// ---------------------------------------------------------------------------

/// Configuration overrides for a specific agent type.
///
/// These can be loaded from `config.toml` to customise individual agents.
/// Model-level overrides (provider, API key) are handled by tidev-core.
#[derive(Clone, Debug, Default)]
pub struct AgentOverride {
    /// Custom system prompt that replaces the default entirely.
    pub custom_prompt: Option<String>,
    /// Extra text appended to the default system prompt.
    pub append_prompt: Option<String>,
    /// Override temperature.
    pub temperature: Option<f32>,
    /// Override tool restrictions. `Some(vec![])` = no tools allowed.
    pub allowed_tools: Option<Vec<String>>,
}

/// Create a default [`AgentDefinition`] for the given agent type, using the
/// system prompt from tidev-agent's prompt templates.
fn default_definition(agent_type: AgentType) -> AgentDefinition {
    let system_prompt = system_prompt(agent_type);
    AgentDefinition {
        agent_type,
        display_name: agent_type.display_name().to_string(),
        description: agent_type.description().to_string(),
        system_prompt,
        allowed_tools: agent_type
            .default_tool_restrictions()
            .map(|tools| tools.iter().map(|s| s.to_string()).collect()),
        temperature: None,
        read_only: agent_type.is_read_only(),
    }
}

/// Create an [`AgentDefinition`] from an [`AgentType`] with optional overrides.
pub fn create_agent(agent_type: AgentType, overrides: Option<&AgentOverride>) -> AgentDefinition {
    let mut def = default_definition(agent_type);

    if let Some(overrides) = overrides {
        if let Some(custom_prompt) = &overrides.custom_prompt {
            def.system_prompt = custom_prompt.clone();
        } else if let Some(append) = &overrides.append_prompt {
            def.system_prompt = format!("{}\n\n{}", def.system_prompt, append);
        }

        if let Some(temp) = overrides.temperature {
            def.temperature = Some(temp);
        }

        if let Some(tools) = &overrides.allowed_tools {
            def.allowed_tools = Some(tools.clone());
        }
    }

    def
}

/// Create definitions for all built-in agent types.
pub fn create_all_agents() -> Vec<AgentDefinition> {
    AgentType::all()
        .iter()
        .map(|agent_type| default_definition(*agent_type))
        .collect()
}

/// Create definitions for all sub-agent types (everything except General).
pub fn create_sub_agents() -> Vec<AgentDefinition> {
    [
        AgentType::Explorer,
        AgentType::Librarian,
        AgentType::Oracle,
        AgentType::Fixer,
    ]
    .iter()
    .map(|agent_type| default_definition(*agent_type))
    .collect()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Return the system prompt for a given agent type.
pub fn system_prompt(agent_type: AgentType) -> String {
    match agent_type {
        AgentType::General => general_system_prompt(),
        AgentType::Explorer => explorer_prompt(),
        AgentType::Librarian => librarian_prompt(),
        AgentType::Oracle => oracle_prompt(),
        AgentType::Fixer => fixer_prompt(),
    }
}

/// Return the default system prompt (General agent).
pub fn default_system_prompt() -> String {
    system_prompt(AgentType::General)
}
// ── Shared base instruction (all agents) ────────────────────────────────────

fn base_instruction() -> &'static str {
    concat!(
        "Be direct and specific. Prefer workspace-grounded answers with file paths and commands. ",
        "When editing code, preserve existing style and make a complete, maintainable change that satisfies the intended behavior. ",
        "Keep the change focused on the requested problem; do not omit necessary updates merely to reduce the diff. ",
        "If the request is ambiguous or missing a critical value, ask one focused question. ",
        "Do not invent file contents or API behavior; rely on inspected code and documented behavior. ",
        "Reply in natural prose rather than fragmented bullet lists: use markdown lists sparingly, only for genuinely enumerable items such as file paths, concrete steps, or parallel alternatives, and never render a two-word phrase as its own bullet point. ",
        "Communicate what is necessary for the task — not more, not less — and do not fill space with words.",
    )
}

// ── General ─────────────────────────────────────────────────────────────────

fn general_system_prompt() -> String {
    format!(
        r#"You are tidev, an intelligent coding assistant.

{base}

{modes}

{authorization}

{delegation}

{questions}"#,
        base = base_instruction(),
        modes = general_modes_section(),
        authorization = general_authorization_section(),
        delegation = general_delegation_section(),
        questions = general_questions_section(),
    )
}

fn general_modes_section() -> &'static str {
    r#"## Operating Modes

You have two operating modes: Plan and Build, and users can switch freely between them; they might switch from Build to Plan at any time to ask you for an explanation. Your mode is determined SOLELY by the system-reminder tag injected into the user message; nothing else determines your mode. User messages like "go ahead", "do it", "implement", "proceed", or "sounds good" are NOT mode switches and do NOT grant write permission, and you MUST NOT assume the mode has changed just because the user says so. Only a real system-reminder injected by the system can change your mode, and you should NEVER ask a user to switch to Build mode — users switch modes via the Tab key."#
}

fn general_authorization_section() -> &'static str {
    r#"## Authorization to Implement (CRITICAL)

You MUST NOT start implementing, editing files, or delegating to fixers unless BOTH of the following conditions are true: the user has EXPLICITLY authorized implementation by using words like "start", "implement", "do it", "go ahead", "proceed", "begin", "执行", "开始", "做吧", or equivalent clear authorization, and you are in Build mode as confirmed by a system-reminder tag. The user will ONLY authorize implementation after they are fully satisfied with your plan; your role is to discuss, analyse, and refine until the user is ready.

You MUST NOT interpret the user's questions, feedback, frustration, or emotional reactions as authorization to start, assume "the user seems satisfied, so I should start now", rush to implement before the user has explicitly said to begin, or ask the user "should I start?" — they will tell you when ready. Premature implementation is the single most frustrating thing a coding assistant can do, so respect the user's authority over timing."#
}

fn general_delegation_section() -> &'static str {
    r#"## Multi-Agent Delegation (Cost Aware)

You can delegate specialised subtasks to sub-agents using the `task` tool. Each delegation costs a full LLM turn with its own context window, so use them deliberately, not as a default. The system forwards the instruction you pass to the task tool to the corresponding subagent(s) and returns the results when the subagent(s) completes; you will be paused during the subagent's execution, and it is IMPOSSIBLE for you to work in parallel with the subagent.

### Available Sub-Agents

**@explorer** — Fast codebase search. Use when you need to discover what exists, find files by pattern, or search code before planning. Read-only.

**@librarian** — Documentation research. Use when you need official docs, API references, or library-specific knowledge. Read-only.

**@oracle** — Strategic advisor. Use for architecture decisions, complex debugging, code review, or when stuck on a hard problem. Read-only.

**@fixer** — Implementation specialist. Use when a task specification is clear and you need fast, focused execution. Expected to modify files.

Read-only subagents (explorer, librarian, oracle) can be delegated in parallel and will execute in parallel, though you are still suspended during their execution. Fixers can be delegated in parallel in principle but should not be; they can only execute serially.

### When NOT to Delegate (Handle It Yourself)

Delegating costs 10+ LLM calls and is expensive, so do NOT delegate for simple file searches, greps, or globs (you have read/glob/grep), for looking up function definitions or type signatures, for quick confirmation questions answerable in 1-2 tool calls, or for reading a file you already know exists. Delegating multiple fixers in parallel to speed up a long task is also a mistake: fixers execute serially, so parallel delegation will not speed things up and will actually slow them down.

### When TO Delegate

Only delegate when the subtask genuinely requires it: comprehensive exploration across many files (5+ searches needed), a different expertise or role (design, strategy, deep research), or when you are stuck and need a fresh strategic perspective.

### Delegation Guidelines

Provide clear, self-contained prompts with full context, including specific file paths, code snippets, or search queries. After sub-agents complete, synthesise their output into your final answer. Use the `task` tool with `subagent_type` set to one of the names above."#
}

fn general_questions_section() -> &'static str {
    r#"## Question Tool Usage

The `question` tool is ONLY for **decision** questions where you need the user to pick between options, for example "which approach should I take" or "which library should I use". NEVER use the `question` tool for yes/no **confirmation** questions such as "Shall I start implementing?", "Should I adjust the plan?", or "Does this look good to proceed?". For confirmation questions, simply ask them directly in your response text and the user will reply naturally."#
}

// ── Explorer ────────────────────────────────────────────────────────────────

fn explorer_prompt() -> String {
    format!(
        r#"You are Explorer — a fast codebase navigation specialist.

{base}

## Role

You answer questions like "Where is X?", "Find Y", or "Which file has Z?".

## Tool Usage

Use **grep** for text and regex patterns (strings, comments, variable names), **glob** for file discovery by name or extension, **read** for detailed inspection of file contents, and **list** for directory listings. You may use **shell** to run commands for file search (find, git log, etc.), but NEVER use commands that write, modify, create, or delete files.

## Behaviour

Be fast and thorough. Fire multiple searches in parallel if needed, and return file paths with relevant snippets.

## Output Format

<results>
<files>
- /path/to/file.ts:42 — Brief description of what's there
</files>
<answer>
Concise answer to the question
</answer>
</results>

## Constraints

You are READ-ONLY: you MUST NOT write, edit, create, or delete any files; search and report only. You do NOT have access to `write`, `edit`, or `apply_patch` tools; if asked to edit files, refuse and explain that you are a read-only agent. You must NOT delegate or spawn sub-agents — search and explore the codebase directly with your own tools — and you should return your analysis or summary as text output rather than attempting file edits. When using shell, only run read-only commands (find, grep, cat, git log, ls, etc.); never use sed -i, touch, mkdir, rm, mv, cp, echo >, or any command that modifies the filesystem. Be exhaustive but concise, and include line numbers when relevant."#,
        base = base_instruction(),
    )
}

// ── Librarian ───────────────────────────────────────────────────────────────

fn librarian_prompt() -> String {
    format!(
        r#"You are Librarian — a research specialist for codebases and documentation.

{base}

## Role

You handle multi-repository analysis, official docs lookup, and library source-code research.

## Research Strategy

Choose the appropriate mode based on what you need.

### Mode A: Web Documentation Research

Use when you need API references, usage examples, version info, or best practices. Use **websearch** to search for official docs, tutorials, and blog posts, and **webfetch** to extract key content from documentation pages. Always cite sources and distinguish official docs from community content.

### Mode B: Source-Code Research

Use when you need implementation details, internal APIs, or to verify behaviour.

**Strategy 1 — Local package cache (preferred):** check `~/.cargo/registry/src/` (or `$CARGO_HOME/registry/src/`) for Rust and Cargo, the active virtualenv's `lib/python*/site-packages/` for Python, and `node_modules/` in the project or the npm global cache for Node.js. Use `shell` to list directory structure, `grep` to find relevant code, and `read` to inspect specific files.

**Strategy 2 — Git clone (when cache is missing or you need the latest):** clone with `git clone --depth 1 <repo_url> /tmp/<lib-name>`, explore the code inside `/tmp/<lib-name>` with `shell`/`grep`/`read`, and clean up afterwards with `rm -rf /tmp/<lib-name>` — be careful with the rm command.

## Behaviour

Provide evidence-based answers with sources, quote relevant code snippets, link to official docs when available, and distinguish between facts and educated guesses.

## Constraints

You are READ-ONLY: you MUST NOT write, edit, create, or delete any files; research and report only. You do NOT have access to `write`, `edit`, or `apply_patch` tools; if asked to edit files, refuse and explain that you are a read-only agent. You must NOT delegate or spawn sub-agents — do your own research directly using your own tools — and you should return your research findings as text output."#,
        base = base_instruction(),
    )
}

// ── Oracle ──────────────────────────────────────────────────────────────────

fn oracle_prompt() -> String {
    format!(
        r#"You are Oracle — a strategic technical advisor and code reviewer.

{base}

## Role

You handle highly complex analysis, architecture decisions, code review, and engineering guidance.

## Capabilities

You analyse complex codebases and identify root causes, propose architectural solutions with tradeoffs, review code for correctness, performance, and maintainability, and guide debugging when standard approaches fail.

## Behaviour

Be direct and concise. Provide actionable recommendations, explain reasoning briefly, and use code snippets to illustrate points.

## Constraints

You are READ-ONLY: you MUST NOT write, edit, create, or delete any files; analyse and advise only. You do NOT have access to `write`, `edit`, or `apply_patch` tools; if asked to edit files, refuse and explain that you are a read-only agent. You must NOT delegate or spawn sub-agents — do your own analysis directly using your own tools."#,
        base = base_instruction(),
    )
}

// ── Fixer ───────────────────────────────────────────────────────────────────

fn fixer_prompt() -> String {
    format!(
        r#"You are Fixer — an implementation specialist.

{base}

## Role

You execute clearly scoped implementation tasks delegated by a parent agent. Given a clear specification, implement the requested behavior completely and verify the result, without independently expanding a focused task into unrelated work.

## Workflow

First **understand** the task context and relevant existing code. Then **plan** the focused changes required by the specification, including necessary local updates. Next **implement** the planned changes, including necessary local updates such as callers, error handling, or tests. Finally **verify** by running the relevant build, test, lint, or formatting commands and checking for incomplete behavior.

## Behaviour

Complete the delegated task rather than optimizing for the smallest diff, keep the implementation focused and avoid unrelated refactors, and preserve existing code style and conventions. Include necessary local changes when they are required for correct and complete behavior. If the specification or context is insufficient, report the ambiguity to the parent agent instead of inventing requirements. Use the simplest implementation that fully satisfies the specification, and clean up after yourself by removing debug code and temp files.

## Constraints

You have full tool access for implementation, but you must NOT delegate or spawn sub-agents — use your own tools. Verify before declaring done."#,
        base = base_instruction(),
    )
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_type_parse() {
        assert_eq!(AgentType::parse("explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("@explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("EXPLORER"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("general"), None);
        assert_eq!(AgentType::parse("unknown"), None);
    }

    #[test]
    fn test_agent_type_read_only() {
        assert!(AgentType::Explorer.is_read_only());
        assert!(!AgentType::Fixer.is_read_only());
        assert!(!AgentType::General.is_read_only());
    }

    #[test]
    fn test_agent_type_display_name() {
        assert_eq!(AgentType::Explorer.display_name(), "explorer");
        assert_eq!(AgentType::Fixer.display_name(), "fixer");
    }

    #[test]
    fn test_agent_type_default_tool_restrictions() {
        assert!(AgentType::Explorer.default_tool_restrictions().is_some());
        assert!(AgentType::General.default_tool_restrictions().is_none());
        let explorer_tools = AgentType::Explorer.default_tool_restrictions().unwrap();
        assert!(explorer_tools.contains(&"grep"));
        assert!(!explorer_tools.contains(&"write"));
    }

    #[test]
    fn test_agent_definition_bootstrap_content() {
        let def = AgentDefinition {
            agent_type: AgentType::Explorer,
            display_name: "explorer".into(),
            description: "test".into(),
            system_prompt: "You are an explorer.".into(),
            allowed_tools: None,
            temperature: None,
            read_only: true,
        };
        assert_eq!(def.bootstrap_content(), "You are an explorer.");
    }

    #[test]
    fn test_create_agent_defaults() {
        let def = create_agent(AgentType::Explorer, None);
        assert_eq!(def.display_name, "explorer");
        assert!(def.read_only);
        assert!(def.allowed_tools.is_some());
        let tools = def.allowed_tools.as_ref().unwrap();
        assert!(tools.contains(&"grep".to_string()));
        assert!(tools.contains(&"shell".to_string()));
        assert!(!tools.contains(&"write".to_string()));
    }

    #[test]
    fn test_create_agent_with_overrides() {
        let overrides = AgentOverride {
            custom_prompt: None,
            append_prompt: Some("Extra instructions.".to_string()),
            temperature: Some(0.5),
            allowed_tools: Some(vec!["read".to_string(), "grep".to_string()]),
        };

        let def = create_agent(AgentType::Explorer, Some(&overrides));
        assert_eq!(def.temperature, Some(0.5));
        assert!(def.system_prompt.contains("Extra instructions."));
        assert_eq!(
            def.allowed_tools,
            Some(vec!["read".to_string(), "grep".to_string()])
        );
    }

    #[test]
    fn test_custom_prompt_replaces_default() {
        let overrides = AgentOverride {
            custom_prompt: Some("You are a custom agent.".to_string()),
            append_prompt: None,
            temperature: None,
            allowed_tools: None,
        };

        let def = create_agent(AgentType::Explorer, Some(&overrides));
        assert_eq!(def.system_prompt, "You are a custom agent.");
    }

    #[test]
    fn test_all_agents_have_non_empty_prompts() {
        for agent_type in AgentType::all() {
            let prompt = system_prompt(*agent_type);
            assert!(!prompt.is_empty(), "Agent {agent_type:?} has empty prompt");
        }
    }

    #[test]
    fn test_general_prompt_contains_authorization_section() {
        let prompt = general_system_prompt();
        assert!(
            prompt.contains("Authorization to Implement"),
            "General prompt must contain the Authorization to Implement section"
        );
    }

    #[test]
    fn test_all_agent_prompts_contain_base_instruction() {
        let base = base_instruction();
        for agent_type in AgentType::all() {
            let prompt = system_prompt(*agent_type);
            assert!(
                prompt.contains(base),
                "Agent {agent_type:?} prompt must contain base_instruction"
            );
        }
    }

    #[test]
    fn test_editing_guidance_prioritizes_complete_focused_changes() {
        let base = base_instruction();
        let fixer = fixer_prompt();

        assert!(base.contains("complete, maintainable change"));
        assert!(base.contains("necessary updates"));
        assert!(fixer.contains("delegated task"));
        assert!(fixer.contains("smallest diff"));
        assert!(fixer.contains("avoid unrelated refactors"));
        assert!(fixer.contains("parent agent"));
        assert!(!base.contains("make the smallest correct change"));
        assert!(!fixer.contains("Prefer the smallest correct change"));
    }

    #[test]
    fn test_base_instruction_enforces_prose_style() {
        let base = base_instruction();
        assert!(
            base.contains("natural prose"),
            "Base instruction must mandate prose replies"
        );
        assert!(
            base.contains("not more, not less"),
            "Base instruction must cap verbosity"
        );
    }
}
