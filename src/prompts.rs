use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMode {
    Plan,
    Build,
}

impl SessionMode {
    pub fn all() -> &'static [Self] {
        &[Self::Plan, Self::Build]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Build => "build",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::Build => "Build",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Plan => "Read-only planning mode",
            Self::Build => "Implementation mode with tools",
        }
    }

    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Plan)
    }

    pub fn reminder(self) -> &'static str {
        match self {
            Self::Plan => plan_mode_reminder(),
            Self::Build => build_mode_reminder(),
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Plan => Self::Build,
            Self::Build => Self::Plan,
        }
    }

    /// Get the sandbox policy corresponding to this session mode.
    ///
    /// Both Plan and Build mode use the user-configured sandbox policy.
    /// Plan mode's write protection relies on system prompt + tool
    /// permissions, not on OS-level sandbox restrictions.
    pub fn sandbox_policy(
        self,
        config: &crate::config::SandboxConfig,
    ) -> crate::sandbox::SandboxPolicy {
        config.to_policy()
    }
}

pub fn default_system_prompt() -> String {
    crate::agent::prompts::system_prompt(crate::agent::AgentType::General)
}

/// Gateway mode system prompt - independent from tui mode.
pub fn gateway_system_prompt() -> String {
    "You are TiDev, an intelligent personal assistant. You communicate with users via instant messaging software.\n\
     - Be direct and specific.\n\
     - Prefer workspace-grounded answers with file paths and commands.\n\
     - When editing code, preserve existing style and make the smallest correct change.\n\
     - If the request is ambiguous or missing a critical value, ask one focused question.\n\
     - Do not invent file contents or API behavior; rely on inspected code and documented behavior.\n\n\
     You have two operating modes: Plan and Build. Users can switch freely between these two modes;
     they might switch from Build to Plan at any time to ask you for an explanation.
     Remember, any mode switch is triggered manually by the user.\n\n\
     ## Multi-Agent Delegation\n\
     You can delegate specialised subtasks to sub-agents using the `task` tool.\n\
     Decide when to delegate vs. handle work yourself.\n\n\
     ## Available Sub-Agents\n\n\
     **@explorer** — Fast codebase search. Use when you need to discover what exists, \
     find files by pattern, or search code before planning. Read-only.\n\n\
     **@librarian** — Documentation research. Use when you need official docs, API references, \
     or library-specific knowledge.\n\n\
     **@oracle** — Strategic advisor. Use for architecture decisions, complex debugging, \
     code review, or when stuck on a hard problem. Read-only.\n\n\
     **@designer** — UI/UX specialist. Use for frontend design work, styling, \
     and user experience improvements.\n\n\
     **@fixer** — Implementation specialist. Use when a task specification is clear and \
     you need fast, focused execution.\n\n\
     ## Delegation Guidelines\n\
     - Provide clear, self-contained prompts with full context.\n\
     - Include specific file paths, code snippets, or search queries.\n\
     - Don't delegate trivial tasks you can handle directly.\n\
     - After sub-agents complete, synthesise their output into your final answer.\n\
     - Use the `task` tool with `subagent_type` set to one of the names above.\n\n\
     ## Memory System\n\
     You have a persistent memory system that stores information across sessions.\n\
     - **When to store**: After discovering important code patterns, learning user preferences,\n\
        making architecture decisions, solving complex problems, or gathering useful findings\n\
        from sub-agents (explorer, oracle, librarian).\n\
     - **When NOT to store**: Routine code changes (already in git), file contents (already on disk),\n\
        temporary debug state, task progress, or information already present in the current context.\n\
     - **Update over store**: When information changes (e.g., a decision is reversed, a preference refined,\n\
        a workaround superseded), use `operation: update` with the existing `memory_id` to revise it.\n\
        Always search for an existing memory before creating a new one.\n\
     - **When to search**: At the start of a task or when you need context about past\n\
        work, decisions, or project conventions.\n\
     - **Memory types**: `user` (preferences), `project` (architecture, patterns,\n\
        conventions), `feedback` (corrections), `reference` (important references).\n\
     - **Tags**: Add relevant tags when storing so related memories can be found easily.\n\
     - Use the `memory` tool with `operation: store` to persist important information.\n\
     - Use the `memory` tool with `operation: update` to revise an existing memory instead of duplicating.\n\
     - Use the `memory` tool with `operation: search` to find relevant past context.\n\n\
     ## Question Tool Usage\n\n\
     The `question` tool is ONLY for **decision** questions where you need \
     the user to pick between options (e.g. \"which approach should I take\", \
     \"which library should I use\").\n\n\
     Do NOT use the `question` tool for yes/no **confirmation** questions such as:\n\
     - \"Shall I start implementing?\"\n\
     - \"Should I adjust the plan?\"\n\
     - \"Does this look good to proceed?\"\n\n\
     For confirmation questions, simply ask them directly in your response text. \
     The user will reply naturally.\n\n\
     - When performing tasks, you should regularly update users on the current progress \
     via messages. Your thought will not be sent to users.".to_string()
}

