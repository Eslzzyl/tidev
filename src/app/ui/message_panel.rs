use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct MessagePanelMessage {
    pub message_id: Uuid,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct MessagePanelState {
    pub selected_index: usize,
    pub messages: Vec<MessagePanelMessage>,
}

impl MessagePanelState {
    pub fn new(messages: Vec<MessagePanelMessage>) -> Self {
        Self {
            selected_index: 0,
            messages,
        }
    }

    pub fn matching_indices(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_ascii_lowercase();
        self.messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| {
                if message_matches_query(&query, message) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn reset_selection(&mut self, query: &str) {
        let matches = self.matching_indices(query);
        self.selected_index = matches.get(0).copied().unwrap_or(0);
    }

    pub fn move_selection(&mut self, query: &str, delta: isize) {
        let matches = self.matching_indices(query);
        if matches.is_empty() {
            self.selected_index = 0;
            return;
        }

        let len = matches.len() as isize;
        let current = self.selected_index.min(matches.len().saturating_sub(1)) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        self.selected_index = next;
    }

    pub fn selected_message(&self, query: &str) -> Option<&MessagePanelMessage> {
        let matches = self.matching_indices(query);
        let message_index = *matches.get(self.selected_index)?;
        self.messages.get(message_index)
    }
}

fn message_matches_query(query: &str, message: &MessagePanelMessage) -> bool {
    if query.is_empty() {
        return true;
    }

    let content = message.content.to_ascii_lowercase();
    let id = message.message_id.to_string().to_ascii_lowercase();

    content.contains(query) || id.contains(query)
}
