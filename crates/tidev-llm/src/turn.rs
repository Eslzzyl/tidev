use std::collections::BTreeMap;

use tidev_types::message::AssistantTurn;

use crate::think_parser::ThinkParser;
use crate::think_parser::strip_think_tags;
use crate::tool_call_format::{ToolCallBuilder, parse_invoke_xml};

/// Build the final [`AssistantTurn`] from the accumulated streaming data.
///
/// When the provider returned no native `tool_calls` (e.g. MiniMax XML format
/// embedded in the content text), this function falls back to XML invoke-block
/// parsing via [`parse_invoke_xml`].
pub(crate) fn finalize_turn(
    assistant_text: String,
    reasoning_text: String,
    finish_reason: Option<String>,
    tool_calls: &BTreeMap<usize, ToolCallBuilder>,
    think_parser: &mut ThinkParser,
) -> AssistantTurn {
    let (visible, reasoning) = think_parser.finish();
    let assistant_text = assistant_text + &visible;
    let reasoning_text = strip_think_tags(&(reasoning_text + &reasoning));

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_plain_text() {
        let mut tp = ThinkParser::default();
        let turn = finalize_turn(
            "Hello".into(),
            String::new(),
            Some("stop".into()),
            &BTreeMap::new(),
            &mut tp,
        );
        assert_eq!(turn.content, "Hello");
        assert!(turn.reasoning.is_empty());
        assert!(turn.tool_calls.is_empty());
        assert_eq!(turn.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn finalize_with_reasoning() {
        let mut tp = ThinkParser::default();
        // Simulate streaming: accumulate visible/reasoning from push, then finalize.
        let (v, r) = tp.push("visible <think>reasoning text</think> after");
        let turn = finalize_turn(v, r, None, &BTreeMap::new(), &mut tp);
        assert_eq!(turn.content, "visible  after");
        assert_eq!(turn.reasoning, "reasoning text");
    }

    #[test]
    fn finalize_finish_reason_defaults_to_stop() {
        let mut tp = ThinkParser::default();
        let turn = finalize_turn("hi".into(), String::new(), None, &BTreeMap::new(), &mut tp);
        assert_eq!(turn.finish_reason.as_deref(), Some("stop"));
    }
}
