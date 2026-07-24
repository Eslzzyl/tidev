//! Temporary file management for tidev.
//!
//! Scans and cleans up tidev-created temp files in the system temp directory.
//! Tidev temp files are identified by the `tidev-` prefix.
//!
//! # Example
//!
//! ```ignore
//! use std::time::Duration;
//! use tidev_utils::tmp;
//!
//! // Clean everything older than 24 hours
//! let removed = tmp::clean_temp_files(Duration::from_secs(24 * 3600), false)?;
//! println!("Removed {} files", removed.len());
//! ```

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Patterns of files/directories in the system temp dir that tidev owns.
const TIDEV_PREFIXES: &[&str] = &["tidev-"];

/// Metadata about a temp file or directory that was found / cleaned.
#[derive(Debug, Clone)]
pub struct CleanEntry {
    /// Full path to the file or directory.
    pub path: PathBuf,
    /// Age of the entry in seconds (best-effort).
    pub age_secs: u64,
}

/// Scan the system temp directory for known tidev temp files.
///
/// Returns all entries matching `tidev-*`, regardless of age.
pub fn scan_temp_files() -> std::io::Result<Vec<CleanEntry>> {
    let now = SystemTime::now();
    let mut entries = Vec::new();

    let tmp_dir = std::env::temp_dir();
    if !tmp_dir.exists() {
        return Ok(entries);
    }

    if let Ok(rd) = std::fs::read_dir(&tmp_dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let fname = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };

            if !TIDEV_PREFIXES.iter().any(|p| fname.starts_with(p)) {
                continue;
            }

            let age = match entry.metadata() {
                Ok(meta) => match meta.created().or(meta.modified()) {
                    Ok(t) => match now.duration_since(t) {
                        Ok(d) => d.as_secs(),
                        Err(_) => 0,
                    },
                    Err(_) => 0,
                },
                Err(_) => 0,
            };

            entries.push(CleanEntry {
                path,
                age_secs: age,
            });
        }
    }

    Ok(entries)
}

/// Remove matching temp files older than `max_age`.
///
/// When `dry_run` is `true` nothing is actually deleted, but the return
/// value still reflects which entries *would* be removed.
///
/// Returns the list of entries that were removed (or would be removed).
pub fn clean_temp_files(max_age: Duration, dry_run: bool) -> std::io::Result<Vec<CleanEntry>> {
    let entries = scan_temp_files()?;
    let mut removed = Vec::new();

    for entry in &entries {
        if entry.age_secs < max_age.as_secs() {
            continue;
        }

        if !dry_run {
            let path = &entry.path;
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(path);
            } else {
                let _ = std::fs::remove_file(path);
            }
        }

        removed.push(entry.clone());
    }

    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_temp(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("tidev-test-{}-{}", name, uuid::Uuid::new_v4()));
        let _ = fs::write(&path, "test");
        path
    }

    #[test]
    fn scan_finds_tidev_prefixed_files() {
        let test_file = create_test_temp("scan-file");
        let test_dir =
            std::env::temp_dir().join(format!("tidev-test-scan-dir-{}", uuid::Uuid::new_v4()));
        let _ = fs::create_dir_all(&test_dir);

        let entries = scan_temp_files().unwrap();
        let found: Vec<_> = entries
            .iter()
            .filter(|e| e.path == test_file || e.path == test_dir)
            .collect();
        assert_eq!(found.len(), 2, "should find both test file and dir");

        let _ = fs::remove_file(&test_file);
        let _ = fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn clean_only_removes_old_files() {
        let test_file = create_test_temp("age-test");
        // Use a very large max_age (1 year) so nothing gets removed
        let removed = clean_temp_files(Duration::from_secs(365 * 24 * 3600), false).unwrap();
        assert!(
            !removed.iter().any(|e| e.path == test_file),
            "new file should not be cleaned with large max_age"
        );
        assert!(test_file.exists(), "file should still exist");
        let _ = fs::remove_file(&test_file);
    }

    #[test]
    fn dry_run_does_not_delete() {
        let test_file = create_test_temp("dryrun");

        let entries_before = scan_temp_files().unwrap();
        let found_before: Vec<_> = entries_before
            .iter()
            .filter(|e| e.path == test_file)
            .collect();

        let removed = clean_temp_files(Duration::ZERO, true).unwrap();
        let found_in_removed: Vec<_> = removed.iter().filter(|e| e.path == test_file).collect();

        // Dry run: file should still exist
        assert!(test_file.exists());

        // If the file was found by scan (which it should be), it should be in dry-run output
        if !found_before.is_empty() {
            assert!(
                !found_in_removed.is_empty(),
                "dry-run should report the file"
            );
        }

        let _ = fs::remove_file(&test_file);
    }
}
