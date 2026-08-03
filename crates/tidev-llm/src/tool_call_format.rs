/// Accumulates a tool call from streaming `tool_calls` delta chunks.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolCallBuilder {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}

impl ToolCallBuilder {
    pub(crate) fn into_tool_call(self, index: usize) -> crate::message::ToolCall {
        crate::message::ToolCall {
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
            thought_signature: None,
        }
    }
}

// ---------------------------------------------------------------------------
// XML-based fallback parser (MiniMax invoke format)
// ---------------------------------------------------------------------------

/// Extract an XML attribute value by name from a tag string.
/// Supports both double-quoted (`name="val"`) and single-quoted (`name='val'`) values.
fn extract_xml_attr(tag: &str, attr: &str) -> Option<String> {
    // Try double-quoted: name="value"
    let dq = format!("{}=\"", attr);
    if let Some(start) = tag.find(&dq) {
        let value_start = start + dq.len();
        if let Some(end) = tag[value_start..].find('"') {
            return Some(tag[value_start..value_start + end].to_string());
        }
    }
    // Try single-quoted: name='value'
    let sq = format!("{}='", attr);
    if let Some(start) = tag.find(&sq) {
        let value_start = start + sq.len();
        if let Some(end) = tag[value_start..].find('\'') {
            return Some(tag[value_start..value_start + end].to_string());
        }
    }
    None
}

/// Parse `<parameter name="key">value</parameter>` blocks from an XML body.
fn parse_parameters(body: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut args = serde_json::Map::new();
    let mut search = 0usize;
    let param_open = "<parameter";
    let param_close = "</parameter>";

    while let Some(tag_start) = body[search..].find(param_open) {
        let abs_start = search + tag_start;
        // Find end of opening <parameter ...> tag
        let after_open = match body[abs_start..].find('>') {
            Some(pos) => abs_start + pos + 1,
            None => break,
        };
        let open_tag = &body[abs_start..after_open];
        let key = extract_xml_attr(open_tag, "name");
        // Find closing </parameter>
        match body[after_open..].find(param_close) {
            Some(pos) => {
                let value = &body[after_open..after_open + pos];
                if let Some(k) = key {
                    args.insert(k, serde_json::Value::String(value.to_string()));
                }
                search = after_open + pos + param_close.len();
            }
            None => break,
        }
    }
    args
}

/// Parse XML-style `<invoke name="tool">...</invoke>` blocks from text.
///
/// Supports the MiniMax format:
/// ```xml
/// <invoke name="tool_name">
///   <parameter name="key1">value1</parameter>
///   <parameter name="key2">value2</parameter>
/// </invoke>
/// ```
///
/// Also consumes any trailing `</minimax:tool_call>` or `</minimax:toolcall>`
/// that immediately follows `</invoke>`.
///
/// Returns `(cleaned_text, extracted_tool_calls)`.
pub(crate) fn parse_invoke_xml(text: &str) -> (String, Vec<crate::message::ToolCall>) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut calls: Vec<crate::message::ToolCall> = Vec::new();
    let mut cursor = 0usize;
    let mut call_index = 0usize;

    while let Some(invoke_start) = text[cursor..].find("<invoke") {
        // Text before <invoke>
        let before = text[cursor..cursor + invoke_start].trim();
        if !before.is_empty() {
            text_parts.push(before.to_string());
        }

        // Find end of opening <invoke ...> tag
        let abs_invoke_start = cursor + invoke_start;
        let tag_end = match text[abs_invoke_start..].find('>') {
            Some(pos) => abs_invoke_start + pos + 1,
            None => break,
        };
        let open_tag = &text[abs_invoke_start..tag_end];

        // Extract tool name
        let name = match extract_xml_attr(open_tag, "name") {
            Some(n) => n,
            None => {
                cursor = tag_end;
                continue;
            }
        };

        // Find </invoke>
        let rest = &text[tag_end..];
        let close = "</invoke>";
        match rest.find(close) {
            Some(close_pos) => {
                let body = &rest[..close_pos];
                let after_close = tag_end + close_pos + close.len();

                // Parse parameters from body
                let args = parse_parameters(body);
                let args_json = serde_json::Value::Object(args).to_string();

                calls.push(crate::message::ToolCall {
                    id: format!("tool-call-{}", call_index),
                    name,
                    arguments: args_json,
                    thought_signature: None,
                });
                call_index += 1;

                // Skip any whitespace and trailing </minimax:tool_call> or </minimax:toolcall>
                cursor = after_close;
                let after = text[cursor..].trim_start();
                let trimmed = cursor + (text[cursor..].len() - after.len());
                for suffix in &["</minimax:tool_call>", "</minimax:toolcall>"] {
                    if text[trimmed..].starts_with(suffix) {
                        cursor = trimmed + suffix.len();
                        break;
                    }
                }
            }
            None => break,
        }
    }

    // Trailing text after last invoke block
    let remaining = text[cursor..].trim();
    if !remaining.is_empty() {
        text_parts.push(remaining.to_string());
    }

    let cleaned = text_parts.join("\n");
    (cleaned, calls)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_invoke_xml_basic() {
        let text = r#"I'll search the codebase.

<invoke name="task">
<parameter name="description">Search for X</parameter>
<parameter name="prompt">Find X in the code</parameter>
<parameter name="subagent_type">explorer</parameter>
</invoke>
</minimax:tool_call>"#;

        let (cleaned, calls) = parse_invoke_xml(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "task");
        assert!(calls[0].arguments.contains("Search for X"));
        assert!(!cleaned.contains("<invoke"));
        assert!(!cleaned.contains("</minimax:tool_call>"));
    }

    #[test]
    fn test_parse_invoke_xml_no_invoke() {
        let text = "Just some plain text without any tool calls.";
        let (cleaned, calls) = parse_invoke_xml(text);
        assert!(calls.is_empty());
        assert_eq!(cleaned, text);
    }

    #[test]
    fn test_parse_invoke_xml_without_minimax_wrapper() {
        let text = r#"<invoke name="shell">
<parameter name="command">echo hello</parameter>
</invoke>"#;

        let (cleaned, calls) = parse_invoke_xml(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
        assert!(calls[0].arguments.contains("echo hello"));
        assert!(cleaned.is_empty());
    }

    #[test]
    fn test_parse_invoke_xml_text_before_and_after() {
        let text = r#"First, let me search.

<invoke name="glob">
<parameter name="pattern">**/*.rs</parameter>
</invoke>

Then I will read the results."#;

        let (cleaned, calls) = parse_invoke_xml(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "glob");
        assert!(cleaned.contains("First, let me search."));
        assert!(cleaned.contains("Then I will read the results."));
        assert!(!cleaned.contains("<invoke"));
    }
}
