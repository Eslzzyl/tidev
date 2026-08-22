//! CLI command handlers for the tidev binary.
//!
//! Each public function corresponds to a subcommand in [`crate::Command`].

use super::SessionOutputFormat;
use anyhow::{Context, Result};
use chrono::Duration;
use std::path::PathBuf;
use tidev_storage::{SessionInspection, StoredMessageView};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open the database and return its store together with the resolved paths.
fn open_store() -> Result<(
    tidev_config::paths::ConfigPaths,
    tidev_storage::SessionStore,
)> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let database = tidev_storage::database::Database::open(&paths.database_file)
        .context("failed to open database")?;
    let store = database.create_store()?;
    Ok((paths, store))
}

// ---------------------------------------------------------------------------
// tidev auth
// ---------------------------------------------------------------------------

/// Set an API key for a provider.
pub fn auth_set(provider: &str, key: &str) -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let mut auth = tidev_config::AuthStore::load_or_create(&paths)?;
    auth.set_api_key(provider, key);
    auth.save(&paths)?;
    println!("API key set for provider '{provider}'");
    Ok(())
}

/// List all configured API keys (masked).
pub fn auth_list() -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let auth = tidev_config::AuthStore::load_or_create(&paths)?;
    if auth.providers.is_empty() {
        println!("No API keys configured.");
    } else {
        for (provider, pa) in &auth.providers {
            let masked = pa
                .api_key
                .as_ref()
                .map(|k| {
                    if k.len() > 8 {
                        format!("{}…{}", &k[..4], &k[k.len() - 4..])
                    } else {
                        "****".to_string()
                    }
                })
                .unwrap_or_default();
            println!("{provider}: {masked}");
        }
    }
    Ok(())
}

/// Remove an API key for a provider.
pub fn auth_remove(provider: &str) -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let mut auth = tidev_config::AuthStore::load_or_create(&paths)?;
    if auth.providers.remove(provider).is_some() {
        auth.save(&paths)?;
        println!("Removed API key for provider '{provider}'");
    } else {
        println!("No API key found for provider '{provider}'");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev model
// ---------------------------------------------------------------------------

/// List all available models from configuration.
pub fn model_list() -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let config = tidev_config::AppConfig::load(&paths)?;
    let models = config.available_models();
    if models.is_empty() {
        println!("No models configured.");
    } else {
        for m in &models {
            println!(
                "{}/{}  (context: {}, max_output: {}, provider: {})",
                m.provider_id,
                m.model_id,
                m.context_window,
                m.max_output_tokens,
                m.provider_display_name,
            );
        }
    }
    Ok(())
}

/// Set the default provider and model.
pub fn model_set(provider: &str, model_id: &str) -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let mut config = tidev_config::AppConfig::load(&paths)?;
    config.default_provider = provider.to_string();
    config.default_model = model_id.to_string();
    config.save(&paths)?;
    println!("Default model set to {provider}/{model_id}");
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev config
// ---------------------------------------------------------------------------

/// Show the current configuration.
pub fn config_show() -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let config = tidev_config::AppConfig::load(&paths)?;
    println!("{config:#?}");
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev info
// ---------------------------------------------------------------------------

