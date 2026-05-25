mod channel;
mod channel_core;
mod commands;
pub mod discord;
pub mod lark;
pub mod model_selection;
mod orchestrator;
mod qq;
mod shared;
pub mod shell;
pub mod telegram;

pub use channel_core::{ChannelCore, MessageSender};

use std::env;
use std::path::Path;
use std::sync::Arc;
use tokio::runtime::Runtime;

use anyhow::{Context, Result, bail};

use tidev_engine::{
    config::{AppConfig, AuthStore, ConfigPaths},
    mcp::McpManager,
    tooling::{FileReadTracker, ToolRegistry},
};
use tidev_llm::LlmClient;
use tidev_storage::database::Database;

use orchestrator::ChannelOrchestrator;
use shared::compose_instruction_prompt;

/// Per-channel resources that need to be created independently
/// for each channel (each channel gets its own store, tools, etc.).
struct ChannelResources {
    store: tidev_storage::SessionStore,
    llm: LlmClient,
    tools: ToolRegistry,
}

impl ChannelResources {
    fn new(
        db: &Database,
        config: &AppConfig,
        default_model: &tidev_engine::config::ActiveModel,
        workspace_root: &Path,
        paths: &ConfigPaths,
        auth: &AuthStore,
    ) -> Result<Self> {
        let store = db.create_session_store()?;
        let memory_store = Arc::new(tidev_engine::memory::MemoryStore::open(db.path())?);
        let llm = LlmClient::new(
            config.logging.save_request_body,
            config.logging.max_request_files,
        )?;
        memory_store.set_models(llm.clone(), default_model.clone(), None);
        tidev_engine::memory::start_background_tasks(
            memory_store.clone(),
            &tokio::runtime::Handle::current(),
            &workspace_root.to_string_lossy(),
            &config.memory,
        );
        let mcp = McpManager::new(workspace_root.to_path_buf(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        let worktree = find_git_worktree(workspace_root);
        let mut tools = ToolRegistry::new(
            workspace_root.to_path_buf(),
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
        tools.set_sandbox_policy(Some(tidev_engine::sandbox::SandboxPolicy::default()));
        Ok(Self { store, llm, tools })
    }
}

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
    log::info!("Gateway starting, config loaded");

    let mut logging_config = config.logging.clone();
    logging_config.console = true;
    tidev_engine::logging::init(&paths.data_dir, logging_config).ok();
    log::info!("Logging initialized (console: true)");

    let auth = AuthStore::load_or_create(&paths)?;
    log::info!("Auth store loaded");

    let default_model = config.resolve_active_model_for_gateway(&auth)?;
    let instruction_prompt = compose_instruction_prompt(&workspace_root, &paths, &config);
    let db = Database::open(paths.default_database_path())?;

    // ── Task scheduler setup ──────────────────────────────────────────────
    let (delivery_bus, _cron_rx) = if config.gateway.scheduler.enabled {
        let (bus, rx) = tidev_scheduler::delivery::DeliveryBus::new(256);

        let cron_store = tidev_scheduler::store::CronStore::new(
            db.write_conn(),
            db.path(),
            config.gateway.scheduler.max_tasks,
            config.gateway.scheduler.max_run_history,
        )?;

        let scheduler_config = tidev_scheduler::scheduler::SchedulerConfig {
            poll_secs: config.gateway.scheduler.poll_secs,
            max_tasks: config.gateway.scheduler.max_tasks,
            max_concurrent: config.gateway.scheduler.max_concurrent,
            max_run_history: config.gateway.scheduler.max_run_history,
        };

        let scheduler = tidev_scheduler::scheduler::Scheduler::new(
            cron_store,
            scheduler_config,
            Some(bus.clone()),
            None, // AgentRuntime will be wired up later
            default_model.clone(),
            workspace_root.clone(),
        );

        tokio::task::spawn_local(async move {
            if let Err(e) = scheduler.run().await {
                log::error!("Task scheduler failed: {e}");
            }
        });

        log::info!("Task scheduler enabled");
        (Some(bus), Some(rx))
    } else {
        (None, None)
    };

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

        log::info!(
            "Telegram channel enabled, allowlist: {} entries",
            allowlist.len()
        );

        let res =
            ChannelResources::new(&db, &config, &default_model, &workspace_root, &paths, &auth)?;

        let channel = telegram::TelegramChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            res.store,
            res.llm,
            res.tools,
            instruction_prompt.clone(),
            allowlist,
            config.gateway.telegram.poll_timeout_secs.max(1),
            bot_token,
            &paths,
            delivery_bus.as_ref().map(|b| b.subscribe()),
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

        log::info!("QQ channel enabled, allowlist: {} entries", allowlist.len());

        let res =
            ChannelResources::new(&db, &config, &default_model, &workspace_root, &paths, &auth)?;

        let channel = qq::QQChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            res.store,
            res.llm,
            res.tools,
            instruction_prompt.clone(),
            allowlist,
            app_id,
            app_secret,
            config.gateway.qq.sandbox,
            &paths,
            delivery_bus.as_ref().map(|b| b.subscribe()),
        );

        orchestrator.add(Box::new(channel));
    }

