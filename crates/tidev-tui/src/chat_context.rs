//! Local chat context — provides the rendering code with the data it needs,
//! replacing the old `tidev_session::session::Conversation` struct.

use std::collections::HashMap;

use uuid::Uuid;

use tidev_core::{MessageAppData, SessionMessage};
use tidev_llm::message::Message;

/// Provider-semantic reasoning display data kept separate from the raw
/// reasoning string that is replayed to the next LLM request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReasoningDisplay {
    pub ordinary: String,
    pub summaries: Vec<ReasoningSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReasoningSummary {
    pub summary_index: Option<u32>,
    pub content: String,
}

/// A lightweight replacement for the old `Conversation` type.
/// Holds the message list and session metadata needed by rendering code.
#[derive(Clone, Debug, Default)]
pub struct ChatContext {
    pub session_id: Uuid,
    pub title: String,
    pub messages: Vec<Message>,
    pub message_app_data: HashMap<Uuid, MessageAppData>,
    /// UI-only reasoning classification. The raw `Message.reasoning` remains
    /// the single value used to construct subsequent provider requests.
    pub(crate) reasoning_display: HashMap<Uuid, ReasoningDisplay>,
    /// When set, only messages up to (not including) this one are visible
    /// (undo revert point).
    pub revert_message_id: Option<Uuid>,
    pub parent_session_id: Option<Uuid>,
    pub model_display_name: String,
    pub provider_display_name: String,
}

impl ChatContext {
    pub fn new(
        session_id: Uuid,
        title: String,
        messages: Vec<Message>,
        parent_session_id: Option<Uuid>,
        model_display_name: String,
        provider_display_name: String,
    ) -> Self {
        let message_app_data = messages
            .iter()
            .map(|message| (message.id, MessageAppData::default()))
            .collect();
        Self {
            session_id,
            title,
            messages,
            message_app_data,
            reasoning_display: HashMap::new(),
            revert_message_id: None,
            parent_session_id,
            model_display_name,
            provider_display_name,
        }
    }

    /// Create a chat context from protocol messages paired with app data.
    pub fn from_session_messages(
        session_id: Uuid,
        title: String,
        session_messages: Vec<SessionMessage>,
        parent_session_id: Option<Uuid>,
        model_display_name: String,
        provider_display_name: String,
    ) -> Self {
        let mut messages = Vec::with_capacity(session_messages.len());
        let mut message_app_data = HashMap::with_capacity(session_messages.len());
        for session_message in session_messages {
            let id = session_message.message.id;
            messages.push(session_message.message);
            message_app_data.insert(id, session_message.app_data);
        }
        Self {
            session_id,
            title,
            messages,
            message_app_data,
            reasoning_display: HashMap::new(),
            revert_message_id: None,
            parent_session_id,
            model_display_name,
            provider_display_name,
        }
    }

    /// Return the visible subset of messages (respecting undo revert).
    pub fn visible_messages(&self) -> &[Message] {
        let end = self
            .revert_message_id
            .and_then(|id| self.messages.iter().position(|m| m.id == id))
            .unwrap_or(self.messages.len());
        &self.messages[..end]
    }

    /// Whether the chat has been reverted (undo active).
    pub fn is_reverted(&self) -> bool {
        self.revert_message_id.is_some()
    }

    /// Return application-owned fields for a message.
    pub fn app_data(&self, message_id: Uuid) -> Option<&MessageAppData> {
        self.message_app_data.get(&message_id)
    }

    pub(crate) fn append_reasoning_delta(&mut self, message_id: Uuid, content: &str) {
        self.reasoning_display
            .entry(message_id)
            .or_default()
            .ordinary
            .push_str(content);
    }

    pub(crate) fn append_reasoning_summary_delta(
        &mut self,
        message_id: Uuid,
        summary_index: Option<u32>,
        content: &str,
    ) {
        let display = self.reasoning_display.entry(message_id).or_default();
        if let Some(last) = display.summaries.last_mut()
            && last.summary_index == summary_index
        {
            last.content.push_str(content);
        } else {
            display.summaries.push(ReasoningSummary {
                summary_index,
                content: content.to_string(),
            });
        }
    }

    /// Return the current messages paired with their application data.
    pub fn session_messages(&self) -> Vec<SessionMessage> {
        self.messages
            .iter()
            .map(|message| {
                let app_data = self.app_data(message.id).cloned().unwrap_or_default();
                SessionMessage::new(message.clone(), app_data)
            })
            .collect()
    }
}

impl ChatContext {
    /// Push a message to the end of the context.
    pub fn push(&mut self, message: Message) {
        self.push_with_app_data(message, MessageAppData::default());
    }

    /// Push a message together with its application-owned fields.
    pub fn push_with_app_data(&mut self, message: Message, app_data: MessageAppData) {
        let id = message.id;
        self.messages.push(message);
        self.message_app_data.insert(id, app_data);
    }

    /// Replace application-owned fields for an existing message.
    pub fn set_app_data(&mut self, message_id: Uuid, app_data: MessageAppData) {
        self.message_app_data.insert(message_id, app_data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_llm::message::{Message, MessageRole};

    #[test]
    fn session_messages_preserve_app_data_by_message_id() {
        let mut message = Message::new(MessageRole::User, "hello");
        let message_id = Uuid::from_u128(7);
        message.id = message_id;
        let app_data = MessageAppData {
            mode: Some("plan".into()),
            child_session_id: Some(Uuid::from_u128(8)),
            ..Default::default()
        };

        let context = ChatContext::from_session_messages(
            Uuid::from_u128(9),
            "title".into(),
            vec![SessionMessage::new(message.clone(), app_data.clone())],
            None,
            "model".into(),
            "provider".into(),
        );

        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].id, message_id);
        assert_eq!(context.messages[0].content, "hello");
        assert_eq!(context.app_data(message_id), Some(&app_data));
        assert_eq!(context.session_messages()[0].app_data, app_data);
    }

    #[test]
    fn push_with_app_data_adds_a_paired_message() {
        let mut context = ChatContext::default();
        let mut message = Message::new(MessageRole::Tool, "output");
        message.id = Uuid::from_u128(10);
        let app_data = MessageAppData {
            file_diffs: Some("diff".into()),
            ..Default::default()
        };

        context.push_with_app_data(message.clone(), app_data.clone());

        assert_eq!(context.messages.len(), 1);
        assert_eq!(context.messages[0].id, message.id);
        assert_eq!(context.messages[0].content, "output");
        assert_eq!(context.session_messages()[0].app_data, app_data);
    }
}
