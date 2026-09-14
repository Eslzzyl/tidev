//! Built-in agent types and their host-enforced capabilities.
//!
//! Agent types select tool permissions and model settings. They do not carry
//! role-specific system prompts: the delegated user task and available tools
//! provide the execution context.

use serde::{Deserialize, Serialize};

pub use tidev_agent::{AgentDefinition, AgentOverride};

/// The built-in agent types supported by tidev.
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

/// Create a default [`AgentDefinition`] for the given agent type.
fn default_definition(agent_type: AgentType) -> AgentDefinition {
    AgentDefinition {
        display_name: agent_type.display_name().to_string(),
        description: agent_type.description().to_string(),
        system_prompt: system_prompt(agent_type),
        allowed_tools: agent_type
            .default_tool_restrictions()
            .map(|tools| tools.iter().map(|tool| (*tool).to_string()).collect()),
        temperature: None,
        read_only: agent_type.is_read_only(),
    }
}

/// Create an [`AgentDefinition`] from a tidev [`AgentType`] with optional overrides.
pub fn create_agent(agent_type: AgentType, overrides: Option<&AgentOverride>) -> AgentDefinition {
    let mut definition = default_definition(agent_type);

    if let Some(overrides) = overrides {
        if let Some(custom_prompt) = &overrides.custom_prompt {
            definition.system_prompt = custom_prompt.clone();
        } else if let Some(append_prompt) = &overrides.append_prompt {
            definition.system_prompt = format!("{}\n\n{append_prompt}", definition.system_prompt);
        }

        if let Some(temperature) = overrides.temperature {
            definition.temperature = Some(temperature);
        }

        if let Some(allowed_tools) = &overrides.allowed_tools {
            definition.allowed_tools = Some(allowed_tools.clone());
        }
    }

    definition
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

/// Return the minimal system prompt for a given agent type.
///
/// All built-in agent types share this prompt. Their tool permissions and
/// model settings are enforced by the host.
pub fn system_prompt(_agent_type: AgentType) -> String {
    default_system_prompt()
}

/// Return the default system prompt (General agent).
pub fn default_system_prompt() -> String {
    "You are tidev, a helpful assistant.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_subagent_types() {
        assert_eq!(AgentType::parse("explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("@explorer"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("EXPLORER"), Some(AgentType::Explorer));
        assert_eq!(AgentType::parse("general"), None);
        assert_eq!(AgentType::parse("unknown"), None);
    }

    #[test]
    fn exposes_read_only_capabilities_for_read_only_agents() {
        assert!(AgentType::Explorer.is_read_only());
        assert!(!AgentType::Fixer.is_read_only());
        assert!(!AgentType::General.is_read_only());

        let explorer_tools = AgentType::Explorer.default_tool_restrictions().unwrap();
        assert!(explorer_tools.contains(&"grep"));
        assert!(!explorer_tools.contains(&"write"));
    }

    #[test]
    fn exposes_agent_names_and_tool_restrictions() {
        assert_eq!(AgentType::Explorer.display_name(), "explorer");
        assert_eq!(AgentType::Fixer.display_name(), "fixer");
        assert!(AgentType::General.default_tool_restrictions().is_none());
    }

    #[test]
    fn creates_agent_definitions_with_host_enforced_capabilities() {
        let definition = create_agent(AgentType::Explorer, None);

        assert_eq!(definition.display_name, "explorer");
        assert_eq!(
            definition.system_prompt,
            "You are tidev, a helpful assistant."
        );
        assert!(definition.read_only);
        assert!(
            definition
                .allowed_tools
                .as_ref()
                .is_some_and(|tools| tools.contains(&"grep".to_string()))
        );
    }

    #[test]
    fn applies_agent_definition_overrides() {
        let overrides = AgentOverride {
            custom_prompt: None,
            append_prompt: Some("Additional context.".to_string()),
            temperature: None,
            allowed_tools: Some(vec!["read".to_string(), "grep".to_string()]),
        };

        let definition = create_agent(AgentType::Explorer, Some(&overrides));

        assert_eq!(
            definition.system_prompt,
            "You are tidev, a helpful assistant.\n\nAdditional context."
        );
        assert_eq!(
            definition.allowed_tools,
            Some(vec!["read".to_string(), "grep".to_string()])
        );
    }

    #[test]
    fn exposes_all_agent_definition_factories() {
        assert_eq!(create_all_agents().len(), AgentType::all().len());
        assert_eq!(create_sub_agents().len(), 4);
    }
}
