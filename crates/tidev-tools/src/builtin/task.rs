use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;

use crate::builtin::utils::decode_tool_args;
use crate::todo_persistence::TodoPersistence;
use crate::types::{TaskArgs, ToolDefinition, ToolPermission};

/// Mirrors `tidev_core::agent_type::AgentType::parse` accepted names.
/// Keep in sync when agent types are added/renamed (see tidev-core agent_type.rs).
const SUBAGENT_TYPES: &[&str] = &["explorer", "librarian", "oracle", "fixer"];

fn normalize_subagent_type(s: &str) -> Option<&'static str> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.strip_prefix('@').unwrap_or(&s);
    SUBAGENT_TYPES.iter().find(|t| **t == s).copied()
}

pub fn definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition::new::<TaskArgs>(
        "task",
        "Run a subagent task. Use `subagent_type` to delegate to a specialist: \
          explorer (code search), librarian (docs), oracle (strategy), \
          fixer (implementation).",
        ToolPermission::Session,
    )]
}

pub fn execute_tool_call(
    _workspace_root: &Path,
    _store: &dyn TodoPersistence,
    _session_id: Uuid,
    tool_name: &str,
    arguments: Value,
) -> Result<String> {
    let args = decode_tool_args::<TaskArgs>(tool_name, arguments)?;

    let description = args.description.trim();
    let prompt = args.prompt.trim();

    ensure!(!description.is_empty(), "task description cannot be empty");
    ensure!(!prompt.is_empty(), "task prompt cannot be empty");

    let subagent_type_str = args.subagent_type.trim();
    ensure!(
        !subagent_type_str.is_empty(),
        "subagent_type is required: specify one of explorer, librarian, oracle, fixer"
    );

    let subagent_type = normalize_subagent_type(&args.subagent_type).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown subagent type '{subagent_type_str}': expected one of explorer, librarian, oracle, fixer"
        )
    })?;

    Ok(format!("Started {subagent_type} subagent task '{description}'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TodoItem;

    struct MockTodoStore;

    impl TodoPersistence for MockTodoStore {
        fn load_todos(&self, _session_id: Uuid) -> anyhow::Result<Vec<TodoItem>> {
            Ok(Vec::new())
        }

        fn replace_todos(&self, _session_id: Uuid, _todos: &[TodoItem]) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_normalize_subagent_type() {
        for name in SUBAGENT_TYPES {
            assert_eq!(normalize_subagent_type(name), Some(*name));
            assert_eq!(normalize_subagent_type(&format!("@{name}")), Some(*name));
            assert_eq!(
                normalize_subagent_type(&name.to_uppercase()),
                Some(*name)
            );
        }
        assert_eq!(normalize_subagent_type("general"), None);
        assert_eq!(normalize_subagent_type("unknown"), None);
    }

    #[test]
    fn test_unknown_subagent_type_error_message() {
        let err = execute_tool_call(
            Path::new("."),
            &MockTodoStore,
            Uuid::new_v4(),
            "task",
            serde_json::json!({
                "description": "desc",
                "prompt": "prompt",
                "subagent_type": "bogus",
            }),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("expected one of explorer, librarian, oracle, fixer"),
            "error message: {err}"
        );
    }
}
