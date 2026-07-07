//! Tui — terminal layer.
//!
//! Owns the `Terminal`, handles setup/teardown and event polling.
//! Uses a dual-channel event loop multiplexing crossterm input, backend events,
//! and tool permission requests via `tokio::select!`.

use std::io;

use anyhow::Result;
use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
    EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event, EventStream};
use crossterm::terminal::{DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::execute;
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::app::App;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableLineWrap,
            EnableBracketedPaste,
            EnableFocusChange,
            EnableMouseCapture,
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub async fn run(&mut self, app: &mut App) -> Result<()> {
        let mut reader = EventStream::new();

        // Take ownership of receivers so select! branches don't conflict with app.
        let mut perm_rx = app.perm_rx.take();
        let mut event_rx = app.event_rx.take();

        // Initial render
        self.terminal.draw(|frame| app.draw(frame))?;

        while !app.should_quit() {
            tokio::select! {
                // ── Crossterm input events ──────────────────────────────
                Some(Ok(event)) = reader.next() => {
                    match event {
                        Event::Key(key) => app.handle_key_event(key),
                        Event::Mouse(mouse) => app.handle_mouse_event(mouse),
                        Event::Resize(w, h) => app.handle_resize(w, h),
                        _ => {}
                    }
                }

                // ── Backend events (streaming, tool results, etc.) ─────
                result = async {
                    match event_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => futures::future::pending().await,
                    }
                } => {
                    if let Some(event) = result {
                        app.handle_backend_event(event);
                    }
                }

                // ── Tool permission requests ────────────────────────────
                result = async {
                    match perm_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => futures::future::pending().await,
                    }
                } => {
                    if let Some(approval) = result {
                        app.handle_pending_approval(approval);
                    }
                }
            }

            self.terminal.draw(|frame| app.draw(frame))?;
        }

        // Cleanup
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableLineWrap,
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
        );

        Ok(())
    }
}
