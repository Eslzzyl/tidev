use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use uuid::Uuid;

mod cli;
mod headless;

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ExportFormat {
    Sqlite,
    Jsonl,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum SessionOutputFormat {
    Text,
    Json,
}

#[derive(Parser, Debug)]
#[command(
    name = "tidev",
    version,
    about = "tidev — A terminal-based AI coding agent"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Start the TUI (default when no subcommand is given)
    Tui,

    /// Run one agent session without starting the TUI
    Run {
        /// Workspace root (defaults to the current directory)
        #[arg(long)]
        workspace: Option<PathBuf>,

        /// User instruction for this run
        #[arg(long, conflicts_with_all = ["instruction_file", "stdin"])]
        instruction: Option<String>,

        /// Read the user instruction from a file
        #[arg(long, conflicts_with_all = ["instruction", "stdin"])]
        instruction_file: Option<PathBuf>,

        /// Read the user instruction from stdin
        #[arg(long, conflicts_with_all = ["instruction", "instruction_file"])]
        stdin: bool,
    },

    // ── Session portability ─────────────────────────────────────────
    /// Start as an ACP agent over stdio
    Acp,

    /// Start the Web frontend and API server
    Web {
        /// Bind address (defaults to 127.0.0.1)
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Listen port (defaults to 26502)
        #[arg(long, default_value_t = 26502)]
        port: u16,

        /// Workspace root (defaults to the current directory)
        #[arg(long)]
        workspace: Option<PathBuf>,
    },

    /// Export session(s) to SQLite or JSONL
    Export {
        /// Session UUID(s) to export (repeat for multiple)
        #[arg(short, long)]
        session: Vec<String>,

        /// Export all sessions
        #[arg(short, long)]
        all: bool,

        /// Export format
        #[arg(short, long, value_enum, default_value_t = ExportFormat::Sqlite)]
        format: ExportFormat,

        /// Output file path (defaults to a format-specific file)
        #[arg(short, long)]
        output: Option<PathBuf>,
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

    /// Output the complete content of a stored tool execution result to stdout
    #[command(name = "tool-output")]
    ToolOutput {
        /// The tool output ID (e.g. out-a8f3b9c1)
        id: String,
    },
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
    /// List sessions, including child sessions
    List {
        /// Search session titles or UUIDs
        #[arg(short, long)]
        query: Option<String>,
        /// Maximum number of sessions to display
        #[arg(short, long, default_value_t = 50)]
        limit: u64,
        /// Number of sessions to skip
        #[arg(long, default_value_t = 0)]
        offset: u64,
        /// Output format
        #[arg(long, value_enum, default_value_t = SessionOutputFormat::Text)]
        format: SessionOutputFormat,
    },
    /// Show a complete session or one message without starting the TUI
    #[command(alias = "view")]
    Show {
        /// Session UUID
        session_id: String,
        /// Show only this message in the session
        #[arg(long)]
        message_id: Option<String>,
        /// Output format
        #[arg(long, value_enum, default_value_t = SessionOutputFormat::Text)]
        format: SessionOutputFormat,
    },
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
        Some(Command::Run {
            workspace,
            instruction,
            instruction_file,
            stdin,
        }) => headless::run(workspace, instruction, instruction_file, stdin).await,
        Some(Command::Acp) => tidev_acp::run_acp_agent().await,
        Some(Command::Web {
            host,
            port,
            workspace,
        }) => {
            tidev_web::run(tidev_web::WebOptions {
                host,
                port,
                workspace,
            })
            .await
        }

        // ── Export ──────────────────────────────────────────────
        Some(Command::Export {
            session,
            all,
            format,
            output,
        }) => run_export(session, all, format, output),

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
            SessionCommand::List {
                query,
                limit,
                offset,
                format,
            } => cli::session_list(query, limit, offset, format),
            SessionCommand::Show {
                session_id,
                message_id,
                format,
            } => cli::session_show(&session_id, message_id.as_deref(), format),
            SessionCommand::Prune { older_than_days } => cli::session_prune(older_than_days),
        },

        // ── Tool output dump ────────────────────────────────────
        Some(Command::ToolOutput { id }) => cli::print_tool_output(&id),
    }
}

// ── Export handler (kept from the original) ──────────────────────────

/// Export one or more sessions in the requested format.
fn run_export(
    session: Vec<String>,
    all: bool,
    format: ExportFormat,
    output: Option<PathBuf>,
) -> Result<()> {
    if session.is_empty() && !all {
        anyhow::bail!("Specify at least one --session <UUID> or --all to export all sessions");
    }

    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let database = tidev_storage::database::Database::open(&paths.database_file)
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
            .map(|s| Uuid::parse_str(&s).with_context(|| format!("invalid session UUID: {s}")))
            .collect::<Result<Vec<_>>>()?
    };

    if session_ids.is_empty() {
        anyhow::bail!("No sessions to export");
    }

    let output = output.unwrap_or_else(|| match format {
        ExportFormat::Sqlite => PathBuf::from("./tidev-export.db"),
        ExportFormat::Jsonl => PathBuf::from("./tidev-export.jsonl"),
    });

    match format {
        ExportFormat::Sqlite => {
            store.export_to_sqlite(&session_ids, &output)?;
            eprintln!(
                "Exported {} session(s) to {}",
                session_ids.len(),
                output.display()
            );
        }
        ExportFormat::Jsonl => {
            let message_count = store.export_to_jsonl(&session_ids, &output)?;
            eprintln!(
                "Exported {} session(s) and {} message(s) to {}",
                session_ids.len(),
                message_count,
                output.display()
            );
        }
    }
    Ok(())
}
