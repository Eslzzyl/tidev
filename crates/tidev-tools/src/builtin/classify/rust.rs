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

/// Unified entry point for all Rust toolchain commands.
///
/// Dispatches to the appropriate classifier based on the command name.
pub(super) fn classify_rust(cmd: &str, args: &[&str]) -> Safety {
    match cmd {
        "cargo" => classify_cargo(args),
        "rustc" => classify_rustc(args),
        "rustup" => classify_rustup(args),
        _ => Safety::Unknown,
    }
}

/// Classify a cargo command by its subcommand.
fn classify_cargo(args: &[&str]) -> Safety {
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

/// Classify a `rustc` command.
///
/// Rules:
/// - If any argument looks like a source file (ends with `.rs` or contains
///   path separators like `/`), it's a compilation file → `WriteOperation`.
/// - Arguments that are purely alphanumeric (like `2021`, `E0308`, `cfg`)
///   are treated as flag values, not source files → still `ReadOnly`.
/// - If only flags and their values are present → `ReadOnly`.
fn classify_rustc(args: &[&str]) -> Safety {
    // A non-flag argument that looks like a file path (contains `.`, `/`, or
    // other path-like characters) is likely a source file → compilation.
    if args
        .iter()
        .any(|a| !a.starts_with('-') && (a.contains('.') || a.contains('/')))
    {
        return Safety::WriteOperation;
    }
    // Only flags and their values (e.g. `--edition 2021`, `--explain E0308`,
    // `--print cfg`) → informational query, no writes.
    Safety::ReadOnly
}

/// Classify a `rustup` command.
///
/// Rules:
/// - `rustup` with no arguments shows help → `ReadOnly`.
/// - Read-only subcommands or sub-subcommands.
/// - Write subcommands or sub-subcommands.
fn classify_rustup(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        // `rustup` with no args prints help or error — no writes
        return Safety::ReadOnly;
    };

    match sub {
        // Simple read-only subcommands
        "show" | "which" | "help" | "check" | "man" | "docs" => Safety::ReadOnly,
        // `rustup list` is an alias for `rustup toolchain list`
        "list" => Safety::ReadOnly,
        // Simple write subcommands
        "install" | "uninstall" | "update" | "default" | "set" => Safety::WriteOperation,
        // Composite subcommands — safety depends on the second argument
        "toolchain" | "target" | "component" | "override" => {
            let subsub = args.get(1).copied();
            match subsub {
                Some("list") => Safety::ReadOnly,
                Some("install" | "add" | "remove" | "uninstall" | "set" | "default") => {
                    Safety::WriteOperation
                }
                _ => Safety::Unknown,
            }
        }
        "self" => {
            let subsub = args.get(1).copied();
            match subsub {
                Some("update" | "uninstall") => Safety::WriteOperation,
                _ => Safety::Unknown,
            }
        }
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Cargo ────────────────────────────────────────────────────────────

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

    // ── Rustc ────────────────────────────────────────────────────────────

    #[test]
    fn rustc_read_commands() {
        assert_eq!(classify_rustc(&["--version"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["-V"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["-Vv"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["--help"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["-h"]), Safety::ReadOnly);
        // Flag values that don't look like file paths → still ReadOnly
        assert_eq!(classify_rustc(&["--print", "cfg"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["--explain", "E0308"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["--edition", "2021"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["-Z", "help"]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["--cfg", "feature=\"foo\""]), Safety::ReadOnly);
        assert_eq!(classify_rustc(&["--crate-type", "lib"]), Safety::ReadOnly);
        // No args at all — just prints help/error, no writes
        assert_eq!(classify_rustc(&[]), Safety::ReadOnly);
    }

    #[test]
    fn rustc_write_commands() {
        // Has a source file argument (.rs extension) → compilation
        assert_eq!(classify_rustc(&["main.rs"]), Safety::WriteOperation);
        assert_eq!(classify_rustc(&["src/lib.rs"]), Safety::WriteOperation);
        assert_eq!(
            classify_rustc(&["-o", "output", "main.rs"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustc(&["--edition", "2021", "main.rs"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustc(&["--crate-type", "lib", "src/lib.rs"]),
            Safety::WriteOperation
        );
        // Non-.rs file path (e.g. assembly output)
        assert_eq!(
            classify_rustc(&["-o", "output.o", "main.rs"]),
            Safety::WriteOperation
        );
        // Path separator triggers write detection
        assert_eq!(
            classify_rustc(&["--out-dir", "target/debug"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn rustc_classify_via_classifier() {
        let cl = Classifier::new();
        // Through the full classifier pipeline
        assert_eq!(cl.classify("rustc --version"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustc --help"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustc --print cfg"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustc --edition 2021"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustc --explain E0308"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustc main.rs"), Safety::WriteOperation);
        assert_eq!(
            cl.classify("rustc -o output main.rs"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("rustc --edition 2021 main.rs"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("rustc src/lib.rs"),
            Safety::WriteOperation
        );
    }

    // ── Rustup ───────────────────────────────────────────────────────────

    #[test]
    fn rustup_read_commands() {
        assert_eq!(classify_rustup(&["show"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["which", "rustc"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["help"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["check"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["man"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["docs"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["list"]), Safety::ReadOnly);
        // Composite read-only sub-subcommands
        assert_eq!(classify_rustup(&["toolchain", "list"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["target", "list"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["component", "list"]), Safety::ReadOnly);
        assert_eq!(classify_rustup(&["override", "list"]), Safety::ReadOnly);
        // No args at all
        assert_eq!(classify_rustup(&[]), Safety::ReadOnly);
    }

    #[test]
    fn rustup_write_commands() {
        assert_eq!(classify_rustup(&["install", "stable"]), Safety::WriteOperation);
        assert_eq!(
            classify_rustup(&["uninstall", "stable"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_rustup(&["update"]), Safety::WriteOperation);
        assert_eq!(classify_rustup(&["default", "stable"]), Safety::WriteOperation);
        assert_eq!(classify_rustup(&["set", "profile", "default"]), Safety::WriteOperation);
        // Composite write sub-subcommands
        assert_eq!(
            classify_rustup(&["toolchain", "install", "stable"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["toolchain", "remove", "stable"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["target", "add", "wasm32-unknown-unknown"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["target", "remove", "wasm32-unknown-unknown"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["component", "add", "clippy"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["component", "remove", "clippy"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["override", "set", "stable"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_rustup(&["override", "remove"]),
            Safety::WriteOperation
        );
        // Self subcommands
        assert_eq!(classify_rustup(&["self", "update"]), Safety::WriteOperation);
        assert_eq!(
            classify_rustup(&["self", "uninstall"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn rustup_unknown_commands() {
        // Unknown subcommands should be let through
        assert_eq!(classify_rustup(&["unknown"]), Safety::Unknown);
        // Composite with unknown sub-subcommand
        assert_eq!(
            classify_rustup(&["toolchain", "unknown"]),
            Safety::Unknown
        );
        // Self with unknown sub-subcommand
        assert_eq!(classify_rustup(&["self", "unknown"]), Safety::Unknown);
    }

    #[test]
    fn rustup_classify_via_classifier() {
        let cl = Classifier::new();
        // Read-only
        assert_eq!(cl.classify("rustup show"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustup check"), Safety::ReadOnly);
        assert_eq!(cl.classify("rustup toolchain list"), Safety::ReadOnly);
        // Write
        assert_eq!(
            cl.classify("rustup toolchain install stable"),
            Safety::WriteOperation
        );
        assert_eq!(
            cl.classify("rustup target add wasm32-unknown-unknown"),
            Safety::WriteOperation
        );
        assert_eq!(cl.classify("rustup default stable"), Safety::WriteOperation);
        // Unknown (let through)
        assert_eq!(cl.classify("rustup foo"), Safety::Unknown);
    }
}
