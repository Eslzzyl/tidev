pub mod app;
pub mod commands;
pub mod config;
pub mod context;
pub mod input;
pub mod llm;
pub mod markdown;
pub mod markdown_render;
pub mod markdown_stream;
pub mod prompts;
pub mod provider_setup;
pub mod render;
pub mod session;
pub mod storage;
pub mod theme;
pub mod tooling;
pub mod wrapping;

pub fn run() -> anyhow::Result<()> {
    app::run()
}
