//! Built-in agent types supported by tidev.
//!
//! Each agent type has a specialized role.  This type is shared across
//! tidev-agent (execution), tidev-tools (validation), tidev-core (routing),
//! and tidev-tui (display), so it lives in tidev-types.

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
            Self::General => {
                "General-purpose assistant with multi-agent delegation"
            }
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
            Self::Explorer => Some(&[
                "read", "glob", "grep", "bash", "websearch", "webfetch",
            ]),
            Self::Librarian => Some(&[
                "read", "glob", "grep", "bash", "websearch", "webfetch", "question",
            ]),
            Self::Oracle => Some(&[
                "read", "glob", "grep", "websearch", "webfetch", "question",
            ]),
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
}
