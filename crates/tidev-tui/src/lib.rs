//! tidev-tui — New-architecture TUI frontend for tidev.
//!
//! Built from scratch alongside tidev-tui. Will eventually replace it.

pub(crate) mod action;
pub(crate) mod ansi;
pub mod app;
pub(crate) mod component;
pub(crate) mod components;
pub(crate) mod context;
pub(crate) mod utils;

mod markdown;
pub mod tui;

/// Run the TUI application (blocking). Called from the binary entry point.
pub fn run() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let runtime = tidev_core::Runtime::builder()
            .workspace_root(std::env::current_dir()?)
            .build()
            .await?;

        let request_rx = runtime.request_rx().await;
        let event_rx = runtime.event_rx().await;

        let mut app = app::App::new(runtime, request_rx, event_rx);
        let mut tui = tui::Tui::new()?;
        tui.run(&mut app).await
    })
}
