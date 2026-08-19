use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Serialize;

use crate::attachments::{image_attachments, message_text_with_file_references};
use crate::message::{Message, MessageAttachment, MessageRole};
use crate::{types::LlmProviderConfig, types::ToolDefinition};

pub(super) fn build_responses_request(
    model: &LlmProviderConfig,
    messages: Vec<Message>,
    stream: bool,
    tools: &[ToolDefinition],
    prompt_cache_key: Option<String>,
) -> Result<ResponsesRequest> {
    // System prompt comes from the model config directly.
    // No context summary merging needed — compaction summaries are now
    // User messages inserted at the compression boundary, not System messages.
    let instructions = if model.system_prompt_str().trim().is_empty() {
        None
    } else {
        Some(model.system_prompt.clone().unwrap_or_default())
    };

    // Build conversation history as an array of input items.
    // This supports image attachments in user and tool messages.
    let mut input_items: Vec<serde_json::Value> = Vec::new();
    for message in &messages {
        if message.streaming {
            continue;
        }

        match message.role {
            MessageRole::System => {}
            MessageRole::User => {
                let text = message_text_with_file_references(message);
                let images: Vec<&MessageAttachment> = image_attachments(message).collect();

                let mut content = Vec::new();
                if !text.trim().is_empty() {
                    content.push(serde_json::json!({
                        "type": "input_text",
                        "text": text,
                    }));
                }
                for attachment in images {
                    if let MessageAttachment::Image { mime, data, .. } = attachment {
                        let b64 = BASE64.encode(data);
                        content.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!("data:{};base64,{}", mime, b64),
                            "detail": "auto",
                        }));
                    }
                }

                if !content.is_empty() {
                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "user",
                        "content": content,
                    }));
                }
            }
            MessageRole::Assistant => {
                // Replay the provider's completed output items whenever they are
                // available. This preserves reasoning items and encrypted content
                // that cannot be reconstructed from the display text alone.
                if !message.metadata.responses_output_items.is_empty() {
                    input_items.extend(canonicalize_output_items(
                        &message.metadata.responses_output_items,
                    ));
                    continue;
                }

                let text = message_text_with_file_references(message);

                // Emit assistant text as a message item (if non-empty).
                if !text.trim().is_empty() {
                    input_items.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": text,
                        }],
                    }));
                }

                // Emit each tool call as a separate function_call item.
                // This is required by the Responses API — tool calls must be
                // distinct items so the server can pair them with
                // function_call_output items via call_id.
                for tool_call in &message.tool_calls {
                    input_items.push(serde_json::json!({
                        "type": "function_call",
                        "call_id": tool_call.id,
                        "name": tool_call.name,
                        "arguments": tool_call.arguments,
                    }));
                }
            }
            MessageRole::Tool => {
                let text = message_text_with_file_references(message);
                let call_id = message.tool_call_id.clone().unwrap_or_default();
                let images: Vec<&MessageAttachment> = image_attachments(message).collect();
                let has_images = !images.is_empty();

                // function_call_output.output supports an array of content
                // parts (input_text, input_image, input_file), so embed
                // images inline — no synthetic user message needed.
                let mut output_parts: Vec<serde_json::Value> = Vec::new();
                if !text.trim().is_empty() {
                    output_parts.push(serde_json::json!({
                        "type": "input_text",
                        "text": text,
                    }));
                }
                for attachment in images {
                    if let MessageAttachment::Image { mime, data, .. } = attachment {
                        let b64 = BASE64.encode(data);
                        output_parts.push(serde_json::json!({
                            "type": "input_image",
                            "image_url": format!(
                                "data:{};base64,{}",
                                mime, b64,
                            ),
                            "detail": "auto",
                        }));
                    }
                }
                if output_parts.is_empty() {
                    output_parts.push(serde_json::json!({
                        "type": "input_text",
                        "text": "",
                    }));
                }

                let output = if !has_images {
                    serde_json::Value::String(text)
                } else {
                    serde_json::Value::Array(output_parts)
                };

                input_items.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            MessageRole::Error => {}
            MessageRole::Shell => {}
        }
    }

    let input = serde_json::Value::Array(input_items);

    let chat_tools = if tools.is_empty() {
        None
    } else {
        Some(tools.iter().map(ResponseTool::from).collect())
    };

    let thinking = model.thinking_config();
    // Always request encrypted reasoning content so reasoning items can be
    // preserved and replayed across multi-turn conversations. Without this,
    // stateless / non-persisted reasoning items (store=false / ZDR) cannot
    // be looked up on subsequent turns.
    let include = Some(vec!["reasoning.encrypted_content".to_string()]);

    let tool_choice = chat_tools.as_ref().map(|_| "auto".to_string());

    Ok(ResponsesRequest {
        model: model.request_model_id.clone().unwrap_or_default(),
        instructions,
        input,
        tools: chat_tools,
        tool_choice,
        parallel_tool_calls: model.supports_parallel_tool_calls && tools.len() > 1,
        temperature: model.temperature,
        max_output_tokens: Some(model.max_output_tokens),
        store: false,
        stream,
        thinking,
        include,
        prompt_cache_key,
        // Responses reasoning is already represented by the dedicated
        // `reasoning` field above. Do not flatten the thinking-level object a
        // second time into the top-level request.
        extra_body: model.extra_body.clone(),
    })
}

