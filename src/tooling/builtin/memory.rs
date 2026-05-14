use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use uuid::Uuid;

use crate::memory::{MemoryStore, MemoryType};

/// Execute a memory tool call.
///
/// Supported operations (agentmemory-style):
/// - remember: Save memory with Jaccard dedup + version chain
/// - search: BM25/FTS5 full-text search
/// - list: List active memories
/// - read: Read a memory by ID
/// - forget: Soft-delete a memory
/// - observations: List raw observations for a session
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
        "remember" => execute_remember(memory_store, &ws, &arguments),
        "search" => execute_search(memory_store, &ws, &arguments),
        "list" => execute_list(memory_store, &ws),
        "read" => execute_read(memory_store, &ws, &arguments),
        "forget" => execute_forget(memory_store, &ws, &arguments),
        "observations" => execute_observations(memory_store, &arguments),
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

// ─── New API (agentmemory-style) ────────────────────────────────────

/// remember: Save a memory with Jaccard dedup and versioning.
fn execute_remember(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let content = arguments
        .get("content")
        .and_then(|v| v.as_str())
        .context("content is required for remember operation")?;

    let memory_type_str = arguments
        .get("memory_type")
        .and_then(|v| v.as_str())
        .unwrap_or("fact");
    let memory_type = MemoryType::parse_str(memory_type_str)
        .context("invalid memory_type, expected one of: pattern, preference, architecture, bug, workflow, fact, user, project, feedback, reference")?;

    let title = arguments
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(&content[..content.len().min(80)]);

    let tags: Vec<String> = arguments
        .get("tags")
        .map(parse_tags)
        .unwrap_or_default();

    let concepts: Vec<String> = arguments
        .get("concepts")
        .map(parse_tags)
        .unwrap_or_default();

    let files: Vec<String> = arguments
        .get("files")
        .map(parse_tags)
        .unwrap_or_default();

    let source_session_id = arguments
        .get("source_session_id")
        .and_then(|v| v.as_str())
        .and_then(|s| Uuid::parse_str(s).ok());

    let entry = memory_store.remember(
        workspace_root,
        memory_type,
        title,
        content,
        &concepts,
        &files,
        &tags,
        source_session_id,
    )?;

    let mut msg = format!(
        "Memory remembered: [{}] {} (v{})",
        entry.memory_type.short_label(),
        entry.title,
        entry.version,
    );

    if let Some(pid) = entry.parent_id {
        msg.push_str(&format!("\nSupersedes: {}", pid));
    }

    msg.push_str(&format!("\nID: {}", entry.id));
    Ok(msg)
}

/// search: BM25/FTS5 full-text search across memories.
fn execute_search(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let query = arguments
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let results = memory_store.search(workspace_root, query)?;

    if results.is_empty() {
        return Ok("No memories found.".to_string());
    }

    let mut lines = vec![format!("Found {} memories:", results.len())];
    for entry in &results {
        let tags_str = if entry.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", entry.tags.join(", "))
        };
        let concepts_str = if entry.concepts.is_empty() {
            String::new()
        } else {
            format!(" [concepts: {}]", entry.concepts.join(", "))
        };
        lines.push(format!(
            "- **[{}]** {} (v{}){}{}\n  {}",
            entry.memory_type.short_label(),
            entry.title,
            entry.version,
            tags_str,
            concepts_str,
            entry.content,
        ));
    }

    Ok(lines.join("\n"))
}

/// list: List all active memories for the workspace.
fn execute_list(memory_store: &Arc<MemoryStore>, workspace_root: &str) -> Result<String> {
    let entries = memory_store.get_or_load(workspace_root)?;

    if entries.is_empty() {
        return Ok("No memories found.".to_string());
    }

    let mut lines = vec![format!("Found {} memories:", entries.len())];
    for (i, entry) in entries.iter().enumerate() {
        let tags_str = if entry.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", entry.tags.join(", "))
        };
        lines.push(format!(
            "{}. **[{}]** {} (v{}){}",
            i + 1,
            entry.memory_type.short_label(),
            entry.title,
            entry.version,
            tags_str,
        ));
    }

    Ok(lines.join("\n"))
}

/// read: Read a full memory entry by ID.
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

    let entry = memory_store.read(workspace_root, id)?;
    let _ = memory_store.record_usage(workspace_root, id);

    let extra = {
        let mut parts = Vec::new();
        if !entry.concepts.is_empty() {
            parts.push(format!("Concepts: {}", entry.concepts.join(", ")));
        }
        if !entry.files.is_empty() {
            parts.push(format!("Files: {}", entry.files.join(", ")));
        }
        if entry.version > 1 {
            parts.push(format!("Version: v{} (latest: {})", entry.version, entry.is_latest));
        }
        if let Some(pid) = entry.parent_id {
            parts.push(format!("Supersedes: {}", pid));
        }
        if parts.is_empty() { String::new() } else { format!("\n{}", parts.join("\n")) }
    };

    Ok(format!(
        "[{}] **{}** ({}){}\n**Created**: {}\n**Updated**: {}\n**Used**: {} times\n\n{}",
        entry.memory_type.short_label(),
        entry.title,
        entry.memory_type.as_str(),
        extra,
        entry.created_at.format("%Y-%m-%d %H:%M"),
        entry.updated_at.format("%Y-%m-%d %H:%M"),
        entry.usage_count,
        entry.content,
    ))
}

/// forget: Soft-delete a memory by ID.
fn execute_forget(
    memory_store: &Arc<MemoryStore>,
    workspace_root: &str,
    arguments: &Value,
) -> Result<String> {
    let id_str = arguments
        .get("memory_id")
        .and_then(|v| v.as_str())
        .context("memory_id is required for forget operation")?;
    let id = Uuid::parse_str(id_str)
        .map_err(|e| anyhow::anyhow!("invalid memory_id '{}': {}", id_str, e))?;

    memory_store.delete(workspace_root, id)?;
    Ok(format!("Memory {} forgotten.", id_str))
}

/// observations: List raw observations for a session.
fn execute_observations(
    _memory_store: &Arc<MemoryStore>,
    arguments: &Value,
) -> Result<String> {
    let session_id = arguments
        .get("session_id")
        .and_then(|v| v.as_str())
        .context("session_id is required for observations operation")?;

    // This would query observations table - for Phase 1 return placeholder
    Ok(format!("Observations for session {}: (Phase 1 - query via storage layer)", session_id))
}
