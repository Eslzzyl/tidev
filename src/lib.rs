pub mod agent;
pub mod app;
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
pub mod snapshot;
pub mod stats;
pub mod storage;
pub mod system_info;
pub mod theme;
pub mod tooling;
pub mod utils;
pub mod web;

use clap::Parser;

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
    }
pub fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => app::run(),
        Some(Command::Gateway) => gateway::run(),
        Some(Command::Web { host, port, dev_fs, workspace }) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(web::run(web::WebOptions { host, port, dev_fs, workspace_root: workspace }))
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
