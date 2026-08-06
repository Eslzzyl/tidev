use crate::message::{Message, MessageAttachment, MessageRole, tool_output_preview};

pub fn message_text_with_file_references(message: &Message) -> String {
    let mut text = if matches!(message.role, MessageRole::Tool) {
        if message.metadata.preserve_full_output {
            message.content.clone()
        } else {
            tool_output_preview(message.tool_name.as_deref(), &message.content)
        }
    } else {
        message.content.clone()
    };

    for attachment in &message.attachments {
        if let Some(prompt_text) = attachment.prompt_text() {
            text.push_str(&prompt_text);
        }
    }

    text
}

pub fn image_attachments(message: &Message) -> impl Iterator<Item = &MessageAttachment> {
    message
        .attachments
        .iter()
        .filter(|attachment| attachment.is_image())
}

fn _is_placeholder() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_tool_message(preserve_full_output: bool) -> Message {
        let mut message = Message::tool_result(
            "call-1",
            "delegate",
            crate::message::ToolExecutionResult::new("x".repeat(9_000)),
        );
        message.metadata.preserve_full_output = preserve_full_output;
        message
    }

    #[test]
    fn tool_output_uses_generic_full_output_marker() {
        let message = long_tool_message(true);
        assert_eq!(message_text_with_file_references(&message), message.content);
    }

    #[test]
    fn tool_output_preview_remains_default_without_marker() {
        let message = long_tool_message(false);
        let text = message_text_with_file_references(&message);
        assert!(text.starts_with("[delegate output truncated: 9000 chars]"));
        assert_ne!(text, message.content);
    }
}
