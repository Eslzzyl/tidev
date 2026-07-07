//! Path utility functions for tidev.
//!
//! Provides path canonicalization, workspace-boundary checking,
//! and display helpers.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Maximum number of ancestor directories to walk up when canonicalizing a
/// non-existent path. This prevents infinite loops on deeply nested paths.
const CANONICALIZE_MAX_DEPTH: usize = 256;

/// Canonicalize a path, stripping the Windows `\\?\` extended-length prefix
/// so the result is comparable with paths from [`std::env::current_dir`]
/// and displays cleanly to users.
///
/// On Windows, [`std::fs::canonicalize`] returns paths prefixed with `\\?\`
/// (e.g. `\\?\C:\Users\foo\project`), while [`std::env::current_dir`] returns
/// normal paths (e.g. `C:\Users\foo\project`).  Mixing the two causes
/// [`Path::strip_prefix`] to fail and produces ugly `\\?\` paths in UI
/// messages.  This function uses the [`dunce`] crate to remove the prefix
/// when it is safe to do so.
///
/// Falls back to the original path if canonicalization fails (e.g. the path
/// does not yet exist).
pub fn canonicalize_display(path: &Path) -> PathBuf {
    dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Canonicalize a path for **boundary comparison**, resolving all symlinks.
///
/// Unlike [`canonicalize_display`] which falls back to the raw path on failure,
/// this function walks up parent directories until it finds an existing ancestor,
/// canonicalizes that, and appends the non-existent tail components. This makes
/// it suitable for paths that do not yet exist (e.g., files to be created by
/// a `write` tool).
///
/// Uses [`dunce`] on Windows so the result is comparable with paths from
/// [`std::env::current_dir`].
///
/// Returns the canonical path on success, or the original path as a last resort.
pub fn canonicalize_for_comparison(path: &Path) -> PathBuf {
    if let Ok(canonical) = dunce::canonicalize(path) {
        return canonical;
    }

    // Walk up ancestors until we find one that exists, then append the
    // non-existent components we peeled off.
    let mut components: Vec<&OsStr> = Vec::new();
    let mut current = path;
    for _ in 0..CANONICALIZE_MAX_DEPTH {
        match dunce::canonicalize(current) {
            Ok(canonical) => {
                let mut result = canonical;
                for c in components.iter().rev() {
                    result.push(c);
                }
                return result;
            }
            Err(_) => match (current.file_name(), current.parent()) {
                (Some(name), Some(parent)) => {
                    components.push(name);
                    current = parent;
                }
                _ => return path.to_path_buf(), // give up, return as-is
            },
        }
    }

    // Depth limit reached — return the original path as-is
    path.to_path_buf()
}

/// Expand a leading tilde (`~` or `~/...`) to the user's home directory.
///
/// Uses `Path::components` instead of string matching, so it correctly handles
/// both `/` (Unix) and `\` (Windows) path separators.
pub fn expand_tilde(candidate: &Path) -> Result<PathBuf> {
    let mut components = candidate.components();
    match components.next() {
        Some(Component::Normal(part)) if part == "~" => {
            let home = dirs::home_dir().context("could not determine home directory")?;
            let mut result = home;
            for component in components {
                result.push(component.as_os_str());
            }
            Ok(result)
        }
        _ => Ok(candidate.to_path_buf()),
    }
}

/// Resolve a candidate path against the workspace root, normalizing `.` and
/// `..` components and expanding `~`.
///
/// If `allow_outside` is `false`, returns an error when the resolved path
/// escapes the workspace root.
pub fn resolve_workspace_path(
    workspace_root: &Path,
    candidate: &Path,
    allow_outside: bool,
) -> Result<PathBuf> {
    let resolved = resolve_path_unchecked(workspace_root, candidate)?;

    if !allow_outside {
        // Canonicalise both sides to handle symlinks:
        //   - workspace_root may itself be a symlink
        //   - the resolved path may traverse symlinks that escape the workspace
        let canonical_resolved = canonicalize_for_comparison(&resolved);
        let canonical_root = canonicalize_for_comparison(workspace_root);
        if !canonical_resolved.starts_with(&canonical_root) {
            bail!("path {} escapes the workspace root", candidate.display());
        }
    }

    Ok(resolved)
}

/// Check if a path would escape the workspace root without failing.
///
/// Returns `true` if the path is outside the workspace. If tilde expansion
/// fails (cannot determine home directory), defaults to `true` (safe default
/// triggering a dialog).
pub fn is_path_outside_workspace(workspace_root: &Path, candidate: &Path) -> bool {
    match resolve_path_unchecked(workspace_root, candidate) {
        Ok(resolved) => {
            let canonical_resolved = canonicalize_for_comparison(&resolved);
            let canonical_root = canonicalize_for_comparison(workspace_root);
            !canonical_resolved.starts_with(&canonical_root)
        }
        Err(_) => true,
    }
}

/// Try to resolve a workspace path, returning `None` if it would escape.
pub fn try_resolve_workspace_path(workspace_root: &Path, candidate: &Path) -> Option<PathBuf> {
    resolve_workspace_path(workspace_root, candidate, false).ok()
}

/// Resolve a path against workspace root, normalizing `.` and `..` components
/// and expanding `~`. Returns the resolved absolute path without checking
/// whether it stays inside the workspace.
///
/// This is useful for workspace boundary permission keys: regardless of the
/// path representation used by the tool call (relative, absolute, with `..`),
/// the resolved path is always the same, so permissions can be matched reliably.
pub fn resolve_path_unchecked(workspace_root: &Path, candidate: &Path) -> Result<PathBuf> {
    let candidate = expand_tilde(candidate)?;

    let mut resolved = if candidate.is_absolute() {
        PathBuf::new()
    } else {
        workspace_root.to_path_buf()
    };

    for component in candidate.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => resolved.push(part),
            Component::RootDir => resolved.push(component.as_os_str()),
            Component::Prefix(prefix) => resolved.push(prefix.as_os_str()),
        }
    }

    Ok(resolved)
}