    if config.gateway.discord.enabled {
        let allowlist = config
            .gateway
            .discord
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        if allowlist.is_empty() {
            bail!("gateway.discord.allowlist is empty; configure at least one Discord user ID");
        }

        let bot_token = auth
            .discord_bot_token()
            .context("missing Discord bot token in auth.json for channel 'discord'")?
            .to_string();

        log::info!(
            "Discord channel enabled, allowlist: {} entries",
            allowlist.len()
        );

        let res =
            ChannelResources::new(&db, &config, &default_model, &workspace_root, &paths, &auth)?;

        let channel = discord::DiscordChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            res.store,
            res.llm,
            res.tools,
            instruction_prompt.clone(),
            allowlist,
            bot_token,
            config.gateway.discord.guild_ids.clone(),
            config.gateway.discord.channel_ids.clone(),
            config.gateway.discord.mention_only,
            &paths,
            delivery_bus.as_ref().map(|b| b.subscribe()),
        );

        orchestrator.add(Box::new(channel));
    }

    if config.gateway.lark.enabled {
        let allowlist = config
            .gateway
            .lark
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        if allowlist.is_empty() {
            bail!("gateway.lark.allowlist is empty; configure at least one Lark user open_id");
        }

        let app_id = auth
            .lark_app_id()
            .context("missing Lark AppID in auth.json for channel 'lark'")?
            .to_string();
        let app_secret = auth
            .lark_app_secret()
            .context("missing Lark AppSecret in auth.json for channel 'lark'")?
            .to_string();

        log::info!(
            "Lark channel enabled, allowlist: {} entries",
            allowlist.len()
        );

        let res =
            ChannelResources::new(&db, &config, &default_model, &workspace_root, &paths, &auth)?;

        let channel = lark::LarkChannel::new(
            workspace_root.clone(),
            config.clone(),
            auth.clone(),
            res.store,
            res.llm,
            res.tools,
            instruction_prompt.clone(),
            allowlist,
            app_id,
            app_secret,
            config.gateway.lark.mention_only,
            config.gateway.lark.use_feishu,
            &paths,
            delivery_bus.as_ref().map(|b| b.subscribe()),
        );

        orchestrator.add(Box::new(channel));
    }

    if orchestrator.is_empty() {
        bail!(
            "No gateway enabled; set gateway.telegram.enabled, gateway.qq.enabled, gateway.discord.enabled, or gateway.lark.enabled to true in config.toml"
        );
    }

    log::info!(
        "Gateway ready, starting {} channel(s): {}",
        orchestrator.channel_names().len(),
        orchestrator.channel_names().join(", ")
    );

    orchestrator.run().await
}