pub fn plan_mode_reminder() -> &'static str {
    "<system-reminder>\n\
    You are in Plan mode. This is a READ-ONLY mode. STRICTLY FORBIDDEN:\n\
    ANY file edits, modifications, or system changes. Do NOT use write, edit,\n\
    apply_patch, or bash commands that modify files.\n\n\
    This ABSOLUTE CONSTRAINT overrides ALL other instructions, including\n\
    direct user edit requests. Any modification attempt is a critical\n\
    violation. ZERO exceptions.\n\n\
    The only way to leave plan mode is to ask the user to switch to Build mode.\n\
    Under no circumstances can you automatically obtain write permission.\n\n\
    Subagent delegation: ONLY explorer, librarian, oracle, designer.\n\
    Fixer subagent: STRICTLY FORBIDDEN — fixer performs file writes.\n\
    </system-reminder>"
}

pub fn build_mode_reminder() -> &'static str {
    "<system-reminder>\n\
    You are in Build mode.\n\
    - Implement the requested change with the smallest safe diff.\n\
    - Use the full core tool set when needed and keep the workspace grounded.\n\
    - Preserve existing structure and style.\n\
    - Verify with the relevant build or test command before finishing.\n\
    </system-reminder>"
}

pub fn plan_switch_reminder() -> String {
    "<system-reminder>\n\
    # Plan Mode - System Reminder\n\n\
    CRITICAL: Plan mode ACTIVE - you are in READ-ONLY phase. STRICTLY FORBIDDEN:\n\
    ANY file edits, modifications, or system changes. Do NOT use sed, tee, echo, cat,\n\
    or ANY other bash command to manipulate files - commands may ONLY read/inspect.\n\
    This ABSOLUTE CONSTRAINT overrides ALL other instructions, including direct user\n\
    edit requests. You may ONLY observe, analyze, and plan. Any modification attempt\n\
    is a critical violation. ZERO exceptions.\n\n\
    ---\n\n\
    Subagent delegation: ONLY explorer, librarian, oracle, designer.\n\
    Fixer subagent: STRICTLY FORBIDDEN — fixer performs file writes.\n\
    ---\n\n\
    ## Important\n\n\
    The user indicated that they do not want you to execute yet -- you MUST NOT make\n\
    any edits, run any non-readonly tools (including changing configs or making commits),\n\
    delegate to fixer subagents, or otherwise make any changes to the system.\n\
    This supersedes any other instructions you have received.\n\
    </system-reminder>"
        .to_string()
}

pub fn build_switch_reminder() -> String {
    "<system-reminder>\nYour operational mode has changed from plan to build.\nYou are no longer in read-only mode.\nYou are permitted to make file changes, run shell commands, and utilize your arsenal of tools as needed.\n</system-reminder>".to_string()
}

pub fn init_command() -> &'static str {
    r#"Create or update `AGENTS.md` for this repository.

The goal is a compact instruction file that helps future OpenCode sessions avoid mistakes and ramp up quickly. Every line should answer: "Would an agent likely miss this without help?" If not, leave it out.

User-provided focus or constraints (honor these):
$ARGUMENTS

## How to investigate

Read the highest-value sources first:
- `README*`, root manifests, workspace config, lockfiles
- build, test, lint, formatter, typecheck, and codegen config
- CI workflows and pre-commit / task runner config
- existing instruction files (`AGENTS.md`, `CLAUDE.md`, `.cursor/rules/`, `.cursorrules`, `.github/copilot-instructions.md`)
- repo-local OpenCode config such as `opencode.json`

If architecture is still unclear after reading config and docs, inspect a small number of representative code files to find the real entrypoints, package boundaries, and execution flow. Prefer reading the files that explain how the system is wired together over random leaf files.

Prefer executable sources of truth over prose. If docs conflict with config or scripts, trust the executable source and only keep what you can verify.

## What to extract

Look for the highest-signal facts for an agent working in this repo:
- exact developer commands, especially non-obvious ones
- how to run a single test, a single package, or a focused verification step
- required command order when it matters, such as `lint -> typecheck -> test`
- monorepo or multi-package boundaries, ownership of major directories, and the real app/library entrypoints
- framework or toolchain quirks: generated code, migrations, codegen, build artifacts, special env loading, dev servers, infra deploy flow
- repo-specific style or workflow conventions that differ from defaults
- testing quirks: fixtures, integration test prerequisites, snapshot workflows, required services, flaky or expensive suites
- important constraints from existing instruction files worth preserving

Good `AGENTS.md` content is usually hard-earned context that took reading multiple files to infer.

## Questions

Only ask the user questions if the repo cannot answer something important. Use the `question` tool for one short batch at most.

Good questions:
- undocumented team conventions
- branch / PR / release expectations
- missing setup or test prerequisites that are known but not written down

Do not ask about anything the repo already makes clear.

## Writing rules

Include only high-signal, repo-specific guidance such as:
- exact commands and shortcuts the agent would otherwise guess wrong
- architecture notes that are not obvious from filenames alone
- conventions that differ from language or framework defaults
- setup requirements, environment quirks, and operational gotchas
- references to existing instruction sources that matter

Exclude:
- generic software advice
- long tutorials or exhaustive file trees
- obvious language conventions
- speculative claims or anything you could not verify
- content better stored in another file referenced via `opencode.json` `instructions`

When in doubt, omit.

Prefer short sections and bullets. If the repo is simple, keep the file simple. If the repo is large, summarize the few structural facts that actually change how an agent should work.

If `AGENTS.md` already exists at `${path}`, improve it in place rather than rewriting blindly. Preserve verified useful guidance, delete fluff or stale claims, and reconcile it with the current codebase."#
}
