//! Agent type classification and definitions.
//!
//! This module defines the built-in agent types supported by tidev.
//! Each agent type has a specialized role, system prompt (in tidev-agent),
//! default tool permissions, and optional model overrides.

/// The built-in agent types supported by tidev.
///
/// Each agent type has a specialized system prompt, default tool permissions,
/// and optional model overrides. The General agent serves as the default and
/// includes multi-agent delegation capabilities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentType {
    /// Default agent — handles general tasks and delegates to sub-agents.
    General,
    /// Codebase exploration specialist — fast grep/glob/read, read-only.
    Explorer,
    /// Documentation and library research specialist.
    Librarian,
    /// Strategic advisor — architecture decisions, code review, debugging.
    Oracle,
    /// UI/UX design specialist.
    Designer,
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
            Self::Designer,
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
            Self::Designer => "designer",
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
            Self::Designer => "UI/UX design specialist: frontend design, styling, user experience",
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
    /// `None` means all tools are allowed (subject to session mode permissions).
    pub fn default_tool_restrictions(self) -> Option<&'static [&'static str]> {
        match self {
            // General can use everything.
            Self::General => None,
            // Explorer: read-only search tools + bash for fast searching (no write commands).
            Self::Explorer => Some(&[
                "read",
                "glob",
                "grep",
                "bash",
                "websearch",
                "webfetch",
            ]),
            // Librarian: research tools + bash for source-code study (no code modification).
            Self::Librarian => Some(&[
                "read",
                "glob",
                "grep",
                "bash",
                "websearch",
                "webfetch",
                "question",
            ]),
            // Oracle: read-only analysis.
            Self::Oracle => Some(&[
                "read",
                "glob",
                "grep",
                "websearch",
                "webfetch",
                "question",
            ]),
            // Designer: read + write for design work.
            Self::Designer => Some(&[
                "read",
                "glob",
                "grep",
                "write",
                "edit",
                "bash",
                "websearch",
                "webfetch",
                "question",
                "apply_patch",
            ]),
            // Fixer: full tool access for implementation.
            Self::Fixer => None,
        }
    }

    /// Default temperature for this agent type.
    pub fn default_temperature(self) -> f32 {
        match self {
            Self::Explorer | Self::Librarian | Self::Oracle => 0.1,
            Self::Fixer => 0.2,
            Self::Designer => 0.7,
            Self::General => 0.3,
        }
    }

    /// Parse from a string (case-insensitive, accepts display_name, optionally @-prefixed).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.strip_prefix('@').unwrap_or(&s);
        match s {
            "general" => Some(Self::General),
            "explorer" => Some(Self::Explorer),
            "librarian" => Some(Self::Librarian),
            "oracle" => Some(Self::Oracle),
            "designer" => Some(Self::Designer),
            "fixer" => Some(Self::Fixer),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_type_parse() {
        assert_eq!(AgentType::parse("explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("@explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("EXPLORER"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("general"), Some(AgentType::General));
        assert_eq!(AgentType::parse("unknown"), None);
    }

    #[test]
    fn test_agent_type_read_only() {
        assert!(AgentType::Explorer.is_read_only());
        assert!(AgentType::Librarian.is_read_only());
        assert!(AgentType::Oracle.is_read_only());
        assert!(!AgentType::Fixer.is_read_only());
        assert!(!AgentType::General.is_read_only());
        assert!(!AgentType::Designer.is_read_only());
    }

    #[test]
    fn test_all_includes_general() {
        let all = AgentType::all();
        assert!(all.contains(&AgentType::General));
        assert!(all.contains(&AgentType::Fixer));
        assert_eq!(all.len(), 6);
    }

    #[test]
    fn test_display_name_roundtrip() {
        for agent in AgentType::all() {
            let name = agent.display_name();
            assert_eq!(AgentType::parse(name), Some(*agent));
        }
    }

    #[test]
    fn test_description_not_empty() {
        for agent in AgentType::all() {
            assert!(!agent.description().is_empty());
        }
    }

    #[test]
    fn test_default_temperature_range() {
        for agent in AgentType::all() {
            let t = agent.default_temperature();
            assert!((0.0..=1.0).contains(&t), "temp {} out of range", t);
        }
    }
}
