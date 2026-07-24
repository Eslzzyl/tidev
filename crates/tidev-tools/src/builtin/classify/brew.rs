use super::*;

/// Classify brew commands by subcommand.
///
/// Read-only: list, info, search, doctor, outdated, missing, desc, home, help,
///            tap-info, livecheck, deps, uses, cat, log, config, analytics
/// Write: install, uninstall, upgrade, reinstall, pin, unpin, update,
///        tap, untap, cleanup, cask install/uninstall, services, autoupdate
pub(super) fn classify_brew(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `brew` — help
    };

    match sub {
        // Read-only
        "list"
        | "info"
        | "search"
        | "doctor"
        | "outdated"
        | "missing"
        | "desc"
        | "home"
        | "help"
        | "version"
        | "config"
        | "analytics"
        | "log"
        | "cat"
        | "deps"
        | "uses"
        | "leaves"
        | "commands"
        | "formulae"
        | "casks"
        | "tap-info"
        | "livecheck"
        | "generate-man-completions"
        | "readall"
        | "style"
        | "typecheck" => Safety::ReadOnly,

        // `brew tap-info` is read (already above), `brew tap` itself is write
        "tap" | "untap" => Safety::WriteOperation,

        // `brew services`: list is read, everything else is write
        "services" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "info" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // start, stop, restart, run, cleanup
            }
        }

        // `brew autoupdate`: status is read, start/stop/toggle are write
        "autoupdate" => {
            let action = args.get(1).copied().unwrap_or("status");
            match action {
                "status" | "info" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // start, stop, toggle, delete, version
            }
        }

        // Explicit write commands
        "install" | "uninstall" | "upgrade" | "reinstall" | "update" | "cleanup" | "pin"
        | "unpin" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brew_read_commands() {
        assert_eq!(classify_brew(&["list"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["list", "--formula"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["info", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["search", "package"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["doctor"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["outdated"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["missing"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["deps", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["uses", "bash"]), Safety::ReadOnly);
        assert_eq!(
            classify_brew(&["tap-info", "homebrew/core"]),
            Safety::ReadOnly
        );
    }

    #[test]
    fn brew_services_read_commands() {
        assert_eq!(classify_brew(&["services"]), Safety::ReadOnly);
        assert_eq!(classify_brew(&["services", "list"]), Safety::ReadOnly);
        assert_eq!(
            classify_brew(&["services", "info", "nginx"]),
            Safety::ReadOnly
        );
    }

    #[test]
    fn brew_write_commands() {
        assert_eq!(classify_brew(&["install", "bash"]), Safety::WriteOperation);
        assert_eq!(
            classify_brew(&["install", "--cask", "app"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_brew(&["uninstall", "bash"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_brew(&["upgrade"]), Safety::WriteOperation);
        assert_eq!(classify_brew(&["update"]), Safety::WriteOperation);
        assert_eq!(
            classify_brew(&["reinstall", "bash"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_brew(&["cleanup"]), Safety::WriteOperation);
        assert_eq!(
            classify_brew(&["tap", "homebrew/core"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_brew(&["untap", "homebrew/core"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_brew(&["pin", "bash"]), Safety::WriteOperation);
        assert_eq!(
            classify_brew(&["services", "start", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_brew(&["services", "stop", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_brew(&["services", "restart", "nginx"]),
            Safety::WriteOperation
        );
    }
}
