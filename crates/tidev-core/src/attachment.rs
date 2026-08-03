//! Attachment building for `@`-reference file paths.
//!
//! These functions convert user-supplied file/directory references (from
//! the Composer's `@mention` autocomplete or inline `@path` text) into
//! [`MessageAttachment`] values suitable for submission with a prompt.
//!
//! Every frontend (TUI, CLI, web) that wants to support `@`-references
//! should call these functions rather than reimplementing the file-read
//! and truncation logic.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use tidev_llm::message::MessageAttachment;

/// Build [`MessageAttachment`] values for a list of `@`-referenced paths.
///
/// Each path is resolved against `workspace_root`.  The result preserves
/// the input order and silently skips paths that cannot be read (logging
/// a warning).
///
/// **Directories**   → [`MessageAttachment::DirectoryReference`] with a
///                     formatted tree (max depth 2, max 80 entries).
///
/// **Images**        → [`MessageAttachment::Image`] with raw bytes, mime
///                     type, and file size.
///
/// **Text files**    → [`MessageAttachment::FileReference`] with full
///                     content for display and a truncated tool-output
///                     snippet for the agent context.
pub fn build_attachments(workspace_root: &Path, paths: &[String]) -> Vec<MessageAttachment> {
    let mut attachments = Vec::with_capacity(paths.len());
    let mut seen = std::collections::BTreeSet::new();

    for path in paths {
        if !seen.insert(path.clone()) {
            continue;
        }
        match build_one(workspace_root, path) {
            Ok(Some(attachment)) => attachments.push(attachment),
            Ok(None) => {}
            Err(e) => log::warn!("build_attachment skipped {path}: {e}"),
        }
    }

    attachments
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a single attachment for one `@`-reference path.
fn build_one(workspace_root: &Path, relative: &str) -> Result<Option<MessageAttachment>> {
    use tidev_utils::path::resolve_workspace_path;

    let absolute = resolve_workspace_path(workspace_root, Path::new(relative), false)
        .with_context(|| format!("failed to resolve path: {relative}"))?;

    let metadata = match std::fs::metadata(&absolute) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("build_attachment: cannot stat {absolute:?}: {e}");
            return Ok(None);
        }
    };

    // ── Directory ────────────────────────────────────────────────────
    if metadata.is_dir() {
        let tree = build_directory_tree(&absolute, 2, 80)?;
        return Ok(Some(MessageAttachment::DirectoryReference {
            path: relative.trim_end_matches(['/', '\\']).to_string(),
            tree: Arc::new(tree),
        }));
    }

    // ── Image ────────────────────────────────────────────────────────
    if let Some(mime) = image_mime_from_path(&absolute) {
        let bytes = std::fs::read(&absolute)
            .with_context(|| format!("failed to read image {absolute:?}"))?;
        let file_size = bytes.len() as u64;
        let filename = absolute
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(relative)
            .to_string();
        return Ok(Some(MessageAttachment::Image {
            filename,
            mime: mime.to_string(),
            data: bytes,
            file_size,
        }));
    }

    // ── Binary (non-image) ───────────────────────────────────────────
    if is_binary(&absolute) {
        log::warn!("build_attachment: skipping binary file {absolute:?}");
        return Ok(None);
    }

    // ── Text file ────────────────────────────────────────────────────
    let content = std::fs::read_to_string(&absolute)
        .with_context(|| format!("failed to read {absolute:?}"))?;

    // Truncated tool-output snippet for the agent context.
    let tool_output = truncate_file_content(&content);
    let truncated = tool_output.len() < content.len();

    Ok(Some(MessageAttachment::FileReference {
        path: relative.to_string(),
        content: Arc::new(content),
        tool_output: Some(Arc::new(tool_output)),
        truncated,
    }))
}

/// Guess the MIME type of an image from the file extension.
fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Quick binary-detection: look for a null byte in the first 8 KiB.
fn is_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    buf[..n].contains(&0)
}

