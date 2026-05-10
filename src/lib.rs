pub mod agent;
pub mod balance;
pub mod config;
pub mod context;
pub mod gateway;
pub mod instructions;
pub mod llm;
pub mod logging;
pub mod markdown_render;
pub mod mcp;
pub mod memory;
pub mod notifications;
pub mod prompts;
pub mod provider_setup;
pub mod session;
pub mod shared;
pub mod snapshot;
pub mod stats;
pub mod storage;
pub mod system_info;
pub mod theme;
pub mod tooling;
pub mod tui;
pub mod tmp;
pub mod utils;
pub mod web;

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
    Gateway,
    /// Start web server
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
    /// Export session(s) to a plain SQLite database (without zstd compression)
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
    },
    /// Manage temporary files created by tidev
    Tmp {
        #[command(subcommand)]
        action: TmpCommand,
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
pub fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => {
            // Auto-cleanup on startup (before TUI starts)
            auto_cleanup_on_startup();
            tui::run()
        }
        Some(Command::Gateway) => {
            auto_cleanup_on_startup();
            gateway::run()
        }
        Some(Command::Web {
            host,
            port,
            dev_fs,
            workspace,
        }) => {
            auto_cleanup_on_startup();
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(web::run(web::WebOptions {
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
        }) => {
            if session.is_empty() && !all {
                anyhow::bail!(
                    "Please specify at least one --session UUID or --all to export all sessions"
                );
            }
            let paths = crate::config::ConfigPaths::discover()?;
            let store = storage::SessionStore::open(paths.database_file)?;

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

            store.export_to_sqlite(&session_ids, &output)?;
            eprintln!(
                "Exported {} session(s) to {}",
                session_ids.len(),
                output.display()
            );
            Ok(())
        }
        Some(Command::Tmp { action }) => match action {
            TmpCommand::List {
                min_age_minutes,
            } => {
                let entries = crate::tmp::scan_temp_files()?;
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
                let removed = crate::tmp::clean_temp_files(max_age, dry_run)?;

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
    }
}

/// Try to perform auto-cleanup of old temp files on startup.
/// Silently ignores errors (e.g., config file not found).
fn auto_cleanup_on_startup() {
    let paths = match crate::config::ConfigPaths::discover() {
        Ok(p) => p,
        Err(_) => return,
    };
    let config = match crate::config::AppConfig::load_or_create(&paths) {
        Ok(c) => c,
        Err(_) => return,
    };
    crate::tmp::auto_cleanup(&config.tmp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_command_as_app_mode() {
        let cli = Cli::parse_from(["tidev"]);
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_gateway_command() {
        let cli = Cli::parse_from(["tidev", "gateway"]);
        assert!(matches!(cli.command, Some(Command::Gateway)));
    }
}
