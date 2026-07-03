//! Model name → thinking level matching.
//!
//! The thinking level types themselves live in `tidev-types`.

pub use tidev_types::reasoning::{
    DeepSeekV4ThinkingLevel, GlmThinkingLevel, Gpt5ThinkingLevel, MiniMaxThinkingLevel,
    Qwen35ThinkingLevel, ThinkingLevel, ThinkingLevelType,
};

/// Model name pattern matching rules.
pub struct ThinkingMatcher;

impl ThinkingMatcher {
    /// Match a model ID to its default thinking level.
    pub fn match_for_model(model_id: &str) -> ThinkingLevelType {
        let model_lower = model_id.to_lowercase();

        if model_lower.contains("deepseek") && model_lower.contains("4") {
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High)
        } else if model_lower.contains("qwen") && model_lower.contains("3.") {
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        } else if model_lower.contains("glm") {
            ThinkingLevelType::Glm(GlmThinkingLevel::High)
        } else if model_lower.contains("gpt") && model_lower.contains("5") {
            ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium)
        } else if model_lower.contains("minimax") && model_lower.contains("m3") {
            ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::High)
        } else {
            ThinkingLevelType::None
        }
    }
}
