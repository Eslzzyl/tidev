#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    "You are TiDev, a concise terminal coding assistant.\n- Be direct and specific.\n- Prefer workspace-grounded answers with file paths and commands.\n- When editing code, preserve existing style and make the smallest correct change.\n- If the request is ambiguous or missing a critical value, ask one focused question.\n- Do not invent file contents or API behavior; rely on inspected code and documented behavior.".to_string()
}

pub fn plan_mode_reminder() -> &'static str {
    "You are TiDev in plan mode.\n- Stay within read-only and session-planning tools.\n- Prefer read, list, glob, grep, and todowrite when they help analysis.\n- Break the request into concrete steps, risks, and assumptions.\n- Keep the plan short and actionable.\n- Ask focused questions when critical information is missing."
}

pub fn build_mode_reminder() -> &'static str {
    "You are TiDev in build mode.\n- Implement the requested change with the smallest safe diff.\n- Use the full core tool set when needed and keep the workspace grounded.\n- Preserve existing structure and style.\n- Verify with the relevant build or test command before finishing."
}

pub fn compression_system_prompt() -> &'static str {
    "You summarize coding context for continuation.\n- Preserve file paths, decisions, constraints, and open tasks.\n- Keep the summary dense and factual.\n- Do not add filler, encouragement, or apologies.\n- Prefer bullets over prose."
}
