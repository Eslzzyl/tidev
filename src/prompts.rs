#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptPreset {
    TidevDefault,
    Plan,
    Review,
    ApplyPatch,
    Compact,
    ProviderSetup,
}

impl PromptPreset {
    pub fn all() -> &'static [Self] {
        &[
            Self::TidevDefault,
            Self::Plan,
            Self::Review,
            Self::ApplyPatch,
            Self::Compact,
            Self::ProviderSetup,
        ]
    }

    pub fn from_str(value: &str) -> Option<Self> {
        let normalized = value
            .trim()
            .to_ascii_lowercase()
            .replace('-', "_")
            .replace(' ', "_");

        match normalized.as_str() {
            "default" | "tidev_default" | "concise" | "coding" => Some(Self::TidevDefault),
            "plan" | "planning" => Some(Self::Plan),
            "review" | "code_review" => Some(Self::Review),
            "apply_patch" | "patch" | "implementation" => Some(Self::ApplyPatch),
            "compact" | "summary" | "compaction" => Some(Self::Compact),
            "provider_setup" | "connect" | "onboarding" => Some(Self::ProviderSetup),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TidevDefault => "tidev_default",
            Self::Plan => "plan",
            Self::Review => "review",
            Self::ApplyPatch => "apply_patch",
            Self::Compact => "compact",
            Self::ProviderSetup => "provider_setup",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::TidevDefault => "TiDev default",
            Self::Plan => "Planning",
            Self::Review => "Review",
            Self::ApplyPatch => "Implementation",
            Self::Compact => "Context compaction",
            Self::ProviderSetup => "Provider setup",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::TidevDefault => "Concise terminal coding assistant",
            Self::Plan => "Break work into a short implementation plan",
            Self::Review => "Focus on bugs, regressions, and missing tests",
            Self::ApplyPatch => "Prefer minimal, safe code edits",
            Self::Compact => "Summarize long conversations for continuation",
            Self::ProviderSetup => "Guide provider onboarding in the TUI",
        }
    }

    pub fn body(self) -> &'static str {
        match self {
            Self::TidevDefault => {
                "You are TiDev, a concise terminal coding assistant.\n- Be direct and specific.\n- Prefer workspace-grounded answers with file paths and commands.\n- When editing code, preserve existing style and make the smallest correct change.\n- If the request is ambiguous or missing a critical value, ask one focused question.\n- Do not invent file contents or API behavior; rely on inspected code and documented behavior."
            }
            Self::Plan => {
                "You are TiDev in planning mode.\n- Break the request into concrete steps.\n- Call out unknowns, risks, and assumptions.\n- Keep the plan short and actionable.\n- Do not edit files unless the user explicitly asks you to implement the plan."
            }
            Self::Review => {
                "You are TiDev in code review mode.\n- Prioritize bugs, regressions, missing tests, and security issues.\n- Reference exact files and lines when possible.\n- Keep findings ordered by severity.\n- Do not over-explain or repeat the diff."
            }
            Self::ApplyPatch => {
                "You are TiDev in implementation mode.\n- Make the smallest safe change that solves the problem at the root cause.\n- Preserve existing structure and style.\n- Prefer direct code edits over speculative refactors.\n- Verify with the relevant build or test command before finishing."
            }
            Self::Compact => {
                "You summarize coding context for continuation.\n- Preserve file paths, decisions, constraints, and open tasks.\n- Keep the summary dense and factual.\n- Do not add filler, encouragement, or apologies.\n- Prefer bullets over prose."
            }
            Self::ProviderSetup => {
                "You help configure a new provider for TiDev.\n- Ask for provider id, display name, base URL, optional API key env, model id, model display name, and prompt preset.\n- Keep the flow short and validation-focused.\n- Prefer OpenAI-compatible base URLs and explicit model ids.\n- If a value is invalid, explain how to fix it in one sentence."
            }
        }
    }
}

pub fn default_system_prompt() -> String {
    PromptPreset::TidevDefault.body().to_string()
}

pub fn resolve_system_prompt(preset: Option<&str>) -> Option<String> {
    preset
        .and_then(PromptPreset::from_str)
        .map(|preset| preset.body().to_string())
}

pub fn compression_system_prompt() -> &'static str {
    PromptPreset::Compact.body()
}

pub fn catalog_lines() -> Vec<String> {
    PromptPreset::all()
        .iter()
        .map(|preset| format!("{} - {}", preset.as_str(), preset.description()))
        .collect()
}
