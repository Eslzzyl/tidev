//! Thinking / reasoning level types shared across the tidev workspace.
//!
//! These types model the per-provider "thinking" configuration that controls
//! how much reasoning effort the model applies.

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

// ---------------------------------------------------------------------------
// Provider-specific thinking level enums
// ---------------------------------------------------------------------------

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
    High,
    Max,
}

impl GlmThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "thinking": { "type": "disabled" }
            }),
            Self::High | Self::Max => serde_json::json!({
                "thinking": { "type": "enabled" }
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
pub enum Gpt5ThinkingLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl Gpt5ThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "reasoning_effort": "none"
            }),
            Self::Low => serde_json::json!({
                "reasoning_effort": "low"
            }),
            Self::Medium => serde_json::json!({
                "reasoning_effort": "medium"
            }),
            Self::High => serde_json::json!({
                "reasoning_effort": "high"
            }),
            Self::XHigh => serde_json::json!({
                "reasoning_effort": "xhigh"
            }),
            Self::Max => serde_json::json!({
                "reasoning_effort": "max"
            }),
        }
    }

    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        match self {
            Self::Off => None,
            Self::Low => Some(serde_json::json!({
                "summary": "auto",
                "effort": "low"
            })),
            Self::Medium => Some(serde_json::json!({
                "summary": "auto",
                "effort": "medium"
            })),
            Self::High => Some(serde_json::json!({
                "summary": "auto",
                "effort": "high"
            })),
            Self::XHigh => Some(serde_json::json!({
                "summary": "auto",
                "effort": "xhigh"
            })),
            Self::Max => Some(serde_json::json!({
                "summary": "auto",
                "effort": "max"
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
            Self::Max => "Max",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Max,
            Self::Max => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            "max" => Self::Max,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MiniMaxThinkingLevel {
    #[default]
    Off,
    High,
    Max,
}

impl MiniMaxThinkingLevel {
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "thinking": { "type": "disabled" }
            }),
            Self::High | Self::Max => serde_json::json!({
                "thinking": { "type": "adaptive" }
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
pub enum ClaudeEffortLevel {
    #[default]
    Off,
    Low,
    Medium,
    High,
}

impl ClaudeEffortLevel {
    /// Native Anthropic API format.
    pub fn extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "thinking": { "type": "disabled" }
            }),
            Self::Low => serde_json::json!({
                "output_config": { "effort": "low" }
            }),
            Self::Medium => serde_json::json!({
                "output_config": { "effort": "medium" }
            }),
            Self::High => serde_json::json!({
                "output_config": { "effort": "high" }
            }),
        }
    }

    /// OpenAI Chat Completions format (for中转站 routing Claude through OpenAI API).
    pub fn openai_extra_body(&self) -> serde_json::Value {
        match self {
            Self::Off => serde_json::json!({
                "reasoning_effort": "none"
            }),
            Self::Low => serde_json::json!({
                "reasoning_effort": "low"
            }),
            Self::Medium => serde_json::json!({
                "reasoning_effort": "medium"
            }),
            Self::High => serde_json::json!({
                "reasoning_effort": "high"
            }),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::Low,
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::Off,
        }
    }

    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            _ => Self::Off,
        }
    }
}

// ---------------------------------------------------------------------------
// Unified thinking level type
// ---------------------------------------------------------------------------

/// Model-specific thinking level type that wraps provider-specific levels.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevelType {
    #[default]
    None,
    DeepSeek(DeepSeekV4ThinkingLevel),
    Qwen(Qwen35ThinkingLevel),
    Glm(GlmThinkingLevel),
    Gpt5(Gpt5ThinkingLevel),
    MiniMax(MiniMaxThinkingLevel),
    Claude(ClaudeEffortLevel),
}