/// Display a path relative to the workspace root.
///
/// If the path is outside the workspace, returns the full path.
/// If the path equals the workspace root, returns `"."`.
///
/// Both paths are canonicalized (stripping the Windows `\\?\` prefix) before
/// comparison, so that paths from different sources (e.g.
/// [`std::env::current_dir`] vs [`std::fs::canonicalize`]) can be compared
/// reliably.
///
/// Unlike the display-only canonicalization, the input `path` uses
/// [`canonicalize_for_comparison`] which walks up parent directories to
/// resolve symlinks even when the path does not yet exist on disk. This
/// prevents issues such as `/tmp/example.txt` being shown as-is on macOS
/// where `/tmp` → `/private/tmp`.
pub fn display_workspace_relative(workspace_root: &Path, path: &Path) -> String {
    let root = canonicalize_display(workspace_root);
    let canonical_path = canonicalize_for_comparison(path);
    let relative = canonical_path.strip_prefix(&root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative.display().to_string()
    }
}

/// Extract the file path from a unified-diff header line like
/// `*** Add File: src/main.rs` or `*** Update File: Cargo.toml`.
pub fn extract_file_path_from_patch(patch: &str) -> Option<String> {
    const ADD_MARKER: &str = "*** Add File: ";
    const UPDATE_MARKER: &str = "*** Update File: ";
    const DELETE_MARKER: &str = "*** Delete File: ";

    for line in patch.lines() {
        let trimmed = line.trim();
        if let Some(path) = trimmed
            .strip_prefix(ADD_MARKER)
            .or_else(|| trimmed.strip_prefix(UPDATE_MARKER))
            .or_else(|| trimmed.strip_prefix(DELETE_MARKER))
        {
            return Some(path.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Sensitive file detection
// ---------------------------------------------------------------------------

const SENSITIVE_FILE_NAME: &str = ".tidev/sensitive.txt";

/// Load sensitive-file glob patterns from `.tidev/sensitive.txt`.
///
/// Returns the patterns as strings exactly as they appear in the file
/// (comments and blank lines excluded).  Patterns are relative to the
/// workspace root — they will be matched against absolute paths.
pub fn load_sensitive_patterns(workspace_root: &Path) -> Vec<String> {
    let path = workspace_root.join(SENSITIVE_FILE_NAME);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// Check whether `resolved_path` matches any of the sensitive-file patterns.
pub fn is_path_sensitive(workspace_root: &Path, resolved_path: &Path, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return false;
    }

    for pattern_str in patterns {
        let abs_pattern = workspace_root.join(pattern_str);
        let pattern_str = abs_pattern.to_string_lossy();

        match globset::Glob::new(&pattern_str) {
            Ok(glob) => {
                let matcher = glob.compile_matcher();
                if matcher.is_match(resolved_path) {
                    return true;
                }
            }
            Err(_) => continue,
        }
    }

    false
}

// ---------------------------------------------------------------------------
// ToolCall analysis helpers (for TUI security dialogs / backend approval)
// ---------------------------------------------------------------------------

/// Extract the first file path from a tool call's arguments that would violate
/// workspace boundaries. Returns the resolved absolute path so it can be used
/// as a consistent key for permission lookups.
///
/// Supports `read`, `write`, `edit` (field `file_path`), `glob`, `grep`
/// (field `path`), and `apply_patch` (extracts path from patch header).
/// Returns `None` if the tool call does not reference any path outside the
/// workspace (e.g. `bash`).
pub fn extract_boundary_violation_path(
    workspace_root: &Path,
    tool_name: &str,
    arguments: &Value,
) -> Option<PathBuf> {
    let canonical_name = tidev_types::tools::canonical_tool_name(tool_name)?;

    let path_buf: PathBuf = match canonical_name {
        "read" | "write" | "edit" | "glob" | "grep" => {
            let path_str = arguments
                .get("file_path")
                .or_else(|| arguments.get("path"))?
                .as_str()?;
            PathBuf::from(path_str)
        }
        "apply_patch" => {
            let patch = arguments.get("patch_text")?.as_str()?;
            PathBuf::from(extract_file_path_from_patch(patch)?)
        }
        "bash" => return None,
        _ => return None,
    };

    if !is_path_outside_workspace(workspace_root, &path_buf) {
        return None;
    }

    let resolved = resolve_path_unchecked(workspace_root, &path_buf)
        .unwrap_or_else(|_| path_buf);

    Some(canonicalize_for_comparison(&resolved))
}

/// Extract the file path from a tool call's arguments that would match a
/// sensitive-file pattern. Returns the resolved path, or `None` if the
/// tool call does not target a sensitive file.
pub fn extract_sensitive_file_path(
    workspace_root: &Path,
    tool_name: &str,
    arguments: &Value,
    sensitive_patterns: &[String],
) -> Option<PathBuf> {
    if sensitive_patterns.is_empty() {
        return None;
    }

    let canonical_name = tidev_types::tools::canonical_tool_name(tool_name)?;

    let path_buf: PathBuf = match canonical_name {
        "read" => {
            let path_str = arguments.get("file_path")?.as_str()?;
            PathBuf::from(path_str)
        }
        _ => return None,
    };

    let resolved = resolve_path_unchecked(workspace_root, &path_buf)
        .unwrap_or_else(|_| path_buf);

    // Check against sensitive patterns using the existing logic.
    let resolved_str = resolved.to_string_lossy();
    for pattern_str in sensitive_patterns {
        let abs_pattern = workspace_root.join(pattern_str);
        let abs_str = abs_pattern.to_string_lossy();
        if let Ok(glob) = globset::Glob::new(&abs_str) {
            let matcher = glob.compile_matcher();
            if matcher.is_match(resolved_str.as_ref()) {
                return Some(resolved);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ─── canonicalize_for_comparison ─────────────────────────────────

    #[test]
    fn test_canonicalize_for_comparison_non_existent() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("new.rs");

        let result = canonicalize_for_comparison(&nested);
        assert!(
            result.ends_with("a/b/new.rs"),
            "non-existent tail should be preserved"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_canonicalize_for_comparison_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("link_to_target");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = canonicalize_for_comparison(&link);
        assert_eq!(fs::canonicalize(&target).unwrap(), result);
    }

    // ─── resolve_workspace_path ──────────────────────────────────────

    #[test]
    fn test_resolve_relative_path() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let file = ws.join("test.txt");
        fs::write(&file, "hello").unwrap();

        let result = resolve_workspace_path(&ws, Path::new("test.txt"), false).unwrap();
        assert!(result.ends_with("test.txt"));
    }

    #[test]
    fn test_resolve_absolute_path_inside_workspace() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let file = ws.join("test.txt");
        fs::write(&file, "hello").unwrap();

        let result = resolve_workspace_path(&ws, &file, false).unwrap();
        assert!(result.ends_with("test.txt"));
    }

    #[test]
    fn test_resolve_path_outside_workspace_rejected() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, "hello").unwrap();

        let result = resolve_workspace_path(&ws, &outside, false);
        assert!(result.is_err(), "outside path should be rejected");
    }

    #[test]
    fn test_resolve_path_outside_workspace_allowed() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let outside = dir.path().join("outside.txt");
        fs::write(&outside, "hello").unwrap();

        let result = resolve_workspace_path(&ws, &outside, true);
        assert!(
            result.is_ok(),
            "allow_outside=true should permit external paths"
        );
    }

    // ─── display_workspace_relative ─────────────────────────────────

    #[test]
    fn test_display_workspace_relative_inside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir_all(ws.join("sub")).unwrap();
        fs::write(ws.join("sub").join("file.txt"), "data").unwrap();

        let result = display_workspace_relative(&ws, &ws.join("sub/file.txt"));
        assert_eq!(result, "sub/file.txt");
    }

    #[test]
    fn test_display_workspace_relative_root() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();

        let result = display_workspace_relative(&ws, &ws);
        assert_eq!(result, ".");
    }

    #[test]
    fn test_display_workspace_relative_non_existent_file() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let result = display_workspace_relative(&ws, &ws.join("new_file.rs"));
        assert_eq!(result, "new_file.rs");
    }

    #[cfg(unix)]
    #[test]
    fn test_display_workspace_relative_symlink_root() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real_ws");
        fs::create_dir(&real).unwrap();
        let ws_symlink = dir.path().join("project_link");
        std::os::unix::fs::symlink(&real, &ws_symlink).unwrap();

        let result = display_workspace_relative(&ws_symlink, &ws_symlink);
        assert_eq!(result, ".");
    }

    #[cfg(unix)]
    #[test]
    fn test_display_workspace_relative_non_existent_through_symlink() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real_ws");
        fs::create_dir(&real).unwrap();
        let ws_symlink = dir.path().join("project_link");
        std::os::unix::fs::symlink(&real, &ws_symlink).unwrap();

        let non_existent = real.join("new_file.rs");
        let result = display_workspace_relative(&ws_symlink, &non_existent);
        assert_eq!(result, "new_file.rs");
    }

    #[cfg(unix)]
    #[test]
    fn test_display_workspace_relative_absolute_path_through_symlink() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real_ws");
        fs::create_dir_all(real.join("sub")).unwrap();
        let ws_symlink = dir.path().join("project_link");
        std::os::unix::fs::symlink(&real, &ws_symlink).unwrap();

        let result = display_workspace_relative(&ws_symlink, &real.join("sub/file.txt"));
        assert_eq!(result, "sub/file.txt");
    }
}
