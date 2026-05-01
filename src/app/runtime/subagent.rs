use anyhow::{Context, Result, bail};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{
    runtime::Handle,
    sync::mpsc::{UnboundedSender, unbounded_channel},
};
use uuid::Uuid;

use crate::{
    agent::AgentDefinition,
    config::ActiveModel,
    llm::LlmClient,
    prompts::SessionMode,
    session::{AssistantTurn, BackendEvent, Message, MessageRole, ToolCall, ToolExecutionResult},
    storage::SessionStore,
    tooling::{ToolRegistry, canonical_tool_name, execute_shell_tool_call},
};

#[derive(Clone, Debug)]
pub(crate) struct SubagentTaskContext {
    pub parent_request_id: u64,
    pub parent_session_id: Uuid,
    pub child_session_id: Uuid,
    pub description: String,
    pub prompt: String,
    pub agent_definition: AgentDefinition,
    pub llm: LlmClient,
    pub tools: ToolRegistry,
    pub model: ActiveModel,
    pub workspace_root: PathBuf,
    pub store_path: PathBuf,
    pub tx: UnboundedSender<BackendEvent>,
    pub cancel_requested: Arc<AtomicBool>,
    pub runtime_handle: Handle,
}

pub(crate) async fn run_subagent_task(context: SubagentTaskContext) -> Result<String> {
    prepare_child_session(&context)?;

    let result = run_subagent_loop(&context).await;
    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            mark_child_session_failed(&context, error.to_string())?;
            Err(error)
        }
    }
}

fn prepare_child_session(context: &SubagentTaskContext) -> Result<()> {
    let store = SessionStore::open(&context.store_path)?;
    let parent_record = store
        .load_session_record(context.parent_session_id)?
        .context("parent session not found")?;

    // Use agent type in title for better session identification
    let agent_label = context.agent_definition.agent_type.display_name();
    let child_title = format!("Task ({agent_label}): {}", context.description);

    // Determine model — use the agent's model override if set
    let model = context
        .agent_definition
        .model_override
        .as_ref()
        .unwrap_or(&context.model);

    store.create_session_with_parent(
        context.child_session_id,
        context.parent_session_id,
        &context.workspace_root,
        &parent_record.provider_id,
        &parent_record.provider_display_name,
        &model.model_id,
        &model.display_name,
        &child_title,
    )?;

    store.copy_tool_permissions(context.parent_session_id, context.child_session_id)?;

    let bootstrap_message = Message::new(
        MessageRole::System,
        context.agent_definition.bootstrap_content(),
    );
    store.append_message(context.child_session_id, &bootstrap_message)?;

    let user_message = Message::new(MessageRole::User, context.prompt.clone());
    store.append_message(context.child_session_id, &user_message)?;
    store.update_session_title(context.child_session_id, &child_title)?;

    Ok(())
}

