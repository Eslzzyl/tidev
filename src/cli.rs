//! CLI command handlers for the tidev binary.
//!
//! Each public function corresponds to a subcommand in [`crate::Command`].

use anyhow::{Context, Result};
use chrono::Duration;
use std::path::PathBuf;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open the database and return its store together with the resolved paths.
fn open_store() -> Result<(tidev_config::paths::ConfigPaths, tidev_storage::SessionStore)> {
    let paths = tidev_config::paths::ConfigPaths::discover()?;
    let database =
        tidev_storage::database::Database::open(&paths.database_file)
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
                .map(|s| {
                    Uuid::parse_str(s).with_context(|| format!("invalid session UUID: {s}"))
                })
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
    let database =
        tidev_storage::database::Database::open(&paths.database_file)
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

/// Delete sessions older than the specified number of days.
pub fn session_prune(older_than_days: u64) -> Result<()> {
    let (_paths, store) = open_store()?;
    let duration = Duration::days(older_than_days as i64);
    let deleted = store.delete_sessions_older_than(duration)?;
    if deleted.is_empty() {
        println!("No sessions older than {older_than_days} days found.");
    } else {
        println!("Deleted {} session(s) older than {older_than_days} days.", deleted.len());
    }
    Ok(())
}
