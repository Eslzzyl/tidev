pub mod app;
pub mod commands;
pub mod config;
pub mod context;
pub mod input;
pub mod instructions;
pub mod llm;
pub mod markdown_render;
pub mod markdown_stream;
pub mod prompts;
pub mod provider_setup;
pub mod session;
pub mod skills;
pub mod storage;
pub mod theme;
pub mod tooling;
pub mod workspace_snapshot;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
