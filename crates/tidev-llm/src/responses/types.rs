use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamResponse {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) model: String,
    #[serde(default)]
    pub(super) created_at: u64,
    #[serde(default)]
    pub(super) usage: Option<ResponseStreamUsage>,
    #[serde(default)]
    pub(super) error: Option<ResponseStreamErrorDetail>,
    #[serde(default)]
    pub(super) incomplete_details: Option<ResponseStreamIncompleteDetails>,
    #[serde(default)]
    pub(super) output: Vec<ResponseStreamItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamItem {
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    #[serde(default)]
    pub(super) item_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(super) arguments: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<Vec<ResponseStreamContentPart>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) finish_reason: Option<String>,
    #[serde(flatten, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamContentPart {
    #[serde(rename = "type", skip_serializing_if = "String::is_empty")]
    #[serde(default)]
    pub(super) part_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) index: Option<u32>,
    #[serde(flatten, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub(super) extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamReasoningPart {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(super) part_type: String,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) summary: Option<Vec<ResponseStreamReasoningStep>>,
    #[serde(default)]
    pub(super) last_summary: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamReasoningStep {
    #[serde(default)]
    pub(super) end: Option<String>,
    #[serde(default)]
    pub(super) text: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamError {
    #[serde(default)]
    pub(super) code: String,
    #[serde(default)]
    pub(super) message: String,
    #[serde(default)]
    pub(super) param: Option<String>,
    #[serde(default)]
    pub(super) error: ResponseStreamErrorDetail,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamErrorDetail {
    #[serde(rename = "type", default)]
    pub(super) r#type: String,
    #[serde(default)]
    pub(super) code: String,
    #[serde(default)]
    pub(super) message: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamIncompleteDetails {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(super) incomplete_type: String,
    #[serde(default)]
    pub(super) reason: String,
}

/// Usage stats from streaming response
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamUsage {
    #[serde(rename = "input_tokens")]
    pub(super) input_tokens: u32,
    #[serde(rename = "output_tokens")]
    pub(super) output_tokens: u32,
    #[serde(rename = "total_tokens")]
    pub(super) total_tokens: u32,
    #[serde(default)]
    pub(super) input_tokens_details: Option<ResponseStreamUsageInputDetails>,
}

/// Input token details (cached tokens)
#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseStreamUsageInputDetails {
    #[serde(default)]
    pub(super) cached_tokens: u32,
}

/// Non-streaming response structures
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponsesCompleteResponse {
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) model: String,
    #[serde(default)]
    pub(super) created_at: u64,
    #[serde(default)]
    pub(super) error: Option<ResponseStreamError>,
    #[serde(default)]
    pub(super) result: Option<ResponseResult>,
    #[serde(default)]
    pub(super) output: Vec<ResponseOutputItem>,
    #[serde(default)]
    pub(super) usage: Option<ResponseStreamUsage>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseResult {
    #[serde(rename = "type")]
    #[serde(default)]
    pub(super) result_type: String,
}

#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseOutputItem {
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) status: String,
    #[serde(default)]
    pub(super) role: String,
    #[serde(default)]
    pub(super) content: Vec<ResponseOutputContent>,
    #[serde(default)]
    pub(super) finish_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(super) struct ResponseOutputContent {
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(default)]
    pub(super) text: Option<String>,
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) call_id: Option<String>,
    #[serde(default)]
    pub(super) arguments: Option<String>,
    #[serde(default)]
    pub(super) index: Option<u32>,
}
