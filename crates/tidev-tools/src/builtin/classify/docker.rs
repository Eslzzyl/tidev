use super::*;

/// Classify docker commands.
pub(super) fn classify_docker(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown;
    };

    // Read-only docker commands
    match sub {
        "ps" | "images" | "logs" | "inspect" | "stats" | "top" | "port" | "version" | "info"
        | "events" | "history" | "network" | "volume" => Safety::ReadOnly,

        // Everything else (run, build, pull, push, exec, stop, rm, etc.)
        _ => Safety::WriteOperation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_read_commands() {
        assert_eq!(classify_docker(&["ps"]), Safety::ReadOnly);
        assert_eq!(classify_docker(&["images"]), Safety::ReadOnly);
        assert_eq!(classify_docker(&["logs", "app"]), Safety::ReadOnly);
        assert_eq!(classify_docker(&["inspect", "app"]), Safety::ReadOnly);
    }

    #[test]
    fn docker_write_commands() {
        assert_eq!(classify_docker(&["build", "."]), Safety::WriteOperation);
        assert_eq!(classify_docker(&["run", "image"]), Safety::WriteOperation);
        assert_eq!(classify_docker(&["push", "image"]), Safety::WriteOperation);
        assert_eq!(classify_docker(&["stop", "app"]), Safety::WriteOperation);
    }
}
