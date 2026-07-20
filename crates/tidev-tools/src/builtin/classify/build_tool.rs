use super::Safety;

/// Classify build tools (make, cmake, ninja, meson, just, task).
///
/// Read-only: help, list, describe, --help, --version, --list, --summary,
///            --dry-run/-n/--just-print
/// Write: build, install, clean, test (anything else)
pub(super) fn classify_build_tool(args: &[&str]) -> Safety {
    let Some(target) = args.first().copied() else {
        // bare `make` → builds (write)
        return Safety::WriteOperation;
    };

    // `make -n` / `just --dry-run` → dry-run (read)
    if args.contains(&"-n") || args.contains(&"--dry-run") || args.contains(&"--just-print") {
        return Safety::ReadOnly;
    }

    // `make --help` / `just --list` / `cmake --version` → read-only flags
    if target.starts_with("--") {
        return match target {
            "--help" | "--version" | "--list" | "--summary" | "--choose" | "--justprint"
            | "--dry-run" | "--evaluate" => Safety::ReadOnly,
            _ => Safety::WriteOperation,
        };
    }

    // Short flag: `make -h`, `make -v`
    if target.starts_with('-') && target.len() == 2 {
        let flag = target.chars().nth(1).unwrap();
        return match flag {
            'h' | 'v' | 'n' | 'l' | 's' => Safety::ReadOnly,
            _ => Safety::WriteOperation,
        };
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
        assert_eq!(classify_build_tool(&["build"]), Safety::WriteOperation);
        assert_eq!(classify_build_tool(&["test"]), Safety::WriteOperation);
    }

    #[test]
    fn make_dry_run_is_read() {
        assert_eq!(classify_build_tool(&["-n"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["--dry-run"]), Safety::ReadOnly);
    }

    #[test]
    fn make_help_is_read() {
        assert_eq!(classify_build_tool(&["--help"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["-h"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["help"]), Safety::ReadOnly);
    }

    #[test]
    fn make_version_is_read() {
        assert_eq!(classify_build_tool(&["--version"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["-v"]), Safety::ReadOnly);
    }

    #[test]
    fn just_list_is_read() {
        assert_eq!(classify_build_tool(&["--list"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["--summary"]), Safety::ReadOnly);
        assert_eq!(classify_build_tool(&["-l"]), Safety::ReadOnly);
    }
}
