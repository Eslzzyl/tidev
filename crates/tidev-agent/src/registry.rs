//! Generic tool registration and dispatch.

use std::sync::Arc;

use anyhow::{Context, Result};

use tidev_llm::ToolDefinition;
use tidev_llm::message::{ToolCall, ToolExecutionResult};

use crate::tool::{Tool, ToolContext};

/// Ordered registry of generic agent tools.
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
    max_output_bytes: usize,
}

impl ToolRegistry {
    /// Create an empty registry with an optional output limit.
    pub fn new(max_output_bytes: usize) -> Self {
        Self {
            tools: Vec::new(),
            max_output_bytes,
        }
    }

    /// Register a tool while preserving registration order.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.push(Arc::new(tool));
    }

    /// Return all protocol definitions in registration order.
    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|tool| tool.definition()).collect()
    }

    /// Return the definition for an exact tool name.
    pub fn definition(&self, name: &str) -> Option<ToolDefinition> {
        self.tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .map(|tool| tool.definition())
    }

    /// Return whether a registered tool is read-only.
    pub fn is_read_only(&self, name: &str) -> Option<bool> {
        self.tools
            .iter()
            .find(|tool| tool.definition().name == name)
            .map(|tool| tool.read_only())
    }

    /// Parse and execute one tool call.
    pub async fn execute(
        &self,
        call: &ToolCall,
        context: &dyn ToolContext,
    ) -> Result<ToolExecutionResult> {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.definition().name == call.name)
            .cloned()
            .with_context(|| format!("unknown tool '{}'", call.name))?;
        let args = serde_json::from_str(&call.arguments)
            .with_context(|| format!("invalid arguments for tool '{}'", call.name))?;
        let mut result = tool.execute(args, context).await?;
        truncate_output(&mut result, self.max_output_bytes);
        Ok(result)
    }
}

fn truncate_output(result: &mut ToolExecutionResult, max_output_bytes: usize) {
    if max_output_bytes == 0 || result.output.len() <= max_output_bytes {
        return;
    }

    let mut end = max_output_bytes;
    while end > 0 && !result.output.is_char_boundary(end) {
        end -= 1;
    }
    result.output.truncate(end);
    result.output.push_str("\n[truncated]");
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::path::{Path, PathBuf};
    use tokio::sync::mpsc::unbounded_channel;

    use crate::event::AgentEvent;
    use crate::tool::{Tool, ToolContext};

    struct StubContext {
        root: PathBuf,
        event_tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    }

    impl ToolContext for StubContext {
        fn workspace_root(&self) -> &Path {
            &self.root
        }

        fn event_tx(&self) -> tokio::sync::mpsc::UnboundedSender<AgentEvent> {
            self.event_tx.clone()
        }
    }

    struct EchoTool {
        name: &'static str,
        read_only: bool,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                display_name: self.name.to_string(),
                description: "echo test tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }
        }

        fn read_only(&self) -> bool {
            self.read_only
        }

        async fn execute(
            &self,
            args: serde_json::Value,
            context: &dyn ToolContext,
        ) -> Result<ToolExecutionResult> {
            assert_eq!(context.workspace_root(), Path::new("/workspace"));
            Ok(ToolExecutionResult::new(
                args.get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            ))
        }
    }

    #[tokio::test]
    async fn preserves_registration_order_and_dispatches_arguments() {
        let mut registry = ToolRegistry::new(0);
        registry.register(EchoTool {
            name: "first",
            read_only: true,
        });
        registry.register(EchoTool {
            name: "second",
            read_only: false,
        });
        let (event_tx, _event_rx) = unbounded_channel();
        let context = StubContext {
            root: PathBuf::from("/workspace"),
            event_tx,
        };

        let definitions = registry.definitions();
        assert_eq!(definitions[0].name, "first");
        assert_eq!(definitions[1].name, "second");
        assert_eq!(registry.is_read_only("first"), Some(true));
        assert_eq!(registry.is_read_only("second"), Some(false));

        let result = registry
            .execute(
                &ToolCall {
                    id: "call-1".to_string(),
                    name: "first".to_string(),
                    arguments: r#"{"text":"ok"}"#.to_string(),
                    thought_signature: None,
                },
                &context,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "ok");
    }

    #[tokio::test]
    async fn truncates_utf8_output_at_a_character_boundary() {
        let mut registry = ToolRegistry::new(4);
        registry.register(EchoTool {
            name: "echo",
            read_only: true,
        });
        let (event_tx, _event_rx) = unbounded_channel();
        let context = StubContext {
            root: PathBuf::from("/workspace"),
            event_tx,
        };

        let result = registry
            .execute(
                &ToolCall {
                    id: "call-1".to_string(),
                    name: "echo".to_string(),
                    arguments: r#"{"text":"你好世界"}"#.to_string(),
                    thought_signature: None,
                },
                &context,
            )
            .await
            .unwrap();
        assert!(result.output.ends_with("[truncated]"));
        assert!(result.output.is_char_boundary(result.output.len()));
    }
}
