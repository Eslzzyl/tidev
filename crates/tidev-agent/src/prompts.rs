//! Mode reminders injected into sessions based on the current session mode.

use tidev_llm::mode::SessionMode;

// ── Public API ──────────────────────────────────────────────────────────────

/// Mode reminder for a given session mode.
pub fn mode_reminder(mode: SessionMode) -> String {
    match mode {
        SessionMode::Plan => plan_mode_reminder(),
        SessionMode::Build => build_mode_reminder(),
    }
}

// ── Mode reminders ──────────────────────────────────────────────────────────

fn plan_constraints() -> &'static str {
    r#"You are FORBIDDEN from writing, editing, applying patches, or running any shell command that modifies files; only read-only commands such as grep, glob, read, ls, cat, and git log are allowed, and you must ensure these commands do not change any state. When delegating sub-agents, only explorer, librarian, and oracle are permitted — never fixer."#
}

fn build_constraints() -> &'static str {
    r#"Implement changes with write, edit, or apply_patch. Preserve existing style, and verify with build or test before finishing."#
}

/// Plan mode reminder injected into the first user message of a session.
pub fn plan_mode_reminder() -> String {
    format!(
        "<system-reminder>\nYou are in Plan mode. READ-ONLY.\n\n{constraints}\n</system-reminder>",
        constraints = plan_constraints(),
    )
}

/// Build mode reminder injected into the first user message of a session.
pub fn build_mode_reminder() -> String {
    format!(
        "<system-reminder>\nYou are in Build mode.\n\n{constraints}\n</system-reminder>",
        constraints = build_constraints(),
    )
}

/// Plan switch reminder shown when switching to Plan mode mid-conversation.
pub fn plan_switch_reminder() -> String {
    format!(
        "<system-reminder>\nThe user switched to Plan mode since this message. READ-ONLY.\n\n{constraints}\n</system-reminder>",
        constraints = plan_constraints(),
    )
}

/// Build switch reminder shown when switching to Build mode mid-conversation.
pub fn build_switch_reminder() -> String {
    format!(
        "<system-reminder>\nThe user switched to Build mode since this message.\n\n{constraints}\n</system-reminder>",
        constraints = build_constraints(),
    )
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_reminders_wrapped_in_system_reminder_tags() {
        let reminders = [
            plan_mode_reminder(),
            build_mode_reminder(),
            plan_switch_reminder(),
            build_switch_reminder(),
        ];
        for reminder in &reminders {
            assert!(
                reminder.starts_with("<system-reminder>"),
                "Reminder should start with <system-reminder>: {reminder:?}"
            );
            assert!(
                reminder.ends_with("</system-reminder>"),
                "Reminder should end with </system-reminder>: {reminder:?}"
            );
        }
    }

    #[test]
    fn test_plan_reminder_forbids_writing_build_reminder_allows() {
        let plan = plan_mode_reminder();
        let build = build_mode_reminder();
        assert!(
            plan.contains("FORBIDDEN"),
            "Plan reminder must forbid writing"
        );
        assert!(
            plan.contains("READ-ONLY"),
            "Plan reminder must state READ-ONLY"
        );
        assert!(build.contains("write"), "Build reminder must allow write");
        assert!(build.contains("edit"), "Build reminder must allow edit");
        assert!(
            !build.contains("FORBIDDEN"),
            "Build reminder must NOT forbid writing"
        );
    }
}
