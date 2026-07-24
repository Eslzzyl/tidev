use super::*;

/// Classify helm commands by subcommand.
///
/// Read-only: list, status, get, history, show, search, lint, template,
///            dependency list, repo list, plugin list, version, help, completion
/// Write: install, upgrade, uninstall, rollback, repo add/remove/update/index,
///        dependency build/update, create, package, push, plugin install/remove,
///        test, get-values, get-notes, get-all
pub(super) fn classify_helm(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `helm` — help
    };

    match sub {
        // Read-only
        "list" | "status" | "history" | "lint" | "template" | "version" | "help" | "completion"
        | "search" | "show" | "test" => Safety::ReadOnly,

        // `helm get`: get values/notes/manifest/all/hooks — all read-only
        "get" => Safety::ReadOnly,

        // `helm repo`: list is read, everything else is write
        "repo" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "index" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // add, remove, update
            }
        }

        // `helm dependency`: list is read, build/update is write
        "dependency" | "dep" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // build, update
            }
        }

        // `helm plugin`: list is read, install/remove/uninstall is write
        "plugin" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // install, remove, uninstall, update
            }
        }

        // Explicit write commands at top level
        "install" | "upgrade" | "uninstall" | "rollback" | "create" | "package" | "push"
        | "env" => Safety::WriteOperation,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helm_read_commands() {
        assert_eq!(classify_helm(&["list"]), Safety::ReadOnly);
        assert_eq!(
            classify_helm(&["list", "--all-namespaces"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_helm(&["status", "release"]), Safety::ReadOnly);
        assert_eq!(classify_helm(&["history", "release"]), Safety::ReadOnly);
        assert_eq!(classify_helm(&["lint", "./chart"]), Safety::ReadOnly);
        assert_eq!(
            classify_helm(&["template", "release", "./chart"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_helm(&["show", "chart", "./chart"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_helm(&["search", "repo", "nginx"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_helm(&["get", "values", "release"]),
            Safety::ReadOnly
        );
        assert_eq!(
            classify_helm(&["get", "manifest", "release"]),
            Safety::ReadOnly
        );
        assert_eq!(classify_helm(&["repo", "list"]), Safety::ReadOnly);
        assert_eq!(classify_helm(&["dependency", "list"]), Safety::ReadOnly);
        assert_eq!(classify_helm(&["plugin", "list"]), Safety::ReadOnly);
    }

    #[test]
    fn helm_write_commands() {
        assert_eq!(
            classify_helm(&["install", "release", "./chart"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["upgrade", "release", "./chart"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["uninstall", "release"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["rollback", "release", "1"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["repo", "add", "stable", "url"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["repo", "remove", "stable"]),
            Safety::WriteOperation
        );
        assert_eq!(classify_helm(&["repo", "update"]), Safety::WriteOperation);
        assert_eq!(
            classify_helm(&["dependency", "build"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["dependency", "update"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["create", "my-chart"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["package", "./chart"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["push", "./chart.tgz", "repo"]),
            Safety::WriteOperation
        );
        assert_eq!(
            classify_helm(&["plugin", "install", "url"]),
            Safety::WriteOperation
        );
    }
}
