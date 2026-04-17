use std::collections::HashSet;

use anyhow::Result;

use crate::{
    config::ActiveModel,
    llm::LlmClient,
    prompts::compression_system_prompt,
    session::{Conversation, Message, MessageAttachment, MessageRole},
};

#[derive(Clone, Debug)]
pub struct ContextManager {
    pub summary: Option<String>,
    pub retained_from: usize,
    pub prune_threshold_tokens: usize,
    pub retain_recent_tokens: usize,
    pub maximum_summary_chars: usize,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextManager {
    pub fn new() -> Self {
        Self {
            summary: None,
            retained_from: 0,
            prune_threshold_tokens: 24_000,
            retain_recent_tokens: 12_000,
            maximum_summary_chars: 8_000,
        }
    }

    pub fn estimate_tokens_for_text(text: &str) -> usize {
        (text.chars().count() / 4).max(1)
    }

    fn message_tokens(message: &Message) -> usize {
        if let Some(tokens) = message.total_tokens {
            return tokens as usize;
        }

        let tool_tokens: usize = message
            .tool_calls
            .iter()
            .map(|tool_call| {
                Self::estimate_tokens_for_text(&tool_call.name)
                    + Self::estimate_tokens_for_text(&tool_call.arguments)
            })
            .sum();

        let attachment_tokens: usize = message
            .attachments
            .iter()
            .map(|attachment| match attachment {
                MessageAttachment::FileReference { content, .. } => {
                    Self::estimate_tokens_for_text(content)
                }
                MessageAttachment::DirectoryReference { tree, .. } => {
                    Self::estimate_tokens_for_text(tree)
                }
                MessageAttachment::Image { filename, mime, .. } => {
                    Self::estimate_tokens_for_text(filename)
                        + Self::estimate_tokens_for_text(mime)
                        + 128
                }
            })
            .sum();

        Self::estimate_tokens_for_text(&message.content)
            + Self::estimate_tokens_for_text(&message.reasoning)
            + tool_tokens
            + attachment_tokens
            + 8
    }

    pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
        messages.iter().map(Self::message_tokens).sum()
    }

    pub fn needs_compaction(&self, conversation: &Conversation) -> bool {
        Self::estimate_tokens_for_messages(conversation.visible_messages())
            > self.prune_threshold_tokens
    }

    pub fn build_request_messages(&self, conversation: &Conversation) -> Vec<Message> {
        let mut messages = Vec::new();
        let mut pending_tool_calls = HashSet::new();

        if let Some(summary) = &self.summary {
            messages.push(Message::new(
                MessageRole::System,
                format!("Context summary for continuation:\n{summary}"),
            ));
        }

        for message in conversation
            .visible_messages()
            .iter()
            .skip(self.retained_from)
        {
            if message.streaming {
                continue;
            }

            match message.role {
                MessageRole::System => {}
                MessageRole::User => {
                    pending_tool_calls.clear();
                    messages.push(message.clone());
                }
                MessageRole::Assistant => {
                    pending_tool_calls = message
                        .tool_calls
                        .iter()
                        .map(|tool_call| tool_call.id.clone())
                        .collect();
                    messages.push(message.clone());
                }
                MessageRole::Tool => {
                    let Some(tool_call_id) = message.tool_call_id.as_ref() else {
                        continue;
                    };

                    if pending_tool_calls.remove(tool_call_id) {
                        messages.push(message.clone());
                    }
                }
                MessageRole::Error => {}
            }
        }

        messages
    }

    pub async fn compact_if_needed(
        &mut self,
        llm: &LlmClient,
        model: &ActiveModel,
        conversation: &Conversation,
    ) -> Result<bool> {
        let messages = conversation.visible_messages();
        if !self.needs_compaction(conversation) || messages.is_empty() {
            return Ok(false);
        }

        let split_index = self.choose_split_index(messages);
        if split_index == 0 || split_index >= messages.len() {
            return Ok(false);
        }

        let compressed_chunk = messages[..split_index].to_vec();
        let prompt = self.build_compression_prompt(&compressed_chunk);
        let summary = llm
            .complete_with_messages(
                model.clone(),
                vec![
                    Message::new(MessageRole::System, self.compression_system_prompt()),
                    Message::new(MessageRole::User, prompt),
                ],
            )
            .await
            .unwrap_or_else(|error| self.fallback_summary(&compressed_chunk, &error.to_string()));

        self.summary = Some(summary.chars().take(self.maximum_summary_chars).collect());
        self.retained_from = split_index;
        Ok(true)
    }

    pub fn compacted_message_count(&self) -> usize {
        self.retained_from
    }

    fn choose_split_index(&self, messages: &[Message]) -> usize {
        let mut token_budget = self.retain_recent_tokens;
        let mut keep_from = messages.len();

        for (index, message) in messages.iter().enumerate().rev() {
            let message_tokens = Self::message_tokens(message);
            if token_budget < message_tokens {
                keep_from = index + 1;
                break;
            }

            token_budget = token_budget.saturating_sub(message_tokens);
            keep_from = index;
        }

        self.align_split_index_to_tool_boundary(messages, keep_from)
    }

    fn align_split_index_to_tool_boundary(
        &self,
        messages: &[Message],
        split_index: usize,
    ) -> usize {
        if split_index == 0 || split_index >= messages.len() {
            return split_index;
        }

        if !matches!(messages[split_index].role, MessageRole::Tool) {
            return split_index;
        }

        let mut aligned_index = split_index;
        while aligned_index > 0 && matches!(messages[aligned_index].role, MessageRole::Tool) {
            aligned_index -= 1;
        }

        aligned_index
    }

