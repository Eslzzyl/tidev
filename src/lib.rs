use anyhow::Context;
use clap::Parser;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(name = "tidev", version, about = "TiDev")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Start gateway server (all enabled platforms: Telegram, QQ, etc.)
    #[cfg(feature = "gateway")]
    Gateway,
    /// Start TUI (default when no subcommand is given)
    #[cfg(feature = "tui")]
    Tui,
    /// Start web server
    #[cfg(feature = "web")]
    Web {
        /// Host to bind to
        #[arg(short = 'H', long)]
        host: Option<String>,
        /// Port to listen on
        #[arg(short, long)]
        port: Option<u16>,
        /// Serve frontend from filesystem (web/dist) instead of embedded assets
        #[arg(long)]
        dev_fs: bool,
        /// Workspace root path (defaults to current directory)
        #[arg(short, long)]
        workspace: Option<std::path::PathBuf>,
    },
    /// Export session(s) to a SQLite database
    Export {
        /// Session UUID(s) to export (repeat the flag for multiple sessions)
        #[arg(short, long)]
        session: Vec<String>,
        /// Export all sessions
        #[arg(short, long)]
        all: bool,
        /// Output SQLite database file path
        #[arg(short, long, default_value = "./export.sqlite")]
        output: std::path::PathBuf,
        /// Keep zstd compression (smaller file, for sync/import)
        #[arg(short, long)]
        compress: bool,
    },
    /// Import session(s) from an exported SQLite database
    Import {
        /// Path to the exported SQLite file
        input: std::path::PathBuf,
        /// Only import specific sessions (by UUID); omit to import all
        #[arg(short, long)]
        session: Vec<String>,
        /// Replace existing sessions with the same UUID
        #[arg(long)]
        replace: bool,
    },
    /// Manage temporary files created by tidev
    Tmp {
        #[command(subcommand)]
        action: TmpCommand,
    },
    /// Manage database schema (migrations, status, etc.)
    Db {
        #[command(subcommand)]
        action: DbCommand,
    },
    /// Sync sessions with remote machines via SSH
    Sync {
        #[command(subcommand)]
        action: SyncCommand,
    },
}

