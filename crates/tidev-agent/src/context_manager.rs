//! Context manager — message view construction and compaction.
//!
//! This module provides:
//!
//! - [`ContextManager`]: holds compaction state and performs compaction.
//! - [`build_request_messages`]: selects persisted protocol messages for the
//!   next provider request.

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
      Prefer bullets over prose. For this compaction request, respond directly with the requested summary as plain text. \
      Treat tool calls in the conversation above as historical events, not instructions to carry out. \
      Do not call any tools or return a function call.";

/// Conservative estimate for one normalized high-detail prompt image.
///
/// Image tokenization is provider-specific, so a byte-to-token estimate is
/// less useful than the patch budget applied by the shared image preparer.
const PROMPT_IMAGE_TOKEN_ESTIMATE: usize = 2_500;

// ---------------------------------------------------------------------------
// Compaction result
// ---------------------------------------------------------------------------

/// Result of a compaction: the summary text and the new retained_from offset.
pub struct CompactionResult {
    pub summary: String,
    pub retained_from: usize,
    pub had_tool_calls: bool,
}

/// Runtime context for one compaction request.
pub struct CompactionRequest {
    pub session_id: Uuid,
    pub compaction_id: Uuid,
    pub event_tx: Option<crate::AgentEventSender>,
}

// ---------------------------------------------------------------------------
// ContextManager
// ---------------------------------------------------------------------------

