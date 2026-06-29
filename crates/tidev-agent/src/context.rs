use std::collections::HashMap;

use anyhow::Result;

use tidev_types::prompts::SessionMode;

use tidev_config::ActiveModel;
use tidev_tools::ToolDefinition;
use tidev_llm::LlmClient;
use tidev_session::session::{
    Conversation, Message, MessageAttachment, MessageRole, ToolExecutionResult,
};

/// Configuration for compaction operations.
#[derive(Clone)]
pub struct CompactionConfig<'a> {
    pub llm: &'a LlmClient,
    pub model: &'a ActiveModel,
    pub conversation: &'a Conversation,
    pub manual: bool,
    pub stream_ctx: Option<(
        u64,
        tokio::sync::mpsc::UnboundedSender<tidev_session::session::BackendEvent>,
    )>,
    pub tools: &'a [ToolDefinition],
    pub mode: SessionMode,
}

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

    pub fn from_state(summary: Option<String>, retained_from: usize) -> Self {
        let mut manager = Self::new();
        manager.summary = summary;
        manager.retained_from = retained_from;
        manager
    }

    pub fn estimate_tokens_for_text(text: &str) -> usize {
        (text.chars().count() / 4).max(1)
    }

    fn message_tokens(message: &Message) -> usize {
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

    fn compaction_budget_for_model(&self, model: &ActiveModel) -> (usize, usize) {
        if model.context_window == 0 {
            return (self.prune_threshold_tokens, self.retain_recent_tokens);
        }

        let context_window = model.context_window;
        let reserved_tokens = model
            .max_output_tokens
            .max(context_window / 8)
            .max(4_000)
            .min(context_window.saturating_sub(1));
        let trigger_tokens = context_window.saturating_sub(reserved_tokens).max(1);
        let retain_recent_tokens = self
            .retain_recent_tokens
            .max(reserved_tokens)
            .min(trigger_tokens);

        (trigger_tokens, retain_recent_tokens)
    }

    pub fn needs_compaction(&self, conversation: &Conversation, model: &ActiveModel) -> bool {
        let (trigger_tokens, _) = self.compaction_budget_for_model(model);

        let last_context_tokens = conversation
            .visible_messages()
            .iter()
            .rev()
            .find_map(|message| message.input_tokens.or(message.total_tokens));

        match last_context_tokens {
            Some(tokens) => tokens as usize >= trigger_tokens,
            None => {
                Self::estimate_tokens_for_messages(conversation.visible_messages())
                    >= trigger_tokens
            }
        }
    }

    pub fn build_request_messages(
        &self,
        conversation: &Conversation,
        _current_mode: SessionMode,
    ) -> Vec<Message> {
        let mut messages = Vec::new();
        // Map from tool_call_id → tool_name to track which tool calls still need results.
        let mut pending_tool_calls: HashMap<String, String> = HashMap::new();
        if let Some(summary) = &self.summary {
            messages.push(Message::new(
                MessageRole::User,
                format!("Earlier conversation summary:\n{summary}"),
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
                    // Inject synthetic failure results for any orphaned tool calls
                    // before the user message, so the provider doesn't see an
                    // assistant(tool_calls) without corresponding tool results.
                    for (tool_call_id, tool_name) in pending_tool_calls.drain() {
                        log::warn!(
                            "build_request_messages: injecting synthetic failure for orphaned \
                             tool call id={} name={} before user message",
                            tool_call_id,
                            tool_name
                        );
                        let msg = format!(
                            "Tool call {tool_call_id} was orphaned — cancelled or interrupted before completion"
                        );
                        messages.push(Message::tool_result(
                            tool_call_id,
                            tool_name,
                            ToolExecutionResult::new(msg),
                        ));
                    }
                    messages.push(message.clone());
                }
                MessageRole::Assistant => {
                    // Skip assistant messages that have neither content nor tool_calls,
                    // as providers reject messages with both fields missing.
                    if message.content.is_empty() && message.tool_calls.is_empty() {
                        continue;
                    }
                    // Inject synthetic failures for any orphaned tool calls
                    // from a *previous* assistant message before adding this
                    // new one. This handles the case where two consecutive
                    // assistant messages both carry tool_calls — without this,
                    // the earlier orphan would be lost when pending_tool_calls
                    // is overwritten below.
                    if !message.tool_calls.is_empty() && !pending_tool_calls.is_empty() {
                        for (tool_call_id, tool_name) in pending_tool_calls.drain() {
                            log::warn!(
                                "build_request_messages: injecting synthetic failure for orphaned \
                                 tool call id={} name={} before next assistant tool_calls",
                                tool_call_id,
                                tool_name,
                            );
                            let msg = format!(
                                "Tool call {tool_call_id} was orphaned — cancelled or interrupted before completion"
                            );
                            messages.push(Message::tool_result(
                                tool_call_id,
                                tool_name,
                                ToolExecutionResult::new(msg),
                            ));
                        }
                    }

                    // Validate tool call arguments: if any tool call has
                    // arguments that are not valid JSON (e.g. truncated by
                    // the model's output token limit), replace them with an
                    // empty object so the provider does not reject the request.
                    let mut fixed_message = message.clone();
                    for tc in &mut fixed_message.tool_calls {
                        if tc.arguments.is_empty() {
                            tc.arguments = "{}".to_string();
                        } else if serde_json::from_str::<serde_json::Value>(&tc.arguments)
                            .is_err()
                        {
                            log::warn!(
                                "build_request_messages: tool call id={} name={} has invalid \
                                 JSON arguments ({} chars), replacing with empty object",
                                tc.id,
                                tc.name,
                                tc.arguments.len(),
                            );
                            tc.arguments = "{}".to_string();
                        }
                    }

                    pending_tool_calls = fixed_message
                        .tool_calls
                        .iter()
                        .map(|tool_call| (tool_call.id.clone(), tool_call.name.clone()))
                        .collect();
                    messages.push(fixed_message);
                }
                MessageRole::Tool => {
                    let Some(tool_call_id) = message.tool_call_id.as_ref() else {
                        continue;
                    };

                    if pending_tool_calls.remove(tool_call_id).is_some() {
                        messages.push(message.clone());
                    }
                }
                MessageRole::Error => {}
                MessageRole::Shell => {}
            }
        }

        // After processing all visible messages, inject synthetic failure results for any
        // tool calls that are still pending (i.e. the assistant asked to call a tool but no
        // corresponding result message was found). This can happen if tool execution was
        // interrupted or if the conversation state is inconsistent.
        for (tool_call_id, tool_name) in &pending_tool_calls {
            log::warn!(
                "build_request_messages: orphaned tool call id={} name={}, injecting synthetic failure",
                tool_call_id,
                tool_name
            );
            messages.push(Message::tool_result(
                tool_call_id.clone(),
                tool_name.clone(),
                ToolExecutionResult::new(format!(
                    "Tool call {tool_call_id} was orphaned — cancelled or interrupted before completion"
                )),
            ));
        }

        messages
    }

    pub async fn compact_if_needed(&mut self, config: CompactionConfig<'_>) -> Result<bool> {
        if !self.needs_compaction(config.conversation, config.model) && !config.manual {
            return Ok(false);
        }

        self.compact(config).await
    }

    pub async fn compact(&mut self, config: CompactionConfig<'_>) -> Result<bool> {
        let messages = config.conversation.visible_messages();
        if messages.is_empty() {
            return Ok(false);
        }

        // Build request messages using the same logic as normal requests.
        // This ensures the prefix (system prompt + summary + retained messages)
        // is byte-for-byte identical with normal requests, maximizing cache hits.
        let mut compact_msgs = self.build_request_messages(config.conversation, config.mode);
        let summary_instruction = "Please provide a detailed summary of the conversation history above, \
             preserving all goals, decisions, file paths, code changes, tool results, \
             and open tasks. Keep the summary dense and factual. Use short sections such \
             as Goal, Decisions, Files, Tool Results, Open Tasks, and Constraints. \
             Prefer bullets over prose.";
        compact_msgs.push(Message::new(MessageRole::User, summary_instruction));

        let summary = if let Some((request_id, ui_tx)) = config.stream_ctx {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let llm_clone = config.llm.clone();
            let model_clone = config.model.clone();
            let msgs = compact_msgs;
            let session_id = config.conversation.session_id;
            let tools_vec = config.tools.to_vec();

            tokio::spawn(async move {
                let thinking_level = model_clone.thinking_level.clone();
                let llm_config = tidev_types::LlmProviderConfig::from(model_clone);
                let llm_tools: Vec<tidev_types::ToolSchema> = tools_vec
                    .iter()
                    .map(|t: &tidev_tools::ToolDefinition| tidev_types::ToolSchema {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    })
                    .collect();
                llm_clone
                    .stream_chat(
                        session_id,
                        request_id,
                        llm_config,
                        msgs,
                        llm_tools,
                        tx,
                        thinking_level,
                    )
                    .await;
            });

            let mut text = String::new();
            while let Some(event) = rx.recv().await {
                match &event {
                    tidev_session::session::BackendEvent::Delta { content, .. } => {
                        text.push_str(content);
                        let _ = ui_tx.send(event.clone());
                    }
                    tidev_session::session::BackendEvent::Finished { .. } => {
                        // Do NOT forward Finished to the UI — it would trigger
                        // finish_assistant_turn() which is designed for real
                        // Assistant turns, not compaction.  The completion
                        // signal is delivered via ContextCompacted instead.
                        break;
                    }
                    tidev_session::session::BackendEvent::Failed { error, .. } => {
                        let _ = ui_tx.send(event.clone());
                        return Err(anyhow::anyhow!("compaction failed: {}", error));
                    }
                    _ => {
                        let _ = ui_tx.send(event.clone());
                    }
                }
            }
            text
        } else {
            let llm_config = tidev_types::LlmProviderConfig::from(config.model.clone());
            let llm_tools: Vec<tidev_types::ToolSchema> = config
                .tools
                .iter()
                .map(|t: &tidev_tools::ToolDefinition| tidev_types::ToolSchema {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    parameters: t.parameters.clone(),
                })
                .collect();
            config
                .llm
                .complete_with_messages(llm_config, compact_msgs, llm_tools)
                .await
                .unwrap_or_else(|error| self.fallback_summary(messages, &error.to_string()))
        };

        self.summary = Some(summary.chars().take(self.maximum_summary_chars).collect());
        self.retained_from = messages.len();
        Ok(true)
    }

    pub fn compacted_message_count(&self) -> usize {
        self.retained_from
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
    use tidev_config::{ActiveModel, ApiType};
    use tidev_session::session::{Message, ToolCall, ToolExecutionResult};

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
            context_summary: None,
            context_retained_from: 0,
            messages,
            revert_message_id: None,
        }
    }

    fn test_model(context_window: usize, max_output_tokens: usize) -> ActiveModel {
        ActiveModel {
            provider_id: "provider".to_string(),
            provider_display_name: "Provider".to_string(),
            base_url: "https://example.com".to_string(),
            api_type: ApiType::OpenAiChatCompletions,
            model_id: "model".to_string(),
            request_model_id: "model".to_string(),
            display_name: "Model".to_string(),
            context_window,
            max_output_tokens,
            temperature: Some(0.0),
            supports_images: false,
            system_prompt: String::new(),
            api_key: None,
            extra_body: None,
            thinking_level: tidev_config::reasoning::ThinkingLevelType::None,
        }
    }

    #[test]
    fn compaction_budget_scales_with_model_window() {
        let manager = ContextManager::new();
        let model = test_model(128_000, 32_768);

        let (trigger_tokens, retain_recent_tokens) = manager.compaction_budget_for_model(&model);

        assert_eq!(trigger_tokens, 95_232);
        assert_eq!(retain_recent_tokens, 32_768);
    }

    #[test]
    fn build_request_messages_keeps_valid_tool_results_and_skips_orphans() {
        let mut assistant = Message::new(MessageRole::Assistant, "call tools");
        assistant.tool_calls = vec![ToolCall {
            id: "tool-call-1".to_string(),
            name: "grep".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        }];

        let valid_conversation = test_conversation(vec![
            Message::new(MessageRole::User, "question"),
            assistant.clone(),
            Message::tool_result("tool-call-1", "grep", ToolExecutionResult::new("found")),
            Message::new(MessageRole::Assistant, "answer"),
        ]);

        let manager = ContextManager::new();
        let valid_request_messages =
            manager.build_request_messages(&valid_conversation, SessionMode::Build);
        let valid_roles: Vec<_> = valid_request_messages
            .iter()
            .map(|message| message.role.label())
            .collect();
        assert_eq!(valid_roles, vec!["user", "assistant", "tool", "assistant"]);

        let mut orphan_manager = ContextManager::new();
        orphan_manager.retained_from = 2;
        let orphan_request_messages =
            orphan_manager.build_request_messages(&valid_conversation, SessionMode::Build);
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

        // Regression test: orphaned tool calls before a user message should
        // get synthetic failure results injected BEFORE the user message,
        // not cleared by it. The sequence must be:
        //   assistant(tool_calls) → tool(synthetic failure) → user
        // not:
        //   assistant(tool_calls) → user  ← rejected by providers
        let mut orphan_tool_call = Message::new(MessageRole::Assistant, "");
        orphan_tool_call.tool_calls = vec![ToolCall {
            id: "orphan-call".to_string(),
            name: "edit".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        }];
        let conversation_with_orphan = test_conversation(vec![
            orphan_tool_call,
            Message::new(MessageRole::User, "the edit failed"),
        ]);
        let manager = ContextManager::new();
        let request_messages =
            manager.build_request_messages(&conversation_with_orphan, SessionMode::Build);
        let roles: Vec<_> = request_messages
            .iter()
            .map(|message| message.role.label())
            .collect();
        // Should be: assistant → tool → user  (NOT assistant → user)
        assert_eq!(roles, vec!["assistant", "tool", "user"]);
        let synthetic_tool = &request_messages[1];
        assert_eq!(synthetic_tool.role, MessageRole::Tool);
        assert_eq!(synthetic_tool.tool_call_id.as_deref(), Some("orphan-call"));
        assert!(
            synthetic_tool.content.contains("interrupted"),
            "synthetic tool result should mention interruption"
        );
    }

    #[test]
    fn build_request_messages_sanitizes_invalid_tool_call_arguments() {
        // Regression: when the model's output is truncated mid-JSON, the
        // tool call arguments are stored as incomplete JSON.  Providers
        // reject the request if the arguments are not valid JSON.
        // build_request_messages must replace them with "{}" so the
        // provider accepts the request.
        let mut assistant = Message::new(MessageRole::Assistant, "writing files");
        assistant.tool_calls = vec![
            ToolCall {
                id: "call-ok".to_string(),
                name: "write".to_string(),
                arguments: r#"{"content":"valid file content"}"#.to_string(),
                thought_signature: None,
            },
            ToolCall {
                id: "call-bad".to_string(),
                name: "write".to_string(),
                arguments: r#"{"content":"truncated at line 1 col"#.to_string(),
                thought_signature: None,
            },
        ];

        let conversation = test_conversation(vec![
            Message::new(MessageRole::User, "write files"),
            assistant,
            Message::tool_result("call-ok", "write", ToolExecutionResult::new("Wrote file")),
            Message::tool_result(
                "call-bad",
                "write",
                ToolExecutionResult::new("Error: failed to parse arguments"),
            ),
            Message::new(MessageRole::Assistant, "done"),
        ]);

        let manager = ContextManager::new();
        let request_messages =
            manager.build_request_messages(&conversation, SessionMode::Build);

        // Find the assistant message in the request and check its tool calls.
        let assistant_msg = request_messages
            .iter()
            .find(|m| m.role == MessageRole::Assistant && !m.tool_calls.is_empty())
            .expect("should have an assistant message with tool_calls");

        let bad_tc = assistant_msg
            .tool_calls
            .iter()
            .find(|tc| tc.id == "call-bad")
            .expect("bad tool call should still be present");

        assert_eq!(
            bad_tc.arguments, "{}",
            "invalid arguments should be replaced with empty JSON object"
        );

        // The good tool call should be unchanged.
        let ok_tc = assistant_msg
            .tool_calls
            .iter()
            .find(|tc| tc.id == "call-ok")
            .expect("ok tool call should be present");

        assert_eq!(
            ok_tc.arguments,
            r#"{"content":"valid file content"}"#,
            "valid arguments should be preserved"
        );
    }

    #[test]
    fn build_request_messages_replaces_empty_arguments() {
        // Empty arguments string is also invalid for some providers.
        let mut assistant = Message::new(MessageRole::Assistant, "running tool");
        assistant.tool_calls = vec![ToolCall {
            id: "call-empty".to_string(),
            name: "bash".to_string(),
            arguments: String::new(),
            thought_signature: None,
        }];

        let conversation = test_conversation(vec![
            Message::new(MessageRole::User, "run command"),
            assistant,
            Message::tool_result("call-empty", "bash", ToolExecutionResult::new("output")),
            Message::new(MessageRole::Assistant, "done"),
        ]);

        let manager = ContextManager::new();
        let request_messages =
            manager.build_request_messages(&conversation, SessionMode::Build);

        let assistant_msg = request_messages
            .iter()
            .find(|m| m.role == MessageRole::Assistant && !m.tool_calls.is_empty())
            .expect("should have an assistant message with tool_calls");

        let tc = &assistant_msg.tool_calls[0];
        assert_eq!(
            tc.arguments, "{}",
            "empty arguments should be replaced with empty JSON object"
        );
    }

    #[test]
    fn compact_request_uses_build_request_messages_prefix() {
        // Verifies that compact() builds its message list using
        // build_request_messages() so the prefix byte-for-byte matches
        // normal requests, maximizing prefix cache hits.

        // ── Case 1: No previous compaction ──
        let messages = vec![
            Message::new(MessageRole::User, "first"),
            Message::new(MessageRole::Assistant, "response one"),
            Message::new(MessageRole::User, "second"),
        ];
        let manager = ContextManager::new();
        let conversation = test_conversation(messages.clone());

        // Compact's message assembly (after the change):
        let mut compact_msgs = manager.build_request_messages(&conversation, SessionMode::Build);
        let compact_instruction =
            "Please provide a detailed summary of the conversation history above";
        compact_msgs.push(Message::new(MessageRole::User, compact_instruction));

        // Normal request structure (same prefix):
        let normal_msgs = manager.build_request_messages(&conversation, SessionMode::Build);

        // Prefix (everything except last message) matches
        assert_eq!(
            compact_msgs.len(),
            normal_msgs.len() + 1,
            "compact should have one extra message (the instruction)"
        );
        for i in 0..normal_msgs.len() {
            assert_eq!(
                compact_msgs[i].role, normal_msgs[i].role,
                "role mismatch at position {}",
                i
            );
            assert_eq!(
                compact_msgs[i].content, normal_msgs[i].content,
                "content mismatch at position {}",
                i
            );
        }

        // Last message in compact is the instruction
        assert_eq!(compact_msgs.last().unwrap().role, MessageRole::User);
        assert!(
            compact_msgs.last().unwrap().content.contains("summary"),
            "last message should be the compact instruction"
        );

        // ── Case 2: With previous compaction (retained_from > 0, summary exists) ──
        let mut manager2 = ContextManager::new();
        manager2.summary = Some("Previous summary content".to_string());
        manager2.retained_from = 1; // skip "first" message

        let mut compact_msgs2 = manager2.build_request_messages(&conversation, SessionMode::Build);
        compact_msgs2.push(Message::new(MessageRole::User, compact_instruction));

        let normal_msgs2 = manager2.build_request_messages(&conversation, SessionMode::Build);

        // Prefix (everything except last message) still matches
        assert_eq!(
            compact_msgs2.len(),
            normal_msgs2.len() + 1,
            "compact should have one extra message (the instruction)"
        );
        for i in 0..normal_msgs2.len() {
            assert_eq!(
                compact_msgs2[i].role, normal_msgs2[i].role,
                "role mismatch at position {} (with summary)",
                i
            );
            assert_eq!(
                compact_msgs2[i].content, normal_msgs2[i].content,
                "content mismatch at position {} (with summary)",
                i
            );
        }

        // First message is the summary placeholder
        assert_eq!(compact_msgs2[0].role, MessageRole::User);
        assert!(
            compact_msgs2[0]
                .content
                .contains("Earlier conversation summary"),
            "first message should be the old summary when one exists"
        );

        // ── Case 3: retained_from after compaction should be messages.len() ──
        assert_eq!(
            manager2.retained_from, 1,
            "retained_from should still be the original value before compact"
        );
        // After compact() runs successfully:
        // manager2.retained_from would become conversation.visible_messages().len()
        // This is verified by compact() setting self.retained_from = messages.len()
    }
}
