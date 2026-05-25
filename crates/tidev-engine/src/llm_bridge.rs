//! Bridge types for converting engine config types into `tidev-llm` config types.

use crate::config::ActiveModel;
use crate::tooling::ToolDefinition as EngineToolDef;
use tidev_llm::{LlmProviderConfig, ToolDefinition as LlmToolDef};

impl From<ActiveModel> for LlmProviderConfig {
    fn from(m: ActiveModel) -> Self {
        LlmProviderConfig {
            provider_id: m.provider_id,
            api_type: m.api_type,
            api_key: m.api_key,
            base_url: m.base_url,
            model_id: m.model_id,
            request_model_id: Some(m.request_model_id).filter(|s| !s.is_empty()),
            system_prompt: Some(m.system_prompt).filter(|s| !s.is_empty()),
            thinking_level: m.thinking_level,
            extra_body: m.extra_body,
            max_output_tokens: m.max_output_tokens,
            temperature: m.temperature,
            supports_images: m.supports_images,
        }
    }
}

impl<'a> From<&'a EngineToolDef> for LlmToolDef {
    fn from(t: &'a EngineToolDef) -> Self {
        LlmToolDef {
            name: t.name.clone(),
            display_name: t.display_name.clone(),
            description: t.description.clone(),
            parameters: t.parameters.clone(),
        }
    }
}
