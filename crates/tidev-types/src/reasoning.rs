//! Thinking / reasoning level types shared across the tidev workspace.
//!
//! These types were originally defined in `tidev-engine/src/config/reasoning.rs`
//! but are needed by `tidev-session` (the `Message` struct stores an optional
//! `ThinkingLevelType`).  Moving them here breaks the circular dependency.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    High,
    Max,
}

impl ThinkingLevel {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::High => "High",
            Self::Max => "Max",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::High => Self::Max,
            Self::Max => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeepSeekV4ThinkingLevel {
    #[default]
    Off,
    High,
    Max,
}

impl DeepSeekV4ThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "thinking": { "type": "disabled" }
            }),
            Self::High => serde_json::json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "high"
            }),
            Self::Max => serde_json::json!({
                "thinking": { "type": "enabled" },
                "reasoning_effort": "max"
            }),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::High => "High",
            Self::Max => "Max",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::High => Self::Max,
            Self::Max => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "high" => Self::High,
            "max" => Self::Max,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Qwen35ThinkingLevel {
    #[default]
    Off,
    On,
}

impl Qwen35ThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "chat_template_kwargs": { "enable_thinking": false }
            }),
            Self::On => serde_json::json!({
                "chat_template_kwargs": { "enable_thinking": true }
            }),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "on" => Self::On,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlmThinkingLevel {
    #[default]
    Off,
    On,
}

impl GlmThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "chat_template_kwargs": { "enable_thinking": false }
            }),
            Self::On => serde_json::json!({
                "chat_template_kwargs": { "enable_thinking": true }
            }),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "on" => Self::On,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gpt5ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
    XHigh,
}

impl Gpt5ThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({}),
            Self::Low => serde_json::json!({ "reasoning_effort": "low" }),
            Self::Medium => serde_json::json!({ "reasoning_effort": "medium" }),
            Self::High => serde_json::json!({ "reasoning_effort": "high" }),
            Self::XHigh => serde_json::json!({ "reasoning_effort": "xhigh" }),
        }
    }

    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        match self {
            Self::Off => None,
            _ => Some(serde_json::json!({
                "reasoning": { "effort": self.as_str() }
            })),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::XHigh => "XHigh",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            _ => Self::Off,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

/// Model-specific thinking level type that wraps provider-specific levels.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevelType {
    #[default]
    None,
    DeepSeek(DeepSeekV4ThinkingLevel),
    Qwen(Qwen35ThinkingLevel),
    Glm(GlmThinkingLevel),
    Gpt5(Gpt5ThinkingLevel),
}

impl ThinkingLevelType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::DeepSeek(level) => level.display_name(),
            Self::Qwen(level) => level.display_name(),
            Self::Glm(level) => level.display_name(),
            Self::Gpt5(level) => level.display_name(),
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::DeepSeek(level) => Self::DeepSeek(level.next()),
            Self::Qwen(level) => Self::Qwen(level.next()),
            Self::Glm(level) => Self::Glm(level.next()),
            Self::Gpt5(level) => Self::Gpt5(level.next()),
        }
    }

    pub fn extra_body(&self) -> Option<serde_json::Value> {
        match self {
            Self::None => None,
            Self::DeepSeek(level) => Some(level.extra_body()),
            Self::Qwen(level) => Some(level.extra_body()),
            Self::Glm(level) => Some(level.extra_body()),
            Self::Gpt5(level) => Some(level.extra_body()),
        }
    }

    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        match self {
            Self::None => None,
            Self::DeepSeek(level) => {
                let effort = match level {
                    DeepSeekV4ThinkingLevel::Off => return None,
                    DeepSeekV4ThinkingLevel::High => "high",
                    DeepSeekV4ThinkingLevel::Max => "max",
                };
                Some(serde_json::json!({
                    "thinking": {
                        "enabled": true,
                        "effort": effort
                    }
                }))
            }
            Self::Qwen(_) => None,
            Self::Glm(_) => None,
            Self::Gpt5(level) => level.thinking_config(),
        }
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn is_supported(&self) -> bool {
        !self.is_none()
    }
}

impl fmt::Display for ThinkingLevelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::DeepSeek(level) => {
                write!(f, "deepseek:{}", level.display_name().to_lowercase())
            }
            Self::Qwen(level) => write!(f, "qwen:{}", level.display_name().to_lowercase()),
            Self::Glm(level) => write!(f, "glm:{}", level.display_name().to_lowercase()),
            Self::Gpt5(level) => write!(f, "gpt5:{}", level.display_name().to_lowercase()),
        }
    }
}

impl ThinkingLevelType {
    pub fn from_string(s: &str) -> Self {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        match parts.as_slice() {
            ["none"] => Self::None,
            ["deepseek", level] => {
                Self::DeepSeek(DeepSeekV4ThinkingLevel::from_display_name(level))
            }
            ["qwen", level] => Self::Qwen(Qwen35ThinkingLevel::from_display_name(level)),
            ["glm", level] => Self::Glm(GlmThinkingLevel::from_display_name(level)),
            ["gpt5", level] => Self::Gpt5(Gpt5ThinkingLevel::from_display_name(level)),
            _ => Self::None,
        }
    }
}
