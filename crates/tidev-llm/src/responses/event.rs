use serde::Deserialize;

use super::types::{
    ResponseStreamContentPart, ResponseStreamError, ResponseStreamIncompleteDetails,
    ResponseStreamItem, ResponseStreamReasoningPart, ResponseStreamResponse,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(from = "ResponseStreamEventRaw")]
#[allow(dead_code)]
pub(super) enum ResponseStreamEvent {
    ResponseCreated {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseInProgress {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseCompleted {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseFailed {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseIncomplete {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    ResponseQueued {
        #[serde(default)]
        response: ResponseStreamResponse,
        #[serde(default)]
        sequence_number: u64,
    },
    OutputItemAdded {
        item: ResponseStreamItem,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
    },
    OutputItemDone {
        #[serde(default)]
        item: ResponseStreamItem,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
    },
    ContentPartAdded {
        #[serde(default)]
        content_part: ResponseStreamContentPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ContentPartDone {
        #[serde(default)]
        content_part: ResponseStreamContentPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    OutputTextDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    OutputTextDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    RefusalDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    RefusalDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningTextDelta {
        delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningTextDone {
        #[serde(default)]
        text: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningPartAdded {
        #[serde(default)]
        part: ResponseStreamReasoningPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningPartDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
    },
    ReasoningSummaryTextDelta {
        #[serde(rename = "summary")]
        summary_delta: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryTextDone {
        #[serde(default)]
        text: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryPartAdded {
        #[serde(default)]
        part: ResponseStreamReasoningPart,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    ReasoningSummaryPartDone {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        summary_index: u32,
    },
    FunctionCallArgumentsDelta {
        #[serde(rename = "id")]
        call_id: String,
        #[serde(rename = "name")]
        call_name: Option<String>,
        arguments: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FunctionCallArgumentsDone {
        #[serde(rename = "id")]
        call_id: String,
        #[serde(rename = "name")]
        call_name: Option<String>,
        #[serde(default)]
        arguments: String,
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallInProgress {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallSearching {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    FileSearchCallCompleted {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallInProgress {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallSearching {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    WebSearchCallCompleted {
        #[serde(default)]
        sequence_number: u64,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        item_id: String,
    },
    Error {
        message: String,
        #[serde(default)]
        code: Option<String>,
    },
    Unknown {
        event_type: String,
    },
}

/// Raw JSON structure for parsing SSE events
#[derive(Clone, Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseStreamEventRaw {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(default)]
    response: ResponseStreamResponse,
    #[serde(default)]
    item: ResponseStreamItem,
    #[serde(default)]
    content_part: ResponseStreamContentPart,
    #[serde(default)]
    part: ResponseStreamReasoningPart,
    #[serde(default)]
    delta: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    index: u32,
    #[serde(default)]
    content_index: u32,
    #[serde(default)]
    summary_index: u32,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
    #[serde(default)]
    item_id: String,
    #[serde(default)]
    output_index: u32,
    #[serde(default)]
    sequence_number: u64,
    #[serde(default)]
    output: Vec<ResponseStreamItem>,
    #[serde(default)]
    error: ResponseStreamError,
    #[serde(default)]
    incomplete_details: ResponseStreamIncompleteDetails,
    #[serde(default)]
    message: String,
    #[serde(default)]
    code: Option<String>,
}

impl From<ResponseStreamEventRaw> for ResponseStreamEvent {
    fn from(raw: ResponseStreamEventRaw) -> Self {
        match raw.event_type.as_str() {
            "response.created" => ResponseStreamEvent::ResponseCreated {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.in_progress" => ResponseStreamEvent::ResponseInProgress {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.completed" => ResponseStreamEvent::ResponseCompleted {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.failed" => ResponseStreamEvent::ResponseFailed {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.incomplete" => ResponseStreamEvent::ResponseIncomplete {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.queued" => ResponseStreamEvent::ResponseQueued {
                response: raw.response,
                sequence_number: raw.sequence_number,
            },
            "response.output_item.added" => ResponseStreamEvent::OutputItemAdded {
                item: raw.item,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
            },
            "response.output_item.done" => ResponseStreamEvent::OutputItemDone {
                item: raw.item,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
            },
            "response.content_part.added" => ResponseStreamEvent::ContentPartAdded {
                content_part: raw.content_part,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.content_part.done" => ResponseStreamEvent::ContentPartDone {
                content_part: raw.content_part,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.output_text.delta" => ResponseStreamEvent::OutputTextDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.output_text.done" => ResponseStreamEvent::OutputTextDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.refusal.delta" => ResponseStreamEvent::RefusalDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.refusal.done" => ResponseStreamEvent::RefusalDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning.delta" => ResponseStreamEvent::ReasoningDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning.done" => ResponseStreamEvent::ReasoningDone {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_text.delta" => ResponseStreamEvent::ReasoningTextDelta {
                delta: raw.delta,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_text.done" => ResponseStreamEvent::ReasoningTextDone {
                text: raw.text,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_part.added" => ResponseStreamEvent::ReasoningPartAdded {
                part: raw.part,
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_part.done" => ResponseStreamEvent::ReasoningPartDone {
                sequence_number: raw.sequence_number,
                item_id: raw.item_id,
                output_index: raw.output_index,
                content_index: raw.content_index,
            },
            "response.reasoning_summary_text.delta" => {
                ResponseStreamEvent::ReasoningSummaryTextDelta {
                    summary_delta: raw.delta,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_text.done" => {
                ResponseStreamEvent::ReasoningSummaryTextDone {
                    text: raw.text,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_part.added" => {
                ResponseStreamEvent::ReasoningSummaryPartAdded {
                    part: raw.part,
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.reasoning_summary_part.done" => {
                ResponseStreamEvent::ReasoningSummaryPartDone {
                    sequence_number: raw.sequence_number,
                    item_id: raw.item_id,
                    output_index: raw.output_index,
                    summary_index: raw.summary_index,
                }
            }
            "response.function_call_arguments.delta" => {
                ResponseStreamEvent::FunctionCallArgumentsDelta {
                    call_id: raw.id,
                    call_name: if raw.name.is_empty() {
                        None
                    } else {
                        Some(raw.name)
                    },
                    arguments: raw.delta,
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.function_call_arguments.done" => {
                ResponseStreamEvent::FunctionCallArgumentsDone {
                    call_id: raw.id,
                    call_name: if raw.name.is_empty() {
                        None
                    } else {
                        Some(raw.name)
                    },
                    arguments: raw.arguments,
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.file_search_call.in_progress" => {
                ResponseStreamEvent::FileSearchCallInProgress {
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.file_search_call.searching" => ResponseStreamEvent::FileSearchCallSearching {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.file_search_call.completed" => ResponseStreamEvent::FileSearchCallCompleted {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.web_search_call.in_progress" => {
                ResponseStreamEvent::WebSearchCallInProgress {
                    sequence_number: raw.sequence_number,
                    output_index: raw.output_index,
                    item_id: raw.item_id,
                }
            }
            "response.web_search_call.searching" => ResponseStreamEvent::WebSearchCallSearching {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "response.web_search_call.completed" => ResponseStreamEvent::WebSearchCallCompleted {
                sequence_number: raw.sequence_number,
                output_index: raw.output_index,
                item_id: raw.item_id,
            },
            "error" => {
                let message = if !raw.message.is_empty() {
                    raw.message
                } else if !raw.error.message.is_empty() {
                    raw.error.message.clone()
                } else {
                    raw.error.error.message.clone()
                };
                let code = raw
                    .code
                    .or_else(|| (!raw.error.code.is_empty()).then(|| raw.error.code.clone()));
                let code = code.or_else(|| {
                    (!raw.error.error.code.is_empty()).then(|| raw.error.error.code.clone())
                });
                ResponseStreamEvent::Error { message, code }
            }
            _ => ResponseStreamEvent::Unknown {
                event_type: raw.event_type,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_summary_delta_reads_delta_field() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.reasoning_summary_text.delta",
            "delta": "summary chunk",
            "item_id": "rs_1",
            "output_index": 0,
            "summary_index": 0
        }))
        .unwrap();

        match event {
            ResponseStreamEvent::ReasoningSummaryTextDelta { summary_delta, .. } => {
                assert_eq!(summary_delta, "summary chunk");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn function_call_done_preserves_complete_arguments() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.function_call_arguments.done",
            "id": "call_1",
            "name": "read",
            "arguments": "{\"path\":\"a.txt\"}",
            "item_id": "fc_1"
        }))
        .unwrap();

        match event {
            ResponseStreamEvent::FunctionCallArgumentsDone { arguments, .. } => {
                assert_eq!(arguments, r#"{"path":"a.txt"}"#);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn output_item_serialization_preserves_encrypted_reasoning_content() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.output_item.done",
            "item": {
                "type": "reasoning",
                "id": "rs_1",
                "status": "completed",
                "encrypted_content": "opaque",
                "summary": []
            }
        }))
        .unwrap();

        let ResponseStreamEvent::OutputItemDone { item, .. } = event else {
            panic!("expected output item done");
        };
        let serialized = serde_json::to_value(item).unwrap();
        assert_eq!(serialized["type"], "reasoning");
        assert_eq!(serialized["encrypted_content"], "opaque");
        assert_eq!(serialized["summary"], serde_json::json!([]));
    }

    #[test]
    fn unknown_responses_event_is_ignored() {
        let event: ResponseStreamEvent = serde_json::from_value(serde_json::json!({
            "type": "response.future_event",
            "foo": "bar"
        }))
        .unwrap();
        assert!(matches!(event, ResponseStreamEvent::Unknown { .. }));
    }
}
