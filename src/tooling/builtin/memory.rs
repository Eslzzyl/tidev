use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::memory::types::{MemoryEntry, MemoryStore, MemoryType};

/// Execute a memory tool call.
///
/// Supported operations:
/// - store: Save a new memory entry
/// - search: Search memories by keyword
/// - list: List all active memories for the workspace
/// - read: Read a specific memory by ID
/// - delete: Soft-delete a memory by ID
pub fn execute_tool_call(
    workspace_root: &Path,
    memory_store: &Arc<MemoryStore>,
    _call: &crate::session::ToolCall,
    arguments: Value,
) -> Result<String> {
    let ws = workspace_root.display().to_string();

    let operation = arguments
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    match operation {
        "store" => execute_store(memory_store, &ws, &arguments),
        "search" => execute_search(memory_store, &ws, &arguments),
        "list" => execute_list(memory_store, &ws),
        "read" => execute_read(memory_store, &ws, &arguments),
        "delete" => execute_delete(memory_store, &ws, &arguments),
        _ => bail!("unknown memory operation '{}'", operation),
    }
}

fn execute_store(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let memory_type_str = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .context("memory_type is required for store operation")?;
    let memory_type =
        MemoryType::from_str(memory_type_str).context("invalid memory_type")?;
    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .context("title is required for store operation")?;
    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .context("content is required for store operation")?;
    let tags: Vec<String> = arguments
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let entry = MemoryEntry {
        id: Uuid::new_v4(),
        workspace_root: workspace_root.to_string(),
        memory_type,
        title: title.to_string(),
        content: content.to_string(),
        tags,
        source_session_id: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        usage_count: 0,
        active: true,
    };

    memory_store.add(&entry)?;

    Ok(format!("Memory saved: [{}] {}", memory_type.as_str(), title))
}

fn execute_search(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .context("query is required for search operation")?;

    let results = memory_store.search(workspace_root, query)?;

    if results.is_empty() {
        return Ok("No memories found matching query.".to_string());
    }

    let mut out = format!("Found {} memories:\n", results.len());
    for entry in &results {
        let preview: String = entry.content.chars().take(120).collect();
        let suffix = if entry.content.len() > 120 { "…" } else { "" };
        out.push_str(&format!(
            "- [{}] **{}**: {}{}\n",
            entry.memory_type.short_label(),
            entry.title,
            preview,
            suffix,
        ));
    }
    Ok(out)
}

fn execute_list(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
) -> Result<String> {
    let entries = memory_store.get_or_load(workspace_root)?;

    if entries.is_empty() {
        return Ok("No memories yet for this workspace.".to_string());
    }

    let mut out = format!("Workspace memories ({} active):\n", entries.len());
    for entry in &entries {
        let preview: String = entry.content.chars().take(80).collect();
        let suffix = if entry.content.len() > 80 { "…" } else { "" };
        out.push_str(&format!(
            "  `{}` [{}] {} — {}{}\n",
            entry.id.to_string().chars().take(8).collect::<String>(),
            entry.memory_type.short_label(),
            entry.title,
            preview,
            suffix,
        ));
    }
    Ok(out)
}

fn execute_read(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let id_str = arguments
        .get("memory_id")
        .and_then(|v| v.as_str())
        .context("memory_id is required for read operation")?;
    let id = Uuid::parse_str(id_str)
        .map_err(|e| anyhow::anyhow!("invalid memory_id '{}': {}", id_str, e))?;

    let entry = memory_store
        .get(workspace_root, id)?
        .context("memory not found")?;

    let tags_str = if entry.tags.is_empty() {
        String::new()
    } else {
        format!("\nTags: {}", entry.tags.join(", "))
    };

    Ok(format!(
        "# [{}] {}\n**Type**: {}{}\n**Created**: {}\n**Updated**: {}\n**Used**: {} times\n\n{}",
        entry.memory_type.short_label(),
        entry.title,
        entry.memory_type.as_str(),
        tags_str,
        entry.created_at.format("%Y-%m-%d %H:%M"),
        entry.updated_at.format("%Y-%m-%d %H:%M"),
        entry.usage_count,
        entry.content,
    ))
}

fn execute_delete(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let id_str = arguments
        .get("memory_id")
        .and_then(|v| v.as_str())
        .context("memory_id is required for delete operation")?;
    let id = Uuid::parse_str(id_str)
        .map_err(|e| anyhow::anyhow!("invalid memory_id '{}': {}", id_str, e))?;

    memory_store.delete(workspace_root, id)?;
    Ok(format!("Memory {} deleted.", id_str))
}
