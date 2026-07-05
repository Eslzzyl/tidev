use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser, Debug)]
#[command(name = "tidev", version, about = "tidev — A terminal-based AI coding agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Start the TUI (default when no subcommand is given)
    Tui,
    /// Export session(s) to an uncompressed SQLite database
    Export {
        /// Session UUID(s) to export (repeat for multiple)
        #[arg(short, long)]
        session: Vec<String>,
        /// Export all sessions
        #[arg(short, long)]
        all: bool,
        /// Output database file path
        #[arg(short, long, default_value = "./tidev-export.db")]
        output: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Logging is initialised inside Runtime::builder().build() via tidev_logging.
    // No env_logger here — the custom TidevLogger handles both file and console.

    match Cli::parse().command {
        None => run_tui().await,
        Some(Command::Tui) => run_tui().await,
        Some(Command::Export {
            session,
            all,
            output,
        }) => run_export(session, all, output),
    }
}

/// Start the terminal UI.
async fn run_tui() -> Result<()> {
    let runtime = tidev_core::Runtime::builder()
        .workspace_root(std::env::current_dir()?)
        .build()
        .await?;

    let mut app = tidev_tui::App::new(runtime);
    app.run().await
}

/// Export one or more sessions to a portable SQLite file.
fn run_export(session: Vec<String>, all: bool, output: PathBuf) -> Result<()> {
    if session.is_empty() && !all {
        anyhow::bail!("Specify at least one --session <UUID> or --all to export all sessions");
    }

    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let database =
        tidev_storage::database::Database::open(&paths.database_file)
            .context("failed to open database")?;
    let store = database.create_store()?;

    let session_ids: Vec<Uuid> = if all {
        store
            .list_sessions(i64::MAX, 0)?
            .into_iter()
            .map(|s| s.session_id)
            .collect()
    } else {
        session
            .into_iter()
            .map(|s| {
                Uuid::parse_str(&s)
                    .with_context(|| format!("invalid session UUID: {s}"))
            })
            .collect::<Result<Vec<_>>>()?
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
