use crate::config::ActiveModel;

use super::{AgentDefinition, AgentType};

/// Configuration overrides for a specific agent type.
///
/// These can be loaded from `config.toml` to customise individual agents.
#[derive(Clone, Debug, Default)]
pub struct AgentOverride {
    /// Override the model used by this agent.
    pub model: Option<ActiveModel>,
    /// Custom system prompt that replaces the default entirely.
    pub custom_prompt: Option<String>,
    /// Extra text appended to the default system prompt.
    pub append_prompt: Option<String>,
    /// Override temperature.
    pub temperature: Option<f32>,
    /// Override tool restrictions. `Some(vec![])` = no tools allowed.
    pub allowed_tools: Option<Vec<String>>,
}

/// Create an `AgentDefinition` from an `AgentType` with optional overrides.
pub fn create_agent(agent_type: AgentType, overrides: Option<&AgentOverride>) -> AgentDefinition {
    let mut def = AgentDefinition::new(agent_type);

    if let Some(overrides) = overrides {
        // Apply model override
        if let Some(model) = &overrides.model {
            def.model_override = Some(model.clone());
        }

        // Apply prompt customisation
        if let Some(custom_prompt) = &overrides.custom_prompt {
            def.system_prompt = custom_prompt.clone();
        } else if let Some(append) = &overrides.append_prompt {
            def.system_prompt = format!("{}\n\n{}", def.system_prompt, append);
        }

        // Apply temperature
        if let Some(temp) = overrides.temperature {
            def.temperature = Some(temp);
        }

        // Apply tool restrictions
        if let Some(tools) = &overrides.allowed_tools {
            def.allowed_tools = Some(tools.clone());
        }
    }

    def
}

/// Create definitions for all built-in agent types with a common base config.
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
    fn test_create_agent_defaults() {
        let def = create_agent(AgentType::Explorer, None);
        assert_eq!(def.agent_type, AgentType::Explorer);
        assert!(def.read_only);
        assert!(def.model_override.is_none());
    }

    #[test]
    fn test_create_agent_with_override() {
        let model = ActiveModel {
            provider_id: "openai".to_string(),
            provider_display_name: "OpenAI".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: crate::config::ApiType::OpenAiChatCompletions,
            model_id: "gpt-4o".to_string(),
            request_model_id: "gpt-4o".to_string(),
            display_name: "GPT-4o".to_string(),
            context_window: 128_000,
            max_output_tokens: 4096,
            temperature: Some(0.1),
            supports_images: true,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: crate::config::reasoning::ThinkingLevelType::None,
        };

        let overrides = AgentOverride {
            model: Some(model.clone()),
            custom_prompt: None,
            append_prompt: Some("Extra instructions.".to_string()),
            temperature: Some(0.5),
            allowed_tools: Some(vec!["read".to_string(), "grep".to_string()]),
        };

        let def = create_agent(AgentType::Explorer, Some(&overrides));
        assert_eq!(def.temperature, Some(0.5));
        assert!(def.model_override.is_some());
        assert!(def.system_prompt.contains("Extra instructions."));
        assert_eq!(
            def.allowed_tools,
            Some(vec!["read".to_string(), "grep".to_string()])
        );
    }

    #[test]
    fn test_custom_prompt_replaces_default() {
        let overrides = AgentOverride {
            model: None,
            custom_prompt: Some("You are a custom agent.".to_string()),
            append_prompt: None,
            temperature: None,
            allowed_tools: None,
        };

        let def = create_agent(AgentType::Explorer, Some(&overrides));
        assert_eq!(def.system_prompt, "You are a custom agent.");
    }
}
