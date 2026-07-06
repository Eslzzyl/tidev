//! Tui — terminal layer.
//!
//! Owns the `Terminal`, handles setup/teardown and event polling.

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
    EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event};
use crossterm::terminal::{DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::execute;
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
        let tick_rate = Duration::from_millis(50);

        // Initial render
        self.terminal.draw(|frame| app.draw(frame))?;

        while !app.should_quit() {
            if event::poll(tick_rate)? {
                let event = event::read()?;
                match event {
                    Event::Key(key) => {
                        app.handle_key_event(key);
                    }
                    Event::Mouse(mouse) => {
                        app.handle_mouse_event(mouse);
                    }
                    Event::Resize(w, h) => {
                        app.handle_resize(w, h);
                    }
                    _ => {}
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