/// Keep one final representation for each provider output item ID.
///
/// New responses use the authoritative `response.completed` output list, but
/// this also makes sessions written by older tidev versions recoverable when
/// they contain both an `output_item.done` snapshot and the final item.
fn canonicalize_output_items(items: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut canonical = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item.get("id").and_then(serde_json::Value::as_str) else {
            canonical.push(item.clone());
            continue;
        };

        if let Some(index) = canonical
            .iter()
            .position(|existing| existing.get("id").and_then(serde_json::Value::as_str) == Some(id))
        {
            // Preserve the original output position while replacing a partial
            // snapshot with the later, complete representation.
            canonical[index] = item.clone();
        } else {
            canonical.push(item.clone());
        }
    }
    canonical
}

// ToolCallBuilder for Responses API
pub(super) struct ToolCallBuilder {
    id: String,
    name: String,
    arguments: String,
}

impl ToolCallBuilder {
    pub(super) fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            arguments: String::new(),
        }
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn arguments(&self) -> Option<&str> {
        if self.arguments.is_empty() {
            None
        } else {
            Some(&self.arguments)
        }
    }

    pub(super) fn append_arguments(&mut self, args: &str) {
        self.arguments.push_str(args);
    }

    pub(super) fn set_arguments(&mut self, args: &str) {
        self.arguments.clear();
        self.arguments.push_str(args);
    }
}

// ============================================================================
// Response data structures
// ============================================================================

#[derive(Clone, Debug, Serialize)]
pub(super) struct ResponsesRequest {
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    input: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponseTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
    parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<usize>,
    stream: bool,
    store: bool,
    #[serde(rename = "reasoning", skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<String>,
    #[serde(flatten)]
    extra_body: Option<serde_json::Value>,
}

// ============================================================================
#[derive(Clone, Debug, Serialize)]
pub(super) struct ResponseTool {
    #[serde(rename = "type")]
    kind: String,
    name: String,
    description: String,
    parameters: serde_json::Value,
}

impl From<&ToolDefinition> for ResponseTool {
    fn from(def: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            name: def.name.to_string(),
            description: def.description.clone(),
            parameters: def.parameters.clone(),
        }
    }
}

