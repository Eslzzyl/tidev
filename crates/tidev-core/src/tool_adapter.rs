//! Adapters from tidev's host-owned tools to the generic agent tool contract.

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio_util::sync::CancellationToken;

use tidev_agent::{Tool, ToolContext};
use tidev_llm::ToolDefinition;
use tidev_llm::message::{ToolCall, ToolExecutionResult};

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
        _context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        Ok(self
            .registry
            .execute(
                &self.call,
                self.session_id,
                self.request_id,
                self.mode,
                self.allow_outside,
                self.sensitive_file_approved,
                &self.cancel,
                None,
            )
            .await)
    }
}

/// Dispatch one non-streaming host tool through `tidev-agent`'s registry.
pub(crate) async fn execute_builtin_via_agent(
    registry: &ToolRegistry,
    call: &ToolCall,
    session_id: uuid::Uuid,
    request_id: u64,
    mode: crate::mode::Mode,
    allow_outside: bool,
    sensitive_file_approved: bool,
    cancel: &CancellationToken,
) -> ToolExecutionResult {
    // Keep tidev-tools' user-facing parse error for malformed calls. The
    // generic registry intentionally returns a dispatch error for malformed
    // JSON, while the host tool contract returns an error result instead.
    if serde_json::from_str::<serde_json::Value>(&call.arguments).is_err() {
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
    }

    let Some(host_definition) = registry.definition_for(&call.name) else {
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

    let mut definition = to_llm_tool_def(&host_definition);
    // Aliases are accepted by tidev-tools but the generic registry matches
    // exact names. This per-call definition keeps alias dispatch equivalent.
    definition.name = call.name.clone();

    let adapter = BuiltinToolAdapter::new(
        registry.clone(),
        call.clone(),
        definition,
        true,
        session_id,
        request_id,
        mode,
        allow_outside,
        sensitive_file_approved,
        cancel.clone(),
    );
    let mut agent_registry = tidev_agent::ToolRegistry::new(0);
    agent_registry.register(adapter);

    let (event_tx, _event_rx) = unbounded_channel();
    let context = AdapterContext {
        workspace_root: registry.workspace_root().to_path_buf(),
        event_tx,
    };

    match agent_registry.execute(call, &context).await {
        Ok(result) => result,
        Err(error) => ToolExecutionResult::new(format!("Error: {error:#}")),
    }
}
