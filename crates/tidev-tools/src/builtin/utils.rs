//! Utility functions used by builtin tools.

use std::collections::HashSet;

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::Path;

use crate::types::ToolArgs;

// ---------------------------------------------------------------------------
// Tool-specific utilities
// ---------------------------------------------------------------------------

/// Read a file's text content, treating a missing file as empty.
pub(super) fn read_existing_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

/// Truncate a string in-place at a UTF-8 boundary, appending `[truncated]`.
pub(super) fn truncate_in_place(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }

    value.truncate(end);
    value.push_str("\n[truncated]");
}

// ---------------------------------------------------------------------------
// Re-exports from tidev-utils (path utilities)
// ---------------------------------------------------------------------------

pub(super) use tidev_utils::path::display_workspace_relative;
pub(super) use tidev_utils::path::resolve_workspace_path;

// ---------------------------------------------------------------------------
// Argument decoding (from old tooling/tools.rs)
// ---------------------------------------------------------------------------

/// Decode tool arguments with an enhanced error message that includes the
/// JSON Schema field descriptions so the model can self-correct.
pub(super) fn decode_tool_args<Args: ToolArgs>(tool_name: &str, arguments: Value) -> Result<Args> {
    let schema = Args::schema();
    serde_json::from_value::<Args>(arguments).map_err(|e| {
        let expected = describe_schema(&schema);
        anyhow::anyhow!(
            "failed to decode arguments for tool '{}': {}\nExpected fields: {}",
            tool_name,
            e,
            expected
        )
    })
}

/// Thin wrapper around decode_tool_args, provided for compatibility.
pub fn parse_arguments<Args>(tool_name: &str, arguments: Value) -> Result<Args>
where
    Args: ToolArgs,
{
    decode_tool_args::<Args>(tool_name, arguments)
}

/// Convert a JSON Schema object to a human-readable field description string.
/// Example output: `path: string, old_text: string, new_text: string, replace_all (optional): boolean`
fn describe_schema(schema: &Value) -> String {
    let properties = match schema.get("properties").and_then(|v| v.as_object()) {
        Some(props) => props,
        None => return String::new(),
    };

    let required: HashSet<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut fields: Vec<String> = Vec::with_capacity(properties.len());
    for (name, prop_schema) in properties {
        let field_type = prop_schema
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("any");
        let optional = if required.contains(name.as_str()) {
            String::new()
        } else {
            " (optional)".to_string()
        };
        fields.push(format!("{}{}: {}", name, optional, field_type));
    }
    fields.join(", ")
}

/// Extract the file path from a codex-format patch string.
/// Returns the first file path found, or None if parsing fails.
/// Looks for `*** Add File:`, `*** Update File:`, `*** Delete File:` markers.
pub use tidev_utils::path::extract_file_path_from_patch;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_in_place_no_truncation() {
        let mut s = "hello".to_string();
        truncate_in_place(&mut s, 100);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_truncate_in_place_exact() {
        let mut s = "hello".to_string();
        truncate_in_place(&mut s, 5);
        assert_eq!(s, "hello");
    }

    #[test]
    fn test_truncate_in_place_truncates() {
        let mut s = "hello world this is a long string".to_string();
        truncate_in_place(&mut s, 10);
        assert!(s.ends_with("[truncated]"));
        assert!(s.len() < "hello world this is a long string".len());
    }

    #[test]
    fn test_truncate_in_place_utf8_boundary() {
        let mut s = "héllo wörld".to_string();
        truncate_in_place(&mut s, 6);
        assert!(s.ends_with("[truncated]"));
    }

    #[test]
    fn test_read_existing_text_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "content").unwrap();
        assert_eq!(read_existing_text(&path).unwrap(), "content");
    }

    #[test]
    fn test_read_existing_text_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.txt");
        assert_eq!(read_existing_text(&path).unwrap(), "");
    }

    #[test]
    fn test_read_existing_text_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir");
        std::fs::create_dir(&path).unwrap();
        assert!(read_existing_text(&path).is_err());
    }

    #[test]
    fn test_extract_file_path_from_patch_add() {
        let patch = "*** Begin Patch\n*** Add File: src/main.rs\n+content\n*** End Patch";
        assert_eq!(
            extract_file_path_from_patch(patch),
            Some("src/main.rs".to_string())
        );
    }

    #[test]
    fn test_extract_file_path_from_patch_update() {
        let patch = "*** Begin Patch\n*** Update File: Cargo.toml\n@@\n-old\n+new\n*** End Patch";
        assert_eq!(
            extract_file_path_from_patch(patch),
            Some("Cargo.toml".to_string())
        );
    }

    #[test]
    fn test_extract_file_path_from_patch_delete() {
        let patch = "*** Begin Patch\n*** Delete File: old.txt\n*** End Patch";
        assert_eq!(
            extract_file_path_from_patch(patch),
            Some("old.txt".to_string())
        );
    }

    #[test]
    fn test_extract_file_path_from_patch_none() {
        let patch = "*** Begin Patch\n*** End Patch";
        assert_eq!(extract_file_path_from_patch(patch), None);
    }
}