/// Truncate file content for the tool-output field.
///
/// Mirrors the behaviour of `tidev_tools::read_file_for_at_reference`:
/// show the first 2000 lines capped at 50 KiB total.
fn truncate_file_content(content: &str) -> String {
    const MAX_LINES: usize = 2000;
    const MAX_BYTES: usize = 50 * 1024;

    let mut out = String::with_capacity(content.len().min(MAX_BYTES));
    let mut bytes = 0usize;
    let mut cut = false;

    for (lines, line) in content.lines().enumerate() {
        if lines >= MAX_LINES {
            cut = true;
            break;
        }
        if bytes + line.len() + 1 > MAX_BYTES {
            cut = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
            bytes += 1;
        }
        out.push_str(line);
        bytes += line.len();
    }

    if cut {
        out.push_str("\n\n... (truncated)");
    }

    out
}

/// Build a formatted directory tree string (for DirectoryReference).
fn build_directory_tree(path: &Path, max_depth: usize, max_entries: usize) -> Result<String> {
    let label = path
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.to_string())
        .unwrap_or_else(|| path.display().to_string());

    let mut lines = vec![format!("{label}/")];
    let mut entry_count = 0usize;
    append_directory_tree(
        path,
        1,
        max_depth,
        max_entries,
        &mut entry_count,
        &mut lines,
    )?;
    Ok(lines.join("\n"))
}

fn append_directory_tree(
    path: &Path,
    depth: usize,
    max_depth: usize,
    max_entries: usize,
    entry_count: &mut usize,
    lines: &mut Vec<String>,
) -> Result<()> {
    if depth > max_depth || *entry_count >= max_entries {
        return Ok(());
    }

    let mut entries: Vec<(bool, String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("failed to read directory {}", path.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read entry in {}", path.display()))?;
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to inspect {}", entry.path().display()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push((file_type.is_dir(), name, entry.path()));
    }

    // Sort: directories first, then alphabetical by name.
    entries.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));

    for (is_dir, name, child_path) in entries {
        if *entry_count >= max_entries {
            lines.push(format!("{}...", "  ".repeat(depth)));
            break;
        }

        let indent = "  ".repeat(depth);
        if is_dir {
            lines.push(format!("{indent}{name}/"));
            *entry_count += 1;
            append_directory_tree(
                &child_path,
                depth + 1,
                max_depth,
                max_entries,
                entry_count,
                lines,
            )?;
        } else {
            lines.push(format!("{indent}{name}"));
            *entry_count += 1;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_mime() {
        assert_eq!(
            image_mime_from_path(Path::new("photo.png")),
            Some("image/png")
        );
        assert_eq!(
            image_mime_from_path(Path::new("photo.jpg")),
            Some("image/jpeg")
        );
        assert_eq!(
            image_mime_from_path(Path::new("photo.jpeg")),
            Some("image/jpeg")
        );
        assert_eq!(
            image_mime_from_path(Path::new("photo.webp")),
            Some("image/webp")
        );
        assert_eq!(
            image_mime_from_path(Path::new("photo.gif")),
            Some("image/gif")
        );
        assert_eq!(image_mime_from_path(Path::new("file.txt")), None);
        assert_eq!(image_mime_from_path(Path::new("file")), None);
    }

    #[test]
    fn test_is_binary() {
        // Use the current file as a known-text file.
        assert!(!is_binary(Path::new(file!())));
    }

    #[test]
    fn test_truncate_file_content_short() {
        let text = "hello\nworld\n";
        let result = truncate_file_content(text);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_truncate_file_content_cut() {
        // Build a string that definitely exceeds the internal limit.
        let long = "x".repeat(60 * 1024);
        let result = truncate_file_content(&long);
        assert!(result.len() < long.len(), "expected truncation");
        assert!(result.ends_with("(truncated)"));
    }
}
