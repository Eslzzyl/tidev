use std::collections::BTreeMap;

use crate::session::{AssistantTurn, ToolCall};

#[derive(Clone, Debug, Default)]
pub(super) struct ToolCallBuilder {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}

impl ToolCallBuilder {
    pub(super) fn into_tool_call(self, index: usize) -> ToolCall {
        ToolCall {
            id: if self.id.is_empty() {
                format!("tool-call-{index}")
            } else {
                self.id
            },
            name: if self.name.is_empty() {
                "unknown_tool".to_string()
            } else {
                self.name
            },
            arguments: self.arguments,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct ThinkParser {
    in_think: bool,
    buffer: String,
}

impl ThinkParser {
    pub(super) fn push(&mut self, text: &str) -> (String, String) {
        self.buffer.push_str(text);

        let mut visible = String::new();
        let mut reasoning = String::new();

        loop {
            if self.in_think {
                if let Some(end) = self.buffer.find("</think>") {
                    reasoning.push_str(&self.buffer[..end]);
                    self.buffer.drain(..end + "</think>".len());
                    self.in_think = false;
                    continue;
                }

                let keep = think_tag_suffix_len(&self.buffer);
                let split = self.buffer.len().saturating_sub(keep);
                reasoning.push_str(&self.buffer[..split]);
                self.buffer.drain(..split);
                break;
            }

            if let Some(start) = self.buffer.find("<think>") {
                visible.push_str(&self.buffer[..start]);
                self.buffer.drain(..start + "<think>".len());
                self.in_think = true;
                continue;
            }

            let keep = think_tag_suffix_len(&self.buffer);
            let split = self.buffer.len().saturating_sub(keep);
            visible.push_str(&self.buffer[..split]);
            self.buffer.drain(..split);
            break;
        }

        (visible, reasoning)
    }

    pub(super) fn finish(&mut self) -> (String, String) {
        let mut visible = String::new();
        let mut reasoning = String::new();

        if self.in_think {
            reasoning.push_str(&self.buffer);
        } else {
            visible.push_str(&self.buffer);
        }

        self.buffer.clear();
        (visible, reasoning)
    }
}

fn think_tag_suffix_len(text: &str) -> usize {
    const TAGS: [&str; 2] = ["</think>", "<think>"];

    for tag in TAGS {
        let max = tag.len().saturating_sub(1);
        for keep in (1..=max).rev() {
            if text.ends_with(&tag[..keep]) {
                return keep;
            }
        }
    }

    0
}

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

    let tool_calls = tool_calls
        .iter()
        .map(|(index, builder)| builder.clone().into_tool_call(*index))
        .collect::<Vec<_>>();

    let final_finish_reason = finish_reason.unwrap_or_else(|| {
        if tool_calls.is_empty() {
            "stop".to_string()
        } else {
            "tool_calls".to_string()
        }
    });

    AssistantTurn {
        content: assistant_text,
        reasoning: reasoning_text,
        tool_calls,
        finish_reason: Some(final_finish_reason),
    }
}
