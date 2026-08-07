//! Generic subagent execution scheduling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use async_trait::async_trait;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use tidev_llm::message::{ToolCall, ToolExecutionResult};

use crate::turn::order_tool_results;

/// Result returned by a host-owned subagent implementation.
#[derive(Clone, Debug)]
pub struct SubagentExecution {
    /// Protocol result returned to the parent tool call.
    pub result: ToolExecutionResult,
    /// Host-owned child session, when one was created.
    pub child_session_id: Option<Uuid>,
}

/// Host boundary for creating and running one subagent.
#[async_trait]
pub trait SubagentExecutor: Send + Sync {
    /// Execute one task call using the supplied child cancellation token.
    async fn execute(
        &self,
        tool_call: ToolCall,
        child_session_id: Option<Uuid>,
        cancel: CancellationToken,
    ) -> Result<SubagentExecution>;
}

/// Host boundary for subagent completion events.
///
/// The generic scheduler does not assume how a host represents child-session
/// metadata. A tidev host can therefore preserve its backend event payload,
/// while another host can map the callback to its own event model.
pub trait SubagentEventSink: Send + Sync {
    /// Emit the completion of one parent task call.
    fn tool_completed(
        &self,
        request_id: u64,
        tool_call: &ToolCall,
        result: &ToolExecutionResult,
        child_session_id: Option<Uuid>,
    );
}

/// Execute subagent calls concurrently and restore their original call order.
///
/// Completion callbacks are emitted as calls finish. Cancellation produces one
/// synthetic completion for every pending call, including tasks that had not
/// been polled before they were aborted.
pub async fn execute_subagent_calls(
    executor: Arc<dyn SubagentExecutor>,
    task_calls: &[(ToolCall, Option<Uuid>)],
    event_sink: Arc<dyn SubagentEventSink>,
    cancel: &CancellationToken,
    request_id: u64,
) -> Result<Vec<(ToolCall, ToolExecutionResult)>> {
    let mut results: Vec<Option<(ToolCall, ToolExecutionResult)>> =
        (0..task_calls.len()).map(|_| None).collect();

    if cancel.is_cancelled() {
        for (index, (tool_call, child_session_id)) in task_calls.iter().cloned().enumerate() {
            let result = cancelled_result();
            event_sink.tool_completed(request_id, &tool_call, &result, child_session_id);
            results[index] = Some((tool_call, result));
        }
        return Ok(order_results(task_calls, results));
    }

    let mut pending = vec![false; task_calls.len()];
    let mut completion_states: Vec<Option<Arc<AtomicBool>>> =
        (0..task_calls.len()).map(|_| None).collect();
    let mut tasks = JoinSet::new();

    for (index, (tool_call, child_session_id)) in task_calls.iter().cloned().enumerate() {
        pending[index] = true;
        let executor = executor.clone();
        let event_sink = event_sink.clone();
        let child_cancel = cancel.child_token();
        let completion_state = Arc::new(AtomicBool::new(false));
        completion_states[index] = Some(completion_state.clone());
        tasks.spawn(async move {
            let mut guard = SubagentCompletionGuard {
                event_sink,
                request_id,
                tool_call: tool_call.clone(),
                child_session_id,
                completion_state,
                disarmed: false,
            };
            let execution = executor
                .execute(tool_call.clone(), child_session_id, child_cancel)
                .await;
            guard.disarm();
            (index, tool_call, child_session_id, execution)
        });
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tasks.abort_all();
                while let Some(joined) = tasks.join_next().await {
                    match joined {
                        Ok((index, tool_call, created_child_session_id, _)) => {
                            pending[index] = false;
                            let result = cancelled_result();
                            emit_completed_once(
                                completion_states[index].as_ref(),
                                event_sink.as_ref(),
                                request_id,
                                &tool_call,
                                &result,
                                created_child_session_id,
                            );
                            results[index] = Some((tool_call, result));
                        }
                        Err(_) => {
                            if let Some(index) = pending.iter().position(|is_pending| *is_pending) {
                                pending[index] = false;
                                let (tool_call, child_session_id) = task_calls[index].clone();
                                let result = cancelled_result();
                                emit_completed_once(
                                    completion_states[index].as_ref(),
                                    event_sink.as_ref(),
                                    request_id,
                                    &tool_call,
                                    &result,
                                    child_session_id,
                                );
                                results[index] = Some((tool_call, result));
                            }
                        }
                    }
                }
                break;
            }
            joined = tasks.join_next() => {
                match joined {
                    Some(Ok((index, tool_call, child_session_id, Ok(execution)))) => {
                        pending[index] = false;
                        if cancel.is_cancelled() {
                            let result = cancelled_result();
                            emit_completed_once(
                                completion_states[index].as_ref(),
                                event_sink.as_ref(),
                                request_id,
                                &tool_call,
                                &result,
                                execution.child_session_id.or(child_session_id),
                            );
                            results[index] = Some((tool_call, result));
                        } else {
                            emit_completed_once(
                                completion_states[index].as_ref(),
                                event_sink.as_ref(),
                                request_id,
                                &tool_call,
                                &execution.result,
                                execution.child_session_id,
                            );
                            results[index] = Some((tool_call, execution.result));
                        }
                    }
                    Some(Ok((_index, _tool_call, _child_session_id, Err(error)))) => {
                        return Err(error);
                    }
                    Some(Err(error)) => {
                        return Err(anyhow::anyhow!("Subagent join error: {error}"));
                    }
                    None => break,
                }
            }
        }
    }

    Ok(order_results(task_calls, results))
}

