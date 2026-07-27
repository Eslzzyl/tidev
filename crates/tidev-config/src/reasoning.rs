//! Model name → thinking level matching.
//!
//! The thinking level types themselves live in `tidev-types`.

pub use tidev_types::reasoning::{
    ClaudeEffortLevel, DeepSeekV4ThinkingLevel, GlmThinkingLevel, Gpt5ThinkingLevel,
    MiniMaxThinkingLevel, Qwen35ThinkingLevel, ThinkingLevel, ThinkingLevelType,
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
        } else if model_lower.contains("claude") {
            ThinkingLevelType::Claude(ClaudeEffortLevel::High)
        } else {
            ThinkingLevelType::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_gpt_5_6_sol() {
        let result = ThinkingMatcher::match_for_model("gpt-5.6-sol");
        assert_eq!(result, ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium));
    }

    #[test]
    fn match_gpt_5_4() {
        let result = ThinkingMatcher::match_for_model("gpt-5.4");
        assert_eq!(result, ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium));
    }

    #[test]
    fn match_gpt_5_with_dashes() {
        // TOML model_id keys use dashes instead of dots (e.g. "gpt-5-6-luna")
        let result = ThinkingMatcher::match_for_model("gpt-5-6-luna");
        assert_eq!(result, ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium));
    }

    #[test]
    fn match_deepseek_v4() {
        let result = ThinkingMatcher::match_for_model("deepseek-v4-flash");
        assert_eq!(
            result,
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High)
        );
    }

    #[test]
    fn match_qwen_3_5() {
        let result = ThinkingMatcher::match_for_model("qwen-3.5-turbo");
        assert_eq!(result, ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On));
    }

    #[test]
    fn match_glm() {
        let result = ThinkingMatcher::match_for_model("glm-4");
        assert_eq!(result, ThinkingLevelType::Glm(GlmThinkingLevel::High));
    }

    #[test]
    fn match_gpt_4o_returns_none() {
        let result = ThinkingMatcher::match_for_model("gpt-4o");
        assert_eq!(result, ThinkingLevelType::None);
    }

    #[test]
    fn match_unknown_returns_none() {
        let result = ThinkingMatcher::match_for_model("unknown-model");
        assert_eq!(result, ThinkingLevelType::None);
    }

    #[test]
    fn match_claude_opus_5() {
        let result = ThinkingMatcher::match_for_model("claude-opus-5");
        assert_eq!(result, ThinkingLevelType::Claude(ClaudeEffortLevel::High));
    }

    #[test]
    fn match_claude_fable_5() {
        let result = ThinkingMatcher::match_for_model("claude-fable-5");
        assert_eq!(result, ThinkingLevelType::Claude(ClaudeEffortLevel::High));
    }

    #[test]
    fn match_claude_sonnet_5() {
        let result = ThinkingMatcher::match_for_model("claude-sonnet-5");
        assert_eq!(result, ThinkingLevelType::Claude(ClaudeEffortLevel::High));
    }

    #[test]
    fn match_claude_mythos_5() {
        let result = ThinkingMatcher::match_for_model("claude-mythos-5");
        assert_eq!(result, ThinkingLevelType::Claude(ClaudeEffortLevel::High));
    }
}
