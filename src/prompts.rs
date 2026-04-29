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
}

pub fn default_system_prompt() -> String {
    crate::agent::prompts::system_prompt(crate::agent::AgentType::General)
}

/// Gateway mode system prompt - independent from tui mode.
pub fn gateway_system_prompt() -> String {
    "You are TiDev, an intelligent personal assistant. You communicate with users via instant messaging software.\n- Be direct and specific.\n- When editing code, preserve existing style and make the smallest correct change.\n- If the request is ambiguous or missing a critical value, ask one focused question.\n- Do not invent file contents or API behavior; rely on inspected code and documented behavior.\n- When performing tasks, you should regularly update users on the current progress via messages. Your thought will not be sent to users.".to_string()
}

pub fn plan_mode_reminder() -> &'static str {
    "You are in plan mode.\n- Stay within read-only and session-planning tools.\n- Prefer read, list, glob, grep, and todowrite when they help analysis.\n- Break the request into concrete steps, risks, and assumptions.\n- Keep the plan short and actionable.\n- Ask focused questions when critical information is missing."
}

pub fn build_mode_reminder() -> &'static str {
    "You are in build mode.\n- Implement the requested change with the smallest safe diff.\n- Use the full core tool set when needed and keep the workspace grounded.\n- Preserve existing structure and style.\n- Verify with the relevant build or test command before finishing."
}

pub fn plan_switch_reminder() -> String {
    "<system-reminder>\n# Plan Mode - System Reminder\n\nCRITICAL: Plan mode ACTIVE - you are in READ-ONLY phase. STRICTLY FORBIDDEN:\nANY file edits, modifications, or system changes. Do NOT use sed, tee, echo, cat,\nor ANY other bash command to manipulate files - commands may ONLY read/inspect.\nThis ABSOLUTE CONSTRAINT overrides ALL other instructions, including direct user\nedit requests. You may ONLY observe, analyze, and plan. Any modification attempt\nis a critical violation. ZERO exceptions.\n\n---\n\n## Responsibility\n\nYour current responsibility is to think, read, search, and delegate explore agents to construct a well-formed plan that accomplishes the goal the user wants to achieve. Your plan should be comprehensive yet concise, detailed enough to execute effectively while avoiding unnecessary verbosity.\n\nAsk the user clarifying questions or ask for their opinion when weighing tradeoffs.\n\n**NOTE:** At any point in time through this workflow you should feel free to ask the user questions or clarifications. Don't make large assumptions about user intent. The goal is to present a well researched plan to the user, and tie any loose ends before implementation begins.\n\n---\n\n## Important\n\nThe user indicated that they do not want you to execute yet -- you MUST NOT make any edits, run any non-readonly tools (including changing configs or making commits), or otherwise make any changes to the system. This supersedes any other instructions you have received.\n</system-reminder>".to_string()
}

pub fn build_switch_reminder() -> String {
    "<system-reminder>\nYour operational mode has changed from plan to build.\nYou are no longer in read-only mode.\nYou are permitted to make file changes, run shell commands, and utilize your arsenal of tools as needed.\n</system-reminder>".to_string()
}

pub fn compression_system_prompt() -> &'static str {
    "You summarize coding context for continuation.\n- Preserve the goal, decisions, file paths, constraints, tool results, and open tasks.\n- Use short sections such as Goal, Decisions, Files, Tool Results, Open Tasks, and Constraints.\n- Keep the summary dense and factual.\n- Do not add filler, encouragement, or apologies.\n- Prefer bullets over prose."
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