/// Show diagnostic information about the tidev installation.
pub fn info() -> Result<()> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    println!("tidev v{}", env!("CARGO_PKG_VERSION"));
    println!("Config directory:  {}", paths.config_dir.display());
    println!("Data directory:    {}", paths.data_dir.display());
    println!("Config file:       {}", paths.config_file.display());
    println!("Auth file:         {}", paths.auth_file.display());
    println!("Database file:     {}", paths.database_file.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev import
// ---------------------------------------------------------------------------

/// Import sessions from an uncompressed SQLite export file.
pub fn import(file: PathBuf, session: Vec<String>, replace: bool) -> Result<()> {
    let (_paths, store) = open_store()?;

    let session_ids: Option<Vec<Uuid>> = if session.is_empty() {
        None
    } else {
        Some(
            session
                .iter()
                .map(|s| Uuid::parse_str(s).with_context(|| format!("invalid session UUID: {s}")))
                .collect::<Result<Vec<_>>>()?,
        )
    };

    let imported = store.import_from_sqlite(&file, session_ids.as_deref(), replace)?;
    if imported.is_empty() {
        println!("No sessions imported (all skipped or already exist).");
    } else {
        for id in &imported {
            println!("Imported session {id}");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev db
// ---------------------------------------------------------------------------

/// Run database maintenance (VACUUM + ANALYZE).
pub fn db_maintain() -> Result<()> {
    let (paths, _store) = open_store()?;
    let database = tidev_storage::database::Database::open(&paths.database_file)
        .context("failed to open database")?;
    database.maintain()?;
    println!("Database maintenance completed.");
    Ok(())
}

/// Run pending schema migrations explicitly.
pub fn db_migrate() -> Result<()> {
    let (_paths, _store) = open_store()?;
    // Database::open already runs migrations.  Explicitly opening the
    // database ensures all pending migrations are applied.
    println!("Database schema is up to date.");
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev tmp
// ---------------------------------------------------------------------------

/// List tidev temporary files in the system temp directory.
pub fn tmp_list(min_age_minutes: u64) -> Result<()> {
    let entries = tidev_utils::tmp::scan_temp_files()?;
    let cutoff_secs = min_age_minutes * 60;
    let filtered: Vec<_> = entries
        .into_iter()
        .filter(|e| e.age_secs >= cutoff_secs)
        .collect();

    if filtered.is_empty() {
        println!("No tidev temporary files found.");
    } else {
        for e in &filtered {
            println!("{}  ({}s old)", e.path.display(), e.age_secs);
        }
    }
    Ok(())
}

/// Clean tidev temporary files older than a given age.
pub fn tmp_clean(min_age_minutes: u64, dry_run: bool) -> Result<()> {
    let max_age = std::time::Duration::from_secs(min_age_minutes * 60);
    let removed = tidev_utils::tmp::clean_temp_files(max_age, dry_run)?;
    if removed.is_empty() {
        println!("No temporary files to clean.");
    } else {
        for e in &removed {
            if dry_run {
                println!("Would remove: {}", e.path.display());
            } else {
                println!("Removed: {}", e.path.display());
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// tidev session
// ---------------------------------------------------------------------------

/// List sessions without starting the TUI.
pub fn session_list(
    query: Option<String>,
    limit: u64,
    offset: u64,
    format: SessionOutputFormat,
) -> Result<()> {
    let limit = i64::try_from(limit).context("session list limit is too large")?;
    let offset = i64::try_from(offset).context("session list offset is too large")?;
    let (_paths, store) = open_store()?;

    let sessions = if let Some(query) = query {
        let search_limit = limit
            .checked_add(offset)
            .context("session list range is too large")?;
        store
            .search_sessions(&query, search_limit)?
            .into_iter()
            .skip(offset as usize)
            .collect()
    } else {
        store.list_sessions_unfiltered(limit, offset)?
    };

    match format {
        SessionOutputFormat::Text => print_session_list(&sessions),
        SessionOutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&sessions)?);
        }
    }
    Ok(())
}

/// Show a complete session or one message without starting the TUI.
pub fn session_show(
    session_id: &str,
    message_id: Option<&str>,
    format: SessionOutputFormat,
) -> Result<()> {
    let session_id = parse_session_id(session_id)?;
    let message_id = message_id
        .map(parse_message_id)
        .transpose()
        .context("invalid message UUID")?;
    let (_paths, store) = open_store()?;
    let mut inspection = store
        .load_session_inspection(session_id)?
        .with_context(|| format!("session not found: {session_id}"))?;

    if let Some(message_id) = message_id {
        inspection
            .messages
            .retain(|message| message.message.id == message_id);
        if inspection.messages.is_empty() {
            anyhow::bail!("message {message_id} not found in session {session_id}");
        }
    }

    match format {
        SessionOutputFormat::Text => print_session_inspection_text(&inspection)?,
        SessionOutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
    }
    Ok(())
}

fn parse_session_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid session UUID: {value}"))
}

fn parse_message_id(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).with_context(|| format!("invalid message UUID: {value}"))
}

fn print_session_list(sessions: &[tidev_storage::SessionRecord]) {
    if sessions.is_empty() {
        println!("No sessions found.");
        return;
    }

    println!("ID  UPDATED  STATUS  PARENT  TITLE");
    for session in sessions {
        let parent = session
            .parent_session_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "-".to_string());
        let title = session
            .title
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t");
        println!(
            "{}  {}  {}  {}  {}",
            session.session_id,
            session.updated_at.to_rfc3339(),
            session.status,
            parent,
            title
        );
    }
}

fn print_session_inspection_text(inspection: &SessionInspection) -> Result<()> {
    let session = &inspection.session;
    println!("Session: {}", session.session_id);
    println!("Title: {}", session.title);
    println!("Status: {}", session.status);
    println!("Workspace: {}", session.workspace_root);
    println!(
        "Provider: {} ({})",
        session.provider_id, session.provider_display_name
    );
    println!(
        "Model: {} ({})",
        session.model_id, session.model_display_name
    );
    println!("Created: {}", session.created_at.to_rfc3339());
    println!("Updated: {}", session.updated_at.to_rfc3339());
    if let Some(parent_session_id) = session.parent_session_id {
        println!("Parent session: {parent_session_id}");
    }
    if !session.system_prompt.is_empty() {
        println!("\nSystem prompt:\n{}", session.system_prompt);
    }
    if let Some(summary) = &session.context_summary {
        println!("\nContext summary:\n{summary}");
        println!("Context retained from: {}", session.context_retained_from);
    }
    println!("\nMessages: {}", inspection.messages.len());

    for message in &inspection.messages {
        print_message_text(message)?;
    }
    Ok(())
}

fn print_message_text(message: &StoredMessageView) -> Result<()> {
    let protocol = &message.message;
    println!(
        "\n--- Message {}: {} ---",
        message.sequence,
        protocol.role.label()
    );
    println!("ID: {}", protocol.id);
    println!("Created: {}", protocol.created_at.to_rfc3339());
    if let Some(completed_at) = protocol.completed_at {
        println!("Completed: {}", completed_at.to_rfc3339());
    }
    println!("Streaming: {}", protocol.streaming);
    println!("\nContent:\n{}", protocol.content);

    if !protocol.reasoning.is_empty() {
        println!("\nReasoning:\n{}", protocol.reasoning);
    }
    if !protocol.tool_calls.is_empty() {
        println!(
            "\nTool calls:\n{}",
            serde_json::to_string_pretty(&protocol.tool_calls)?
        );
    }
    if !protocol.attachments.is_empty() {
        println!(
            "\nAttachments:\n{}",
            serde_json::to_string_pretty(&protocol.attachments)?
        );
    }
    if protocol.tool_call_id.is_some() || protocol.tool_name.is_some() {
        println!(
            "\nTool result metadata:\n{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "tool_call_id": protocol.tool_call_id,
                "tool_name": protocol.tool_name,
            }))?
        );
    }
    if protocol.input_tokens.is_some()
        || protocol.output_tokens.is_some()
        || protocol.total_tokens.is_some()
        || protocol.cache_read_tokens.is_some()
        || protocol.cache_write_tokens.is_some()
        || protocol.model_id.is_some()
        || protocol.tokens_per_second.is_some()
        || protocol.thinking_level.is_some()
    {
        println!(
            "\nUsage and model metadata:\n{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "input_tokens": protocol.input_tokens,
                "output_tokens": protocol.output_tokens,
                "total_tokens": protocol.total_tokens,
                "cache_read_tokens": protocol.cache_read_tokens,
                "cache_write_tokens": protocol.cache_write_tokens,
                "model_id": protocol.model_id,
                "tokens_per_second": protocol.tokens_per_second,
                "thinking_level": protocol.thinking_level,
            }))?
        );
    }
    if protocol.reasoning_started_at.is_some() || protocol.reasoning_completed_at.is_some() {
        println!(
            "\nReasoning timing:\n{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "reasoning_started_at": protocol.reasoning_started_at,
                "reasoning_completed_at": protocol.reasoning_completed_at,
            }))?
        );
    }

    println!(
        "\nProtocol metadata:\n{}",
        serde_json::to_string_pretty(&protocol.metadata)?
    );
    println!(
        "\nApplication data:\n{}",
        serde_json::to_string_pretty(&message.app_data)?
    );
    if let Some(tool_output) = &message.tool_output {
        println!(
            "\nRetained full tool output ({}):\n{}",
            tool_output.tool_name, tool_output.output
        );
        println!("Tool output created: {}", tool_output.created_at);
    }
    Ok(())
}

/// Delete sessions older than the specified number of days.
pub fn session_prune(older_than_days: u64) -> Result<()> {
    let (_paths, store) = open_store()?;
    let duration = Duration::days(older_than_days as i64);
    let deleted = store.delete_sessions_older_than(duration)?;
    if deleted.is_empty() {
        println!("No sessions older than {older_than_days} days found.");
    } else {
        println!(
            "Deleted {} session(s) older than {older_than_days} days.",
            deleted.len()
        );
    }
    Ok(())
}