fn order_results(
    task_calls: &[(ToolCall, Option<Uuid>)],
    results: Vec<Option<(ToolCall, ToolExecutionResult)>>,
) -> Vec<(ToolCall, ToolExecutionResult)> {
    let calls: Vec<ToolCall> = task_calls
        .iter()
        .map(|(tool_call, _)| tool_call.clone())
        .collect();
    let completed: Vec<(ToolCall, ToolExecutionResult)> = results.into_iter().flatten().collect();
    order_tool_results(&calls, completed)
}

fn cancelled_result() -> ToolExecutionResult {
    ToolExecutionResult::new("User cancelled the request")
}

fn emit_completed_once(
    state: Option<&Arc<AtomicBool>>,
    event_sink: &dyn SubagentEventSink,
    request_id: u64,
    tool_call: &ToolCall,
    result: &ToolExecutionResult,
    child_session_id: Option<Uuid>,
) {
    let should_emit = state
        .map(|state| !state.swap(true, Ordering::AcqRel))
        .unwrap_or(true);
    if should_emit {
        event_sink.tool_completed(request_id, tool_call, result, child_session_id);
    }
}

struct SubagentCompletionGuard {
    event_sink: Arc<dyn SubagentEventSink>,
    request_id: u64,
    tool_call: ToolCall,
    child_session_id: Option<Uuid>,
    completion_state: Arc<AtomicBool>,
    disarmed: bool,
}

impl SubagentCompletionGuard {
    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for SubagentCompletionGuard {
    fn drop(&mut self) {
        if !self.disarmed && !self.completion_state.swap(true, Ordering::AcqRel) {
            self.event_sink.tool_completed(
                self.request_id,
                &self.tool_call,
                &cancelled_result(),
                self.child_session_id,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use async_trait::async_trait;

    struct TestExecutor;

    #[async_trait]
    impl SubagentExecutor for TestExecutor {
        async fn execute(
            &self,
            tool_call: ToolCall,
            child_session_id: Option<Uuid>,
            cancel: CancellationToken,
        ) -> Result<SubagentExecution> {
            let delay = if tool_call.id == "slow" {
                Duration::from_millis(20)
            } else {
                Duration::from_millis(1)
            };
            tokio::select! {
                _ = cancel.cancelled() => Ok(SubagentExecution {
                    result: cancelled_result(),
                    child_session_id,
                }),
                _ = tokio::time::sleep(delay) => Ok(SubagentExecution {
                    result: ToolExecutionResult::new(tool_call.id.clone()),
                    child_session_id,
                }),
            }
        }
    }

    struct TestEventSink {
        completed: Mutex<Vec<String>>,
    }

    impl SubagentEventSink for TestEventSink {
        fn tool_completed(
            &self,
            _request_id: u64,
            tool_call: &ToolCall,
            _result: &ToolExecutionResult,
            _child_session_id: Option<Uuid>,
        ) {
            self.completed.lock().unwrap().push(tool_call.id.clone());
        }
    }

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: "task".to_string(),
            arguments: "{}".to_string(),
            thought_signature: None,
        }
    }

    #[tokio::test]
    async fn returns_subagent_results_in_parent_call_order() {
        let calls = vec![call("slow"), call("fast")];
        let sink = Arc::new(TestEventSink {
            completed: Mutex::new(Vec::new()),
        });
        let results = execute_subagent_calls(
            Arc::new(TestExecutor),
            &calls
                .iter()
                .cloned()
                .map(|call| (call, None))
                .collect::<Vec<_>>(),
            sink.clone(),
            &CancellationToken::new(),
            1,
        )
        .await
        .unwrap();

        assert_eq!(
            results
                .iter()
                .map(|(call, result)| (call.id.as_str(), result.output.as_str()))
                .collect::<Vec<_>>(),
            [("slow", "slow"), ("fast", "fast")]
        );
        assert_eq!(sink.completed.lock().unwrap().as_slice(), ["fast", "slow"]);
    }

    #[tokio::test]
    async fn cancellation_completes_each_pending_subagent_once() {
        let calls = vec![call("slow"), call("slow-2")];
        let task_calls: Vec<_> = calls.into_iter().map(|call| (call, None)).collect();
        let sink = Arc::new(TestEventSink {
            completed: Mutex::new(Vec::new()),
        });
        let cancel = CancellationToken::new();
        let cancel_task = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(5)).await;
            cancel_task.cancel();
        });

        let results = execute_subagent_calls(
            Arc::new(TestExecutor),
            &task_calls,
            sink.clone(),
            &cancel,
            1,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(sink.completed.lock().unwrap().len(), 2);
    }
}
