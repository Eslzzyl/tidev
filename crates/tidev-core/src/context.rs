//! Context manager — message view construction and compaction.
//!
//! This module provides:
//!
//! - [`ContextManager`]: holds compaction state (summary, retained_from) and
//!   performs compaction by injecting a user message and calling the LLM.
//! - [`build_request_messages`]: builds the message list sent to the LLM,
//!   skipping already-compacted messages and injecting the summary.

use std::collections::HashMap;

use anyhow::Result;
use tidev_types::message::{BackendEvent, Message, MessageRole};
use tidev_types::prompts::SessionMode;
use tidev_types::tools::ToolDefinition;
use uuid::Uuid;

use tidev_llm::{LlmClient, LlmProviderConfig};

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// Convert a `tidev_types::tools::ToolDefinition` to the `tidev_llm` variant.
pub(crate) fn to_llm_tool_def(def: &ToolDefinition) -> tidev_llm::ToolDefinition {
    tidev_llm::ToolDefinition {
        name: def.name.clone(),
        display_name: def.display_name.clone(),
        description: def.description.clone(),
        parameters: def.parameters.clone(),
    }
}

use crate::MessageBuffer;

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
            summary,
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
                        tidev_types::message::MessageAttachment::FileReference {
                            content, ..
                        } => tokens += Self::estimate_tokens_for_text(content),
                        tidev_types::message::MessageAttachment::DirectoryReference {
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
        let retain = self
            .retain_recent_tokens
            .max(reserved)
            .clamp(1, trigger);
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
        let last_tokens = visible.iter().rev().find_map(|m| m.input_tokens.or(m.total_tokens));
        let (trigger_tokens, _) = self.compaction_budget(context_window, max_output_tokens);

        match last_tokens {
            Some(tokens) => return tokens as usize >= trigger_tokens,
            None => {
                let owned: Vec<Message> = visible.iter().copied().cloned().collect();
                return Self::estimate_tokens_for_messages(&owned) >= trigger_tokens;
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
        mode: SessionMode,
        session_id: Uuid,
        event_tx: Option<tokio::sync::mpsc::UnboundedSender<BackendEvent>>,
    ) -> Result<CompactionResult> {
        // 1. Build prefix (same logic as normal request -> prefix cache hit).
        let mut compact_msgs = self.build_request_messages_raw(messages, mode);

        // 2. Append summary instruction.
        compact_msgs.push(Message::new(MessageRole::User, SUMMARY_INSTRUCTION));

        // 3. Record how many messages will be covered by this summary.
        let retained_from = messages.len();

        // 4. Convert tool definitions to tidev-llm format.
        let llm_tools: Vec<tidev_llm::ToolDefinition> = tools.iter().map(to_llm_tool_def).collect();

        // 5. Call the LLM (streaming or non-streaming).
        let summary = if let Some(tx) = event_tx {
            self.compact_streaming(llm, model, &llm_tools, compact_msgs, session_id, tx)
                .await?
        } else {
            self.compact_non_streaming(llm, model, &llm_tools, compact_msgs)
                .await?
        };

        // 6. Truncate to configured maximum.
        let summary = summary.chars().take(self.maximum_summary_chars).collect();

        Ok(CompactionResult {
            summary,
            retained_from,
        })
    }

    /// Internal: build request messages from a raw message slice (without summary injection).
    fn build_request_messages_raw(&self, messages: &[Message], mode: SessionMode) -> Vec<Message> {
        let mut out = Vec::new();
        let mut pending_tool_calls: HashMap<String, String> = HashMap::new();
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
                    for tc in &sanitized.tool_calls {
                        pending_tool_calls.insert(tc.id.clone(), tc.name.clone());
                    }
                    out.push(sanitized);
                }
                MessageRole::Tool => {
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        if pending_tool_calls.remove(tool_call_id).is_some() {
                            out.push(msg.clone());
                        }
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
        llm.complete_with_messages(model.clone(), messages, tools.to_vec())
            .await
    }

    async fn compact_streaming(
        &self,
        llm: &LlmClient,
        model: &LlmProviderConfig,
        tools: &[tidev_llm::ToolDefinition],
        messages: Vec<Message>,
        session_id: Uuid,
        event_tx: tokio::sync::mpsc::UnboundedSender<BackendEvent>,
    ) -> Result<String> {
        use tidev_types::message::BackendEvent;

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let llm_clone = llm.clone();
        let model = model.clone();
        let tools = tools.to_vec();

        let handle = tokio::spawn(async move {
            llm_clone
                .stream_chat(
                    session_id,
                    0,
                    model,
                    messages,
                    tools,
                    tx,
                    tidev_types::reasoning::ThinkingLevelType::None,
                )
                .await;
        });

        let mut accumulated = String::new();
        while let Some(event) = rx.recv().await {
            match event {
                BackendEvent::Delta { content, .. } => {
                    accumulated.push_str(&content);
                    // Forward delta to the UI so the user sees progress.
                    let _ = event_tx.send(BackendEvent::Delta {
                        session_id,
                        request_id: 0,
                        content: content.clone(),
                    });
                }
                BackendEvent::Finished { .. } => {
                    // Intercepted — not forwarded to the UI because
                    // it would trigger `finish_assistant_turn` logic.
                    break;
                }
                BackendEvent::Failed { error, .. } => {
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
    pub fn build_request_messages(
        &self,
        buffer: &MessageBuffer,
        mode: SessionMode,
    ) -> Vec<Message> {
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
        out.extend(self.build_request_messages_raw(&messages, mode));
        out
    }

    /// Inject synthetic failure tool results for orphaned tool_calls.
    fn drain_pending_tool_calls(
        out: &mut Vec<Message>,
        pending: &mut HashMap<String, String>,
    ) {
        if pending.is_empty() {
            return;
        }
        // Collect to avoid borrow issues.
        let orphans: Vec<(String, String)> = pending.drain().collect();
        for (tool_call_id, tool_name) in orphans {
            out.push(Message::tool_result(
                &tool_call_id,
                &tool_name,
                tidev_types::message::ToolExecutionResult::new(
                    "[Tool result was not captured before context compaction. \
                     The tool may need to be re-run if still relevant.]".to_string(),
                ),
            ));
        }
    }
}
