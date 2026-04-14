use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderSource {
    User,
    Bundled,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiType {
    #[default]
    OpenAi,
    Anthropic,
}

impl ApiType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "anthropic" => Self::Anthropic,
            _ => Self::OpenAi,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub api_type: Option<String>,
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelConfig {
    pub display_name: String,
    pub context_window: usize,
    pub max_output_tokens: usize,
    pub temperature: f32,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default)]
    pub extra_body: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}
