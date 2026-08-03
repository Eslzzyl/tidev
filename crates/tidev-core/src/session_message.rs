use std::ops::{Deref, DerefMut};

use tidev_llm::message::Message;
use tidev_storage::MessageAppData;

use crate::Mode;

/// A protocol message paired with tidev application state.
///
/// LLM providers consume the inner [`Message`]. UI, undo, and mode-aware
/// orchestration consume the separate application data without putting those
/// fields back into the protocol crate.
#[derive(Clone, Debug)]
pub struct SessionMessage {
    pub message: Message,
    pub app_data: MessageAppData,
}

impl SessionMessage {
    pub fn new(message: Message, app_data: MessageAppData) -> Self {
        Self { message, app_data }
    }

    pub fn mode(&self) -> Option<Mode> {
        self.app_data.mode.as_deref()?.parse().ok()
    }
}

impl Deref for SessionMessage {
    type Target = Message;

    fn deref(&self) -> &Self::Target {
        &self.message
    }
}

impl DerefMut for SessionMessage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.message
    }
}
