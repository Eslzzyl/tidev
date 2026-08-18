use super::*;

/// Classify zstd — stdout output is read-only unless an option also writes or
/// removes a file.
pub(super) fn classify_zstd(args: &[&str]) -> Safety {
    let writes_file = args.iter().any(|arg| {
        matches!(*arg, "--rm" | "--output")
            || arg.starts_with("--output=")
            || *arg == "-o"
            || (arg.starts_with("-o") && !arg.starts_with("--"))
    });

    let writes_stdout = args.iter().any(|arg| {
        *arg == "--stdout"
            || *arg == "--to-stdout"
            || *arg == "-c"
            || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('c'))
    });

    if writes_stdout && !writes_file {
        Safety::ReadOnly
    } else {
        Safety::WriteOperation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdout_decompression_is_read_only() {
        assert_eq!(
            classify_zstd(&["-d", "-c", "archive.zst"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_zstd(&["-dc", "archive.zst"]), Safety::ReadOnly);
        assert_eq!(
            classify_zstd(&["--decompress", "--stdout", "archive.zst"]),
            Safety::ReadOnly
        );
    }

    #[test]
    fn stdout_compression_is_read_only() {
        assert_eq!(classify_zstd(&["-c", "input.txt"]), Safety::ReadOnly);
        assert_eq!(classify_zstd(&["--stdout", "input.txt"]), Safety::ReadOnly);
    }

    #[test]
    fn file_output_is_write_operation() {
        assert_eq!(
            classify_zstd(&["-d", "archive.zst"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_zstd(&["-d", "-o", "output.txt", "archive.zst"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_zstd(&["--stdout", "--rm", "archive.zst"]),
            Safety::WriteOperation
        );
    }
}
