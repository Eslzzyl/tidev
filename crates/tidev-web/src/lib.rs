pub mod assets;
pub mod auth;
pub mod error;
pub mod event_bus;
pub mod routes;
pub mod server;
pub mod state;
pub mod terminal;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::Mutex;

use tidev_engine::{
    agent::runtime::AgentRuntime,
    config::{AppConfig, AuthStore, ConfigPaths},
    mcp::McpManager,
    tooling::{FileReadTracker, ToolRegistry},
};
use tidev_llm::LlmClient;
use tidev_storage::database::Database;

use self::{
    event_bus::EventBus,
    routes::static_file::{StaticConfig, StaticMode},
    server::{ServerConfig, start_server},
    state::AppState,
};

/// Find the git worktree root by looking for a .git directory.
fn find_git_worktree(start: &std::path::Path) -> Option<PathBuf> {
    for ancestor in start.ancestors() {
        if ancestor.join(".git").is_dir() {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

/// Web subcommand options
#[derive(Debug, Clone, Default)]
pub struct WebOptions {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Use filesystem instead of default dev mode (for development)
    pub dev_fs: bool,
    /// Workspace root path (current directory if not specified)
    pub workspace_root: Option<std::path::PathBuf>,
}

/// Run the web server
pub async fn run(options: WebOptions) -> anyhow::Result<()> {
    // Load configuration first (needed for logging setup)
    let paths = ConfigPaths::discover()?;
    let mut config = AppConfig::load_or_create(&paths)?;

    // Determine workspace root (use current dir or provided path)
    let workspace_root = options
        .workspace_root
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let workspace_root = workspace_root.canonicalize().unwrap_or(workspace_root);

    // Load project-local config overlay (`.tidev/config.toml`)
    let project_config_path = workspace_root.join(".tidev/config.toml");
    if project_config_path.exists() {
        let project_toml = std::fs::read_to_string(&project_config_path)
            .with_context(|| format!("failed to read {}", project_config_path.display()))?;
        let keys = tidev_engine::config::top_level_toml_keys(&project_toml);
        let project_config: AppConfig = toml::from_str(&project_toml)
            .with_context(|| format!("failed to parse {}", project_config_path.display()))?;
        config.merge_overlay(project_config, &keys);
    }

    // Initialize shell detection (Windows: auto-detect bash, Unix: sh).
    tidev_engine::shell::init(config.shell.windows_shell.clone(), Some(&paths));

    // Initialize logging (console enabled for web mode)
    let mut logging_config = config.logging.clone();
    logging_config.console = true;
    std::fs::create_dir_all(&paths.data_dir)?;
    tidev_engine::logging::init(&paths.data_dir, logging_config).ok();
    log::info!("Starting TiDev web server...");

    // Open database (use same path as TUI mode via ConfigPaths)
    let db = Database::open(&paths.database_file)?;
    let store = db.create_session_store()?;

    // Create LLM client
    let llm_client = LlmClient::new(
        config.logging.save_request_body,
        config.logging.max_request_files,
    )?;

    // Create event bus
    let event_bus = EventBus::new(1024);

    log::info!("Workspace root: {}", workspace_root.display());

    // Load auth store
    let auth = AuthStore::load_or_create(&paths)?;
    log::info!("Auth store loaded");

    // Create shared agent runtime (ToolRegistry, MemoryStore, etc.)
    let memory_store = Arc::new(tidev_engine::memory::MemoryStore::open(
        &paths.database_file,
    )?);
    // Configure memory store with LLM
    if let Ok(default_model) = config.resolve_active_model(&auth) {
        memory_store.set_models(llm_client.clone(), default_model, None);
    }
    tidev_engine::memory::start_background_tasks(
        memory_store.clone(),
        &tokio::runtime::Handle::current(),
        &workspace_root.to_string_lossy(),
        &config.memory,
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

    // Resolve default model to set on tools
    if let Ok(default_model) = config.resolve_active_model(&auth) {
        tools.set_active_model(default_model);
    }
    // Web mode has full access — same as direct TUI usage
    tools.set_sandbox_policy(Some(tidev_engine::sandbox::SandboxPolicy::DangerFullAccess));

    let agent = AgentRuntime {
        workspace_root: workspace_root.clone(),
        config_dir: paths.config_dir.clone(),
        config_paths: paths.clone(),
        config: config.clone(),
        auth: auth.clone(),
        store: Arc::new(Mutex::new(store.clone())),
        llm_client: llm_client.clone(),
        tools,
        instructions: config.instructions.clone(),
        instruction_content_cache: std::collections::HashMap::new(),
        queued_messages: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
        auto_approve_permissions: false,
        hooks: tidev_engine::hooks::HookEngine::new(config.hooks.clone(), workspace_root.clone()),
    };

    log::info!("Agent runtime created");
    // Create app state
    let database_path = paths.database_file.clone();
    let state = AppState::new(
        store,
        database_path,
        event_bus,
        llm_client,
        config,
        auth,
        workspace_root,
        &paths,
        agent,
    )?;

    // Determine static file serving mode
    let static_mode = StaticMode::detect();

    let static_config = StaticConfig {
        mode: static_mode,
        use_fs: options.dev_fs,
        ..Default::default()
    };

    // Server configuration
    let server_config = ServerConfig {
        host: options.host.unwrap_or_else(|| "127.0.0.1".to_string()),
        port: options.port.unwrap_or(26502),
        static_config,
    };

    // Log the mode
    if cfg!(debug_assertions) {
        if options.dev_fs {
            log::info!("Frontend mode: DevFs (serving from web/dist)");
        } else {
            log::info!("Frontend mode: Dev (showing development page)");
            log::info!("Tip: Run `cd web && pnpm dev` and visit http://localhost:5173 for HMR");
            log::info!("     Or use --dev-fs to serve from web/dist");
        }
    } else {
        log::info!("Frontend mode: Embedded (serving from binary)");
    }

    // Start server
    start_server(state.clone(), server_config).await?;

    // Graceful shutdown complete; clean up terminal sessions.
    log::info!("Web server stopped, cleaning up terminal sessions...");
    state.terminal_manager.shutdown().await;

    Ok(())
}
