//! Core image attachment normalization.
//!
//! Image bytes are normalized before a user message or tool result becomes
//! part of the durable protocol message buffer. This makes the prepared bytes
//! the canonical bytes reused by every later provider request.

use tidev_llm::message::MessageAttachment;

pub(crate) fn normalize_attachments(attachments: &mut [MessageAttachment]) {
    for attachment in attachments {
        let MessageAttachment::Image {
            mime,
            data,
            file_size,
            ..
        } = attachment
        else {
            continue;
        };

        match tidev_utils::image::prepare_prompt_image(data, mime) {
            Ok(prepared) => {
                if prepared.resized() {
                    log::info!(
                        "resized image from {}x{} to {}x{} for prompt budget",
                        prepared.source_width,
                        prepared.source_height,
                        prepared.width,
                        prepared.height,
                    );
                }
                *data = prepared.data;
                *mime = prepared.mime;
                *file_size = data.len() as u64;
            }
            Err(error) => {
                // Keep the original attachment when local preparation fails.
                // The provider can report its usual validation error, while a
                // malformed or unsupported image cannot corrupt the message
                // buffer or prevent the rest of a tool result from being saved.
                log::warn!("image preparation skipped: {error:#}");
            }
        }
    }
}
