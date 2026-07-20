use super::*;

/// Classify gem commands by subcommand.
///
/// Read-only: list, which, environment, contents, dependency, search, help,
///            version, outdated, pristine --check, cert --check
/// Write: install, uninstall, update, build, push, owner, cleanup,
///        generate_index, server, unpack, fetch, pristine, cert, sign,
///        specification, sources
pub(super) fn classify_gem(args: &[&str]) -> Safety {
    let Some(sub) = args.first().copied() else {
        return Safety::Unknown; // bare `gem` — help
    };

    match sub {
        // Read-only
        "list" | "which" | "environment" | "contents" | "dependency" | "search"
        | "help" | "version" | "outdated" => Safety::ReadOnly,

        // `gem pristine --check` is read-only; `gem pristine` without --check is write
        "pristine" => {
            if args.contains(&"--check") {
                Safety::ReadOnly
            } else {
                Safety::WriteOperation
            }
        }

        // `gem cert --check` is read-only; other cert ops are write
        "cert" => {
            let action = args.get(1).copied().unwrap_or("");
            match action {
                "-c" | "--check" | "help" => Safety::ReadOnly,
                _ => Safety::WriteOperation, // --add, --remove, --build, --sign
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
    fn gem_read_commands() {
        assert_eq!(classify_gem(&["list"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["list", "--local"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["which", "rake"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["environment"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["contents", "rake"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["dependency", "rake"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["search", "rails"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["outdated"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["pristine", "--check"]), Safety::ReadOnly);
        assert_eq!(classify_gem(&["cert", "--check"]), Safety::ReadOnly);
    }

    #[test]
    fn gem_write_commands() {
        assert_eq!(classify_gem(&["install", "rails"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["install", "--no-doc", "rails"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["uninstall", "rails"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["update"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["build", "mygem.gemspec"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["push", "mygem-1.0.gem"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["owner", "-a", "gem", "email"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["cleanup"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["pristine", "rails"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["cert", "--build", "email"]), Safety::WriteOperation);
        assert_eq!(classify_gem(&["sources", "-a", "url"]), Safety::WriteOperation);
    }
}