// ============================================================================
// Response parsing structures
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, MessageRole};
    use crate::types::ApiType;

    #[test]
    fn test_responses_request_basic() {
        let model = LlmProviderConfig {
            provider_id: "openai-responses".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: true,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();

        assert_eq!(request.model, "gpt-4.5");
        assert_eq!(request.instructions, Some("You are helpful.".to_string()));
        assert!(request.stream);
        // Input should be an array with a single user message item
        let input = request.input.as_array().expect("input should be an array");
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["type"], "input_text");
        assert_eq!(input[0]["content"][0]["text"], "Hello");
    }

    #[test]
    fn test_responses_request_with_system_prompt() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("Base system prompt".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        // System messages in the list are now skipped; only model.system_prompt is used
        let messages = vec![
            Message::new(MessageRole::System, "Context summary"),
            Message::new(MessageRole::User, "Hello"),
        ];

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();

        assert!(request.instructions.is_some());
        let instructions = request.instructions.unwrap();
        assert!(instructions.contains("Base system prompt"));
        assert!(
            !instructions.contains("Context summary"),
            "System messages should no longer be merged into instructions"
        );
    }

    #[test]
    fn test_responses_request_assistant_and_tool_messages() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![
            Message::new(MessageRole::User, "Run command"),
            Message::new(MessageRole::Assistant, "I'll help you run a command."),
            Message::new(MessageRole::Tool, "Tool result: success"),
        ];

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();

        // Input should be an array with 3 items: user message, assistant message, tool result
        let input = request.input.as_array().expect("input should be an array");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"][0]["text"], "Run command");
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["type"], "output_text");
        assert_eq!(
            input[1]["content"][0]["text"],
            "I'll help you run a command."
        );
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["output"], "Tool result: success");
    }

    #[test]
    fn test_responses_tool_message_images_embedded_inline() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: true,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        // Build a tool result message that carries an image attachment.
        use crate::message::ToolExecutionResult;
        let mut result = ToolExecutionResult::new("Image read successfully.");
        result.attachments.push(MessageAttachment::Image {
            filename: "icon.png".to_string(),
            mime: "image/png".to_string(),
            data: vec![0x89, 0x50, 0x4E, 0x47],
            file_size: 4,
        });
        let tool_msg = Message::tool_result("call_xyz", "read", result);

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(crate::message::ToolCall {
            id: "call_xyz".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"icon.png"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "describe the icon"),
            assistant,
            tool_msg,
        ];

        let request = build_responses_request(&model, messages, false, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, function_call, function_call_output — no synthetic
        // user message and no assistant message (empty text).
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_xyz");
        assert_eq!(input[1]["name"], "read");
        assert_eq!(input[1]["arguments"], r#"{"file_path":"icon.png"}"#);
        assert_eq!(input[2]["type"], "function_call_output");

        let output = input[2]["output"]
            .as_array()
            .expect("output should be an array with text + image");
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "input_text");
        assert_eq!(output[1]["type"], "input_image");
        assert!(
            output[1]["image_url"]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }

    #[test]
    fn test_responses_assistant_with_text_and_tool_calls() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        let mut assistant = Message::new(MessageRole::Assistant, "Let me read that file.");
        assistant.tool_calls.push(crate::message::ToolCall {
            id: "call_abc".to_string(),
            name: "read".to_string(),
            arguments: r#"{"file_path":"/tmp/test.txt"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "read /tmp/test.txt"),
            assistant,
            Message::tool_result(
                "call_abc",
                "read",
                crate::message::ToolExecutionResult::new("file content"),
            ),
        ];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, assistant text message, function_call, function_call_output
        assert_eq!(input.len(), 4);
        // [0] user
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        // [1] assistant text
        assert_eq!(input[1]["type"], "message");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"][0]["text"], "Let me read that file.");
        // [2] function_call
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_abc");
        assert_eq!(input[2]["name"], "read");
        assert_eq!(input[2]["arguments"], r#"{"file_path":"/tmp/test.txt"}"#);
        // [3] function_call_output
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_abc");
    }

    #[test]
    fn test_responses_assistant_multiple_tool_calls() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        let mut assistant = Message::new(MessageRole::Assistant, "");
        assistant.tool_calls.push(crate::message::ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: r#"{"path":"a.txt"}"#.to_string(),
            thought_signature: None,
        });
        assistant.tool_calls.push(crate::message::ToolCall {
            id: "call_2".to_string(),
            name: "grep".to_string(),
            arguments: r#"{"pattern":"fn main"}"#.to_string(),
            thought_signature: None,
        });

        let messages = vec![
            Message::new(MessageRole::User, "find and read"),
            assistant,
            Message::tool_result(
                "call_1",
                "read",
                crate::message::ToolExecutionResult::new("file content"),
            ),
            Message::tool_result(
                "call_2",
                "grep",
                crate::message::ToolExecutionResult::new("found matches"),
            ),
        ];

        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let input = request.input.as_array().expect("input should be an array");

        // user message, function_call x2, function_call_output x2 — no
        // assistant message (empty text).
        assert_eq!(input.len(), 5);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_1");
        assert_eq!(input[1]["name"], "read");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_2");
        assert_eq!(input[2]["name"], "grep");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[4]["type"], "function_call_output");
        assert_eq!(input[4]["call_id"], "call_2");
    }

    #[test]
    fn test_responses_request_fields() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-4.5".to_string(),
            request_model_id: Some("gpt-4.5".to_string()),
            max_output_tokens: 4096,
            temperature: Some(0.7),
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: Some("You are helpful.".to_string()),
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };

        let messages = vec![Message::new(MessageRole::User, "Hello")];

        // With tools — tool_choice should be "auto"
        let tools = vec![ToolDefinition {
            name: "read".to_string(),
            display_name: "Read".to_string(),
            description: "Read a file".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];
        let request =
            build_responses_request(&model, messages.clone(), true, &tools, None).unwrap();
        assert!(!request.parallel_tool_calls);
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["tool_choice"], "auto");
        assert_eq!(json["parallel_tool_calls"], false);

        // Without tools — tool_choice should be absent
        let request = build_responses_request(&model, messages, true, &[], None).unwrap();
        let json = serde_json::to_value(&request).unwrap();
        assert!(!json.as_object().unwrap().contains_key("tool_choice"));
        assert_eq!(json["parallel_tool_calls"], false);
    }

    #[test]
    fn test_response_tool_spec() {
        let tool = ToolDefinition {
            name: "shell".to_string(),
            display_name: "Shell".to_string(),
            description: "Execute shell command".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"}
                }
            }),
        };

        let response_tool = ResponseTool::from(&tool);

        assert_eq!(response_tool.kind, "function");
        assert_eq!(response_tool.name, "shell");
        assert!(!response_tool.description.is_empty());
    }

    #[test]
    fn responses_assistant_replays_raw_output_items() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-5".to_string(),
            request_model_id: Some("gpt-5".to_string()),
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };
        let mut assistant = Message::new(MessageRole::Assistant, "visible text");
        assistant.metadata.responses_output_items = vec![
            serde_json::json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "opaque"
            }),
            serde_json::json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "visible text"}]
            }),
        ];

        let request = build_responses_request(&model, vec![assistant], false, &[], None).unwrap();
        assert_eq!(
            request.input,
            serde_json::json!([
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "encrypted_content": "opaque"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible text"}]
                }
            ])
        );
    }

    #[test]
    fn responses_assistant_replays_duplicate_output_ids_once() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-5".to_string(),
            request_model_id: Some("gpt-5".to_string()),
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: None,
            thinking_level: crate::reasoning::ThinkingLevelType::None,
        };
        let mut assistant = Message::new(MessageRole::Assistant, "visible text");
        assistant.metadata.responses_output_items = vec![
            serde_json::json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "partial"
            }),
            serde_json::json!({
                "type": "message",
                "id": "msg_1",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "visible text"}]
            }),
            serde_json::json!({
                "type": "reasoning",
                "id": "rs_1",
                "encrypted_content": "complete"
            }),
        ];

        let request = build_responses_request(&model, vec![assistant], false, &[], None).unwrap();
        assert_eq!(
            request.input,
            serde_json::json!([
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "encrypted_content": "complete"
                },
                {
                    "type": "message",
                    "id": "msg_1",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "visible text"}]
                }
            ])
        );
    }

    #[test]
    fn responses_extra_body_is_flattened_without_thinking_duplicates() {
        let model = LlmProviderConfig {
            provider_id: "test".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            api_type: ApiType::OpenAiResponses,
            model_id: "gpt-5".to_string(),
            request_model_id: Some("gpt-5".to_string()),
            max_output_tokens: 4096,
            temperature: None,
            supports_images: false,
            supports_parallel_tool_calls: true,
            context_window: 128000,
            system_prompt: None,
            api_key: None,
            extra_body: Some(serde_json::json!({"service_tier": "flex"})),
            thinking_level: crate::reasoning::ThinkingLevelType::Gpt5(
                crate::reasoning::Gpt5ThinkingLevel::High,
            ),
        };

        let request = build_responses_request(
            &model,
            vec![Message::new(MessageRole::User, "Hello")],
            true,
            &[],
            None,
        )
        .unwrap();
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["service_tier"], "flex");
        assert_eq!(json["reasoning"]["effort"], "high");
        assert_eq!(json["reasoning"]["summary"], "auto");
        assert!(json.get("effort").is_none());
        assert!(json.get("summary").is_none());
    }
}
