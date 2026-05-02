pub mod assets;
pub mod error;
pub mod event_bus;
pub mod routes;
pub mod server;
pub mod state;

use crate::{
    config::{AppConfig, ConfigPaths},
    llm::LlmClient,
    storage::SessionStore,
};

use self::{
    event_bus::EventBus,
    routes::static_file::{StaticConfig, StaticMode},
    server::{ServerConfig, start_server},
    state::AppState,
};

/// Web subcommand options
#[derive(Debug, Clone, Default)]
pub struct WebOptions {
    pub host: Option<String>,
    pub port: Option<u16>,
    /// Use filesystem instead of default dev mode (for development)
    pub dev_fs: bool,
}

/// Run the web server
pub async fn run(options: WebOptions) -> anyhow::Result<()> {
    // Load configuration first (needed for logging setup)
    let paths = ConfigPaths::discover()?;
    let config = AppConfig::load_or_create(&paths)?;

    // Initialize logging (console enabled for web mode)
    let mut logging_config = config.logging.clone();
    logging_config.console = true;
    crate::logging::init(&paths.data_dir, logging_config);
    crate::log_info!("Starting TiDev web server...");

    // Open database
    let data_dir = dirs::data_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine data directory"))?
        .join("tidev");
    std::fs::create_dir_all(&data_dir)?;
    let db_path = data_dir.join("sessions.sqlite3");
    let store = SessionStore::open(&db_path)?;

    // Create LLM client
    let llm_client = LlmClient::new()?;

    // Create event bus
    let event_bus = EventBus::new(1024);

    // Create app state
    let state = AppState::new(store, event_bus, llm_client, config)?;

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
            crate::log_info!("Frontend mode: DevFs (serving from web/dist)");
        } else {
            crate::log_info!("Frontend mode: Dev (showing development page)");
            crate::log_info!("Tip: Run `cd web && pnpm dev` and visit http://localhost:5173 for HMR");
            crate::log_info!("     Or use --dev-fs to serve from web/dist");
        }
    } else {
        crate::log_info!("Frontend mode: Embedded (serving from binary)");
    }

    // Start server
    start_server(state, server_config).await?;

    Ok(())
}
