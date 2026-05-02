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
    server::{ServerConfig, start_server},
    state::AppState,
};

/// Web subcommand options
#[derive(Debug, Clone)]
pub struct WebOptions {
    pub host: Option<String>,
    pub port: Option<u16>,
}

/// Run the web server
pub async fn run(options: WebOptions) -> anyhow::Result<()> {
    eprintln!("Starting TiDev web server...");

    // Load configuration
    let paths = ConfigPaths::discover()?;
    let config = AppConfig::load_or_create(&paths)?;

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

    // Server configuration
    let server_config = ServerConfig {
        host: options.host.unwrap_or_else(|| "127.0.0.1".to_string()),
        port: options.port.unwrap_or(26502),
    };

    // Start server
    start_server(state, server_config).await?;

    Ok(())
}
