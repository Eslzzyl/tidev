use crate::session::{Message, MessageAttachment};

pub fn message_text_with_file_references(message: &Message) -> String {
    let mut text = message.content.clone();

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
