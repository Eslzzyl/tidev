//! Prompt text helpers shared across the workspace.

use crate::Mode;

/// Mode reminder for a given session mode.
pub fn mode_reminder(mode: Mode) -> String {
    match mode {
        Mode::Plan => plan_mode_reminder(),
        Mode::Build => build_mode_reminder(),
    }
}

pub fn plan_mode_reminder() -> String {
    "<system-reminder>\nYou are in Plan mode.\n</system-reminder>".to_string()
}

pub fn build_mode_reminder() -> String {
    "<system-reminder>\nYou are in Build mode.\n</system-reminder>".to_string()
}

pub fn plan_switch_reminder() -> String {
    "<system-reminder>\nThe user switched to Plan mode since this message.\n</system-reminder>"
        .to_string()
}

pub fn build_switch_reminder() -> String {
    "<system-reminder>\nThe user switched to Build mode since this message.\n</system-reminder>"
        .to_string()
}

/// Reminder appended to a user message that was steered into a running turn.
///
/// Steering messages are submitted while the agent loop is busy and are
/// inserted at the next request boundary without interrupting the in-flight
/// model stream. The reminder tells the model to keep advancing the task
/// while adjusting direction according to the message.
///
/// The reminder is appended as a suffix (mode reminders are injected as a
/// prefix by `inject_mode_reminder_impl`) so both can coexist on the same
/// message. It is persisted with the message so the request bytes stay
/// stable across replays; the TUI strips the tag for display.
pub fn steer_reminder() -> String {
    "<system-reminder>\nThis is a steering message from the user. Continue the current task and adjust your direction according to this message.\n</system-reminder>"
        .to_string()
}

/// Generate the `/init` command text with `$ARGUMENTS` replaced by the
/// given args, so the user can pre-fill the prompt before editing.
pub fn init_command_with_args(args: &str) -> String {
    let template = r#"Create or update `AGENTS.md` for this repository.

The goal is a compact instruction file that helps future tidev sessions avoid mistakes and ramp up quickly. Every line should answer: "Would an agent likely miss this without help?" If not, leave it out.

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

If `AGENTS.md` already exists at `${path}`, improve it in place rather than rewriting blindly. Preserve verified useful guidance, delete fluff or stale claims, and reconcile it with the current codebase."#;
    template.replace("$ARGUMENTS", args.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_reminders_are_short_and_mode_specific() {
        assert_eq!(
            plan_mode_reminder(),
            "<system-reminder>\nYou are in Plan mode.\n</system-reminder>"
        );
        assert_eq!(
            build_mode_reminder(),
            "<system-reminder>\nYou are in Build mode.\n</system-reminder>"
        );
    }

    #[test]
    fn switch_reminders_describe_the_new_mode() {
        assert_eq!(
            plan_switch_reminder(),
            "<system-reminder>\nThe user switched to Plan mode since this message.\n</system-reminder>"
        );
        assert_eq!(
            build_switch_reminder(),
            "<system-reminder>\nThe user switched to Build mode since this message.\n</system-reminder>"
        );
    }
}
