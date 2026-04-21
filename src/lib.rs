pub mod app;
pub mod config;
pub mod context;
pub mod gateway;
pub mod instructions;
pub mod llm;
pub mod logging;
pub mod markdown_render;
pub mod mcp;
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

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "tidev", version, about = "TiDev")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Gateway(GatewayArgs),
}

#[derive(Args, Debug)]
struct GatewayArgs {
    #[command(subcommand)]
    target: Option<GatewayTarget>,
}

#[derive(Subcommand, Debug)]
enum GatewayTarget {
    Telegram,
}

pub fn run() -> anyhow::Result<()> {
    match Cli::parse().command {
        None => app::run(),
        Some(Command::Gateway(args)) => match args.target.unwrap_or(GatewayTarget::Telegram) {
            GatewayTarget::Telegram => gateway::run(),
        },
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
    fn parses_gateway_without_target() {
        let cli = Cli::parse_from(["tidev", "gateway"]);

        match cli.command {
            Some(Command::Gateway(args)) => assert!(args.target.is_none()),
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_gateway_telegram_target() {
        let cli = Cli::parse_from(["tidev", "gateway", "telegram"]);

        match cli.command {
            Some(Command::Gateway(args)) => {
                assert!(matches!(args.target, Some(GatewayTarget::Telegram)));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
