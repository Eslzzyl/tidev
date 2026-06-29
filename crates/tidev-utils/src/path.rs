use anyhow::{Result, bail};
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
/// `write`).
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
                _ => return path.to_path_buf(),
            },
        }
    }

    // Safety valve: if we walked 256 levels up, return as-is.
    path.to_path_buf()
}

/// Expand `~` or `~/...` to the user's home directory.
pub fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
        && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
    {
        return home.join(rest.trim_start_matches('/').trim_start_matches('\\'));
    }
    path.to_path_buf()
}

/// Resolve a path against a workspace root.
///
/// - If `candidate` is relative, it is joined with `workspace_root`.
/// - If `candidate` starts with `~`, tilde is expanded first.
/// - The result is normalized (`.`, `..` resolved).
/// - If `allow_outside` is false, paths that escape the workspace are rejected.
pub fn resolve_workspace_path(
    workspace_root: &Path,
    candidate: &Path,
    allow_outside: bool,
) -> Result<PathBuf> {
    let expanded = expand_tilde(candidate);

    let resolved = if expanded.is_absolute() {
        expanded
    } else {
        workspace_root.join(&expanded)
    };

    // Normalize `.` and `..`
    let resolved = normalize_path(&resolved);

    if !allow_outside {
        let canonical_resolved = canonicalize_for_comparison(&resolved);
        let canonical_root = canonicalize_for_comparison(workspace_root);
        if !canonical_resolved.starts_with(&canonical_root) {
            bail!("path {} escapes the workspace root", candidate.display());
        }
    }

    Ok(resolved)
}

/// Check if a path would escape the workspace root without failing.
/// Returns true if the path is outside the workspace.
///
/// If tilde expansion fails (cannot determine home directory), defaults to
/// treating the path as outside the workspace (safe default triggering a dialog).
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

/// Resolve a path against the workspace root, expanding tilde and normalizing,
/// but **without** checking whether it escapes the workspace boundary.
///
/// This is useful when you need the resolved absolute path for display or
/// further processing, but want to handle boundary violations separately.
pub fn resolve_path_unchecked(
    workspace_root: &Path,
    candidate: &Path,
) -> Result<PathBuf> {
    let expanded = expand_tilde(candidate);

    let resolved = if expanded.is_absolute() {
        expanded
    } else {
        workspace_root.join(&expanded)
    };

    // Normalize `.` and `..`
    Ok(normalize_path(&resolved))
}

/// Display a path relative to the workspace root for user-facing messages.
///
/// If the path is inside the workspace, returns the relative form as a String.
/// If the path is outside or cannot be relativized, returns the
/// canonicalized absolute form.
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

/// Normalize a path by resolving `.` and `..` components without touching the
/// filesystem.
fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            Component::ParentDir => {
                // Pop the last non-root component if possible
                match components.last() {
                    Some(Component::Normal(_)) => {
                        components.pop();
                    }
                    Some(Component::RootDir) | Some(Component::Prefix(_)) => {
                        // Cannot go above root
                    }
                    _ => {}
                }
            }
            Component::CurDir => {
                // Skip `.` components
            }
            other => {
                components.push(other);
            }
        }
    }

    components.iter().collect()
}

/// Truncate a string in place at a UTF-8 safe boundary, appending a
/// `[truncated]` marker if the string exceeds `max_bytes`.
pub fn truncate_in_place(value: &mut String, max_bytes: usize) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_expand_tilde_no_tilde() {
        let path = PathBuf::from("/usr/local/bin");
        assert_eq!(expand_tilde(&path), path);
    }

    #[test]
    fn test_expand_tilde_home() {
        let expanded = expand_tilde(Path::new("~"));
        assert!(expanded != PathBuf::from("~"));
        // Should be the actual home directory
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home);
        }
    }

    #[test]
    fn test_expand_tilde_subpath() {
        let expanded = expand_tilde(Path::new("~/foo/bar"));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home.join("foo/bar"));
        }
    }

    #[test]
    fn test_normalize_path_simple() {
        assert_eq!(
            normalize_path(Path::new("/a/b/../c")),
            PathBuf::from("/a/c")
        );
    }

    #[test]
    fn test_normalize_path_dot() {
        assert_eq!(
            normalize_path(Path::new("/a/./b/./c")),
            PathBuf::from("/a/b/c")
        );
    }

    #[test]
    fn test_normalize_path_parent_at_root() {
        // Cannot go above root
        assert_eq!(normalize_path(Path::new("/../a")), PathBuf::from("/a"));
    }

    #[test]
    fn test_resolve_workspace_path_relative() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let result = resolve_workspace_path(ws, Path::new("src/main.rs"), false).unwrap();
        assert_eq!(result, ws.join("src/main.rs"));
    }

    #[test]
    fn test_resolve_workspace_path_absolute() {
        let result = resolve_workspace_path(
            Path::new("/workspace"),
            Path::new("/other/file.txt"),
            true,
        )
        .unwrap();
        assert_eq!(result, PathBuf::from("/other/file.txt"));
    }

    #[test]
    fn test_resolve_workspace_path_outside() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        let result = resolve_workspace_path(ws, Path::new("../../etc/passwd"), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_path_unchecked_outside() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        // Should NOT error even though path is outside workspace
        let result = resolve_path_unchecked(ws, Path::new("../../etc/passwd"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_path_outside_workspace() {
        let dir = tempdir().unwrap();
        let ws = dir.path();
        // Create a file inside workspace
        fs::write(ws.join("inside.txt"), "hello").unwrap();
        assert!(!is_path_outside_workspace(ws, Path::new("inside.txt")));
        assert!(is_path_outside_workspace(ws, Path::new("../../etc/passwd")));
    }

    #[test]
    fn test_display_workspace_relative_inside() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("sub/file.txt");
        let result = display_workspace_relative(dir.path(), &file);
        let expected = std::path::Path::new("sub").join("file.txt").display().to_string();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_display_workspace_relative_root() {
        let dir = tempdir().unwrap();
        let result = display_workspace_relative(dir.path(), dir.path());
        assert_eq!(result, ".");
    }

    #[test]
    fn test_display_workspace_relative_outside() {
        let dir = tempdir().unwrap();
        let outside = Path::new("/tmp/outside.txt");
        let result = display_workspace_relative(dir.path(), outside);
        // Outside paths are returned as-is
        assert_eq!(result, "/tmp/outside.txt");
    }

    #[test]
    fn test_display_workspace_relative_non_existent_inside() {
        let dir = tempdir().unwrap();
        let non_existent = dir.path().join("does/not/exist.rs");
        let result = display_workspace_relative(dir.path(), &non_existent);
        let expected = Path::new("does").join("not").join("exist.rs").display().to_string();
        assert_eq!(result, expected);
    }

    #[cfg(unix)]
    #[test]
    fn test_display_workspace_relative_symlinked_root() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real_ws");
        fs::create_dir_all(real.join("sub")).unwrap();
        let ws_symlink = dir.path().join("project_link");
        std::os::unix::fs::symlink(&real, &ws_symlink).unwrap();

        // An existing file inside through the symlink root
        let existing = real.join("sub/file.txt");
        let result = display_workspace_relative(&ws_symlink, &existing);
        assert_eq!(result, "sub/file.txt");

        // A non-existent file inside through the symlink root
        // (this is the case that failed on macOS: /tmp → /private/tmp)
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

        // Absolute path via the real (non-symlink) path should still be relativized
        let result = display_workspace_relative(&ws_symlink, &real.join("sub/file.txt"));
        assert_eq!(result, "sub/file.txt");
    }
}
