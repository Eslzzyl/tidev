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
/// - update: Modify an existing memory by ID
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
        "update" => execute_update(memory_store, &ws, &arguments),
        "search" => execute_search(memory_store, &ws, &arguments),
        "list" => execute_list(memory_store, &ws),
        "read" => execute_read(memory_store, &ws, &arguments),
        "delete" => execute_delete(memory_store, &ws, &arguments),
        _ => bail!("unknown memory operation '{}'", operation),
    }
}

/// Parse tags from a JSON Value that may be a JSON array or a comma-separated string.
fn parse_tags(value: &Value) -> Vec<String> {
    match value {
        Value::String(s) => s
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Value::Array(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        _ => Vec::new(),
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
    let memory_type = MemoryType::parse_str(memory_type_str).context("invalid memory_type")?;
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
        .map(parse_tags)
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

    // Auto-dedup hint: check for similar existing memories
    let hint = find_similar_hint(memory_store, workspace_root, title, entry.id);

    Ok(format!(
        "Memory saved: [{}] {}{}",
        memory_type.as_str(),
        title,
        hint,
    ))
}

/// After storing a new memory, search for similar ones and return a hint string.
fn find_similar_hint(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    title: &str,
    new_id: Uuid,
) -> String {
    if let Ok(similar) = memory_store.search(workspace_root, title) {
        let others: Vec<&MemoryEntry> = similar.iter().filter(|e| e.id != new_id).collect();
        if !others.is_empty() {
            let mut hint = String::new();
            if others.len() == 1 {
                let e = others[0];
                let short_id: String = e.id.to_string().chars().take(8).collect();
                hint.push_str(&format!(
                    "\n\n⚠️ Note: a similar memory already exists (`{}` [{}] **{}**). \
                     Consider using `operation: update` with `memory_id=\"{}\"` to merge them \
                     instead of creating duplicates.",
                    short_id,
                    e.memory_type.short_label(),
                    e.title,
                    e.id,
                ));
            } else {
                hint.push_str(&format!(
                    "\n\n⚠️ Note: {} similar memories already exist:",
                    others.len(),
                ));
                for e in &others {
                    let short_id: String = e.id.to_string().chars().take(8).collect();
                    hint.push_str(&format!(
                        "\n  - `{}` [{}] **{}**",
                        short_id,
                        e.memory_type.short_label(),
                        e.title,
                    ));
                }
                hint.push_str(
                    "\nConsider reviewing and merging via `operation: update`.",
                );
            }
            return hint;
        }
    }
    String::new()
}

/// Update an existing memory entry. Fields not provided keep their existing values.
fn execute_update(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let id_str = arguments
        .get("memory_id")
        .and_then(|v| v.as_str())
        .context("memory_id is required for update operation")?;
    let id = Uuid::parse_str(id_str)
        .map_err(|e| anyhow::anyhow!("invalid memory_id '{}': {}", id_str, e))?;

    let existing = memory_store
        .get(workspace_root, id)?
        .context("memory not found")?;

    let memory_type = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .and_then(MemoryType::parse_str)
        .unwrap_or(existing.memory_type);

    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&existing.title);

    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or(&existing.content);

    let tags = if let Some(tags_val) = arguments.get("tags") {
        parse_tags(tags_val)
    } else {
        existing.tags.clone()
    };

    let updated = MemoryEntry {
        id: existing.id,
        workspace_root: workspace_root.to_string(),
        memory_type,
        title: title.to_string(),
        content: content.to_string(),
        tags,
        source_session_id: existing.source_session_id,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now(),
        usage_count: existing.usage_count,
        active: true,
    };

    memory_store.update(&updated)?;

    // Record usage to boost its hotness after update
    let _ = memory_store.record_usage(workspace_root, id);

    Ok(format!(
        "Memory updated: [{}] {}",
        memory_type.as_str(),
        title,
    ))
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

    // Record usage for each result so hotness reflects real retrieval
    for entry in &results {
        let _ = memory_store.record_usage(workspace_root, entry.id);
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

fn execute_list(memory_store: &Arc<MemoryStore>, workspace_root: &str) -> Result<String> {
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

    // Record usage so hotness reflects real interest
    let _ = memory_store.record_usage(workspace_root, id);

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
