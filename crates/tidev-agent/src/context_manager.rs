//! Context manager — message view construction and compaction.
//!
//! This module provides:
//!
//! - [`ContextManager`]: holds compaction state (summary, retained_from) and
//!   performs compaction by injecting a user message and calling the LLM.
//! - [`build_request_messages`]: builds the message list sent to the LLM,
//!   skipping already-compacted messages and injecting the summary.

use anyhow::Result;
use tidev_llm::message::{Message, MessageRole};
use tidev_llm::{LlmClient, LlmProviderConfig, ToolDefinition};
use uuid::Uuid;

use crate::event::{AgentEvent, llm_event_to_agent_event};
use crate::message_buf::MessageBuffer;

// ---------------------------------------------------------------------------
// Compaction prompt
// ---------------------------------------------------------------------------

const SUMMARY_INSTRUCTION: &str = "Please provide a detailed summary of the conversation history above, \
     preserving all goals, decisions, file paths, code changes, tool results, \
     and open tasks. Keep the summary dense and factual. Use short sections such \
     as Goal, Decisions, Files, Tool Results, Open Tasks, and Constraints. \
     Prefer bullets over prose.";

// ---------------------------------------------------------------------------
// Compaction result
// ---------------------------------------------------------------------------

/// Result of a compaction: the summary text and the new retained_from offset.
pub struct CompactionResult {
    pub summary: String,
    pub retained_from: usize,
}

// ---------------------------------------------------------------------------
// ContextManager
// ---------------------------------------------------------------------------

/// Holds compaction state and performs context compression.
///
/// The manager tracks which messages have been compacted (via `retained_from`)
/// and the current summary. The message view seen by the LLM is constructed
/// by [`build_request_messages`], which skips messages before `retained_from`
/// and prepends the summary (if any).
#[derive(Debug)]
pub struct ContextManager {
    pub summary: Option<String>,
    pub retained_from: usize,
    /// Fallback compaction threshold (tokens) when model window is unknown.
    pub prune_threshold_tokens: usize,
    /// Tokens to retain uncompressed below the threshold.
    pub retain_recent_tokens: usize,
    /// Maximum character length of a generated summary.
    pub maximum_summary_chars: usize,
}

impl Default for ContextManager {
    fn default() -> Self {
        Self {
            summary: None,
            retained_from: 0,
            prune_threshold_tokens: 24_000,
            retain_recent_tokens: 12_000,
            maximum_summary_chars: 8_000,
        }
    }
}

impl ContextManager {
    /// Create with default compaction settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restore compaction state previously persisted in the database.
    pub fn from_state(summary: Option<String>, retained_from: usize) -> Self {
        Self {
            summary: summary.filter(|s| !s.trim().is_empty()),
            retained_from,
            ..Default::default()
        }
    }

    // -----------------------------------------------------------------------
    // Token estimation
    // -----------------------------------------------------------------------

    /// Rough token estimate for a piece of text (chars / 4).
    pub fn estimate_tokens_for_text(text: &str) -> usize {
        text.chars().count() / 4
    }