impl ThinkingLevelType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::DeepSeek(level) => level.display_name(),
            Self::Qwen(level) => level.display_name(),
            Self::Glm(level) => level.display_name(),
            Self::Gpt5(level) => level.display_name(),
            Self::MiniMax(level) => level.display_name(),
            Self::Claude(level) => level.display_name(),
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::DeepSeek(level) => Self::DeepSeek(level.next()),
            Self::Qwen(level) => Self::Qwen(level.next()),
            Self::Glm(level) => Self::Glm(level.next()),
            Self::Gpt5(level) => Self::Gpt5(level.next()),
            Self::MiniMax(level) => Self::MiniMax(level.next()),
            Self::Claude(level) => Self::Claude(level.next()),
        }
    }

    pub fn extra_body(&self) -> Option<serde_json::Value> {
        match self {
            Self::None => None,
            Self::DeepSeek(level) => Some(level.extra_body()),
            Self::Qwen(level) => Some(level.extra_body()),
            Self::Glm(level) => Some(level.extra_body()),
            Self::Gpt5(level) => Some(level.extra_body()),
            Self::MiniMax(level) => Some(level.extra_body()),
            Self::Claude(level) => Some(level.extra_body()),
        }
    }

    /// Extract the semantic effort string from the inner variant, if any.
    /// `None` means thinking is disabled or not expressible as an effort level.
    fn effort_str(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::DeepSeek(DeepSeekV4ThinkingLevel::Off)
            | Self::Qwen(_)
            | Self::Glm(GlmThinkingLevel::Off)
            | Self::Gpt5(Gpt5ThinkingLevel::Off)
            | Self::MiniMax(MiniMaxThinkingLevel::Off)
            | Self::Claude(ClaudeEffortLevel::Off) => None,
            Self::DeepSeek(DeepSeekV4ThinkingLevel::High)
            | Self::Glm(GlmThinkingLevel::High)
            | Self::MiniMax(MiniMaxThinkingLevel::High)
            | Self::Claude(ClaudeEffortLevel::High) => Some("high"),
            Self::DeepSeek(DeepSeekV4ThinkingLevel::Max)
            | Self::Glm(GlmThinkingLevel::Max)
            | Self::MiniMax(MiniMaxThinkingLevel::Max) => Some("max"),
            Self::Gpt5(Gpt5ThinkingLevel::Low) | Self::Claude(ClaudeEffortLevel::Low) => {
                Some("low")
            }
            Self::Gpt5(Gpt5ThinkingLevel::Medium) | Self::Claude(ClaudeEffortLevel::Medium) => {
                Some("medium")
            }
            Self::Gpt5(Gpt5ThinkingLevel::High) => Some("high"),
            Self::Gpt5(Gpt5ThinkingLevel::XHigh) => Some("xhigh"),
            Self::Gpt5(Gpt5ThinkingLevel::Max) => Some("max"),
        }
    }

    /// Return the thinking/reasoning configuration for the given API type.
    ///
    /// - `openai_chat_completions`: native extra_body per variant
    /// - `openai_responses`: unified `reasoning.effort` format
    /// - `anthropic`: unified `output_config.effort` / `thinking.type` format
    pub fn for_api(&self, api_type: &str) -> Option<serde_json::Value> {
        // None: never produces anything
        if matches!(self, Self::None) {
            return None;
        }
        match api_type {
            // Native format: each variant knows its own Chat Completions format.
            // Claude's native format is Anthropic, so translate for OpenAI-compatible APIs.
            "openai_chat_completions" => match self {
                Self::Claude(level) => Some(level.openai_extra_body()),
                _ => self.extra_body(),
            },

            // Responses API: unified reasoning.effort format
            // Gpt5 adds summary support.
            // Qwen On/Off needs explicit handling since it has no effort levels.
            "openai_responses" => {
                let effort = self.effort_str();
                match (self, effort) {
                    // Qwen(On) → enable with default effort
                    (Self::Qwen(Qwen35ThinkingLevel::On), _) => {
                        Some(serde_json::json!({"effort": "high"}))
                    }
                    // Gpt5 with effort → include summary
                    (Self::Gpt5(_), Some(e)) => Some(serde_json::json!({
                        "effort": e, "summary": "auto"
                    })),
                    // Other variants with effort → just effort
                    (_, Some(e)) => Some(serde_json::json!({"effort": e})),
                    // Off → effort: "none"
                    (_, None) => Some(serde_json::json!({"effort": "none"})),
                }
            }

            // Anthropic: unified output_config / thinking format
            // Qwen(On) has no Anthropic equivalent and returns None.
            // MiniMax uses thinking.type (not output_config).
            "anthropic" => match self {
                Self::Qwen(Qwen35ThinkingLevel::On) => None,
                Self::MiniMax(MiniMaxThinkingLevel::Off) => {
                    Some(serde_json::json!({"thinking": {"type": "disabled"}}))
                }
                Self::MiniMax(_) => Some(serde_json::json!({"thinking": {"type": "adaptive"}})),
                _ => match self.effort_str() {
                    None => Some(serde_json::json!({"thinking": {"type": "disabled"}})),
                    Some(e) => Some(serde_json::json!({"output_config": {"effort": e}})),
                },
            },

            _ => None,
        }
    }

    /// Returns the extra body for a specific API type.
    ///
    /// Delegates to [`for_api`](Self::for_api).
    pub fn extra_body_for_api(&self, api_type: &str) -> Option<serde_json::Value> {
        self.for_api(api_type)
    }

    /// Returns the thinking/reasoning configuration for OpenAI Responses API.
    ///
    /// Delegates to [`for_api`](Self::for_api) with `"openai_responses"`.
    pub fn thinking_config(&self) -> Option<serde_json::Value> {
        self.for_api("openai_responses")
    }

    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn is_supported(&self) -> bool {
        !self.is_none()
    }

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
            ["minimax", level] => Self::MiniMax(MiniMaxThinkingLevel::from_display_name(level)),
            ["claude", level] => Self::Claude(ClaudeEffortLevel::from_display_name(level)),
            _ => Self::None,
        }
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
            Self::MiniMax(level) => {
                write!(f, "minimax:{}", level.display_name().to_lowercase())
            }
            Self::Claude(level) => write!(f, "claude:{}", level.display_name().to_lowercase()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_skip_for_anthropic() {
        let qwen = ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On);
        assert!(qwen.extra_body_for_api("anthropic").is_none());
        assert!(qwen.extra_body_for_api("openai_chat_completions").is_some());
    }

    #[test]
    fn qwen_off_to_anthropic_disables() {
        let qwen = ThinkingLevelType::Qwen(Qwen35ThinkingLevel::Off);
        assert_eq!(
            qwen.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
    }

    #[test]
    fn deepseek_off_to_anthropic_disables() {
        let level = ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Off);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
    }

    #[test]
    fn deepseek_high_to_anthropic_sets_output_config() {
        let level = ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "high"}}))
        );
    }

    #[test]
    fn gpt5_max_to_anthropic_sets_output_config() {
        let level = ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Max);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "max"}}))
        );
    }

    #[test]
    fn gpt5_high_to_anthropic_sets_output_config() {
        let level = ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "high"}}))
        );
    }

    #[test]
    fn gpt5_responses_includes_summary() {
        let level = ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "high", "summary": "auto"}))
        );
    }

    #[test]
    fn deepseek_responses_effort_only() {
        let level = ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Max);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "max"}))
        );
    }

    #[test]
    fn all_off_responses_send_effort_none() {
        for level in [
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Off),
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::Off),
            ThinkingLevelType::Glm(GlmThinkingLevel::Off),
            ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off),
            ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::Off),
        ] {
            assert_eq!(
                level.thinking_config(),
                Some(serde_json::json!({"effort": "none"})),
                "off variant {:?} should produce effort: none",
                level,
            );
        }
    }

    #[test]
    fn none_never_produces_anything() {
        assert!(
            ThinkingLevelType::None
                .extra_body_for_api("openai_chat_completions")
                .is_none()
        );
        assert!(
            ThinkingLevelType::None
                .extra_body_for_api("openai_responses")
                .is_none()
        );
        assert!(
            ThinkingLevelType::None
                .extra_body_for_api("anthropic")
                .is_none()
        );
    }

    #[test]
    fn glm_max_accepted() {
        assert_eq!(
            GlmThinkingLevel::from_display_name("max"),
            GlmThinkingLevel::Max,
        );
        assert_eq!(GlmThinkingLevel::Max.display_name(), "Max");
    }

    #[test]
    fn glm_high_to_anthropic_sets_output_config() {
        let level = ThinkingLevelType::Glm(GlmThinkingLevel::High);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "high"}}))
        );
    }

    #[test]
    fn glm_max_to_anthropic_sets_output_config() {
        let level = ThinkingLevelType::Glm(GlmThinkingLevel::Max);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "max"}}))
        );
    }

    #[test]
    fn glm_off_to_anthropic_disables() {
        let level = ThinkingLevelType::Glm(GlmThinkingLevel::Off);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
    }

    #[test]
    fn minimax_high_to_anthropic_uses_adaptive() {
        let level = ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::High);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "adaptive"}}))
        );
    }

    #[test]
    fn minimax_max_to_anthropic_uses_adaptive() {
        let level = ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::Max);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "adaptive"}}))
        );
    }

    #[test]
    fn minimax_off_to_anthropic_disables() {
        let level = ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::Off);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
    }

    #[test]
    fn gpt5_chat_completions_uses_reasoning_effort() {
        let level = ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High);
        assert_eq!(
            level.extra_body_for_api("openai_chat_completions"),
            Some(serde_json::json!({"reasoning_effort": "high"}))
        );
    }

    #[test]
    fn gpt5_off_chat_completions_uses_none() {
        let level = ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off);
        assert_eq!(
            level.extra_body_for_api("openai_chat_completions"),
            Some(serde_json::json!({"reasoning_effort": "none"}))
        );
    }

    #[test]
    fn qwen_on_responses_sends_effort_high() {
        let level = ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "high"}))
        );
    }

    #[test]
    fn qwen_off_responses_sends_effort_none() {
        let level = ThinkingLevelType::Qwen(Qwen35ThinkingLevel::Off);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "none"}))
        );
    }

    #[test]
    fn minimax_high_responses_sends_effort_high() {
        let level = ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::High);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "high"}))
        );
    }

    #[test]
    fn glm_max_responses_sends_effort_max() {
        let level = ThinkingLevelType::Glm(GlmThinkingLevel::Max);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "max"}))
        );
    }

    // -----------------------------------------------------------------------
    // Claude
    // -----------------------------------------------------------------------

    #[test]
    fn claude_basic_levels() {
        assert_eq!(
            ClaudeEffortLevel::from_display_name("off"),
            ClaudeEffortLevel::Off
        );
        assert_eq!(
            ClaudeEffortLevel::from_display_name("low"),
            ClaudeEffortLevel::Low
        );
        assert_eq!(
            ClaudeEffortLevel::from_display_name("medium"),
            ClaudeEffortLevel::Medium
        );
        assert_eq!(
            ClaudeEffortLevel::from_display_name("high"),
            ClaudeEffortLevel::High
        );
        assert_eq!(ClaudeEffortLevel::Off.display_name(), "Off");
        assert_eq!(ClaudeEffortLevel::Low.display_name(), "Low");
        assert_eq!(ClaudeEffortLevel::Medium.display_name(), "Medium");
        assert_eq!(ClaudeEffortLevel::High.display_name(), "High");
    }

    #[test]
    fn claude_next_cycles() {
        let mut level = ClaudeEffortLevel::Off;
        level = level.next();
        assert_eq!(level, ClaudeEffortLevel::Low);
        level = level.next();
        assert_eq!(level, ClaudeEffortLevel::Medium);
        level = level.next();
        assert_eq!(level, ClaudeEffortLevel::High);
        level = level.next();
        assert_eq!(level, ClaudeEffortLevel::Off);
    }

    #[test]
    fn claude_off_to_anthropic_disables() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::Off);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"thinking": {"type": "disabled"}}))
        );
    }

    #[test]
    fn claude_high_to_anthropic_uses_output_config() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::High);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "high"}}))
        );
    }

    #[test]
    fn claude_low_to_anthropic_uses_output_config() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::Low);
        assert_eq!(
            level.extra_body_for_api("anthropic"),
            Some(serde_json::json!({"output_config": {"effort": "low"}}))
        );
    }

    #[test]
    fn claude_off_to_chat_completions_uses_reasoning_effort() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::Off);
        assert_eq!(
            level.extra_body_for_api("openai_chat_completions"),
            Some(serde_json::json!({"reasoning_effort": "none"}))
        );
    }

    #[test]
    fn claude_high_to_chat_completions_uses_reasoning_effort() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::High);
        assert_eq!(
            level.extra_body_for_api("openai_chat_completions"),
            Some(serde_json::json!({"reasoning_effort": "high"}))
        );
    }

    #[test]
    fn claude_responses_follows_unified_format() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::High);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "high"}))
        );
    }

    #[test]
    fn claude_off_responses_sends_effort_none() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::Off);
        assert_eq!(
            level.thinking_config(),
            Some(serde_json::json!({"effort": "none"}))
        );
    }

    #[test]
    fn claude_to_gemini_returns_none() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::High);
        assert!(level.extra_body_for_api("google_gemini").is_none());
    }

    #[test]
    fn claude_round_trip_string() {
        let level = ThinkingLevelType::Claude(ClaudeEffortLevel::Medium);
        let s = level.to_string();
        assert_eq!(s, "claude:medium");
        let parsed = ThinkingLevelType::from_string(&s);
        assert_eq!(parsed, level);
    }
}
