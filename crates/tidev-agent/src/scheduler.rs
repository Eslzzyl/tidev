//! Generic tool-call scheduling for agent runtimes.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use tidev_llm::message::{ToolCall, ToolExecutionResult};

use crate::event::{AgentEvent, AgentEventSender};
use crate::turn::order_tool_results;

/// Host execution boundary used by the generic tool scheduler.
#[async_trait]
pub trait ToolCallExecutor: Send + Sync {
    /// Return whether a call is safe to execute concurrently with other calls.
    fn is_read_only(&self, tool_call: &ToolCall) -> bool;

    /// Execute one already-authorized tool call.
    async fn execute(&self, tool_call: ToolCall) -> Result<ToolExecutionResult>;
}

/// Execute authorized tool calls with generic read/write scheduling.
///
/// Read-only calls are started concurrently. Write calls are executed in their
/// original order. Completion events are emitted as execution completes, while
/// the returned results are restored to the assistant's original call order so
/// the next protocol request remains deterministic.
pub async fn execute_tool_calls(
    executor: Arc<dyn ToolCallExecutor>,
    tool_calls: &[ToolCall],
    event_tx: &AgentEventSender,
    cancel: &CancellationToken,
    request_id: u64,
) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
    let mut results: Vec<Option<(ToolCall, ToolExecutionResult)>> =
        (0..tool_calls.len()).map(|_| None).collect();
    let mut read_only = Vec::new();
    let mut write = Vec::new();

    for (index, tool_call) in tool_calls.iter().cloned().enumerate() {
        if executor.is_read_only(&tool_call) {
            read_only.push((index, tool_call));
        } else {
            write.push((index, tool_call));
        }
    }

    if !read_only.is_empty() {
        if cancel.is_cancelled() {
            for (index, tool_call) in read_only {
                let result = cancelled_result();
                emit_completed(event_tx, request_id, &tool_call, &result);
                results[index] = Some((tool_call, result));
            }
        } else {
            let mut pending = vec![false; tool_calls.len()];
            let mut completion_states: Vec<Option<Arc<AtomicBool>>> =
                (0..tool_calls.len()).map(|_| None).collect();
            let mut tasks = JoinSet::new();
            for (index, tool_call) in read_only {
                pending[index] = true;
                emit_starting(event_tx, request_id, &tool_call);
                let executor = executor.clone();
                let event_tx = event_tx.clone();
                let completion_state = Arc::new(AtomicBool::new(false));
                completion_states[index] = Some(completion_state.clone());
                tasks.spawn(async move {
                    let mut guard = ToolCompletionGuard::new(
                        event_tx,
                        request_id,
                        tool_call.clone(),
                        completion_state,
                    );
                    let tool_fut = executor.execute(tool_call.clone());
                    let result = tool_fut.await.unwrap_or_else(error_result);
                    guard.disarm();
                    (index, tool_call, result)
                });
            }

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        tasks.abort_all();
                        while let Some(joined) = tasks.join_next().await {
                            match joined {
                                Ok((index, tool_call, result)) => {
                                    pending[index] = false;
                                    emit_completed(event_tx, request_id, &tool_call, &result);
                                    results[index] = Some((tool_call, result));
                                }
                                Err(_) => {
                                    if let Some(index) = pending.iter().position(|is_pending| *is_pending) {
                                        pending[index] = false;
                                        let tool_call = tool_calls[index].clone();
                                        let result = cancelled_result();
                                        if let Some(state) = &completion_states[index] {
                                            emit_completed_once(
                                                state,
                                                event_tx,
                                                request_id,
                                                &tool_call,
                                                &result,
                                            );
                                        }
                                        results[index] = Some((tool_call, result));
                                    }
                                }
                            }
                        }
                        break;
                    }
                    joined = tasks.join_next() => {
                        match joined {
                            Some(Ok((index, tool_call, result))) => {
                                pending[index] = false;
                                emit_completed(event_tx, request_id, &tool_call, &result);
                                results[index] = Some((tool_call, result));
                            }
                            Some(Err(error)) => {
                                if let Some(index) = pending.iter().position(|is_pending| *is_pending) {
                                    pending[index] = false;
                                    let tool_call = tool_calls[index].clone();
                                    let result = ToolExecutionResult::new(
                                        format!("Error: tool task failed: {error}"),
                                    );
                                    if let Some(state) = &completion_states[index] {
                                        emit_completed_once(
                                            state,
                                            event_tx,
                                            request_id,
                                            &tool_call,
                                            &result,
                                        );
                                    }
                                    results[index] = Some((tool_call, result));
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
        }
    }

    for (index, tool_call) in write {
        if cancel.is_cancelled() {
            let result = cancelled_result();
            emit_completed(event_tx, request_id, &tool_call, &result);
            results[index] = Some((tool_call, result));
            continue;
        }

        emit_starting(event_tx, request_id, &tool_call);
        let mut guard = ToolCompletionGuard::new(
            event_tx.clone(),
            request_id,
            tool_call.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        let tool_fut = executor.execute(tool_call.clone());
        let result = tool_fut.await.unwrap_or_else(error_result);
        guard.disarm();
        emit_completed(event_tx, request_id, &tool_call, &result);
        results[index] = Some((tool_call, result));
    }

    let completed: Vec<(ToolCall, ToolExecutionResult)> = results.into_iter().flatten().collect();
    Ok(order_tool_results(tool_calls, completed))
}

fn cancelled_result() -> ToolExecutionResult {
    ToolExecutionResult::new("User cancelled the request")
}

fn error_result(error: anyhow::Error) -> ToolExecutionResult {
    ToolExecutionResult::new(format!("Error: tool call failed: {error:#}"))
}

fn emit_starting(event_tx: &AgentEventSender, request_id: u64, tool_call: &ToolCall) {
    let _ = event_tx.send(AgentEvent::ToolStarting {
        request_id,
        tool_call: tool_call.clone(),
    });
}

fn emit_completed(
    event_tx: &AgentEventSender,
    request_id: u64,
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
) {
    let _ = event_tx.send(AgentEvent::ToolCompleted {
        request_id,
        tool_call: tool_call.clone(),
        result: Box::new(result.clone()),
    });
}

fn emit_completed_once(
    state: &AtomicBool,
    event_tx: &AgentEventSender,
    request_id: u64,
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
) {
    if !state.swap(true, Ordering::AcqRel) {
        emit_completed(event_tx, request_id, tool_call, result);
    }
}

struct ToolCompletionGuard {
    event_tx: AgentEventSender,
    request_id: u64,
    tool_call: ToolCall,
    completion_state: Arc<AtomicBool>,
    disarmed: bool,
}

impl ToolCompletionGuard {
    fn new(
        event_tx: AgentEventSender,
        request_id: u64,
        tool_call: ToolCall,
        completion_state: Arc<AtomicBool>,
    ) -> Self {
        Self {
            event_tx,
            request_id,
            tool_call,
            completion_state,
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for ToolCompletionGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            emit_completed_once(
                &self.completion_state,
                &self.event_tx,
                self.request_id,
                &self.tool_call,
                &cancelled_result(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use crate::event::AgentEventSink;
    use async_trait::async_trait;
    use tidev_llm::message::ToolCall;
    use tokio::sync::mpsc;

    struct TestExecutor {
        event_tx: AgentEventSender,
        delay: Duration,
    }

    #[async_trait]
    impl ToolCallExecutor for TestExecutor {
        fn is_read_only(&self, _tool_call: &ToolCall) -> bool {
            true
        }

        async fn execute(&self, tool_call: ToolCall) -> Result<ToolExecutionResult> {
            self.event_tx.send(AgentEvent::ShellOutput {
                request_id: 7,
                tool_call_id: tool_call.id,
                content: "shell output".to_string(),
                finished: true,
                exit_code: Some(0),
            });
            tokio::time::sleep(self.delay).await;
            Ok(ToolExecutionResult::new("done"))
        }
    }

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        }
    }

    fn events() -> (AgentEventSender, mpsc::UnboundedReceiver<AgentEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (tx.into(), rx)
    }

    struct CancelOnStartingSink {
        tx: mpsc::UnboundedSender<AgentEvent>,
        cancel: CancellationToken,
    }

    impl AgentEventSink for CancelOnStartingSink {
        fn send_event(&self, event: AgentEvent) -> bool {
            if matches!(event, AgentEvent::ToolStarting { .. }) {
                self.cancel.cancel();
            }
            self.tx.send(event).is_ok()
        }
    }

    #[tokio::test]
    async fn shell_output_precedes_tool_completion() {
        let (event_tx, mut events) = events();
        let cancel = CancellationToken::new();
        let calls = vec![call("one")];
        let executor = Arc::new(TestExecutor {
            event_tx: event_tx.clone(),
            delay: Duration::from_millis(1),
        });

        execute_tool_calls(executor, &calls, &event_tx, &cancel, 7)
            .await
            .unwrap();

        let first = events.recv().await.unwrap();
        let second = events.recv().await.unwrap();
        assert!(matches!(first, AgentEvent::ToolStarting { .. }));
        assert!(matches!(second, AgentEvent::ShellOutput { .. }));
        let third = events.recv().await.unwrap();
        assert!(matches!(third, AgentEvent::ToolCompleted { .. }));
        assert!(events.try_recv().is_err());
    }

    #[tokio::test]
    async fn cancellation_emits_one_completion_per_read_only_call() {
        let (event_tx, mut events) = events();
        let cancel = CancellationToken::new();
        let calls = vec![call("one"), call("two")];
        let executor = Arc::new(TestExecutor {
            event_tx: event_tx.clone(),
            delay: Duration::from_secs(10),
        });
        let cancel_task = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            cancel_task.cancel();
        });

        let results = execute_tool_calls(executor, &calls, &event_tx, &cancel, 7)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0.id, "one");
        assert_eq!(results[1].0.id, "two");
        assert!(
            results
                .iter()
                .all(|(_, result)| result.output == "User cancelled the request")
        );

        let mut completed = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let AgentEvent::ToolCompleted { tool_call, .. } = event {
                completed.push(tool_call.id);
            }
        }
        completed.sort();
        assert_eq!(completed, vec!["one", "two"]);
    }

    #[tokio::test]
    async fn cancellation_before_task_poll_still_emits_completion() {
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let cancel = CancellationToken::new();
        let event_tx = AgentEventSender::from_sink(CancelOnStartingSink {
            tx: events_tx,
            cancel: cancel.clone(),
        });
        let calls = vec![call("one")];
        let executor = Arc::new(TestExecutor {
            event_tx: event_tx.clone(),
            delay: Duration::from_secs(10),
        });

        let results = execute_tool_calls(executor, &calls, &event_tx, &cancel, 7)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(
            events.recv().await,
            Some(AgentEvent::ToolStarting { .. })
        ));
        assert!(matches!(
            events.recv().await,
            Some(AgentEvent::ToolCompleted { .. })
        ));
        assert!(events.try_recv().is_err());
    }
}
