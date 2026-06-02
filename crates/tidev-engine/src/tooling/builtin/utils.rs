use anyhow::{Context, Result, bail};
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
                _ => return path.to_path_buf(), // give up, return as-is
            },
        }
    }

    // Depth limit reached — return the original path as-is
    path.to_path_buf()
}

/// Expand a leading tilde (`~` or `~/...`) to the user's home directory.
/// Uses `Path::components` instead of string matching, so it correctly handles
/// both `/` (Unix) and `\` (Windows) path separators.
pub(crate) fn expand_tilde(candidate: &Path) -> Result<PathBuf> {
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

/// Try to resolve a workspace path, returning None if it would escape.
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
/// If the path is outside the workspace, returns the full path.
/// If the path equals the workspace root, returns ".".
///
/// Both paths are canonicalized for display (stripping the Windows `\\?\`
/// prefix) before comparison, so that paths from different sources
/// (e.g. [`std::env::current_dir`] vs [`std::fs::canonicalize`]) can
/// be compared reliably.
pub fn display_workspace_relative(workspace_root: &Path, path: &Path) -> String {
    let root = canonicalize_display(workspace_root);
    let canonical_path = canonicalize_display(path);
    let relative = canonical_path.strip_prefix(&root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        relative.display().to_string()
    }
}

pub(super) fn read_existing_text(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    // ─── canonicalize_for_comparison ───────────────────────────────

    #[test]
    fn test_canonicalize_for_comparison_existing_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("hello.txt");
        fs::write(&file, "world").unwrap();

        let result = canonicalize_for_comparison(&file);
        assert!(result.ends_with("hello.txt"), "should end with file name");
        assert!(result.is_absolute(), "canonical path should be absolute");
    }

    #[test]
    fn test_canonicalize_for_comparison_non_existent() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("new.rs");

        let result = canonicalize_for_comparison(&nested);
        // The tail should be preserved even though the file doesn't exist
        assert!(
            result.ends_with("a/b/new.rs"),
            "non-existent tail should be preserved"
        );
    }

    #[test]
    fn test_canonicalize_for_comparison_existing_dir() {
        let dir = tempdir().unwrap();
        let result = canonicalize_for_comparison(dir.path());
        assert!(result.is_absolute());
        // Should be the real path (no symlink remains).
        // Use dunce::canonicalize to match the function's behaviour on
        // Windows (where std::fs::canonicalize adds a \\?\ prefix that
        // dunce strips).
        assert_eq!(dunce::canonicalize(dir.path()).unwrap(), result);
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
        // Should resolve the symlink to the target's real path
        assert_eq!(fs::canonicalize(&target).unwrap(), result);
    }

    // ─── resolve_workspace_path ────────────────────────────────────

    #[test]
    fn test_resolve_workspace_path_normal_relative() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir_all(ws.join("sub")).unwrap();

        let result = resolve_workspace_path(&ws, Path::new("sub/file.txt"), false);
        assert!(
            result.is_ok(),
            "relative path inside workspace should be allowed"
        );
        assert_eq!(result.unwrap(), ws.join("sub/file.txt"));
    }

    #[test]
    fn test_resolve_workspace_path_absolute_inside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir_all(ws.join("sub")).unwrap();
        let abs_path = ws.join("sub/file.txt");

        let result = resolve_workspace_path(&ws, &abs_path, false);
        assert!(
            result.is_ok(),
            "absolute path inside workspace should be allowed"
        );
    }

    #[test]
    fn test_resolve_workspace_path_outside_blocked() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();

        let result = resolve_workspace_path(&ws, Path::new("../outside.txt"), false);
        assert!(result.is_err(), "path escaping via .. should be blocked");
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_workspace_path_symlink_escape_blocked() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        // Symlink inside the workspace that points outside
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();

        let result = resolve_workspace_path(&ws, Path::new("link/secret.txt"), false);
        assert!(
            result.is_err(),
            "symlink escape via direct symlink should be blocked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_workspace_path_symlink_to_outside_non_existent() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();

        // Write to a non-existent path through a symlink that escapes
        let result = resolve_workspace_path(&ws, Path::new("link/new_file.txt"), false);
        assert!(
            result.is_err(),
            "writing through symlink escape should be blocked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_workspace_path_workspace_root_is_symlink() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real_workspace");
        fs::create_dir_all(real.join("sub")).unwrap();
        // Workspace root is a symlink to the real directory
        let ws_symlink = dir.path().join("project_link");
        std::os::unix::fs::symlink(&real, &ws_symlink).unwrap();

        // Absolute path using the real path should be allowed
        let result = resolve_workspace_path(&ws_symlink, &real.join("sub/file.txt"), false);
        assert!(
            result.is_ok(),
            "absolute path via real path should be allowed when workspace_root is a symlink"
        );
    }

    #[test]
    fn test_resolve_workspace_path_allow_outside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();

        let result = resolve_workspace_path(&ws, Path::new("/tmp/outside.txt"), true);
        assert!(
            result.is_ok(),
            "allow_outside=true should permit external paths"
        );
    }

    // ─── is_path_outside_workspace ─────────────────────────────────

    #[test]
    fn test_is_path_outside_normal_inside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();

        assert!(!is_path_outside_workspace(&ws, Path::new("test.txt")));
    }

    #[test]
    fn test_is_path_outside_normal_outside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();

        assert!(is_path_outside_workspace(&ws, Path::new("../outside.txt")));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_path_outside_symlink_escape() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir(&ws).unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, ws.join("link")).unwrap();

        assert!(
            is_path_outside_workspace(&ws, Path::new("link")),
            "symlink pointing outside should be detected as outside"
        );
        assert!(
            is_path_outside_workspace(&ws, Path::new("link/secret.txt")),
            "file through symlink escape should be detected as outside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_is_path_outside_symlink_inside_not_detected() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir_all(ws.join("real_dir")).unwrap();
        // Symlink inside the workspace that points to another place inside
        std::os::unix::fs::symlink(ws.join("real_dir"), ws.join("link")).unwrap();

        assert!(
            !is_path_outside_workspace(&ws, Path::new("link")),
            "symlink pointing inside should not be flagged as outside"
        );
    }

    #[test]
    fn test_is_path_outside_absolute_path_inside() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        fs::create_dir_all(ws.join("sub")).unwrap();

        assert!(
            !is_path_outside_workspace(&ws, &ws.join("sub/file.txt")),
            "absolute path inside workspace should not be flagged"
        );
    }

    // ─── resolve_path_unchecked (unchanged behaviour) ───────────────

    #[test]
    fn test_resolve_path_unchecked_relative() {
        let ws = Path::new("/home/user/project");
        let result = resolve_path_unchecked(ws, Path::new("src/main.rs")).unwrap();
        assert_eq!(result, Path::new("/home/user/project/src/main.rs"));
    }

    #[test]
    fn test_resolve_path_unchecked_absolute() {
        let ws = Path::new("/home/user/project");
        let result =
            resolve_path_unchecked(ws, Path::new("/home/user/project/src/main.rs")).unwrap();
        assert_eq!(result, Path::new("/home/user/project/src/main.rs"));
    }

    #[test]
    fn test_resolve_path_unchecked_with_parent_dir() {
        let ws = Path::new("/home/user/project");
        let result = resolve_path_unchecked(ws, Path::new("src/../lib/utils.rs")).unwrap();
        assert_eq!(result, Path::new("/home/user/project/lib/utils.rs"));
    }
}
