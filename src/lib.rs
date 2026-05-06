pub mod agent;
pub mod balance;
pub mod config;
pub mod context;
pub mod delegate;
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
pub mod utils;
pub mod web;

use clap::Parser;
use anyhow::Context;

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
}
pub fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => tui::run(),
        Some(Command::Gateway) => gateway::run(),
        Some(Command::Web {
            host,
            port,
            dev_fs,
            workspace,
        }) => {
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
                anyhow::bail!("Please specify at least one --session UUID or --all to export all sessions");
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
            eprintln!("Exported {} session(s) to {}", session_ids.len(), output.display());
            Ok(())
        }
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

    #[test]
    fn parses_gateway_command() {
        let cli = Cli::parse_from(["tidev", "gateway"]);
        assert!(matches!(cli.command, Some(Command::Gateway)));
    }
}
