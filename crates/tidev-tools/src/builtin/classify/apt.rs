use super::*;

/// Classify apt/apt-get commands by subcommand.
///
/// Read-only: list, show, search, policy, depends, rdepends, cache
/// Write: install, remove, purge, update, upgrade, full-upgrade, dist-upgrade,
///        autoremove, autoclean, clean, build-dep, source, download, satisfies
pub(super) fn classify_apt(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `apt` — help
    };

    match sub {
        // Read-only
        "list" | "show" | "search" | "policy" | "depends" | "rdepends" | "help" | "version" => {
            Safety::ReadOnly
        }

        // `apt cache`: sub-subcommand based
        "cache" => {
            let action = args.get(1).copied().unwrap_or("show");
            match action {
                // `apt cache show/search/policy/dump/...` — read
                "show" | "search" | "policy" | "dump" | "dumpavail" | "stats" | "madison"
                | "showpkg" | "showsrc" | "gencaches" | "depends" | "rdepends" => Safety::ReadOnly,
                // `apt cache add` — write (adds package to cache)
                _ => Safety::WriteOperation,
            }
        }

        // `apt-mark`: showmanual/showauto are read, everything else write
        "mark" => {
            let action = args.get(1).copied().unwrap_or("showmanual");
            match action {
                "showmanual" | "showauto" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // hold, unhold, auto, manual
            }
        }

        // Explicit write commands
        "install" | "remove" | "purge" | "update" | "upgrade" | "full-upgrade" | "dist-upgrade"
        | "autoremove" | "autoclean" | "clean" | "build-dep" | "source" | "download"
        | "satisfies" | "add" | "delete" | "hold" | "unhold" | "auto" | "manual" => {
            Safety::WriteOperation
        }

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_read_commands() {
        assert_eq!(classify_apt(&["list", "--installed"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["show", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["search", "package"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["policy", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["depends", "bash"]), Safety::ReadOnly);
    }

    #[test]
    fn apt_cache_read_commands() {
        assert_eq!(classify_apt(&["cache", "show", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["cache", "search", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["cache", "policy", "bash"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["cache", "stats"]), Safety::ReadOnly);
    }

    #[test]
    fn apt_write_commands() {
        assert_eq!(classify_apt(&["install", "bash"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["remove", "bash"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["purge", "bash"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["update"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["upgrade"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["full-upgrade"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["autoremove"]), Safety::WriteOperation);
        assert_eq!(classify_apt(&["autoclean"]), Safety::WriteOperation);
        assert_eq!(
            classify_apt(&["build-dep", "package"]),
            Safety::WriteOperation
        );
    }

    #[test]
    fn apt_mark_read_commands() {
        assert_eq!(classify_apt(&["mark", "showmanual"]), Safety::ReadOnly);
        assert_eq!(classify_apt(&["mark", "showauto"]), Safety::ReadOnly);
    }

    #[test]
    fn apt_mark_write_commands() {
        assert_eq!(
            classify_apt(&["mark", "hold", "bash"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_apt(&["mark", "auto", "bash"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_apt(&["mark", "unhold", "bash"]),
            Safety::WriteOperation
        );
    }
}
