//! Generic agent profile types.

/// A host-independent description of an agent role.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentDefinition {
    /// Human-readable display name.
    pub display_name: String,
    /// Short description suitable for tool schemas or UI summaries.
    pub description: String,
    /// System prompt for this role.
    pub system_prompt: String,
    /// Optional tool allow-list. `None` means the host may expose all tools.
    pub allowed_tools: Option<Vec<String>>,
    /// Optional model temperature override.
    pub temperature: Option<f32>,
    /// Whether the role must remain read-only.
    pub read_only: bool,
}

impl AgentDefinition {
    /// Return the bootstrap content for a newly-created role session.
    pub fn bootstrap_content(&self) -> String {
        self.system_prompt.clone()
    }
}

/// Generic overrides that hosts may load from their own configuration.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentOverride {
    /// Replace the role's default prompt.
    pub custom_prompt: Option<String>,
    /// Append text to the role's default prompt.
    pub append_prompt: Option<String>,
    /// Override the role's temperature.
    pub temperature: Option<f32>,
    /// Override the role's tool allow-list.
    pub allowed_tools: Option<Vec<String>>,
}
