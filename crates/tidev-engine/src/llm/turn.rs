use std::collections::BTreeMap;

use tidev_session::session::AssistantTurn;

use super::think_parser::ThinkParser;
use super::tool_call_format::{ToolCallBuilder, parse_invoke_xml};

/// Build the final [`AssistantTurn`] from the accumulated streaming data.
///
/// When the provider returned no native `tool_calls` (e.g. MiniMax XML format
/// embedded in the content text), this function falls back to XML invoke-block
/// parsing via [`parse_invoke_xml`].
pub(super) fn finalize_turn(
    assistant_text: String,
    reasoning_text: String,
    finish_reason: Option<String>,
    tool_calls: &BTreeMap<usize, ToolCallBuilder>,
    think_parser: &mut ThinkParser,
) -> AssistantTurn {
    let (visible, reasoning) = think_parser.finish();
    let assistant_text = assistant_text + &visible;
    let reasoning_text = reasoning_text + &reasoning;

    // Convert native tool calls from the streaming delta
    let tool_calls = tool_calls
        .iter()
        .map(|(index, builder)| builder.clone().into_tool_call(*index))
        .collect::<Vec<_>>();

    // Fallback: when no native tool_calls, try XML invoke parsing
    let (final_content, final_tool_calls) = if tool_calls.is_empty() {
        let (cleaned, parsed) = parse_invoke_xml(&assistant_text);
        if !parsed.is_empty() {
            (cleaned, parsed)
        } else {
            (assistant_text, tool_calls)
        }
    } else {
        (assistant_text, tool_calls)
    };

    let final_finish_reason = finish_reason.unwrap_or_else(|| {
        if final_tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        }
    });

    AssistantTurn {
        content: final_content,
        reasoning: reasoning_text,
        tool_calls: final_tool_calls,
        finish_reason: Some(final_finish_reason),
        ..Default::default()
    }
}
