//! Sensitive file detection for the `read` tool.
//!
//! Users list sensitive file patterns in `<workspace>/.tidev/sensitive.txt`,
//! one per line (supports glob patterns).  When the `read` tool tries to
//! access a matching path, the frontend shows a confirmation dialog.

use globset::Glob;
use std::path::Path;

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
        Err(_) => return Vec::new(), // file doesn't exist or can't be read
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

/// Check whether `resolved_path` (an absolute path inside the workspace)
/// matches any of the sensitive-file patterns.
///
/// Each pattern is interpreted relative to `workspace_root` and compiled
/// as a glob.  The resolved path is used as-is (already absolute after
/// `resolve_workspace_path`), without canonicalization, to avoid
/// symlink-resolution differences (e.g. `/var` → `/private/var` on macOS)
/// that would break matching against patterns joined to `workspace_root`.
pub fn is_path_sensitive(
    workspace_root: &Path,
    resolved_path: &Path,
    patterns: &[String],
) -> bool {
    if patterns.is_empty() {
        return false;
    }

    for pattern_str in patterns {
        // Build absolute glob pattern by joining with workspace root
        let abs_pattern = workspace_root.join(pattern_str);
        let pattern_str = abs_pattern.to_string_lossy();

        match Glob::new(&pattern_str) {
            Ok(glob) => {
                let matcher = glob.compile_matcher();
                if matcher.is_match(resolved_path) {
                    return true;
                }
            }
            Err(_) => {
                // Invalid glob — skip silently (user can fix the file)
                continue;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn setup_test_workspace() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        (dir, root)
    }

    #[test]
    fn test_no_file_returns_empty() {
        let (_tmp, root) = setup_test_workspace();
        let patterns = load_sensitive_patterns(&root);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_load_single_pattern() {
        let (_tmp, root) = setup_test_workspace();
        let sensitive_file = root.join(".tidev").join("sensitive.txt");
        fs::create_dir_all(root.join(".tidev")).unwrap();
        fs::write(&sensitive_file, ".env\n# comment\n*.pem\n").unwrap();

        let patterns = load_sensitive_patterns(&root);
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0], ".env");
        assert_eq!(patterns[1], "*.pem");
    }

    #[test]
    fn test_is_path_sensitive_exact_match() {
        let (_tmp, root) = setup_test_workspace();
        let patterns = vec![".env".to_string()];

        // Create the file so canonicalize works
        fs::write(root.join(".env"), "SECRET=1").unwrap();
        let resolved = root.join(".env");

        assert!(is_path_sensitive(&root, &resolved, &patterns));
    }

    #[test]
    fn test_is_path_sensitive_glob_match() {
        let (_tmp, root) = setup_test_workspace();
        let patterns = vec!["*.pem".to_string()];

        fs::write(root.join("key.pem"), "private").unwrap();
        let resolved = root.join("key.pem");

        assert!(is_path_sensitive(&root, &resolved, &patterns));
    }

    #[test]
    fn test_is_path_sensitive_no_match() {
        let (_tmp, root) = setup_test_workspace();
        let patterns = vec![".env".to_string()];

        fs::write(root.join("README.md"), "docs").unwrap();
        let resolved = root.join("README.md");

        assert!(!is_path_sensitive(&root, &resolved, &patterns));
    }

    #[test]
    fn test_is_path_sensitive_empty_patterns() {
        let (_tmp, root) = setup_test_workspace();
        let patterns: Vec<String> = Vec::new();

        fs::write(root.join(".env"), "SECRET=1").unwrap();
        let resolved = root.join(".env");

        assert!(!is_path_sensitive(&root, &resolved, &patterns));
    }

    #[test]
    fn test_is_path_sensitive_subdirectory() {
        let (_tmp, root) = setup_test_workspace();
        let patterns = vec!["secrets/*".to_string()];

        fs::create_dir_all(root.join("secrets")).unwrap();
        fs::write(root.join("secrets").join("token"), "abc123").unwrap();
        let resolved = root.join("secrets").join("token");

        assert!(is_path_sensitive(&root, &resolved, &patterns));
    }
}
