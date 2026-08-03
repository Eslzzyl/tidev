//! Conversion from tidev's tool metadata to the LLM protocol definition.

use tidev_tools::types::ToolDefinition;

/// Convert a host-owned tool definition to the protocol representation.
pub(crate) fn to_llm_tool_def(def: &ToolDefinition) -> tidev_llm::ToolDefinition {
    tidev_llm::ToolDefinition {
        name: def.name.clone(),
        display_name: def.display_name.clone(),
        description: def.description.clone(),
        parameters: def.parameters.clone(),
    }
}
