use super::*;

/// Classify pip/pip3 commands by subcommand.
///
/// Read-only: list, show, freeze, check, search, index versions, inspect
/// Write: install, uninstall, download, wheel, hash, cache
pub(super) fn classify_pip(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `pip` — let through (help)
    };

    match sub {
        // Read-only
        "list" | "show" | "freeze" | "check" | "search" | "index" | "inspect" | "help"
        | "version" | "debug" | "completion" => Safety::ReadOnly,

        // `pip cache list/purge/remove`: list is read, purge/remove is write
        "cache" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "info" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // purge, remove
            }
        }

        // Write (install, uninstall, download, wheel, hash, etc.)
        _ => Safety::WriteOperation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_read_commands() {
        assert_eq!(classify_pip(&["list"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["list", "--outdated"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["show", "requests"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["freeze"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["check"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["search", "requests"]), Safety::ReadOnly);
        assert_eq!(classify_pip(&["cache", "list"]), Safety::ReadOnly);
    }

    #[test]
    fn pip_write_commands() {
        assert_eq!(classify_pip(&["install", "requests"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["install", "-r", "requirements.txt"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["uninstall", "requests"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["download", "requests"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["wheel", "requests"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["cache", "purge"]), Safety::WriteOperation);
        assert_eq!(classify_pip(&["cache", "remove", "pkg"]), Safety::WriteOperation);
    }
}
