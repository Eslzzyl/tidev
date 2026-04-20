mod qq;
mod qq_client;
mod shared;
mod telegram;

use std::env;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::time::Duration;

use anyhow::{Context, Result, bail};

use crate::{
    config::{AppConfig, AuthStore, ConfigPaths},
    llm::LlmClient,
    mcp::McpManager,
    storage::SessionStore,
    tooling::{FileReadTracker, ToolRegistry},
};

use shared::{compose_instruction_prompt, compose_system_prompt};

pub fn run() -> Result<()> {
    let runtime = Runtime::new().context("failed to create runtime")?;
    let local = tokio::task::LocalSet::new();
    local.block_on(&runtime, run_async())
}

async fn run_async() -> Result<()> {
    let workspace_root = env::current_dir().context("failed to determine workspace root")?;
    let paths = ConfigPaths::discover()?;
    let config = AppConfig::load_or_create(&paths)?;
    crate::log_info!("Gateway starting, config loaded");

    let mut logging_config = config.logging.clone();
    logging_config.console = true;
    crate::logging::init(&paths.data_dir, logging_config);
    crate::log_info!("Logging initialized (console: true)");

    let auth = AuthStore::load_or_create(&paths)?;
    crate::log_info!("Auth store loaded");

    if !config.gateway.telegram.enabled && !config.gateway.qq.enabled {
        bail!(
            "No gateway enabled; set either gateway.telegram.enabled or gateway.qq.enabled to true in config.toml"
        );
    }

    if config.gateway.telegram.enabled {
        crate::log_info!("Telegram gateway enabled");

        let allowlist = config
            .gateway
            .telegram
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        crate::log_info!("Telegram allowlist loaded, {} entries", allowlist.len());

        if allowlist.is_empty() {
            bail!(
                "gateway.telegram.allowlist is empty; configure at least one Telegram user/chat id"
            );
        }

        let bot_token = auth
            .telegram_bot_token()
            .context("missing Telegram bot token in auth.json for channel 'telegram'")?
            .to_string();
        crate::log_info!("Telegram Bot token loaded");

        let default_model = config.resolve_active_model_for_gateway(&auth)?;
        let instruction_prompt = compose_instruction_prompt(&workspace_root, &paths, &config);
        let llm = LlmClient::new()?;
        let store = SessionStore::open(paths.default_database_path())?;
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        let mut tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
            file_read_tracker,
        );
        tools.set_active_model(default_model.clone());

        let poll_timeout_secs = config.gateway.telegram.poll_timeout_secs.max(1);

        let mut runner = telegram::TelegramGatewayRunner {
            workspace_root: workspace_root.clone(),
            config: config.clone(),
            auth: auth.clone(),
            store,
            llm,
            tools,
            instruction_prompt,
            allowlist,
            poll_timeout_secs,
            bot: telegram::TelegramBot::new(bot_token),
            offset: 0,
            request_seq: 0,
        };

        runner.bootstrap_offset().await?;
        crate::log_info!("Telegram Bootstrap offset initialized: {}", runner.offset);

        crate::log_info!("Telegram Gateway ready, entering main loop");
        let _runner_handle = tokio::task::spawn_local(async move {
            if let Err(e) = runner.run_loop().await {
                crate::log_error!("Telegram Gateway loop failed: {e}");
            }
        });
    }

    if config.gateway.qq.enabled {
        crate::log_info!("QQ gateway enabled");

        let allowlist = config
            .gateway
            .qq
            .allowlist
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect::<std::collections::HashSet<_>>();

        crate::log_info!("QQ allowlist loaded, {} entries", allowlist.len());

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

        let default_model = config.resolve_active_model_for_gateway(&auth)?;
        let instruction_prompt = compose_instruction_prompt(&workspace_root, &paths, &config);
        let llm = LlmClient::new()?;
        let store = SessionStore::open(paths.default_database_path())?;
        let mcp = McpManager::new(workspace_root.clone(), config.mcp.servers.clone());
        let file_read_tracker = Arc::new(FileReadTracker::new());
        let mut tools = ToolRegistry::new(
            workspace_root.clone(),
            paths.config_dir.clone(),
            config.skills.clone(),
            mcp,
            config.permissions.clone(),
            file_read_tracker,
        );
        tools.set_active_model(default_model.clone());

        let mut runner = qq::QQGatewayRunner {
            workspace_root: workspace_root.clone(),
            config: config.clone(),
            auth: auth.clone(),
            store,
            llm,
            tools,
            instruction_prompt,
            allowlist,
            client: qq_client::QQClient::new(app_id, app_secret, config.gateway.qq.sandbox),
            session_id: None,
            last_seq: None,
        };

        crate::log_info!("QQ Gateway ready, entering main loop");
        runner.run_loop().await?;
    }

    // Keep the main thread alive if we spawned gateways
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}
