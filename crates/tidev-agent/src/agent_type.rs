//! Agent creation — concrete agent definitions and constructor functions.
//!
//! The types [`AgentType`], [`AgentDefinition`], and [`AgentOverride`] are
//! defined in tidev-types.  This module provides constructors that depend on
//! tidev-agent's prompt templates.

use tidev_types::agent_type::{AgentDefinition, AgentOverride, AgentType};

/// Create a default [`AgentDefinition`] for the given agent type, using the
/// system prompt from tidev-agent's prompt templates.
fn default_definition(agent_type: AgentType) -> AgentDefinition {
    let system_prompt = crate::prompts::system_prompt(agent_type);
    AgentDefinition {
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

/// Create an [`AgentDefinition`] from an [`AgentType`] with optional overrides.
pub fn create_agent(agent_type: AgentType, overrides: Option<&AgentOverride>) -> AgentDefinition {
    let mut def = default_definition(agent_type);

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
        .map(|agent_type| default_definition(*agent_type))
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
    .map(|agent_type| default_definition(*agent_type))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_types::agent_type::AgentOverride;

    #[test]
    fn test_create_agent_defaults() {
        let def = create_agent(AgentType::Explorer, None);
        assert_eq!(def.display_name, "explorer");
        assert!(def.read_only);
        assert!(def.allowed_tools.is_some());
        let tools = def.allowed_tools.as_ref().unwrap();
        assert!(tools.contains(&"grep".to_string()));
        assert!(tools.contains(&"bash".to_string()));
        assert!(!tools.contains(&"write".to_string()));
    }

    #[test]
    fn test_create_agent_with_overrides() {
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
