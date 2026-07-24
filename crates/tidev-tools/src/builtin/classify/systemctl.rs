use super::*;

/// Classify systemctl commands by subcommand.
///
/// Read-only: status, show, list-units, list-sockets, list-timers, list-mounts,
///            list-automounts, list-paths, list-dependencies, list-jobs,
///            is-active, is-enabled, is-failed, is-system-running, show-environment,
///            get-default, help, version, daemon-reload (safe)
/// Write: start, stop, restart, reload, enable, disable, enable-now, mask, unmask,
///        set-default, set-property, edit, add-wants, add-requires, reenable,
///        preset, preset-all, revert, kill, clean, freeze, thaw
pub(super) fn classify_systemctl(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `systemctl` — list units
    };

    match sub {
        // Read-only
        "status" | "show" | "list-units" | "list-sockets" | "list-timers" | "list-mounts"
        | "list-automounts" | "list-paths" | "list-dependencies" | "list-jobs" | "is-active"
        | "is-enabled" | "is-failed" | "is-system-running" | "show-environment" | "get-default"
        | "help" | "version" | "daemon-reload" | "list-unit-files" => Safety::ReadOnly,

        // `systemctl cat` outputs unit file content — read-only
        "cat" => Safety::ReadOnly,

        // Explicit write commands
        "start"
        | "stop"
        | "restart"
        | "reload"
        | "enable"
        | "disable"
        | "enable-now"
        | "mask"
        | "unmask"
        | "set-default"
        | "set-property"
        | "edit"
        | "add-wants"
        | "add-requires"
        | "reenable"
        | "preset"
        | "preset-all"
        | "revert"
        | "kill"
        | "clean"
        | "freeze"
        | "thaw"
        | "reset-failed"
        | "condreload"
        | "condrestart"
        | "try-restart"
        | "reload-or-restart"
        | "reload-or-try-restart"
        | "isolate"
        | "switch-root"
        | "cancel"
        | "poweroff"
        | "reboot"
        | "halt"
        | "kexec"
        | "exit"
        | "suspend"
        | "hibernate"
        | "hybrid-sleep"
        | "suspend-then-hibernate"
        | "service" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systemctl_read_commands() {
        assert_eq!(classify_systemctl(&["status", "nginx"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["show", "nginx"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["list-units"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["list-unit-files"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["list-timers"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["list-sockets"]), Safety::ReadOnly);
        assert_eq!(
            classify_systemctl(&["list-dependencies", "nginx"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_systemctl(&["is-active", "nginx"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_systemctl(&["is-enabled", "nginx"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_systemctl(&["is-system-running"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["show-environment"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["get-default"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["daemon-reload"]), Safety::ReadOnly);
        assert_eq!(classify_systemctl(&["cat", "nginx"]), Safety::ReadOnly);
    }

    #[test]
    fn systemctl_write_commands() {
        assert_eq!(
            classify_systemctl(&["start", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["stop", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["restart", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["reload", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["enable", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["disable", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["mask", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["unmask", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["set-default", "multi-user.target"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["set-property", "nginx", "CPUShares=500"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["edit", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["reenable", "nginx"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_systemctl(&["kill", "nginx"]),
            Safety::WriteOperation
        );
    }
}
