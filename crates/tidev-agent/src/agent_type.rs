//! Agent type definitions — the built-in agent roles supported by tidev.

use serde::{Deserialize, Serialize};

/// The built-in agent types supported by tidev.
///
/// Each agent type has a specialized system prompt, default tool permissions,
/// and optional model overrides. The [`General`](AgentType::General) agent
/// serves as the default and includes multi-agent delegation capabilities.
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
            Self::Designer => {
                "UI/UX design specialist: frontend design, styling, user experience"
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
            Self::Designer => Some(&[
                "read", "glob", "grep", "write", "edit", "bash",
                "websearch", "webfetch", "question", "apply_patch",
            ]),
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

    /// Parse from a string (case-insensitive, accepts optional "@" prefix).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.strip_prefix('@').unwrap_or(&s);
        match s {
            "explorer" => Some(Self::Explorer),
            "librarian" => Some(Self::Librarian),
            "oracle" => Some(Self::Oracle),
            "designer" => Some(Self::Designer),
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
/// by tidev-core's [`AgentContext`](crate::context::AgentContext).
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
    /// Create a new agent definition with defaults for the given type.
    pub fn new(agent_type: AgentType) -> Self {
        let system_prompt = crate::prompts::system_prompt(agent_type);
        Self {
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

/// Create an [`AgentDefinition`] from an [`AgentType`] with optional overrides.
pub fn create_agent(agent_type: AgentType, overrides: Option<&AgentOverride>) -> AgentDefinition {
    let mut def = AgentDefinition::new(agent_type);

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
        .map(|agent_type| AgentDefinition::new(*agent_type))
        .collect()
}

/// Create definitions for all sub-agent types (everything except General).
pub fn create_sub_agents() -> Vec<AgentDefinition> {
    [
        AgentType::Explorer,
        AgentType::Librarian,
        AgentType::Oracle,
        AgentType::Designer,
        AgentType::Fixer,
    ]
    .iter()
    .map(|agent_type| AgentDefinition::new(*agent_type))
        .collect()
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
        assert!(AgentType::Librarian.is_read_only());
        assert!(AgentType::Oracle.is_read_only());
        assert!(!AgentType::Fixer.is_read_only());
        assert!(!AgentType::General.is_read_only());
        assert!(!AgentType::Designer.is_read_only());
    }

    #[test]
    fn test_agent_definition_defaults() {
        let def = AgentDefinition::new(AgentType::Explorer);
        assert_eq!(def.display_name, "explorer");
        assert!(def.read_only);
        assert!(def.allowed_tools.is_some());
        let tools = def.allowed_tools.as_ref().unwrap();
        assert!(tools.contains(&"grep".to_string()));
        assert!(tools.contains(&"bash".to_string()));
        assert!(!tools.contains(&"write".to_string()));
    }

    #[test]
    fn test_create_agent_defaults() {
        let def = create_agent(AgentType::Explorer, None);
        assert_eq!(def.agent_type, AgentType::Explorer);
        assert!(def.read_only);
    }

    #[test]
    fn test_create_agent_with_override() {
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
}