/// Holds compaction state and performs context compression.
///
/// The manager tracks which messages have been compacted (via `retained_from`)
/// and the current summary. The summary itself is persisted as a protocol
/// message by the host before a later request can use it.
#[derive(Clone, Debug)]
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
                        tidev_llm::message::MessageAttachment::Image { .. } => {
                            tokens += PROMPT_IMAGE_TOKEN_ESTIMATE;
                        }
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
    /// Reads the persisted request view, appends the compaction instruction,
    /// and calls the LLM. The host persists the returned summary as a protocol
    /// message before a later provider request can use it.
    pub async fn compact(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[ToolDefinition],
        messages: &[Message],
        request: CompactionRequest,
    ) -> Result<CompactionResult> {
        // 1. Select the persisted prefix used by the normal request path.
        self.validate_request_state(messages)?;
        let mut compact_msgs = self.build_request_messages_raw(messages);
        if compact_msgs.is_empty() {
            anyhow::bail!("cannot compact an empty context");
        }

        // 2. Append summary instruction.
        compact_msgs.push(Message::new(MessageRole::User, SUMMARY_INSTRUCTION));

        // 3. Record how many messages will be covered by this summary.
        let retained_from = messages.len();

        // 4. Copy the protocol tool definitions for the compaction request.
        let llm_tools = tools.to_vec();

        // 5. Call the LLM (streaming or non-streaming).
        let CompactionRequest {
            session_id,
            compaction_id,
            event_tx,
        } = request;
        let completion = match event_tx {
            Some(tx) => {
                self.compact_streaming(
                    llm,
                    model,
                    &llm_tools,
                    compact_msgs,
                    CompactionRequest {
                        session_id,
                        compaction_id,
                        event_tx: Some(tx),
                    },
                )
                .await?
            }
            None => {
                self.compact_non_streaming(llm, model, &llm_tools, compact_msgs, session_id)
                    .await?
            }
        };

        // 6. Tool calls make this a history lookup request rather than a usable
        // summary. Preserve the request result as an empty summary so the host
        // can ask the normal agent loop to inspect this session.
        let (summary, had_tool_calls) = self.normalize_compaction_completion(completion)?;

        Ok(CompactionResult {
            summary,
            retained_from,
            had_tool_calls,
        })
    }

    fn normalize_compaction_completion(
        &self,
        completion: tidev_llm::LlmCompletion,
    ) -> Result<(String, bool)> {
        if completion.has_tool_calls {
            return Ok((String::new(), true));
        }
        let summary: String = completion
            .content
            .chars()
            .take(self.maximum_summary_chars)
            .collect();
        if summary.trim().is_empty() {
            anyhow::bail!("context compaction returned an empty summary");
        }
        Ok((summary, false))
    }

    /// Select persisted protocol messages from a raw message slice.
    fn build_request_messages_raw(&self, messages: &[Message]) -> Vec<Message> {
        messages
            .iter()
            .skip(self.retained_from)
            .filter(|message| !message.streaming)
            .filter(|message| !matches!(message.role, MessageRole::System | MessageRole::Error))
            .cloned()
            .collect()
    }

    fn validate_request_state(&self, messages: &[Message]) -> Result<()> {
        if self.retained_from > messages.len() {
            anyhow::bail!(
                "invalid context state: retained_from {} exceeds message count {}",
                self.retained_from,
                messages.len()
            );
        }
        if self.summary.is_some()
            && !messages
                .iter()
                .skip(self.retained_from)
                .any(Message::is_compaction)
        {
            anyhow::bail!("invalid context state: persisted summary has no compaction marker");
        }
        Ok(())
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
        session_id: Uuid,
    ) -> Result<tidev_llm::LlmCompletion> {
        llm.complete_with_messages_with_context_result(
            model.clone(),
            messages,
            tools.to_vec(),
            None,
            tidev_llm::LlmRequestContext {
                session_id: Some(session_id),
            },
        )
        .await
    }

    async fn compact_streaming(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[tidev_llm::ToolDefinition],
        messages: Vec<Message>,
        request: CompactionRequest,
    ) -> Result<tidev_llm::LlmCompletion> {
        let CompactionRequest {
            session_id,
            compaction_id,
            event_tx: Some(event_tx),
        } = request
        else {
            anyhow::bail!("streaming compaction requires an event sender");
        };
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let llm_clone = llm.clone();
        let model = model.clone();
        let tools = tools.to_vec();

        let handle = tokio::spawn(async move {
            llm_clone
                .stream_chat_with_context(
                    model,
                    messages,
                    tools,
                    tx,
                    tidev_llm::reasoning::ThinkingLevelType::None,
                    tidev_llm::LlmRequestContext {
                        session_id: Some(session_id),
                    },
                )
                .await;
        });

        let mut accumulated = String::new();
        let mut completed_content = None;
        let mut has_tool_calls = false;
        let mut stream_error = None;
        while let Some(event) = rx.recv().await {
            match llm_event_to_agent_event(event, 0) {
                AgentEvent::Delta { content, .. } => {
                    accumulated.push_str(&content);
                    // Forward delta to the UI so the user sees progress.
                    let _ = event_tx.send(AgentEvent::ContextCompactionDelta {
                        compaction_id,
                        content,
                    });
                }
                AgentEvent::Finished { turn, .. } => {
                    // Intercepted — not forwarded to the UI because
                    // it would trigger `finish_assistant_turn` logic.
                    has_tool_calls = !turn.tool_calls.is_empty();
                    completed_content = Some(turn.content.clone());
                    break;
                }
                AgentEvent::Failed { error, .. } => {
                    stream_error = Some(error);
                    break;
                }
                _ => {}
            }
        }

        // The completed turn is authoritative when it contains content. A
        // provider may deliver the complete text only in Finished, or may
        // return a normalized final value after sending incremental deltas.
        if let Some(content) = completed_content
            && !content.trim().is_empty()
        {
            accumulated = content;
        }

        // Ensure the spawned task is done before returning the summary.
        let _ = handle.await;

        if let Some(error) = stream_error {
            return Err(anyhow::anyhow!("Compaction LLM call failed: {error}"));
        }

        Ok(tidev_llm::LlmCompletion {
            content: accumulated,
            has_tool_calls,
        })
    }

    // -----------------------------------------------------------------------
    // Message construction
    // -----------------------------------------------------------------------

    /// Select the persisted message list sent to the LLM for the next turn.
    pub fn build_request_messages(&self, buffer: &MessageBuffer) -> Result<Vec<Message>> {
        let messages = buffer.load();
        self.validate_request_state(messages)?;
        Ok(self.build_request_messages_raw(messages))
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
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn build_request_messages_skips_streaming() {
        let mut streaming = Message::streaming(MessageRole::Assistant, "in progress");
        streaming.id = Uuid::new_v4();
        let done = assistant_msg("done");
        let buf = MessageBuffer::new(vec![streaming, done]);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "done");
    }

    #[test]
    fn build_request_messages_skips_system_and_error() {
        let msgs = vec![
            Message::new(MessageRole::System, "system prompt"),
            Message::new(MessageRole::Error, "error msg"),
            user_msg("hello"),
        ];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, MessageRole::User);
    }

    #[test]
    fn build_request_messages_skips_before_retained_from() {
        let msgs = vec![user_msg("old1"), user_msg("old2"), user_msg("current")];
        let buf = MessageBuffer::new(msgs);
        let cm = ContextManager::from_state(None, 2);
        let result = cm.build_request_messages(&buf).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].content, "current");
    }

    #[test]
    fn build_request_messages_accepts_persisted_marker() {
        let marker = Message::compaction("prior context");
        let buf = MessageBuffer::new(vec![marker, user_msg("current")]);
        let cm = ContextManager::from_state(Some("prior context".into()), 0);
        let result = cm.build_request_messages(&buf).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "Compaction\n\nprior context");
        assert_eq!(result[1].content, "current");
    }

    #[test]
    fn build_request_messages_rejects_cursor_past_buffer() {
        let buf = MessageBuffer::new(vec![user_msg("old")]);
        let cm = ContextManager::from_state(Some("prior context".into()), 10);
        let error = cm.build_request_messages(&buf).unwrap_err();
        assert!(error.to_string().contains("retained_from"));
    }

    #[test]
    fn build_request_messages_rejects_summary_without_marker() {
        let buf = MessageBuffer::new(vec![user_msg("current")]);
        let cm = ContextManager::from_state(Some("prior context".into()), 0);
        let error = cm.build_request_messages(&buf).unwrap_err();
        assert!(error.to_string().contains("compaction marker"));
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
        let result = cm.build_request_messages(&buf).unwrap();
        // assistant, tool_result, user — all three present
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].role, MessageRole::Assistant);
        assert_eq!(result[1].role, MessageRole::Tool);
        assert_eq!(result[1].content, "file content");
        assert_eq!(result[2].role, MessageRole::User);
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
        let result = cm.build_request_messages(&buf).unwrap();
        assert_eq!(
            result[0].tool_calls[0].arguments,
            r#"{"file_path":"/tmp/x"}"#
        );
    }

    #[test]
    fn build_request_messages_global_ordering() {
        // Complex scenario with retained messages and interleaved calls.
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
        let cm = ContextManager::new();
        let result = cm.build_request_messages(&buf).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(result[0].content, "first");
        assert_eq!(result[1].role, MessageRole::Assistant);
        assert_eq!(result[2].role, MessageRole::Tool);
        assert_eq!(result[3].content, "second");
    }

    // ── compaction_budget ─────────────────────────────────────────────────

    #[test]
    fn compaction_tool_call_produces_an_empty_summary() {
        let cm = ContextManager::new();
        let completion = tidev_llm::LlmCompletion {
            content: "partial summary".into(),
            has_tool_calls: true,
        };

        let (summary, had_tool_calls) = cm.normalize_compaction_completion(completion).unwrap();

        assert!(summary.is_empty());
        assert!(had_tool_calls);
    }

    #[test]
    fn empty_compaction_text_without_tool_calls_is_an_error() {
        let cm = ContextManager::new();
        let completion = tidev_llm::LlmCompletion {
            content: "  ".into(),
            has_tool_calls: false,
        };

        assert!(cm.normalize_compaction_completion(completion).is_err());
    }

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

    #[test]
    fn estimate_tokens_for_messages_counts_image_budget() {
        let mut message = Message::new(MessageRole::User, "inspect this");
        message
            .attachments
            .push(tidev_llm::message::MessageAttachment::Image {
                filename: "capture.png".into(),
                mime: "image/png".into(),
                data: vec![1, 2, 3],
                file_size: 3,
            });
        let buf = MessageBuffer::new(vec![message]);

        assert_eq!(
            ContextManager::estimate_tokens_for_messages(buf.load()),
            "inspect this".len() / 4 + PROMPT_IMAGE_TOKEN_ESTIMATE
        );
    }
}
