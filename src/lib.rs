pub mod app;
pub mod config;
pub mod context;
pub mod instructions;
pub mod llm;
pub mod logging;
pub mod markdown_render;
pub mod mcp;
pub mod prompts;
pub mod provider_setup;
pub mod session;
pub mod snapshot;
pub mod storage;
pub mod theme;
pub mod tooling;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
