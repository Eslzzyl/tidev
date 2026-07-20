use super::*;

/// Classify terraform/tofu (OpenTofu) commands by subcommand.
///
/// Read-only: plan, show, output, graph, version, validate, console,
///            fmt -check, state list/show, workspace list/show,
///            providers schema/mirror
/// Write: apply, destroy, init, fmt, refresh, import, taint, untaint,
///        force-unlock, state mv/rm/push/replace-provider,
///        workspace new/delete/select, providers lock, test, bench
pub(super) fn classify_terraform(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `terraform` — help
    };

    match sub {
        // Read-only — no sub-subcommand nuance
        "plan" | "show" | "output" | "graph" | "version" | "validate" | "metadata"
        | "console" => Safety::ReadOnly,

        // `terraform fmt -check` / `terraform fmt -list` is read-only
        // `terraform fmt` without flags is write (rewrites files)
        "fmt" => {
            if args.contains(&"-check") || args.contains(&"-list") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }

        // `terraform state` read vs write
        "state" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "show" | "pull" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // mv, rm, push, replace-provider
            }
        }

        // `terraform workspace` read vs write
        "workspace" => {
            let action = args.get(1).copied().unwrap_or("list");
            match action {
                "list" | "show" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // new, delete, select
            }
        }

        // `terraform providers` — read-only (schema/mirror) vs write (lock)
        "providers" => {
            let action = args.get(1).copied().unwrap_or("");
            match action {
                "" | "schema" | "mirror" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // lock
            }
        }

        // Write
        _ => Safety::WriteOperation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terraform_read_commands() {
        assert_eq!(classify_terraform(&["plan"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["show"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["output"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["graph"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["version"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["validate"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["fmt", "-check"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["fmt", "-list"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["state", "list"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["state", "show", "resource"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["workspace", "list"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["workspace", "show"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["providers"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["providers", "schema"]), Safety::ReadOnly);
        assert_eq!(classify_terraform(&["providers", "mirror", "/dir"]), Safety::ReadOnly);
    }

    #[test]
    fn terraform_write_commands() {
        assert_eq!(classify_terraform(&["apply"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["apply", "-auto-approve"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["destroy"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["init"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["fmt"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["import", "resource", "id"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["refresh"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["taint", "resource"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["untaint", "resource"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["state", "mv", "old", "new"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["state", "rm", "resource"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["workspace", "new", "prod"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["workspace", "delete", "prod"]), Safety::WriteOperation);
        assert_eq!(classify_terraform(&["providers", "lock"]), Safety::WriteOperation);
    }
}
