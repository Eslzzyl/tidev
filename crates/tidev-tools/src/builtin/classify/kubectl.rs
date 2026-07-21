use super::*;

/// Classify kubectl commands by subcommand.
///
/// Read-only: get, describe, logs, top, explain, api-resources, api-versions,
///            version, cluster-info, config view, diff, options, help
/// Write: apply, create, delete, edit, patch, replace, rollout, scale,
///        autoscale, cordon, uncordon, drain, taint, label, annotate,
///        exec, port-forward, proxy, cp, auth, set, expose, run
pub(super) fn classify_kubectl(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `kubectl` — help
    };

    match sub {
        // Read-only
        "get" | "describe" | "logs" | "top" | "explain" | "api-resources" | "api-versions"
        | "version" | "cluster-info" | "diff" | "options" | "help" | "completion"
        | "plugin" => Safety::ReadOnly,

        // `kubectl config`: view/current-context/get-cluster/get-context/get-users/get-credentials
        // are read-only; everything else is write
        "config" => {
            let action = args.get(1).copied().unwrap_or("view");
            match action {
                "view" | "current-context" | "get-clusters" | "get-contexts"
                | "get-users" | "get-credentials" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // set-cluster, set-context, set-credentials,
                                             // unset, use-context, rename-context, delete-cluster,
                                             // delete-context, delete-user
            }
        }

        // Explicit write commands
        "apply" | "create" | "delete" | "edit" | "patch" | "replace" | "rollout"
        | "scale" | "autoscale" | "cordon" | "uncordon" | "drain" | "taint"
        | "label" | "annotate" | "port-forward" | "proxy" | "cp"
        | "auth" | "set" | "expose" | "run" | "attach" | "debug" => Safety::WriteOperation,

        // exec runs an arbitrary command inside a pod — could be read or write
        "exec" => Safety::Unknown,

        // Everything else — ambiguous, let through
        _ => Safety::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kubectl_read_commands() {
        assert_eq!(classify_kubectl(&["get", "pods"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["get", "pods", "-o", "yaml"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["describe", "pod", "nginx"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["logs", "nginx"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["top", "pod"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["explain", "pod"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["api-resources"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["version"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["cluster-info"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["diff", "-f", "file.yaml"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["config", "view"]), Safety::ReadOnly);
        assert_eq!(classify_kubectl(&["config", "current-context"]), Safety::ReadOnly);
    }

    #[test]
    fn kubectl_write_commands() {
        assert_eq!(classify_kubectl(&["apply", "-f", "deploy.yaml"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["create", "deployment", "nginx"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["delete", "pod", "nginx"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["edit", "deployment/nginx"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["patch", "pod/nginx", "-p", "{}"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["rollout", "restart", "deploy/nginx"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["scale", "--replicas=3", "deploy/nginx"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["exec", "-it", "pod", "--", "bash"]), Safety::Unknown);
        assert_eq!(classify_kubectl(&["port-forward", "pod", "8080:80"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["label", "pod/nginx", "env=prod"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["annotate", "pod/nginx", "key=val"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["config", "set-context", "prod"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["config", "use-context", "prod"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["cordon", "node1"]), Safety::WriteOperation);
        assert_eq!(classify_kubectl(&["drain", "node1"]), Safety::WriteOperation);
    }
}
