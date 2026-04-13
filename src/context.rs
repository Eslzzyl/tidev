use anyhow::Result;

use crate::{
    config::ActiveModel,
    llm::LlmClient,
    prompts::compression_system_prompt,
    session::{Conversation, Message, MessageRole},
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

    pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|message| {
                let tool_tokens: usize = message
                    .tool_calls
                    .iter()
                    .map(|tool_call| {
                        Self::estimate_tokens_for_text(&tool_call.name)
                            + Self::estimate_tokens_for_text(&tool_call.arguments)
                    })
                    .sum();

                Self::estimate_tokens_for_text(&message.content)
                    + Self::estimate_tokens_for_text(&message.reasoning)
                    + tool_tokens
                    + 8
            })
            .sum()
    }

    pub fn needs_compaction(&self, conversation: &Conversation) -> bool {
        Self::estimate_tokens_for_messages(conversation.visible_messages())
            > self.prune_threshold_tokens
    }

    pub fn build_request_messages(&self, conversation: &Conversation) -> Vec<Message> {
        let mut messages = Vec::new();

        if let Some(summary) = &self.summary {
            messages.push(Message::new(
                MessageRole::System,
                format!("Context summary for continuation:\n{summary}"),
            ));
        }

        messages.extend(
            conversation
                .visible_messages()
                .iter()
                .skip(self.retained_from)
                .filter(|message| !message.streaming)
                .filter(|message| {
                    matches!(
                        message.role,
                        MessageRole::User | MessageRole::Assistant | MessageRole::Tool
                    )
                })
                .cloned(),
        );

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
            let message_tokens = Self::estimate_tokens_for_text(&message.content) + 8;
            if token_budget < message_tokens {
                keep_from = index + 1;
                break;
            }

            token_budget = token_budget.saturating_sub(message_tokens);
            keep_from = index;
        }

        keep_from
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
            prompt.push_str(&format!(
                "- {}: {}\n",
                message.role.label(),
                message.content
            ));

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