    /// Sum token estimates over a set of messages.
    pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
        messages
            .iter()
            .map(|msg| {
                let mut tokens = Self::estimate_tokens_for_text(&msg.content)
                    + Self::estimate_tokens_for_text(&msg.reasoning);
                for attachment in &msg.attachments {
                    match attachment {
                        tidev_llm::message::MessageAttachment::FileReference {
                            content, ..
                        } => tokens += Self::estimate_tokens_for_text(content),
                        tidev_llm::message::MessageAttachment::DirectoryReference {
                            tree, ..
                        } => tokens += Self::estimate_tokens_for_text(tree),
                        _ => {}
                    }
                }
                for tc in &msg.tool_calls {
                    tokens += Self::estimate_tokens_for_text(&tc.name)
                        + Self::estimate_tokens_for_text(&tc.arguments);
                }
                tokens
            })
            .sum()
    }

    /// Determine the compaction trigger and retain thresholds for a model.
    ///
    /// Returns `(trigger_tokens, retain_tokens)`.
    pub fn compaction_budget(
        &self,
        context_window: usize,
        max_output_tokens: usize,
    ) -> (usize, usize) {
        if context_window == 0 {
            return (self.prune_threshold_tokens, self.retain_recent_tokens);
        }
        let reserved = max_output_tokens
            .max(context_window / 8)
            .max(4000)
            .clamp(1, context_window - 1);
        let trigger = context_window.saturating_sub(reserved);
        let retain = self.retain_recent_tokens.max(reserved).clamp(1, trigger);
        (trigger, retain)
    }

    // -----------------------------------------------------------------------
    // Compaction decision
    // -----------------------------------------------------------------------

    /// Returns `true` if the conversation is large enough to warrant compaction.
    pub fn needs_compaction(
        &self,
        buffer: &MessageBuffer,
        context_window: usize,
        max_output_tokens: usize,
    ) -> bool {
        let messages = buffer.load();
        let visible: Vec<&Message> = messages
            .iter()
            .skip(self.retained_from)
            .filter(|m| !m.streaming)
            .collect();

        // Prefer token counts from the provider when available.
        let last_tokens = visible
            .iter()
            .rev()
            .find_map(|m| m.input_tokens.or(m.total_tokens));
        let (trigger_tokens, _) = self.compaction_budget(context_window, max_output_tokens);

        match last_tokens {
            Some(tokens) => tokens as usize >= trigger_tokens,
            None => {
                let owned: Vec<Message> = visible.iter().copied().cloned().collect();
                Self::estimate_tokens_for_messages(&owned) >= trigger_tokens
            }
        }
    }

    // -----------------------------------------------------------------------
    // Compaction
    // -----------------------------------------------------------------------

    /// Perform context compaction.
    ///
    /// Builds the request message list (reusing [`build_request_messages`]
    /// for prefix-cache compatibility), appends a summary instruction as a
    /// User message, and calls the LLM. The returned
    /// [`CompactionResult`] must be applied by the caller: update
    /// `self.summary` / `self.retained_from`, persist to the DB, and
    /// update the message buffer.
    pub async fn compact(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[ToolDefinition],
        messages: &[Message],
        _session_id: Uuid,
        event_tx: Option<crate::AgentEventSender>,
    ) -> Result<CompactionResult> {
        // 1. Build prefix (same logic as build_request_messages -> prefix cache hit).
        let mut compact_msgs = Vec::new();
        if let Some(summary) = &self.summary {
            compact_msgs.push(Message::new(
                MessageRole::User,
                format!("Earlier conversation summary:\n{summary}"),
            ));
        }
        compact_msgs.extend(self.build_request_messages_raw(messages));

        // 2. Append summary instruction.
        compact_msgs.push(Message::new(MessageRole::User, SUMMARY_INSTRUCTION));

        // 3. Record how many messages will be covered by this summary.
        let retained_from = messages.len();

        // 4. Copy the protocol tool definitions for the compaction request.
        let llm_tools = tools.to_vec();

        // 5. Call the LLM (streaming or non-streaming).
        let summary = match &event_tx {
            Some(tx) => {
                self.compact_streaming(llm, model, &llm_tools, compact_msgs, tx.clone())
                    .await?
            }
            None => {
                self.compact_non_streaming(llm, model, &llm_tools, compact_msgs)
                    .await?
            }
        };

        // 6. Truncate to configured maximum.
        let summary = summary.chars().take(self.maximum_summary_chars).collect();

        Ok(CompactionResult {
            summary,
            retained_from,
        })
    }

    /// Internal: build request messages from a raw message slice (without summary injection).
    fn build_request_messages_raw(&self, messages: &[Message]) -> Vec<Message> {
        let mut out = Vec::new();
        let mut pending_tool_calls: Vec<(String, String)> = Vec::new();
        for msg in messages.iter().skip(self.retained_from) {
            if msg.streaming {
                continue;
            }
            match msg.role {
                MessageRole::System | MessageRole::Error | MessageRole::Shell => continue,
                MessageRole::User => {
                    Self::drain_pending_tool_calls(&mut out, &mut pending_tool_calls);
                    out.push(msg.clone());
                }
                MessageRole::Assistant => {
                    let mut sanitized = msg.clone();
                    for tc in &mut sanitized.tool_calls {
                        if serde_json::from_str::<serde_json::Value>(&tc.arguments).is_err() {
                            tc.arguments = "{}".to_string();
                        }
                    }
                    if sanitized.content.is_empty() && sanitized.tool_calls.is_empty() {
                        continue;
                    }
                    pending_tool_calls.extend(
                        sanitized
                            .tool_calls
                            .iter()
                            .map(|tc| (tc.id.clone(), tc.name.clone())),
                    );
                    out.push(sanitized);
                }
                MessageRole::Tool => {
                    if let Some(tool_call_id) = &msg.tool_call_id
                        && let Some(index) = pending_tool_calls
                            .iter()
                            .position(|(id, _)| id == tool_call_id)
                    {
                        pending_tool_calls.remove(index);
                        out.push(msg.clone());
                    }
                }
            }
        }
        Self::drain_pending_tool_calls(&mut out, &mut pending_tool_calls);
        out
    }

    /// Apply a compaction result to this manager's state.
    pub fn apply_compaction(&mut self, summary: String, retained_from: usize) {
        self.summary = Some(summary);
        self.retained_from = retained_from;
    }
    async fn compact_non_streaming(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[tidev_llm::ToolDefinition],
        messages: Vec<Message>,
    ) -> Result<String> {
        llm.complete_with_messages(model.clone(), messages, tools.to_vec(), None)
            .await
    }

    async fn compact_streaming(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[tidev_llm::ToolDefinition],
        messages: Vec<Message>,
        event_tx: crate::AgentEventSender,
    ) -> Result<String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let llm_clone = llm.clone();
        let model = model.clone();
        let tools = tools.to_vec();

        let handle = tokio::spawn(async move {
            llm_clone
                .stream_chat(
                    model,
                    messages,
                    tools,
                    tx,
                    tidev_llm::reasoning::ThinkingLevelType::None,
                )
                .await;
        });

        let mut accumulated = String::new();
        while let Some(event) = rx.recv().await {
            match llm_event_to_agent_event(event, 0) {
                AgentEvent::Delta { content, .. } => {
                    accumulated.push_str(&content);
                    // Forward delta to the UI so the user sees progress.
                    let _ = event_tx.send(AgentEvent::Delta {
                        request_id: 0,
                        content,
                    });
                }
                AgentEvent::Finished { .. } => {
                    // Intercepted — not forwarded to the UI because
                    // it would trigger `finish_assistant_turn` logic.
                    break;
                }
                AgentEvent::Failed { error, .. } => {
                    return Err(anyhow::anyhow!("Compaction LLM call failed: {error}"));
                }
                _ => {}
            }
        }

        // Ensure the spawned task is done (or abort it on panic).
        if handle.is_finished() {
            handle.await.ok();
        }

        Ok(accumulated)
    }

    // -----------------------------------------------------------------------
    // Message construction
    // -----------------------------------------------------------------------

    /// Build the message list sent to the LLM for the next turn.
    ///
    /// - Skips messages before `retained_from` (they are covered by the summary).
    /// - Prepends the existing summary (if any) as a User message.
    /// - Filters out System, Error, and Shell messages.
    /// - Validates and sanitizes assistant tool_call arguments.
    /// - Tracks tool_call / tool_result pairing, injecting synthetic failures
    ///   for orphaned tool_calls.
    pub fn build_request_messages(&self, buffer: &MessageBuffer) -> Vec<Message> {
        let messages = buffer.load();
        let mut out = Vec::new();

        // 1. Inject the summary as a User message (if any).
        if let Some(summary) = &self.summary {
            out.push(Message::new(
                MessageRole::User,
                format!("Earlier conversation summary:\n{summary}"),
            ));
        }

        // 2. Append remaining visible messages.
        out.extend(self.build_request_messages_raw(messages));
        out
    }

    /// Inject synthetic failure tool results for orphaned tool_calls.
    fn drain_pending_tool_calls(out: &mut Vec<Message>, pending: &mut Vec<(String, String)>) {
        if pending.is_empty() {
            return;
        }
        let orphans = std::mem::take(pending);
        for (tool_call_id, tool_name) in orphans {
            out.push(Message::tool_result(
                &tool_call_id,
                &tool_name,
                tidev_llm::message::ToolExecutionResult::new(
                    "[Tool result was not captured before context compaction. \
                     The tool may need to be re-run if still relevant.]"
                        .to_string(),
                ),
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tidev_llm::message::{Message, MessageRole, ToolCall, ToolExecutionResult};
    use uuid::Uuid;

    fn user_msg(content: &str) -> Message {
        Message::new(MessageRole::User, content)
    }

    fn assistant_msg(content: &str) -> Message {
        Message::new(MessageRole::Assistant, content)
    }

    fn assistant_with_tool_calls(tool_calls: Vec<ToolCall>) -> Message {
        let mut m = Message::new(MessageRole::Assistant, "thinking...");
        m.tool_calls = tool_calls;
        m
    }

    fn tool_result_msg(tool_call_id: &str, tool_name: &str, output: &str) -> Message {
        Message::tool_result(tool_call_id, tool_name, ToolExecutionResult::new(output))
    }

    #[test]
    fn build_request_messages_empty_buffer() {
        let cm = ContextManager::new();
        let buf = MessageBuffer::new(vec![]);
        let result = cm.build_request_messages(&buf);
        assert!(result.is_empty());
    }

    #[test]
    fn build_request_messages_skips_streaming() {
        let mut streaming = Message::streaming(MessageRole::Assistant, "in progress");
        streaming.id = Uuid::new_v4();
        let done = assistant_msg("done");
        let buf = MessageBuffer::new(vec![streaming, done]);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "done");
    }

    #[test]
    fn build_request_messages_skips_system_error_shell() {
        let msgs = vec![
            Message::new(MessageRole::System, "system prompt"),
            Message::new(MessageRole::Error, "error msg"),
            Message::new(MessageRole::Shell, "shell output"),
            user_msg("hello"),
        ];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, MessageRole::User);
    }

    #[test]
    fn build_request_messages_injects_summary() {
        let cm = ContextManager::from_state(Some("previous summary".into()), 2);
        let buf = MessageBuffer::new(vec![user_msg("msg1"), user_msg("msg2"), user_msg("msg3")]);
        let result = cm.build_request_messages(&buf);
        // First message is the summary injection
        assert_eq!(result.len(), 2); // summary + msg3 (since retained_from=2)
        assert_eq!(result[0].role, MessageRole::User);
        assert!(result[0].content.contains("previous summary"));
    }

    #[test]
    fn build_request_messages_skips_before_retained_from() {
        let msgs = vec![user_msg("old1"), user_msg("old2"), user_msg("current")];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::from_state(None, 2);
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "current");
    }

    #[test]
    fn build_request_messages_drains_pending_on_user_message() {
        // If there are pending tool_calls when a User message arrives, they
        // should be drained first.
        let tc = ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let msgs = vec![assistant_with_tool_calls(vec![tc]), user_msg("never mind")];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        // Should have: assistant msg, synthetic tool_result, user msg
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, MessageRole::Assistant);
        assert_eq!(result[1].role, MessageRole::Tool);
        assert_eq!(result[1].tool_name.as_deref(), Some("read"));
        assert!(result[1].content.contains("not captured"));
        assert_eq!(result[2].role, MessageRole::User);
    }

    #[test]
    fn build_request_messages_pairs_tool_call_with_result() {
        let tc = ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let msgs = vec![
            assistant_with_tool_calls(vec![tc]),
            tool_result_msg("call_1", "read", "file content"),
            user_msg("thanks"),
        ];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        // assistant, tool_result, user — all three present
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, MessageRole::Assistant);
        assert_eq!(result[1].role, MessageRole::Tool);
        assert_eq!(result[1].content, "file content");
        assert_eq!(result[2].role, MessageRole::User);
    }

    #[test]
    fn build_request_messages_skips_orphan_tool_result() {
        // A tool result without a matching pending tool_call should be dropped.
        let msgs = vec![
            tool_result_msg("call_ghost", "read", "should not appear"),
            user_msg("hi"),
        ];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "hi");
    }

    #[test]
    fn build_request_messages_skips_assistant_with_no_content_and_no_tool_calls() {
        let empty = Message::new(MessageRole::Assistant, "");
        let buf = MessageBuffer::new(vec![empty, user_msg("hello")]);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "hello");
    }

    #[test]
    fn build_request_messages_sanitizes_invalid_tool_call_arguments() {
        let tc = ToolCall {
            id: "bad".into(),
            name: "read".into(),
            arguments: "not valid json".into(),
            thought_signature: None,
        };
        let msgs = vec![assistant_with_tool_calls(vec![tc])];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        // assistant msg + synthetic tool_result for orphaned call
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].tool_calls[0].arguments, "{}");
        assert_eq!(result[1].role, MessageRole::Tool);
    }

    #[test]
    fn build_request_messages_preserves_valid_tool_call_arguments() {
        let tc = ToolCall {
            id: "good".into(),
            name: "read".into(),
            arguments: r#"{"file_path":"/tmp/x"}"#.into(),
            thought_signature: None,
        };
        let msgs = vec![assistant_with_tool_calls(vec![tc])];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(
            result[0].tool_calls[0].arguments,
            r#"{"file_path":"/tmp/x"}"#
        );
    }

    #[test]
    fn build_request_messages_drains_pending_at_end() {
        let tc = ToolCall {
            id: "orphan".into(),
            name: "shell".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let msgs = vec![assistant_with_tool_calls(vec![tc])];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, MessageRole::Assistant);
        assert_eq!(result[1].role, MessageRole::Tool);
        assert_eq!(result[1].tool_name.as_deref(), Some("shell"));
    }

    #[test]
    fn build_request_messages_preserves_multiple_orphan_order() {
        let calls = ["first", "second", "third"]
            .into_iter()
            .map(|id| ToolCall {
                id: id.into(),
                name: format!("tool-{id}"),
                arguments: "{}".into(),
                thought_signature: None,
            })
            .collect();
        let buf = MessageBuffer::new(vec![assistant_with_tool_calls(calls)]);
        let result = ContextManager::new().build_request_messages(&buf);

        let result_ids: Vec<&str> = result[1..]
            .iter()
            .map(|message| message.tool_call_id.as_deref().unwrap())
            .collect();
        assert_eq!(result_ids, ["first", "second", "third"]);
        let result_names: Vec<&str> = result[1..]
            .iter()
            .map(|message| message.tool_name.as_deref().unwrap())
            .collect();
        assert_eq!(result_names, ["tool-first", "tool-second", "tool-third"]);
    }

    #[test]
    fn build_request_messages_global_ordering() {
        // Complex scenario with summary, retained_from, and interleaved calls.
        let tc1 = ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            thought_signature: None,
        };
        let msgs = vec![
            user_msg("first"),
            assistant_with_tool_calls(vec![tc1]),
            tool_result_msg("c1", "read", "data"),
            user_msg("second"),
        ];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::from_state(Some("sum".into()), 0);
        let result = cm.build_request_messages(&buf);
        assert_eq!(result.len(), 5);
        // summary injection, user, assistant, tool_result, user
        assert_eq!(result[0].content, "Earlier conversation summary:\nsum");
        assert_eq!(result[1].content, "first");
        assert_eq!(result[2].role, MessageRole::Assistant);
        assert_eq!(result[3].role, MessageRole::Tool);
        assert_eq!(result[4].content, "second");
    }

    // ── compaction_budget ─────────────────────────────────────────────────

    #[test]
    fn compaction_budget_zero_context_uses_fallback() {
        let cm = ContextManager::new();
        let (trigger, retain) = cm.compaction_budget(0, 0);
        assert_eq!(trigger, cm.prune_threshold_tokens);
        assert_eq!(retain, cm.retain_recent_tokens);
    }

    #[test]
    fn compaction_budget_respects_max_output() {
        let cm = ContextManager::new();
        // context_window=100000, max_output=8000
        // reserved = max(8000, 12500, 4000) = 12500
        // trigger = 100000 - 12500 = 87500
        // retain = max(12000, 12500) = 12500
        let (trigger, retain) = cm.compaction_budget(100_000, 8_000);
        // reserved = max(8000, 100000/8=12500, 4000) = 12500
        assert_eq!(trigger, 100_000 - 12_500);
        assert_eq!(retain, 12_500);
    }

    #[test]
    fn compaction_budget_large_reserved_triggers_at_least_1() {
        // When context_window is very small, trigger should be at least 1
        let cm = ContextManager::new();
        let (trigger, retain) = cm.compaction_budget(10_000, 9_999);
        // reserved = max(9999, 1250, 4000) = 9999
        // trigger = 10000 - 9999 = 1
        assert_eq!(trigger, 1);
        // retain = max(12000, 9999) clamped to trigger=1 => 1
        assert_eq!(retain, 1);
    }

    // ── estimate_tokens_for_messages ──────────────────────────────────────

    #[test]
    fn estimate_tokens_for_messages_sums_content_and_reasoning() {
        let mut m1 = Message::new(MessageRole::User, "hello world"); // 11 / 4 = 2
        m1.reasoning = "think".into(); // 5 / 4 = 1
        let m2 = Message::new(MessageRole::Assistant, "a".repeat(40)); // 40 / 4 = 10
        let buf = MessageBuffer::new(vec![m1, m2]);
        let tokens = ContextManager::estimate_tokens_for_messages(buf.load());
        assert_eq!(tokens, 2 + 1 + 10);
    }
}
