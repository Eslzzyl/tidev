use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use uuid::Uuid;

mod cli;

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

    // ── Session portability ─────────────────────────────────────────
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

    /// Import sessions from an uncompressed SQLite export file
    Import {
        /// Path to the export SQLite file
        file: PathBuf,

        /// Session UUID(s) to import (repeat for multiple; all if omitted)
        #[arg(short, long)]
        session: Vec<String>,

        /// Replace existing sessions with the same UUID
        #[arg(short, long)]
        replace: bool,
    },

    // ── Authentication ──────────────────────────────────────────────
    /// Manage API keys
    #[command(subcommand)]
    Auth(AuthCommand),

    // ── Model management ────────────────────────────────────────────
    /// Manage models
    #[command(subcommand)]
    Model(ModelCommand),

    // ── Configuration ───────────────────────────────────────────────
    /// Manage configuration
    #[command(subcommand)]
    Config(ConfigCommand),

    // ── Diagnostics ─────────────────────────────────────────────────
    /// Show diagnostic information about the tidev installation
    Info,

    // ── Database maintenance ────────────────────────────────────────
    /// Manage the tidev database
    #[command(subcommand)]
    Db(DbCommand),

    // ── Temporary files ─────────────────────────────────────────────
    /// Manage tidev temporary files
    #[command(subcommand)]
    Tmp(TmpCommand),

    // ── Session maintenance ─────────────────────────────────────────
    /// Manage sessions
    #[command(subcommand)]
    Session(SessionCommand),
}

// ── Auth subcommands ─────────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum AuthCommand {
    /// Set an API key for a provider
    Set {
        /// Provider name (e.g. "openai", "anthropic")
        provider: String,
        /// API key value
        key: String,
    },
    /// List configured API keys (masked)
    List,
    /// Remove an API key for a provider
    Remove {
        /// Provider name
        provider: String,
    },
}

// ── Model subcommands ────────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum ModelCommand {
    /// List all available models from configuration
    List,
    /// Set the default provider and model
    Set {
        /// Provider name (e.g. "openai")
        provider: String,
        /// Model ID (e.g. "gpt-4o")
        model_id: String,
    },
}

// ── Config subcommands ───────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum ConfigCommand {
    /// Show the current configuration
    Show,
}

// ── Db subcommands ───────────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum DbCommand {
    /// Run database maintenance (VACUUM + ANALYZE)
    Maintain,
    /// Run pending schema migrations
    Migrate,
}

// ── Tmp subcommands ──────────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum TmpCommand {
    /// List tidev temporary files
    List {
        /// Minimum age in minutes (only show files older than this)
        #[arg(short, long, default_value = "0")]
        min_age_minutes: u64,
    },
    /// Clean tidev temporary files older than the given age
    Clean {
        /// Minimum age in minutes (clean files older than this)
        #[arg(short, long, default_value = "60")]
        min_age_minutes: u64,
        /// Perform a dry run without deleting anything
        #[arg(short, long)]
        dry_run: bool,
    },
}

// ── Session subcommands ──────────────────────────────────────────────

#[derive(clap::Subcommand, Debug)]
enum SessionCommand {
    /// Delete sessions not updated for more than N days
    Prune {
        /// Number of days
        older_than_days: u64,
    },
}

// ── Main ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        None => tidev_tui::run().await,
        Some(Command::Tui) => tidev_tui::run().await,

        // ── Export ──────────────────────────────────────────────
        Some(Command::Export {
            session,
            all,
            output,
        }) => run_export(session, all, output),

        // ── Import ──────────────────────────────────────────────
        Some(Command::Import {
            file,
            session,
            replace,
        }) => cli::import(file, session, replace),

        // ── Auth ────────────────────────────────────────────────
        Some(Command::Auth(cmd)) => match cmd {
            AuthCommand::Set { provider, key } => cli::auth_set(&provider, &key),
            AuthCommand::List => cli::auth_list(),
            AuthCommand::Remove { provider } => cli::auth_remove(&provider),
        },

        // ── Model ───────────────────────────────────────────────
        Some(Command::Model(cmd)) => match cmd {
            ModelCommand::List => cli::model_list(),
            ModelCommand::Set { provider, model_id } => cli::model_set(&provider, &model_id),
        },

        // ── Config ──────────────────────────────────────────────
        Some(Command::Config(cmd)) => match cmd {
            ConfigCommand::Show => cli::config_show(),
        },

        // ── Info ────────────────────────────────────────────────
        Some(Command::Info) => cli::info(),

        // ── Db ──────────────────────────────────────────────────
        Some(Command::Db(cmd)) => match cmd {
            DbCommand::Maintain => cli::db_maintain(),
            DbCommand::Migrate => cli::db_migrate(),
        },

        // ── Tmp ─────────────────────────────────────────────────
        Some(Command::Tmp(cmd)) => match cmd {
            TmpCommand::List { min_age_minutes } => cli::tmp_list(min_age_minutes),
            TmpCommand::Clean {
                min_age_minutes,
                dry_run,
            } => cli::tmp_clean(min_age_minutes, dry_run),
        },

        // ── Session ─────────────────────────────────────────────
        Some(Command::Session(cmd)) => match cmd {
            SessionCommand::Prune { older_than_days } => cli::session_prune(older_than_days),
        },
    }
}

// ── Export handler (kept from the original) ──────────────────────────

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
