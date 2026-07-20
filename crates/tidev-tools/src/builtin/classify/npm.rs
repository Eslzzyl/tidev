use super::*;

/// Classify npm/pnpm/yarn commands.
pub(super) fn classify_npm(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    match sub {
        "run" | "test" | "start" | "ls" | "list" | "outdated" | "help" | "version" | "why"
        | "audit" | "doctor" | "completion" | "cache" => {
            // Check for `--dry-run`
            if args.contains(&"--dry-run") || args.contains(&"--dry") {
                return Safety::ReadOnly;
            }
            // `npm cache clean` is write
            if sub == "cache" && args.get(1) == Some(&"clean") {
                return Safety::WriteOperation;
            }
            if sub == "run" || sub == "test" || sub == "start" {
                return Safety::Unknown; // depends on the script
            }
            Safety::ReadOnly
        }
        _ => Safety::WriteOperation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_read_commands() {
        assert_eq!(classify_npm(&["run", "test"]), Safety::Unknown);
        assert_eq!(classify_npm(&["list"]), Safety::ReadOnly);
        assert_eq!(classify_npm(&["outdated"]), Safety::ReadOnly);
        assert_eq!(classify_npm(&["audit"]), Safety::ReadOnly);
        assert_eq!(classify_npm(&["cache", "ls"]), Safety::ReadOnly);
    }

    #[test]
    fn npm_write_commands() {
        assert_eq!(classify_npm(&["install", "express"]), Safety::WriteOperation);
        assert_eq!(classify_npm(&["publish"]), Safety::WriteOperation);
        assert_eq!(classify_npm(&["cache", "clean"]), Safety::WriteOperation);
    }
}
