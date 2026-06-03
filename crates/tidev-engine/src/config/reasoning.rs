//! Model-agnostic thinking/reasoning level types and matcher.
//!
//! The types themselves (`ThinkingLevelType`, `DeepSeekV4ThinkingLevel`, …)
//! live in `tidev-types` so they can be shared with `tidev-session` without
//! creating a circular dependency.  This module re‑exports them for existing
//! callers that import via `crate::config::reasoning::X`.

pub use tidev_types::reasoning::{
    DeepSeekV4ThinkingLevel, GlmThinkingLevel, Gpt5ThinkingLevel, Qwen35ThinkingLevel,
    ThinkingLevel, ThinkingLevelType,
};

/// Model name pattern matching rules.
pub struct ThinkingMatcher;

impl ThinkingMatcher {
    /// 根据模型名称获取匹配的思考级别类型
    pub fn match_for_model(model_id: &str) -> ThinkingLevelType {
        let model_lower = model_id.to_lowercase();

        if model_lower.contains("deepseek") && model_lower.contains("4") {
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High)
        } else if model_lower.contains("qwen") && model_lower.contains("3.") {
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        } else if model_lower.contains("glm") {
            ThinkingLevelType::Glm(GlmThinkingLevel::On)
        } else if model_lower.contains("gpt") && model_lower.contains("5") {
            ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium)
        } else {
            ThinkingLevelType::None
        }
    }
}
