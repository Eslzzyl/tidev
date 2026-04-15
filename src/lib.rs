pub mod app;
pub mod commands;
pub mod config;
pub mod context;
pub mod input;
pub mod instructions;
pub mod llm;
pub mod logging;
pub mod markdown_render;
pub mod mcp;
pub mod prompts;
pub mod provider_setup;
pub mod session;
pub mod skills;
pub mod snapshot;
pub mod storage;
pub mod theme;
pub mod tooling;
pub mod webtools;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