#[derive(clap::Subcommand, Debug)]
enum TmpCommand {
    /// List known temp files in /tmp
    List {
        /// Only show files older than this many minutes (default: 0 = all)
        #[arg(long, default_value = "0")]
        min_age_minutes: u64,
    },
    /// Delete old temp files
    Clean {
        /// Only delete files older than this many minutes (default: 60)
        #[arg(long, default_value = "60")]
        min_age_minutes: u64,
        /// Dry-run: list what would be deleted without actually removing anything
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(clap::Subcommand, Debug)]
enum DbCommand {
    /// Apply pending schema migrations
    Migrate,
    /// Show migration status (current version vs. latest)
    Status,
}

#[derive(clap::Subcommand, Debug)]
enum SyncCommand {
    /// Add or update a remote machine configuration
    AddRemote {
        /// Name for this remote (e.g. "devbox")
        name: String,
        /// SSH host alias or user@host (e.g. "devbox" or "eslzzyl@192.168.1.100")
        host: String,
        /// Override tidev binary path on remote
        #[arg(long)]
        tidev_path: Option<String>,
    },
    /// Remove a remote machine configuration
    Remove {
        /// Name of the remote to remove
        name: String,
    },
    /// List configured remote machines
    List,
    /// Push local session(s) to a remote machine
    Push {
        /// Remote machine name
        remote: String,
        /// Session UUID(s) to push (repeat the flag for multiple sessions)
        #[arg(short, long)]
        session: Vec<String>,
        /// Push all sessions
        #[arg(short, long)]
        all: bool,
        /// Replace existing sessions on remote
        #[arg(long)]
        replace: bool,
    },
    /// Pull session(s) from a remote machine
    Pull {
        /// Remote machine name
        remote: String,
        /// Session UUID(s) to pull (omit to pull all)
        #[arg(short, long)]
        session: Vec<String>,
        /// Replace existing local sessions
        #[arg(long)]
        replace: bool,
    },
}
pub fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => {
            // Auto-cleanup on startup
            auto_cleanup_on_startup();
            #[cfg(feature = "tui")]
            return tidev_tui::run();
            #[cfg(not(feature = "tui"))]
            {
                // No default frontend enabled, show help
                use clap::CommandFactory;
                let mut cmd = Cli::command();
                cmd.print_help()?;
                println!();
                Ok(())
            }
        }
        #[cfg(feature = "gateway")]
        Some(Command::Gateway) => {
            auto_cleanup_on_startup();
            tidev_gateway::run()
        }
        #[cfg(feature = "tui")]
        Some(Command::Tui) => {
            auto_cleanup_on_startup();
            tidev_tui::run()
        }
        #[cfg(feature = "web")]
        Some(Command::Web {
            host,
            port,
            dev_fs,
            workspace,
        }) => {
            auto_cleanup_on_startup();
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(tidev_web::run(tidev_web::WebOptions {
                host,
                port,
                dev_fs,
                workspace_root: workspace,
            }))
        }
        Some(Command::Export {
            session,
            all,
            output,
            compress,
        }) => {
            if session.is_empty() && !all {
                anyhow::bail!(
                    "Please specify at least one --session UUID or --all to export all sessions"
                );
            }
            let paths = tidev_engine::config::ConfigPaths::discover()?;
            let store = tidev_storage::SessionStore::open(paths.database_file)?;

            let session_ids: Vec<uuid::Uuid> = if all {
                store
                    .load_all_sessions()?
                    .into_iter()
                    .map(|s| s.session_id)
                    .collect()
            } else {
                session
                    .into_iter()
                    .map(|s| {
                        uuid::Uuid::parse_str(&s)
                            .with_context(|| format!("invalid session UUID: {s}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };

            if session_ids.is_empty() {
                anyhow::bail!("No sessions to export");
            }

            store.export_to_sqlite(&session_ids, &output, compress)?;
            eprintln!(
                "Exported {} session(s) to {}",
                session_ids.len(),
                output.display()
            );
            Ok(())
        }
        Some(Command::Import {
            input,
            session,
            replace,
        }) => {
            if !input.exists() {
                anyhow::bail!("Import file not found: {}", input.display());
            }
            let paths = tidev_engine::config::ConfigPaths::discover()?;
            let store = tidev_storage::SessionStore::open(paths.database_file)?;
            let count = store.import_from_sqlite(&input, &session, replace)?;
            eprintln!("Imported {} session(s) from {}", count, input.display());
            Ok(())
        }
        Some(Command::Tmp { action }) => match action {
            TmpCommand::List { min_age_minutes } => {
                let entries = tidev_engine::tmp::scan_temp_files()?;
                let min_age = Duration::from_secs(min_age_minutes * 60);

                if entries.is_empty() {
                    println!("No tidev temp files found in /tmp");
                    return Ok(());
                }

                for entry in &entries {
                    if entry.age_secs < min_age.as_secs() {
                        continue;
                    }
                    let kind = if entry.path.is_dir() { "dir " } else { "file" };
                    println!(
                        "  {}  {:>8}s  {}",
                        kind,
                        entry.age_secs,
                        entry.path.display()
                    );
                }
                Ok(())
            }
            TmpCommand::Clean {
                min_age_minutes,
                dry_run,
            } => {
                let max_age = Duration::from_secs(min_age_minutes * 60);
                let removed = tidev_engine::tmp::clean_temp_files(max_age, dry_run)?;

                if removed.is_empty() {
                    println!("No temp files to clean");
                    return Ok(());
                }

                for entry in &removed {
                    let kind = if entry.path.is_dir() { "dir " } else { "file" };
                    let action = if dry_run { "would remove" } else { "removed" };
                    println!(
                        "  {}  {}  {} ({}s old)",
                        kind,
                        action,
                        entry.path.display(),
                        entry.age_secs
                    );
                }

                if !dry_run {
                    println!("Cleaned {} temp file(s)", removed.len());
                } else {
                    println!("Would clean {} temp file(s) (dry-run)", removed.len());
                }
                Ok(())
            }
        },
        Some(Command::Db { action }) => match action {
            DbCommand::Migrate => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let db = tidev_storage::database::Database::open(&paths.database_file)?;
                eprintln!("Database migrated successfully.");
                drop(db);
                Ok(())
            }
            DbCommand::Status => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let db_path = &paths.database_file;
                if !db_path.exists() {
                    println!("Database does not exist yet at: {}", db_path.display());
                    println!(
                        "Latest schema version: {}",
                        tidev_storage::schema::SCHEMA_VERSION
                    );
                    return Ok(());
                }
                let conn = rusqlite::Connection::open(db_path)?;
                let status = tidev_storage::migration::status(&conn)?;
                println!("Current version: {}", status.current_version);
                println!("Latest version:  {}", status.latest_version);
                println!("Pending:         {}", status.pending_count);
                Ok(())
            }
        },
        Some(Command::Sync { action }) => match action {
            SyncCommand::List => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let config = tidev_engine::config::AppConfig::load_or_create(&paths)?;
                if config.sync.remotes.is_empty() {
                    println!("No remotes configured. Use 'tidev sync add-remote' to add one.");
                } else {
                    println!("Configured remotes:");
                    for remote in &config.sync.remotes {
                        let last = remote.last_sync_at.as_deref().unwrap_or("never");
                        println!("  {}  {}  ({})", remote.name, remote.display_name(), last);
                    }
                }
                Ok(())
            }
            SyncCommand::AddRemote {
                name,
                host,
                tidev_path,
            } => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let mut config = tidev_engine::config::AppConfig::load_or_create(&paths)?;

                let remote = tidev_engine::sync::RemoteMachine {
                    name: name.clone(),
                    host: host.clone(),
                    tidev_path,
                    last_sync_at: None,
                };

                config.sync.remotes.push(remote);
                config.save(&paths)?;
                eprintln!("Remote '{}' added successfully.", name);
                Ok(())
            }
            SyncCommand::Remove { name } => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let mut config = tidev_engine::config::AppConfig::load_or_create(&paths)?;
                let len = config.sync.remotes.len();
                config.sync.remotes.retain(|r| r.name != name);
                if config.sync.remotes.len() < len {
                    config.save(&paths)?;
                    eprintln!("Remote '{}' removed.", name);
                } else {
                    anyhow::bail!("Remote '{}' not found.", name);
                }
                Ok(())
            }
            SyncCommand::Push {
                remote,
                session,
                all,
                replace,
            } => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let config = tidev_engine::config::AppConfig::load_or_create(&paths)?;
                let store = tidev_storage::SessionStore::open(&paths.database_file)?;
                let manager = tidev_engine::sync::SyncManager::new(config.sync.clone(), store);

                let session_ids: Vec<uuid::Uuid> = if all {
                    manager
                        .store
                        .load_all_sessions()?
                        .into_iter()
                        .map(|s| s.session_id)
                        .collect()
                } else if !session.is_empty() {
                    session
                        .into_iter()
                        .map(|s| {
                            uuid::Uuid::parse_str(&s)
                                .with_context(|| format!("invalid session UUID: {s}"))
                        })
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    anyhow::bail!("Specify --session or --all to select sessions to push.");
                };

                let summary = manager.push(&session_ids, &remote, replace)?;
                eprintln!(
                    "Pushed {} session(s) to '{}' ({} bytes)",
                    summary.sessions_count, summary.remote_name, summary.total_bytes
                );
                Ok(())
            }
            SyncCommand::Pull {
                remote,
                session,
                replace,
            } => {
                let paths = tidev_engine::config::ConfigPaths::discover()?;
                let config = tidev_engine::config::AppConfig::load_or_create(&paths)?;
                let store = tidev_storage::SessionStore::open(&paths.database_file)?;
                let manager = tidev_engine::sync::SyncManager::new(config.sync.clone(), store);

                let summary = manager.pull(&session, &remote, replace)?;
                eprintln!(
                    "Pulled {} session(s) from '{}' ({} bytes)",
                    summary.sessions_count, summary.remote_name, summary.total_bytes
                );
                Ok(())
            }
        },
    }
}

/// Try to perform auto-cleanup of old temp files on startup.
/// Silently ignores errors (e.g., config file not found).
fn auto_cleanup_on_startup() {
    let paths = match tidev_engine::config::ConfigPaths::discover() {
        Ok(p) => p,
        Err(_) => return,
    };
    let config = match tidev_engine::config::AppConfig::load_or_create(&paths) {
        Ok(c) => c,
        Err(_) => return,
    };
    tidev_engine::tmp::auto_cleanup(&config.tmp);

    // Clean tool outputs older than 7 days
    if let Ok(store) = tidev_storage::SessionStore::open(&paths.database_file)
        && let Ok(count) = store.delete_expired_tool_outputs(7)
        && count > 0
    {
        log::info!("Cleaned up {count} old tool output(s)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_command_as_app_mode() {
        let cli = Cli::parse_from(["tidev"]);
        assert!(cli.command.is_none());
    }

    #[cfg(feature = "gateway")]
    #[test]
    fn parses_gateway_command() {
        let cli = Cli::parse_from(["tidev", "gateway"]);
        assert!(matches!(cli.command, Some(Command::Gateway)));
    }
}
