//! Model name → thinking level matching.
//!
//! The thinking level types themselves live in `tidev-llm`.

pub use tidev_llm::reasoning::{
    ClaudeEffortLevel, DeepSeekV4ThinkingLevel, GlmThinkingLevel, Gpt5ThinkingLevel,
    MiniMaxThinkingLevel, MuseSparkThinkingLevel, Qwen35ThinkingLevel, Qwen38ThinkingLevel,
    ThinkingLevel, ThinkingLevelType,
};

/// Model name pattern matching rules.
pub struct ThinkingMatcher;

/// A model ID belongs to the Qwen3.8 family. Matches both dotted
/// (`qwen3.8-27b`, `qwen3.8-max`) and dashed (`qwen-3-8-27b`) spellings.
fn is_qwen38(id: &str) -> bool {
    id.contains("qwen") && (id.contains("3.8") || id.contains("3-8"))
}

impl ThinkingMatcher {
    /// Match a model ID to its default thinking level.
    pub fn match_for_model(model_id: &str) -> ThinkingLevelType {
        let model_lower = model_id.to_lowercase();

        if model_lower.contains("deepseek") && model_lower.contains("4") {
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High)
        } else if is_qwen38(&model_lower) {
            ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh)
        } else if model_lower.contains("qwen") && model_lower.contains("3.") {
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        } else if model_lower.contains("glm") {
            ThinkingLevelType::Glm(GlmThinkingLevel::High)
        } else if model_lower.contains("gpt") && model_lower.contains("5") {
            ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium)
        } else if model_lower.contains("minimax") && model_lower.contains("m3") {
            ThinkingLevelType::MiniMax(MiniMaxThinkingLevel::High)
        } else if model_lower.contains("muse") || model_lower.contains("spark") {
            ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::Medium)
        } else if model_lower.contains("claude") {
            ThinkingLevelType::Claude(ClaudeEffortLevel::High)
        } else {
            ThinkingLevelType::None
        }
    }

    /// Return all supported thinking level variants for a given model ID.
    ///
    /// The `model_id` should be a `request_model_id` or `display_name`
    /// (lowercased before matching).
    pub fn supported_levels(model_id: &str) -> Vec<ThinkingLevelType> {
        let id = model_id.to_ascii_lowercase();

        if id.contains("deepseek") && id.contains("4") {
            vec![
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Off),
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High),
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Max),
            ]
        } else if is_qwen38(&id) {
            vec![
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Off),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Low),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Medium),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh),
            ]
        } else if id.contains("qwen") && id.contains("3.") {
            vec![
                ThinkingLevelType::Qwen(Qwen35ThinkingLevel::Off),
                ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On),
            ]
        } else if id.contains("glm") {
            vec![
                ThinkingLevelType::Glm(GlmThinkingLevel::Off),
                ThinkingLevelType::Glm(GlmThinkingLevel::High),
                ThinkingLevelType::Glm(GlmThinkingLevel::Max),
            ]
        } else if id.contains("gpt") && id.contains("5.6") {
            vec![
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Low),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::XHigh),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Max),
            ]
        } else if id.contains("gpt") && id.contains("5") {
            vec![
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Low),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::XHigh),
            ]
        } else if (id.contains("claude") && id.contains("fable"))
            || (id.contains("claude") && id.contains("mythos"))
        {
            // Fable/Mythos: adaptive thinking always on, cannot disable.
            vec![
                ThinkingLevelType::Claude(ClaudeEffortLevel::Low),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Medium),
                ThinkingLevelType::Claude(ClaudeEffortLevel::High),
            ]
        } else if id.contains("claude") && id.contains("5") {
            vec![
                ThinkingLevelType::Claude(ClaudeEffortLevel::Off),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Low),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Medium),
                ThinkingLevelType::Claude(ClaudeEffortLevel::High),
            ]
        } else if id.contains("muse") || id.contains("spark") {
            vec![
                ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::Minimal),
                ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::Low),
                ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::Medium),
                ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::High),
                ThinkingLevelType::MuseSpark(MuseSparkThinkingLevel::XHigh),
            ]
        } else {
            vec![]
        }
    }

    /// Coerce a persisted thinking level against a model's current family.
    ///
    /// Levels saved under an older family (e.g. `qwen:on` on a model that
    /// now matches the Qwen3.8 family) are stale: the model default is
    /// returned instead. Same-family levels are kept as parsed.
    pub fn coerce_saved(saved: &str, model_id: &str) -> ThinkingLevelType {
        let saved = ThinkingLevelType::from_string(saved);
        let default = Self::match_for_model(model_id);
        if Self::family(&saved) == Self::family(&default) {
            saved
        } else {
            default
        }
    }

    /// Stable family identifier for persisted-level compatibility checks.
    fn family(level: &ThinkingLevelType) -> &'static str {
        match level {
            ThinkingLevelType::None => "none",
            ThinkingLevelType::DeepSeek(_) => "deepseek",
            ThinkingLevelType::Qwen(_) => "qwen",
            ThinkingLevelType::Qwen38(_) => "qwen38",
            ThinkingLevelType::Glm(_) => "glm",
            ThinkingLevelType::Gpt5(_) => "gpt5",
            ThinkingLevelType::MiniMax(_) => "minimax",
            ThinkingLevelType::Claude(_) => "claude",
            ThinkingLevelType::MuseSpark(_) => "muse_spark",
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
    fn match_qwen38_27b() {
        let result = ThinkingMatcher::match_for_model("qwen3.8-27b");
        assert_eq!(
            result,
            ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh)
        );
    }

    #[test]
    fn match_qwen38_max() {
        let result = ThinkingMatcher::match_for_model("qwen3.8-max");
        assert_eq!(
            result,
            ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh)
        );
    }

    #[test]
    fn match_qwen38_dashed_toml_key() {
        let result = ThinkingMatcher::match_for_model("qwen-3-8-27b");
        assert_eq!(
            result,
            ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh)
        );
    }

    #[test]
    fn match_qwen38_hf_style_id() {
        let result = ThinkingMatcher::match_for_model("Qwen/Qwen3.8-27B-FP8");
        assert_eq!(
            result,
            ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh)
        );
    }

    #[test]
    fn qwen38_does_not_shadow_older_families() {
        assert_eq!(
            ThinkingMatcher::match_for_model("qwen3.5-plus"),
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        );
        assert_eq!(
            ThinkingMatcher::match_for_model("qwen3.7-max"),
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        );
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

    // ── supported_levels tests ────────────────────────────────────────

    #[test]
    fn deepseek_v4_levels() {
        let opts = ThinkingMatcher::supported_levels("deepseek-v4-flash");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Off),
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High),
                ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::Max),
            ]
        );
    }

    #[test]
    fn qwen_3_5_levels() {
        let opts = ThinkingMatcher::supported_levels("qwen-3.5-turbo");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Qwen(Qwen35ThinkingLevel::Off),
                ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On),
            ]
        );
    }

    #[test]
    fn qwen38_levels() {
        let opts = ThinkingMatcher::supported_levels("qwen3.8-27b");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Off),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Low),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Medium),
                ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh),
            ]
        );
    }

    #[test]
    fn glm_levels() {
        let opts = ThinkingMatcher::supported_levels("glm-4");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Glm(GlmThinkingLevel::Off),
                ThinkingLevelType::Glm(GlmThinkingLevel::High),
                ThinkingLevelType::Glm(GlmThinkingLevel::Max),
            ]
        );
    }

    #[test]
    fn gpt_5_6_levels_includes_max() {
        let opts = ThinkingMatcher::supported_levels("gpt-5.6-sol");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Low),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::XHigh),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Max),
            ]
        );
    }

    #[test]
    fn gpt_5_5_levels_no_max() {
        let opts = ThinkingMatcher::supported_levels("gpt-5.5");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Off),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Low),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::Medium),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::High),
                ThinkingLevelType::Gpt5(Gpt5ThinkingLevel::XHigh),
            ]
        );
    }

    #[test]
    fn claude_fable_levels_adaptive_only() {
        let opts = ThinkingMatcher::supported_levels("claude-fable-5");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Claude(ClaudeEffortLevel::Low),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Medium),
                ThinkingLevelType::Claude(ClaudeEffortLevel::High),
            ]
        );
    }

    #[test]
    fn claude_mythos_levels_adaptive_only() {
        let opts = ThinkingMatcher::supported_levels("claude-mythos-5");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Claude(ClaudeEffortLevel::Low),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Medium),
                ThinkingLevelType::Claude(ClaudeEffortLevel::High),
            ]
        );
    }

    #[test]
    fn claude_5_levels_includes_off() {
        let opts = ThinkingMatcher::supported_levels("claude-sonnet-5");
        assert_eq!(
            opts,
            vec![
                ThinkingLevelType::Claude(ClaudeEffortLevel::Off),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Low),
                ThinkingLevelType::Claude(ClaudeEffortLevel::Medium),
                ThinkingLevelType::Claude(ClaudeEffortLevel::High),
            ]
        );
    }

    #[test]
    fn unsupported_model_returns_empty() {
        let opts = ThinkingMatcher::supported_levels("claude-opus-4-8");
        assert!(opts.is_empty());
    }

    // ── coerce_saved tests ─────────────────────────────────────────────

    #[test]
    fn coerce_stale_qwen_on_to_qwen38_default() {
        let level = ThinkingMatcher::coerce_saved("qwen:on", "qwen3.8-27b");
        assert_eq!(level, ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh));
    }

    #[test]
    fn coerce_same_family_keeps_saved_level() {
        let level = ThinkingMatcher::coerce_saved("qwen38:low", "qwen3.8-27b");
        assert_eq!(level, ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::Low));
    }

    #[test]
    fn coerce_foreign_family_to_model_default() {
        let level = ThinkingMatcher::coerce_saved("claude:high", "qwen3.8-max");
        assert_eq!(level, ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh));
    }

    #[test]
    fn coerce_unparseable_to_model_default() {
        let level = ThinkingMatcher::coerce_saved("not-a-level", "qwen3.8-27b");
        assert_eq!(level, ThinkingLevelType::Qwen38(Qwen38ThinkingLevel::XHigh));
    }
}
