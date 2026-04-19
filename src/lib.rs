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
pub mod theme;
pub mod tooling;

pub fn run() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);

    match args.next().as_deref() {
        None => app::run(),
        Some("gateway") => match args.next().as_deref() {
            None | Some("telegram") => gateway::run(),
            Some(other) => {
                anyhow::bail!("unknown gateway target '{other}', expected 'telegram'")
            }
        },
        Some("-h") | Some("--help") => {
            print_usage();
            Ok(())
        }
        Some("-V") | Some("--version") => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => anyhow::bail!(
            "unknown command '{other}'.\n\nUsage:\n  tidev\n  tidev gateway\n  tidev gateway telegram"
        ),
    }
}

fn print_usage() {
    println!(
        "TiDev\n\nUsage:\n  tidev                    Start TUI mode\n  tidev gateway            Start Telegram gateway mode\n  tidev gateway telegram   Start Telegram gateway mode"
    );
}
