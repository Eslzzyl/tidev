use super::*;

/// Classify tar — -t/-tf is list (read-only), everything else writes.
pub(super) fn classify_tar(args: &[&str]) -> Safety {
    // Look for `-t` or `--list` anywhere in args (including combined: -tf, -tvf, -vtf)
    let is_list = args
        .iter()
        .any(|a| a == &"-t" || a == &"--list" || a.starts_with("-t"));
    if is_list {
        Safety::ReadOnly
    } else {
        Safety::WriteOperation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_list_is_read_only() {
        assert_eq!(classify_tar(&["-tf", "archive.tar"]), Safety::ReadOnly);
        assert_eq!(classify_tar(&["-tvf", "archive.tar"]), Safety::ReadOnly);
    }

    #[test]
    fn tar_write_commands() {
        assert_eq!(classify_tar(&["-cf", "archive.tar", "files/"]), Safety::WriteOperation);
        assert_eq!(classify_tar(&["-xf", "archive.tar"]), Safety::WriteOperation);
    }
}
