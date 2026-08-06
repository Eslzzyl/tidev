//! Generic tool contracts for agent runtimes.

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use tidev_llm::ToolDefinition;
use tidev_llm::message::ToolExecutionResult;

use crate::event::AgentEventSender;

/// Host capabilities exposed to a generic tool implementation.
pub trait ToolContext: Send + Sync {
    /// Return the workspace root selected by the host.
    fn workspace_root(&self) -> &Path;

    /// Return the event channel used by the tool implementation.
    fn event_tx(&self) -> AgentEventSender;
}

/// A protocol-level tool implementation.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Return the protocol definition advertised to the model.
    fn definition(&self) -> ToolDefinition;

    /// Return whether this tool can run concurrently with other read-only tools.
    fn read_only(&self) -> bool;

    /// Execute parsed JSON arguments using host-provided capabilities.
    async fn execute(
        &self,
        args: serde_json::Value,
        context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult>;
}
