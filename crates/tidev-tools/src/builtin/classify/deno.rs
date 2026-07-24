use super::*;

/// Classify deno commands by subcommand.
///
/// Read-only: check, doc, info, lint, fmt --check, types, help, version,
///            completions, repl, eval (read from stdin, output to stdout)
/// Write: run, compile, cache, bundle, fmt (without --check), task, test,
///        bench, vendor, publish, init, add, install, uninstall, upgrade,
///        jupyter, serve, pack, remove
pub(super) fn classify_deno(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `deno` — help
    };

    match sub {
        // Read-only
        "check" | "doc" | "info" | "types" | "help" | "version" | "completions" | "repl"
        | "lsp" => Safety::ReadOnly,

        // `deno lint` is read-only, `deno lint --fix` is write
        "lint" => {
            if args.contains(&"--fix") {
                Safety::WriteOperation
            } else {
                Safety::ReadOnly
            }
        }

        // `deno fmt --check` is read-only, `deno fmt` (without --check) is write
        "fmt" => {
            if args.contains(&"--check") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }

        // `deno eval` runs code from stdin — could do anything, classified as Unknown
        "eval" | "eval-file" => Safety::Unknown,

        // Explicit write commands
        "run" | "compile" | "cache" | "bundle" | "install" | "uninstall" | "upgrade"
        | "publish" | "init" | "add" | "remove" | "task" | "test" | "bench" | "serve" | "pack"
        | "vendor" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deno_read_commands() {
        assert_eq!(classify_deno(&["check", "main.ts"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["doc", "main.ts"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["info"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["lint"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["lint", "main.ts"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["fmt", "--check"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["types"]), Safety::ReadOnly);
        assert_eq!(classify_deno(&["repl"]), Safety::ReadOnly);
    }

    #[test]
    fn deno_write_commands() {
        assert_eq!(classify_deno(&["run", "main.ts"]), Safety::WriteOperation);
        assert_eq!(
            classify_deno(&["compile", "main.ts"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_deno(&["cache", "deps.ts"]), Safety::WriteOperation);
        assert_eq!(
            classify_deno(&["bundle", "main.ts"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_deno(&["fmt"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["lint", "--fix"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["task", "build"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["test"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["bench"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["publish"]), Safety::WriteOperation);
        assert_eq!(
            classify_deno(&["init", "my_project"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_deno(&["add", "@std/fs"]), Safety::WriteOperation);
        assert_eq!(
            classify_deno(&["install", "cli.ts"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_deno(&["uninstall", "cli"]), Safety::WriteOperation);
        assert_eq!(classify_deno(&["upgrade"]), Safety::WriteOperation);
    }
}
