//! Adapters from tidev's host-owned tools to the generic agent tool contract.

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use tidev_agent::{Tool, ToolContext};
use tidev_llm::ToolDefinition;
use tidev_llm::message::{ToolCall, ToolExecutionResult};
use tidev_tools::ShellOutput;

use crate::registry::ToolRegistry;
use crate::tool_def::to_llm_tool_def;
use tidev_agent::AgentEvent;

/// Context passed to a host adapter while a generic tool dispatch is running.
struct AdapterContext {
    workspace_root: PathBuf,
    event_tx: UnboundedSender<AgentEvent>,
}

impl ToolContext for AdapterContext {
    fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    fn event_tx(&self) -> UnboundedSender<AgentEvent> {
        self.event_tx.clone()
    }
}

/// Forward shell output while retaining the old drain-before-completion rule.
struct ShellOutputForwardGuard {
    shell_rx: tokio::sync::mpsc::UnboundedReceiver<ShellOutput>,
    event_tx: UnboundedSender<AgentEvent>,
    request_id: u64,
    disarmed: bool,
}

impl ShellOutputForwardGuard {
    fn new(
        shell_rx: tokio::sync::mpsc::UnboundedReceiver<ShellOutput>,
        event_tx: UnboundedSender<AgentEvent>,
        request_id: u64,
    ) -> Self {
        Self {
            shell_rx,
            event_tx,
            request_id,
            disarmed: false,
        }
    }

    fn drain(&mut self) {
        while let Ok(output) = self.shell_rx.try_recv() {
            let _ = self.event_tx.send(AgentEvent::ShellOutput {
                request_id: self.request_id,
                tool_call_id: output.tool_call_id,
                content: output.content,
                finished: output.finished,
                exit_code: output.exit_code,
            });
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for ShellOutputForwardGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            self.drain();
        }
    }
}

/// A per-call adapter that preserves the original [`ToolCall`] for the host.
///
/// The generic `Tool` contract receives parsed JSON arguments. Host tools also
/// need the original call id and argument bytes, so this adapter captures the
/// call and deliberately delegates with it unchanged.
pub(crate) struct BuiltinToolAdapter {
    registry: ToolRegistry,
    call: ToolCall,
    definition: ToolDefinition,
    read_only: bool,
    session_id: uuid::Uuid,
    request_id: u64,
    mode: crate::mode::Mode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    cancel: CancellationToken,
    stream_shell: bool,
}

impl BuiltinToolAdapter {
    pub(crate) fn new(
        registry: ToolRegistry,
        call: ToolCall,
        definition: ToolDefinition,
        read_only: bool,
        session_id: uuid::Uuid,
        request_id: u64,
        mode: crate::mode::Mode,
        allow_outside: bool,
        sensitive_file_approved: bool,
        cancel: CancellationToken,
        stream_shell: bool,
    ) -> Self {
        Self {
            registry,
            call,
            definition,
            read_only,
            session_id,
            request_id,
            mode,
            allow_outside,
            sensitive_file_approved,
            cancel,
            stream_shell,
        }
    }
}

#[async_trait]
impl Tool for BuiltinToolAdapter {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        let event_tx = context.event_tx();
        Ok(execute_host_call(
            &self.registry,
            &self.call,
            self.session_id,
            self.request_id,
            self.mode,
            self.allow_outside,
            self.sensitive_file_approved,
            &self.cancel,
            self.stream_shell.then_some(event_tx),
        )
        .await)
    }
}

async fn execute_host_call(
    registry: &ToolRegistry,
    call: &ToolCall,
    session_id: uuid::Uuid,
    request_id: u64,
    mode: crate::mode::Mode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    cancel: &CancellationToken,
    event_tx: Option<UnboundedSender<AgentEvent>>,
) -> ToolExecutionResult {
    let Some(event_tx) = event_tx else {
        return registry
            .execute(
                call,
                session_id,
                request_id,
                mode,
                allow_outside,
                sensitive_file_approved,
                cancel,
                None,
            )
            .await;
    };

    let (shell_tx, shell_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut guard = ShellOutputForwardGuard::new(shell_rx, event_tx, request_id);
    let result = registry
        .execute(
            call,
            session_id,
            request_id,
            mode,
            allow_outside,
            sensitive_file_approved,
            cancel,
            Some(shell_tx),
        )
        .await;
    guard.drain();
    guard.disarm();
    result
}

/// Dispatch one host tool through `tidev-agent`'s registry.
pub(crate) async fn execute_builtin_via_agent(
    registry: &ToolRegistry,
    call: &ToolCall,
    session_id: uuid::Uuid,
    request_id: u64,
    mode: crate::mode::Mode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    cancel: &CancellationToken,
    event_tx: Option<UnboundedSender<AgentEvent>>,
    stream_shell: bool,
) -> ToolExecutionResult {
    // Keep tidev-tools' user-facing parse error for malformed calls. The
    // generic registry intentionally returns a dispatch error for malformed
    // JSON, while the host tool contract returns an error result instead.
    if serde_json::from_str::<serde_json::Value>(&call.arguments).is_err() {
        return execute_host_call(
            registry,
            call,
            session_id,
            request_id,
            mode,
            allow_outside,
            sensitive_file_approved,
            cancel,
            if stream_shell { event_tx } else { None },
        )
        .await;
    }

    let Some(host_definition) = registry.definition_for(&call.name) else {
        return execute_host_call(
            registry,
            call,
            session_id,
            request_id,
            mode,
            allow_outside,
            sensitive_file_approved,
            cancel,
            if stream_shell { event_tx } else { None },
        )
        .await;
    };

    let mut definition = to_llm_tool_def(&host_definition);
    // Aliases are accepted by tidev-tools but the generic registry matches
    // exact names. This per-call definition keeps alias dispatch equivalent.
    definition.name = call.name.clone();

    let adapter = BuiltinToolAdapter::new(
        registry.clone(),
        call.clone(),
        definition,
        !stream_shell,
        session_id,
        request_id,
        mode,
        allow_outside,
        sensitive_file_approved,
        cancel.clone(),
        stream_shell,
    );
    let mut agent_registry = tidev_agent::ToolRegistry::new(0);
    agent_registry.register(adapter);

    let event_tx = event_tx.unwrap_or_else(|| unbounded_channel().0);
    let context = AdapterContext {
        workspace_root: registry.workspace_root().to_path_buf(),
        event_tx,
    };

    match agent_registry.execute(call, &context).await {
        Ok(result) => result,
        Err(error) => ToolExecutionResult::new(format!("Error: {error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_output_guard_forwards_pending_output_on_drop() {
        let (event_tx, mut event_rx) = unbounded_channel();
        let (shell_tx, shell_rx) = tokio::sync::mpsc::unbounded_channel();
        shell_tx
            .send(ShellOutput {
                session_id: uuid::Uuid::nil(),
                request_id: 99,
                tool_call_id: "call-raw".to_string(),
                content: "partial".to_string(),
                finished: true,
                exit_code: Some(0),
            })
            .unwrap();
        drop(shell_tx);

        drop(ShellOutputForwardGuard::new(shell_rx, event_tx, 7));

        assert!(matches!(
            event_rx.try_recv(),
            Ok(AgentEvent::ShellOutput {
                request_id: 7,
                tool_call_id,
                content,
                finished: true,
                exit_code: Some(0),
            }) if tool_call_id == "call-raw" && content == "partial"
        ));
    }
}
