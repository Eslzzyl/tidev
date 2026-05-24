use serde::{Deserialize, Serialize};
use std::fmt;

/// 通用思考级别枚举，支持不同 provider
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    High,
    Max,
}

impl ThinkingLevel {
    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::High => "High",
            Self::Max => "Max",
        }
    }

    /// 获取切换后的下一个级别（循环）
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
    /// 获取对应的 extra_body（用于 OpenAI Chat Completions API）
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

    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::High => "High",
            Self::Max => "Max",
        }
    }

    /// 获取切换后的下一个级别
    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::High,
            Self::High => Self::Max,
            Self::Max => Self::Off,
        }
    }

    /// 从显示名称解析（用于数据库加载）
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
    /// 获取对应的 extra_body（用于 OpenAI Chat Completions API）
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

    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
        }
    }

    /// 获取切换后的下一个级别
    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }

    /// 从显示名称解析（用于数据库加载）
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
    /// 获取对应的 extra_body（用于 OpenAI Chat Completions API）
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

    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
        }
    }

    /// 获取切换后的下一个级别
    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::On,
            Self::On => Self::Off,
        }
    }

    /// 从显示名称解析（用于数据库加载）
    pub fn from_display_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "off" => Self::Off,
            "on" => Self::On,
            _ => Self::Off,
        }
    }
}

/// 模型匹配的思考级别类型
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevelType {
    #[default]
    None,
    DeepSeek(DeepSeekV4ThinkingLevel),
    Qwen(Qwen35ThinkingLevel),
    Glm(GlmThinkingLevel),
}

impl ThinkingLevelType {
    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::DeepSeek(level) => level.display_name(),
            Self::Qwen(level) => level.display_name(),
            Self::Glm(level) => level.display_name(),
        }
    }

    /// 获取切换后的下一个级别
    pub fn next(&self) -> Self {
        match self {
            Self::None => Self::None,
            Self::DeepSeek(level) => Self::DeepSeek(level.next()),
            Self::Qwen(level) => Self::Qwen(level.next()),
            Self::Glm(level) => Self::Glm(level.next()),
        }
    }

    /// 获取 extra_body（用于 OpenAI Chat Completions API）
    pub fn extra_body(&self) -> Option<serde_json::Value> {
        match self {
            Self::None => None,
            Self::DeepSeek(level) => Some(level.extra_body()),
            Self::Qwen(level) => Some(level.extra_body()),
            Self::Glm(level) => Some(level.extra_body()),
        }
    }

    /// 获取 thinking 配置（用于 OpenAI Responses API）
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
        }
    }

    /// 检查是否为 None
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// 检查是否支持思考模式
    pub fn is_supported(&self) -> bool {
        !self.is_none()
    }
}

impl fmt::Display for ThinkingLevelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::DeepSeek(level) => write!(f, "deepseek:{}", level.display_name().to_lowercase()),
            Self::Qwen(level) => write!(f, "qwen:{}", level.display_name().to_lowercase()),
            Self::Glm(level) => write!(f, "glm:{}", level.display_name().to_lowercase()),
        }
    }
}

impl ThinkingLevelType {
    /// 从字符串解析（用于数据库加载）
    pub fn from_string(s: &str) -> Self {
        let parts: Vec<&str> = s.splitn(2, ':').collect();
        match parts.as_slice() {
            ["none"] => Self::None,
            ["deepseek", level] => {
                Self::DeepSeek(DeepSeekV4ThinkingLevel::from_display_name(level))
            }
            ["qwen", level] => Self::Qwen(Qwen35ThinkingLevel::from_display_name(level)),
            ["glm", level] => Self::Glm(GlmThinkingLevel::from_display_name(level)),
            _ => Self::None,
        }
    }
}

/// 模型名称模式匹配规则
pub struct ThinkingMatcher;

impl ThinkingMatcher {
    /// 根据模型名称获取匹配的思考级别类型
    pub fn match_for_model(model_id: &str) -> ThinkingLevelType {
        // 首先将模型 id 的所有字母变成小写
        let model_lower = model_id.to_lowercase();

        // DeepSeek V4 模型: 包含 "deepseek" 和数字 "4"
        if model_lower.contains("deepseek") && model_lower.contains("4") {
            ThinkingLevelType::DeepSeek(DeepSeekV4ThinkingLevel::High)
        }
        // Qwen 3.5/3.6 模型: 包含 "qwen" 和 "3."
        // 不能简单地包含 "3" 因为会匹配到 Qwen 3，而 Qwen 3 使用不同的开关方式
        else if model_lower.contains("qwen") && model_lower.contains("3.") {
            ThinkingLevelType::Qwen(Qwen35ThinkingLevel::On)
        }
        // GLM 模型: 包含 "glm"
        else if model_lower.contains("glm") {
            ThinkingLevelType::Glm(GlmThinkingLevel::On)
        } else {
            ThinkingLevelType::None
        }
    }
}
