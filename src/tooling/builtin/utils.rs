use anyhow::{Context, Result, bail};
use std::path::{Component, Path, PathBuf};

/// Expand a leading tilde (`~` or `~/...`) to the user's home directory.
/// Uses `Path::components` instead of string matching, so it correctly handles
/// both `/` (Unix) and `\` (Windows) path separators.
fn expand_tilde(candidate: &Path) -> Result<PathBuf> {
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

    if !resolved.starts_with(workspace_root) && !allow_outside {
        bail!("path {} escapes the workspace root", candidate.display());
    }

    Ok(resolved)
}

/// Check if a path would escape the workspace root without failing.
/// Returns true if the path is outside the workspace.
///
/// If tilde expansion fails (cannot determine home directory), defaults to
/// treating the path as outside the workspace (safe default triggering a dialog).
pub fn is_path_outside_workspace(workspace_root: &Path, candidate: &Path) -> bool {
    let candidate = match expand_tilde(candidate) {
        Ok(p) => p,
        Err(_) => return true,
    };

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

    !resolved.starts_with(workspace_root)
}

/// Try to resolve a workspace path, returning None if it would escape.
pub fn try_resolve_workspace_path(workspace_root: &Path, candidate: &Path) -> Option<PathBuf> {
    resolve_workspace_path(workspace_root, candidate, false).ok()
}

/// Display a path relative to the workspace root.
/// If the path is outside the workspace, returns the full path.
/// If the path equals the workspace root, returns ".".
pub fn display_workspace_relative(workspace_root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(workspace_root).unwrap_or(path);
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
