use super::Safety;

/// Classify build tools (make, cmake, ninja, meson).
pub(super) fn classify_build_tool(args: &[&str]) -> Safety {
    let Some(target) = args.first().copied() else {
        // bare `make` → builds (write)
        return Safety::WriteOperation;
    };

    // `make clean`, `make install` → write
    // `make -n` → dry-run (read)
    if args.contains(&"-n") || args.contains(&"--dry-run") || args.contains(&"--just-print") {
        return Safety::ReadOnly;
    }

    // Common read-only targets
    match target {
        "help" | "list" | "describe" => Safety::ReadOnly,
        _ => Safety::WriteOperation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_write() {
        assert_eq!(classify_build_tool(&[]), Safety::WriteOperation);
        assert_eq!(classify_build_tool(&["install"]), Safety::WriteOperation);
        assert_eq!(classify_build_tool(&["clean"]), Safety::WriteOperation);
    }

    #[test]
    fn make_dry_run_is_read() {
        assert_eq!(classify_build_tool(&["-n"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["--dry-run"]), Safety::ReadOnly);
    }
}
