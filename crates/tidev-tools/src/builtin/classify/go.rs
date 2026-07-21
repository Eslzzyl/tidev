use super::*;

/// Classify go commands.
pub(super) fn classify_go(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    match sub {
        "vet" | "doc" | "list" | "version" | "env" | "help"
        | "fmt"                                        // `go fmt` reports diffs by default; -w writes
        => {
            // Check for `-w` (write) flag on go fmt
            if sub == "fmt" && args.contains(&"-w") {
                return Safety::WriteOperation;
            }
            Safety::ReadOnly
        }

        // Explicit write commands
        "build" | "run" | "test" | "install" | "get" | "mod" | "work" | "generate"
        | "fix" | "clean" | "tool" | "telemetry" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_read_commands() {
        assert_eq!(classify_go(&["vet", "./..."]), Safety::ReadOnly);
        assert_eq!(classify_go(&["doc", "fmt"]), Safety::ReadOnly);
        assert_eq!(classify_go(&["list", "./..."]), Safety::ReadOnly);
        assert_eq!(classify_go(&["version"]), Safety::ReadOnly);
    }

    #[test]
    fn go_write_commands() {
        assert_eq!(classify_go(&["build", "./..."]), Safety::WriteOperation);
        assert_eq!(classify_go(&["run", "main.go"]), Safety::WriteOperation);
        assert_eq!(classify_go(&["install", "./cmd/..."]), Safety::WriteOperation);
        assert_eq!(classify_go(&["mod", "tidy"]), Safety::WriteOperation);
        assert_eq!(classify_go(&["mod", "download"]), Safety::WriteOperation);
        assert_eq!(classify_go(&["get", "example.com/pkg"]), Safety::WriteOperation);
    }
}
