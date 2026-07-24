use super::*;

/// Classify nix commands (nix, nix-shell, nix-env, nix-build, nix-collect-garbage, etc.)
/// by subcommand.
///
/// Modern `nix <subcommand>`:
///   Read-only: show, search, flake show, flake metadata, flake lock --show,
///              eval, why-depends, path-info, registry list, nar ls/cat,
///              store ls/cat, edit (opens editor, but doesn't modify by itself)
///   Write: build, run, develop, shell, flake update/lock, profile install/remove,
///          store add/delete, registry add/remove/pin, edit (modifies flake)
///
/// Legacy nix-* commands:
///   nix-shell: starts a shell (write-like)
///   nix-env: --query/-q is read, --install/-i is write
///   nix-build: builds (write)
///   nix-collect-garbage: write
pub(super) fn classify_nix(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `nix` — help
    };

    // Handle both modern `nix <sub>` and legacy `nix-shell`, `nix-env`, etc.
    match sub {
        // Modern nix read-only subcommands
        "show" | "search" | "eval" | "why-depends" | "edit" | "help" | "version"
        | "completions" | "fmt" => Safety::ReadOnly,

        // `nix flake`: show/metadata/lock --show is read
        "flake" => {
            let action = args.get(1).copied().unwrap_or("show");
            match action {
                "show" | "metadata" | "archive" => Safety::ReadOnly,
                "lock" => {
                    // `nix flake lock --show` is read-only
                    if args.contains(&"--show") || args.contains(&"--dry-run") {
                        Safety::ReadOnly
                    } else {
                        Safety::WriteOperation
                    }
                }
                _ => Safety::WriteOperation, // update, clone, check, prefetch
            }
        }

        // `nix registry`: list is read, add/remove/pin is write
        "registry" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // add, remove, pin, set
            }
        }

        // `nix store`: ls/cat are read, add/delete/repair are write
        "store" => {
            let action = args.get(1).copied().unwrap_or("ls");
            match action {
                "ls" | "cat" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // add, delete, repair, optimise, gc
            }
        }

        // `nix nar`: ls/cat are read
        "nar" => {
            let action = args.get(1).copied().unwrap_or("ls");
            match action {
                "ls" | "cat" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // --repair
            }
        }

        // `nix path-info`: read-only
        "path-info" => Safety::ReadOnly,

        // `nix profile`: list is read, install/remove/upgrade is write
        "profile" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "history" | "diff" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // install, remove, upgrade, wipe-history, rollback
            }
        }

        // Explicit write commands at top level
        "build"
        | "run"
        | "develop"
        | "shell"
        | "bundle"
        | "copy"
        | "daemon"
        | "derivation"
        | "dump"
        | "hash"
        | "log"
        | "make-content-addressable"
        | "optimise-store"
        | "prefetch"
        | "realisation"
        | "repl"
        | "upgrade-nix" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

/// Classify nix-env commands: --query is read, --install is write.
pub(super) fn classify_nix_env(args: &[&str]) -> Safety {
    if args.contains(&"-q") || args.contains(&"--query") {
        Safety::ReadOnly
    } else {
        // Without -q/--query, nix-env could be install/upgrade — ambiguous
        Safety::Unknown
    }
}

/// Classify nix-shell: always starts a shell (ambiguous).
pub(super) fn classify_nix_shell(_args: &[&str]) -> Safety {
    // nix-shell starts an interactive shell or runs a command — could do anything
    Safety::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Modern nix ────────────────────────────────────────────────────────

    #[test]
    fn nix_read_commands() {
        assert_eq!(classify_nix(&["show", "config"]), Safety::ReadOnly);
        assert_eq!(
            classify_nix(&["search", "nixpkgs", "hello"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_nix(&["eval", "-f", "default.nix", "name"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_nix(&["why-depends", "pkg", "dep"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_nix(&["path-info", "pkg"]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["flake", "show", "."]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["flake", "metadata", "."]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["flake", "lock", "--show"]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["registry", "list"]), Safety::ReadOnly);
        assert_eq!(
            classify_nix(&["store", "ls", "store-path"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_nix(&["store", "cat", "store-path", "file"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_nix(&["nar", "ls", "nar-path"]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["profile", "list"]), Safety::ReadOnly);
        assert_eq!(classify_nix(&["profile", "history"]), Safety::ReadOnly);
    }

    #[test]
    fn nix_write_commands() {
        assert_eq!(classify_nix(&["build", ".#hello"]), Safety::WriteOperation);
        assert_eq!(classify_nix(&["run", ".#app"]), Safety::WriteOperation);
        assert_eq!(
            classify_nix(&["develop", ".#devShell"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_nix(&["shell", "nixpkgs#hello"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_nix(&["flake", "update"]), Safety::WriteOperation);
        assert_eq!(classify_nix(&["flake", "lock"]), Safety::WriteOperation);
        assert_eq!(
            classify_nix(&["registry", "add", "name", "url"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_nix(&["store", "delete", "store-path"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_nix(&["profile", "install", "nixpkgs#hello"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_nix(&["profile", "remove", "index"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_nix(&["edit", "flake.nix"]), Safety::ReadOnly);
    }

    // ── Legacy nix-env ────────────────────────────────────────────────────

    #[test]
    fn nix_env_read_commands() {
        assert_eq!(classify_nix_env(&["-q"]), Safety::ReadOnly);
        assert_eq!(classify_nix_env(&["--query"]), Safety::ReadOnly);
        assert_eq!(classify_nix_env(&["-q", "--available"]), Safety::ReadOnly);
    }

    #[test]
    fn nix_env_write_commands() {
        assert_eq!(classify_nix_env(&["-i", "hello"]), Safety::Unknown);
        assert_eq!(classify_nix_env(&["--install", "hello"]), Safety::Unknown);
        assert_eq!(classify_nix_env(&["-e", "hello"]), Safety::Unknown);
        assert_eq!(classify_nix_env(&["-u", "hello"]), Safety::Unknown);
    }

    // ── Legacy nix-shell ──────────────────────────────────────────────────

    #[test]
    fn nix_shell_is_unknown() {
        assert_eq!(classify_nix_shell(&[]), Safety::Unknown);
        assert_eq!(classify_nix_shell(&["-p", "hello"]), Safety::Unknown);
        assert_eq!(
            classify_nix_shell(&["--command", "echo hi"]),
            Safety::Unknown
        );
    }
}
