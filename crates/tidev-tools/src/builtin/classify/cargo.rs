use super::*;
use std::sync::LazyLock;

/// Cargo sub-commands that are read-only.
static CARGO_READ_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "check",
        "test",
        "clippy", // `fix` and `fmt` modify code — write
        "doc",
        "metadata",
        "tree",
        "locate-project",
        "pkgid",
        "help",
        "version",
        "search",
        "info",
        "report",
        "generate-lockfile", // safe (regenerates Cargo.lock)
        "verify-project",
        "audit",
    ]
});

/// Cargo sub-commands that are always write operations.
static CARGO_WRITE_SUBCOMMANDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    vec![
        "build",
        "run",
        "publish",
        "install",
        "uninstall",
        "add",
        "remove",
        "update",
        "upgrade",
        "fix",
        "fmt",
        "init",
        "new",
        "clean",
        "config",
        "login",
        "logout",
        "owner",
        "yank",
        "package",
        "vendor",
    ]
});

/// Classify a cargo command by its subcommand.
pub(super) fn classify_cargo(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    if CARGO_READ_SUBCOMMANDS.contains(&sub) {
        Safety::ReadOnly
    } else if CARGO_WRITE_SUBCOMMANDS.contains(&sub) {
        Safety::WriteOperation
    } else {
        // Everything else (bench, rustdoc, rustc, etc.) — ambiguous, let through
        Safety::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_fix_and_fmt_are_write() {
        assert_eq!(classify_cargo(&["fix"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["fmt"]), Safety::WriteOperation);
    }

    #[test]
    fn cargo_read_commands() {
        assert_eq!(classify_cargo(&["check"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["test"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["clippy"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["doc"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["metadata"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["tree"]), Safety::ReadOnly);
        assert_eq!(classify_cargo(&["audit"]), Safety::ReadOnly);
    }

    #[test]
    fn cargo_write_commands() {
        assert_eq!(classify_cargo(&["build"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["run"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["publish"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["install"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["add", "serde"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["remove", "serde"]), Safety::WriteOperation);
        assert_eq!(classify_cargo(&["update"]), Safety::WriteOperation);
    }
}