async fn run_subagent_loop(context: &SubagentTaskContext) -> Result<String> {
    let mut request_sequence = 0u64;

    let output = loop {
        if context.cancel_requested.load(Ordering::SeqCst) {
            bail!("subagent cancelled");
        }

        request_sequence = request_sequence.wrapping_add(1);
        send_status(context, "Thinking", None, None, None, None);

        let mut assistant_message = Message::streaming(MessageRole::Assistant, "");
        {
            let store = SessionStore::open(&context.store_path)?;
            store.append_message(context.child_session_id, &assistant_message)?;
        }

        let messages = {
            let store = SessionStore::open(&context.store_path)?;
            store.load_messages(context.child_session_id)?
        };
        let tools = if let Some(allowed_tools) = &context.agent_definition.allowed_tools {
            // Filter tools based on agent definition
            context
                .tools
                .definitions()
                .iter()
                .filter(|definition| {
                    allowed_tools.contains(&definition.name)
                        || matches!(canonical_tool_name(&definition.name), Some("question"))
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            // No restrictions: use all available tools
            context.tools.definitions().to_vec()
        };
        let (stream_tx, mut stream_rx) = unbounded_channel();
        let llm = context.llm.clone();
        let model = context.model.clone();
        let stream_request_id = request_sequence;
        let stream_session_id = context.child_session_id;
        let thinking_level = model.thinking_level.clone();
        let stream_handle = tokio::spawn(async move {
            llm.stream_chat(
                stream_session_id,
                stream_request_id,
                model,
                messages,
                tools,
                stream_tx,
                thinking_level,
            )
            .await;
        });

        let mut turn = AssistantTurn::default();
        let mut finished = false;
        let mut last_sent_content_len: usize = 0;
        let mut last_sent_reasoning_len: usize = 0;

        while let Some(event) = stream_rx.recv().await {
            match event {
                BackendEvent::Delta { content, .. } => {
                    assistant_message.content.push_str(&content);
                    update_child_message(context, &assistant_message)?;
                    send_status_with_delta(
                        context,
                        "Writing output",
                        None,
                        &assistant_message,
                        &mut last_sent_content_len,
                        &mut last_sent_reasoning_len,
                    );
                }
                BackendEvent::ToolCallUpdated { tool_call, .. } => {
                    assistant_message.upsert_tool_call(tool_call.clone());
                    update_child_message(context, &assistant_message)?;
                    send_status(
                        context,
                        "Tool",
                        Some(tool_call),
                        Some(&assistant_message),
                        None,
                        None,
                    );
                }
                BackendEvent::ReasoningDelta { content, .. } => {
                    assistant_message.reasoning.push_str(&content);
                    update_child_message(context, &assistant_message)?;
                    send_status_with_delta(
                        context,
                        "Thinking",
                        None,
                        &assistant_message,
                        &mut last_sent_content_len,
                        &mut last_sent_reasoning_len,
                    );
                }
                BackendEvent::Retrying {
                    attempt,
                    max_attempts,
                    reason,
                    retry_after_secs,
                    ..
                } => {
                    let retry = retry_after_secs
                        .map(|seconds| {
                            format!(
                                "Retrying subagent turn {attempt}/{max_attempts} in {seconds}s: {reason}"
                            )
                        })
                        .unwrap_or_else(|| {
                            format!("Retrying subagent turn {attempt}/{max_attempts}: {reason}")
                        });
                    send_status_with_delta(
                        context,
                        retry,
                        None,
                        &assistant_message,
                        &mut last_sent_content_len,
                        &mut last_sent_reasoning_len,
                    );
                }
                BackendEvent::Finished {
                    turn: next_turn, ..
                } => {
                    turn = next_turn;
                    finished = true;
                    break;
                }
                BackendEvent::Failed { error, .. } => {
                    let error_message = format!("Subagent failed: {error}");
                    assistant_message.role = MessageRole::Error;
                    assistant_message.content = error_message.clone();
                    assistant_message.reasoning.clear();
                    assistant_message.tool_calls.clear();
                    assistant_message.streaming = false;
                    update_child_message(context, &assistant_message)?;
                    let _ = stream_handle.await;
                    return Err(anyhow::anyhow!(error_message));
                }
                BackendEvent::UsageStats {
                    input_tokens,
                    output_tokens,
                    total_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    model_id,
                    duration_ms,
                    ..
                } => {
                    let _ = context.tx.send(BackendEvent::UsageStats {
                        session_id: context.child_session_id,
                        request_id: context.parent_request_id,
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        model_id: model_id.clone(),
                        duration_ms,
                    });
                    let _ = context.tx.send(BackendEvent::UsageStats {
                        session_id: context.parent_session_id,
                        request_id: context.parent_request_id,
                        input_tokens,
                        output_tokens,
                        total_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        model_id,
                        duration_ms,
                    });
                }
                BackendEvent::ToolCompleted { .. } => {}
                BackendEvent::InstructionsLoaded { .. } => {}
                BackendEvent::SubagentStatus { .. } => {}
                BackendEvent::SubagentToolResult { .. } => {}
                BackendEvent::SubagentCompleted { .. } => {}
                BackendEvent::ContextCompacted { .. } => {}
                BackendEvent::SidebarSnapshotReady { .. } => {}
                BackendEvent::ShellOutput { .. } => {}
            }
        }

        let _ = stream_handle.await;

        if !finished {
            let error_message = "Subagent stream ended without a final turn".to_string();
            assistant_message.role = MessageRole::Error;
            assistant_message.content = error_message.clone();
            assistant_message.reasoning.clear();
            assistant_message.tool_calls.clear();
            assistant_message.streaming = false;
            update_child_message(context, &assistant_message)?;
            bail!(error_message);
        }

        assistant_message.content = turn.content.clone();
        assistant_message.reasoning = turn.reasoning.clone();
        assistant_message.tool_calls = turn.tool_calls.clone();
        assistant_message.streaming = false;
        update_child_message(context, &assistant_message)?;

        if turn.tool_calls.is_empty() {
            send_status(
                context,
                "Completed",
                None,
                Some(&assistant_message),
                None,
                None,
            );
            break turn.content;
        }

        for tool_call in turn.tool_calls {
            if context.cancel_requested.load(Ordering::SeqCst) {
                bail!("subagent cancelled");
            }

            let summary = summarize_tool_call(&tool_call.name, &tool_call.arguments, 64);
            send_status(
                context,
                format!("Tool: {summary}"),
                Some(tool_call.clone()),
                Some(&assistant_message),
                None,
                None,
            );

            let result = execute_child_tool_call(context, &tool_call)
                .await
                .unwrap_or_else(|error| ToolExecutionResult::new(format!("Tool failed: {error}")));

            record_tool_result(context, &tool_call, &result)?;
            send_status(
                context,
                "Working",
                None,
                Some(&assistant_message),
                None,
                None,
            );
        }
    };

    let output = if output.is_empty() {
        read_last_assistant_text(context)?
    } else {
        output
    };

    Ok(output)
}

async fn execute_child_tool_call(
    context: &SubagentTaskContext,
    tool_call: &ToolCall,
) -> Result<ToolExecutionResult> {
    match canonical_tool_name(&tool_call.name) {
        Some("bash") => {
            let result = execute_shell_tool_call(
                &context.workspace_root,
                tool_call,
                context.tools.max_output_bytes(),
                context.tools.rtk_enabled(),
                context.cancel_requested.clone(),
            )?;
            Ok(result)
        }
        _ => {
            let result = tokio::task::block_in_place(|| {
                let store = SessionStore::open(&context.store_path)?;
                context.tools.execute_call(
                    &context.runtime_handle,
                    &store,
                    context.child_session_id,
                    tool_call,
                    SessionMode::Build,
                    false, // allow_outside: subagent execution doesn't allow outside workspace
                )
            })?;
            Ok(result)
        }
    }
}

fn update_child_message(context: &SubagentTaskContext, message: &Message) -> Result<()> {
    let store = SessionStore::open(&context.store_path)?;
    store.update_message(context.child_session_id, message)
}

fn record_tool_result(
    context: &SubagentTaskContext,
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
) -> Result<()> {
    let store = SessionStore::open(&context.store_path)?;
    let display_result = result.preview_for_storage(Some(tool_call.name.as_str()));
    let message =
        Message::tool_result(tool_call.id.clone(), tool_call.name.clone(), display_result);

    store.append_tool_event(
        context.child_session_id,
        message.id,
        &tool_call.name,
        &tool_call.arguments,
        &result.output,
    )?;

    store.append_message(context.child_session_id, &message)?;

    let _ = context.tx.send(BackendEvent::SubagentToolResult {
        session_id: context.child_session_id,
        request_id: context.parent_request_id,
        child_session_id: context.child_session_id,
        message: message.clone(),
    });

    Ok(())
}

fn send_status(
    context: &SubagentTaskContext,
    status_text: impl Into<String>,
    current_tool_call: Option<ToolCall>,
    assistant_message: Option<&Message>,
    content_delta: Option<String>,
    reasoning_delta: Option<String>,
) {
    let status_text = status_text.into();
    let _ = context.tx.send(BackendEvent::SubagentStatus {
        session_id: context.child_session_id,
        request_id: context.parent_request_id,
        child_session_id: context.child_session_id,
        status_text: status_text.clone(),
        current_tool_call: current_tool_call.clone(),
        assistant_message: assistant_message.cloned(),
        content_delta: content_delta.clone(),
        reasoning_delta: reasoning_delta.clone(),
    });

    let _ = context.tx.send(BackendEvent::SubagentStatus {
        session_id: context.parent_session_id,
        request_id: context.parent_request_id,
        child_session_id: context.child_session_id,
        status_text,
        current_tool_call,
        assistant_message: assistant_message.cloned(),
        content_delta,
        reasoning_delta,
    });
}

fn send_status_with_delta(
    context: &SubagentTaskContext,
    status_text: impl Into<String>,
    current_tool_call: Option<ToolCall>,
    assistant_message: &Message,
    last_sent_content_len: &mut usize,
    last_sent_reasoning_len: &mut usize,
) {
    let content_delta = if assistant_message.content.len() > *last_sent_content_len {
        let delta = assistant_message.content[*last_sent_content_len..].to_string();
        *last_sent_content_len = assistant_message.content.len();
        Some(delta)
    } else {
        None
    };

    let reasoning_delta = if assistant_message.reasoning.len() > *last_sent_reasoning_len {
        let delta = assistant_message.reasoning[*last_sent_reasoning_len..].to_string();
        *last_sent_reasoning_len = assistant_message.reasoning.len();
        Some(delta)
    } else {
        None
    };

    send_status(
        context,
        status_text,
        current_tool_call,
        Some(assistant_message),
        content_delta,
        reasoning_delta,
    );
}

fn mark_child_session_failed(context: &SubagentTaskContext, error: String) -> Result<()> {
    let store = SessionStore::open(&context.store_path)?;
    let mut messages = store.load_messages(context.child_session_id)?;

    if let Some(message) = messages
        .iter_mut()
        .rev()
        .find(|message| message.streaming && matches!(message.role, MessageRole::Assistant))
    {
        message.role = MessageRole::Error;
        message.content = format!("Subagent failed: {error}");
        message.reasoning.clear();
        message.tool_calls.clear();
        message.streaming = false;
        store.update_message(context.child_session_id, message)?;
    } else {
        let message = Message::new(MessageRole::Error, format!("Subagent failed: {error}"));
        store.append_message(context.child_session_id, &message)?;
    }

    Ok(())
}

fn read_last_assistant_text(context: &SubagentTaskContext) -> Result<String> {
    let store = SessionStore::open(&context.store_path)?;
    let messages = store.load_messages(context.child_session_id)?;
    Ok(messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::Assistant))
        .map(|message| message.content.clone())
        .unwrap_or_default())
}

fn summarize_tool_call(tool_name: &str, arguments: &str, body_width: usize) -> String {
    let canonical_name = canonical_tool_name(tool_name).unwrap_or(tool_name);
    let parsed = serde_json::from_str::<serde_json::Value>(arguments).ok();
    let field = |name: &str| {
        parsed
            .as_ref()
            .and_then(|value| value.get(name))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };

    let summary = match canonical_name {
        "read" => field("path"),
        "write" => field("path"),
        "edit" => field("path"),
        "list" => field("path"),
        "glob" => format!("{} in {}", field("pattern"), field("path")),
        "grep" => format!("{} in {}", field("pattern"), field("path")),
        "bash" => field("command"),
        "task" => field("description"),
        _ => tool_name.to_string(),
    };

    shorten_single_line(&summary, body_width)
}

fn shorten_single_line(value: &str, max_chars: usize) -> String {
    let value = value.replace('\n', " ").replace('\r', "");
    if value.chars().count() <= max_chars {
        return value;
    }

    let mut shortened = value.chars().take(max_chars).collect::<String>();
    shortened.push_str("...");
    shortened
}
