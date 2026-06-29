use anyhow::{Context, Result};

// Re-export path utilities from tidev-utils so that existing
// `super::utils::*` paths inside tidev-tools continue to work.
pub use tidev_utils::path::*;
// Re-export truncate_in_place for internal use (search.rs, exec.rs).
pub use tidev_utils::process::truncate_in_place;

/// Read a file as text, returning an empty string if it does not exist.
pub(super) fn read_existing_text(path: &std::path::Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_existing_text_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        std::fs::write(&file, "hello").unwrap();
        let result = read_existing_text(&file).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_read_existing_text_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("does_not_exist.txt");
        let result = read_existing_text(&file).unwrap();
        assert_eq!(result, "");
    }
}
