mod channel;
mod commands;
mod orchestrator;
mod qq;
mod qq_client;
mod shared;
pub mod telegram;

use std::env;
use std::path::Path;
use std::sync::Arc;
use tokio::runtime::Runtime;

use anyhow::{Context, Result, bail};

use crate::{
    config::{AppConfig, AuthStore, ConfigPaths},
    llm::LlmClient,
    mcp::McpManager,
    storage::database::Database,
    tooling::{FileReadTracker, ToolRegistry},
};

use orchestrator::ChannelOrchestrator;
use shared::compose_instruction_prompt;

/// Find the git worktree root by looking for a .git directory,
/// starting from the given path and walking up to the ancestors.
fn find_git_worktree(start: &Path) -> Option<std::path::PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, run_async())
}

async fn run_async() -> Result<()> {
    let workspace_root = env::current_dir().context("failed to determine workspace root")?;
    let paths = ConfigPaths::discover()?;
    let config = AppConfig::load_with_project_overlay(&paths, &workspace_root)?;
    crate::log_info!("Gateway starting, config loaded");

    let mut logging_config = config.logging.clone();
    logging_config.console = true;
    crate::logging::init(&paths.data_dir, logging_config);
    crate::log_info!("Logging initialized (console: true)");

    let auth = AuthStore::load_or_create(&paths)?;
    crate::log_info!("Auth store loaded");

    let default_model = config.resolve_active_model_for_gateway(&auth)?;
    let instruction_prompt = compose_instruction_prompt(&workspace_root, &paths, &config);
    let db = Database::open(paths.default_database_path())?;

    // Build orchestrator with enabled channels
    let mut orchestrator = ChannelOrchestrator::new();

    if config.gateway.telegram.enabled {
        let allowlist = config
            .gateway
            .telegram
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        if allowlist.is_empty() {
            bail!(
                "gateway.telegram.allowlist is empty; configure at least one Telegram user/chat id"
            );
        }

        let bot_token = auth
            .telegram_bot_token()
            .context("missing Telegram bot token in auth.json for channel 'telegram'")?
            .to_string();

        crate::log_info!(
            "Telegram channel enabled, allowlist: {} entries",
            allowlist.len()
        );

        // Each channel gets its own resources
        let store = db.create_session_store()?;
        let memory_store = Arc::new(db.create_memory_store()?);
        let llm = LlmClient::new(&config.logging)?;
        memory_store.set_models(llm.clone(), default_model.clone(), None);
        crate::memory::start_background_tasks(
            memory_store.clone(),
            &tokio::runtime::Handle::current(),
            &workspace_root.to_string_lossy(),
        );
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        let worktree = find_git_worktree(&workspace_root);
        let mut tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
            file_read_tracker,
            memory_store,
            config.rtk.enabled,
            worktree,
            config.websearch.clone(),
            Arc::new(auth.clone()),
        );
        tools.set_active_model(default_model.clone());
        // Use default sandbox policy for gateway mode
        tools.set_sandbox_policy(Some(crate::sandbox::SandboxPolicy::default()));

        let channel = telegram::TelegramChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            store,
            llm,
            tools,
            instruction_prompt.clone(),
            allowlist,
            config.gateway.telegram.poll_timeout_secs.max(1),
            bot_token,
            &paths,
        );

        orchestrator.add(Box::new(channel));
    }

    if config.gateway.qq.enabled {
        let allowlist = config
            .gateway
            .qq
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        if allowlist.is_empty() {
            bail!("gateway.qq.allowlist is empty; configure at least one QQ user/channel id");
        }

        let app_id = auth
            .qq_app_id()
            .context("missing QQ AppID in auth.json")?
            .to_string();
        let app_secret = auth
            .qq_app_secret()
            .context("missing QQ AppSecret in auth.json")?
            .to_string();

        crate::log_info!("QQ channel enabled, allowlist: {} entries", allowlist.len());

        // Each channel gets its own resources
        let store = db.create_session_store()?;
        let memory_store2 = Arc::new(db.create_memory_store()?);
        let llm = LlmClient::new(&config.logging)?;
        memory_store2.set_models(llm.clone(), default_model.clone(), None);
        crate::memory::start_background_tasks(
            memory_store2.clone(),
            &tokio::runtime::Handle::current(),
            &workspace_root.to_string_lossy(),
        );
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        let worktree2 = find_git_worktree(&workspace_root);
        let mut tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
            file_read_tracker,
            memory_store2,
            config.rtk.enabled,
            worktree2,
            config.websearch.clone(),
            Arc::new(auth.clone()),
        );
        tools.set_active_model(default_model.clone());
        // Use default sandbox policy for gateway mode
        tools.set_sandbox_policy(Some(crate::sandbox::SandboxPolicy::default()));

        let channel = qq::QQChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            store,
            llm,
            tools,
            instruction_prompt.clone(),
            allowlist,
            app_id,
            app_secret,
            config.gateway.qq.sandbox,
            &paths,
        );

        orchestrator.add(Box::new(channel));
    }

    if orchestrator.is_empty() {
        bail!(
            "No gateway enabled; set either gateway.telegram.enabled or gateway.qq.enabled to true in config.toml"
        );
    }

    crate::log_info!(
        "Gateway ready, starting {} channel(s): {}",
        orchestrator.channel_names().len(),
        orchestrator.channel_names().join(", ")
    );

    orchestrator.run().await
}