    fn build_compression_prompt(&self, messages: &[Message]) -> String {
        let mut prompt = String::from(
            "Provide a detailed continuation summary for this coding conversation.\n\n",
        );

        if let Some(summary) = &self.summary {
            prompt.push_str("Existing summary:\n");
            prompt.push_str(summary);
            prompt.push_str("\n\n");
        }

        prompt.push_str("Messages to compress:\n");
        for message in messages {
            let attachment_summary = message
                .attachments
                .iter()
                .map(|attachment| attachment.summary())
                .collect::<Vec<_>>()
                .join(" ");
            prompt.push_str(&format!(
                "- {}: {}\n",
                message.role.label(),
                message.content
            ));

            if !attachment_summary.trim().is_empty() {
                prompt.push_str(&format!(
                    "  attachments: {}\n",
                    truncate(&attachment_summary, 240)
                ));
            }

            if !message.reasoning.trim().is_empty() {
                prompt.push_str(&format!(
                    "  thinking: {}\n",
                    truncate(&message.reasoning, 240)
                ));
            }

            for tool_call in &message.tool_calls {
                prompt.push_str(&format!(
                    "  tool call: {} {}\n",
                    tool_call.name,
                    truncate(&tool_call.arguments, 240)
                ));
            }
        }

        prompt.push_str(
            "\nFocus on: goals, decisions, file paths, code changes, active tasks, tool results, constraints, and anything needed to continue the work without re-reading prior context.",
        );
        prompt
    }

    fn compression_system_prompt(&self) -> String {
        compression_system_prompt().to_string()
    }

    fn fallback_summary(&self, messages: &[Message], error: &str) -> String {
        let mut summary = String::from("Context summary fallback\n");
        summary.push_str(&format!("Compression request failed: {error}\n"));
        for message in messages.iter().rev().take(12).rev() {
            summary.push_str(&format!(
                "- {}: {}\n",
                message.role.label(),
                truncate(&message.content, 240)
            ));

            if !message.attachments.is_empty() {
                let attachment_summary = message
                    .attachments
                    .iter()
                    .map(|attachment| attachment.summary())
                    .collect::<Vec<_>>()
                    .join(" ");
                summary.push_str(&format!(
                    "  attachments: {}\n",
                    truncate(&attachment_summary, 240)
                ));
            }
        }
        summary
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }

    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;
    use crate::session::{Message, ToolCall, ToolExecutionResult};

    fn message_with_tokens(role: MessageRole, content: &str, total_tokens: u32) -> Message {
        let mut message = Message::new(role, content);
        message.total_tokens = Some(total_tokens);
        message
    }

    fn test_conversation(messages: Vec<Message>) -> Conversation {
        Conversation {
            session_id: Uuid::new_v4(),
            parent_session_id: None,
            workspace_root: String::new(),
            provider_id: "provider".to_string(),
            provider_display_name: "Provider".to_string(),
            model_id: "model".to_string(),
            model_display_name: "Model".to_string(),
            title: "Test".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            messages,
            revert_message_id: None,
        }
    }

    #[test]
    fn choose_split_index_keeps_tool_block_together() {
        let mut manager = ContextManager::new();
        manager.retain_recent_tokens = 2;

        let mut assistant = message_with_tokens(MessageRole::Assistant, "call tools", 1);
        assistant.tool_calls = vec![ToolCall {
            id: "tool-call-1".to_string(),
            name: "grep".to_string(),
            arguments: "{}".to_string(),
        }];

        let messages = vec![
            message_with_tokens(MessageRole::User, "first", 1),
            assistant,
            message_with_tokens(MessageRole::Tool, "result", 1),
            message_with_tokens(MessageRole::Assistant, "follow up", 1),
        ];

        assert_eq!(manager.choose_split_index(&messages), 1);
    }

    #[test]
    fn build_request_messages_keeps_valid_tool_results_and_skips_orphans() {
        let mut assistant = Message::new(MessageRole::Assistant, "call tools");
        assistant.tool_calls = vec![ToolCall {
            id: "tool-call-1".to_string(),
            name: "grep".to_string(),
            arguments: "{}".to_string(),
        }];

        let valid_conversation = test_conversation(vec![
            Message::new(MessageRole::User, "question"),
            assistant.clone(),
            Message::tool_result("tool-call-1", "grep", ToolExecutionResult::new("found")),
            Message::new(MessageRole::Assistant, "answer"),
        ]);

        let manager = ContextManager::new();
        let valid_request_messages = manager.build_request_messages(&valid_conversation);
        let valid_roles: Vec<_> = valid_request_messages
            .iter()
            .map(|message| message.role.label())
            .collect();
        assert_eq!(valid_roles, vec!["user", "assistant", "tool", "assistant"]);

        let mut orphan_manager = ContextManager::new();
        orphan_manager.retained_from = 2;
        let orphan_request_messages = orphan_manager.build_request_messages(&valid_conversation);
        let orphan_roles: Vec<_> = orphan_request_messages
            .iter()
            .map(|message| message.role.label())
            .collect();
        assert_eq!(orphan_roles, vec!["assistant"]);
        assert!(
            orphan_request_messages
                .iter()
                .all(|message| !matches!(message.role, MessageRole::Tool))
        );
    }
}
