use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Patterns of files/directories in /tmp that tidev is responsible for.
const TIDEV_PREFIXES: &[&str] = &["tidev-"];

/// Result entry for a file/dir that would be or was cleaned.
#[derive(Debug, Clone)]
pub struct CleanEntry {
    pub path: PathBuf,
    pub age_secs: u64,
}

/// Scan /tmp for known tidev temp files.
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
/// Returns the list of entries that were actually removed.
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

/// Perform auto-cleanup on startup based on config.
pub fn auto_cleanup(config: &crate::config::TmpConfig) {
    if !config.auto_cleanup {
        return;
    }

    let max_age = Duration::from_secs(config.max_age_hours * 3600);
    match clean_temp_files(max_age, false) {
        Ok(removed) => {
            if !removed.is_empty() {
                crate::log_info!(
                    "Cleaned up {} temp file(s) older than {}h",
                    removed.len(),
                    config.max_age_hours
                );
            }
        }
        Err(e) => {
            crate::log_warn!("Failed to auto-clean temp files: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Creates a temp file in /tmp with a unique name for testing.
    fn create_test_temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tidev-test-{name}-{}", uuid::Uuid::new_v4()));
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
