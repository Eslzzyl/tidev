pub mod chat_context;
pub mod commands;
pub mod panel_launcher;
pub mod theme;
pub mod utils;

mod ansi;
mod input;
mod markdown;
mod render;
mod ui;

/// The main TUI application.
///
/// Created by [`App::new`] with a pre-built [`tidev_core::Runtime`].
/// The caller is expected to call [`App::run`] to enter the event loop.
pub struct App {
    /// The shared tidev runtime (config, session manager, LLM, tools, etc.).
    pub runtime: tidev_core::Runtime,
}

impl App {
    /// Create a new TUI application from a pre-built Runtime.
    pub fn new(runtime: tidev_core::Runtime) -> Self {
        Self { runtime }
    }

    /// Run the TUI event loop (blocking).
    pub async fn run(&mut self) -> anyhow::Result<()> {
        // TODO: implement event loop
        Ok(())
    }
}
