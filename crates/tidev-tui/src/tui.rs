//! Tui — terminal layer.
//!
//! Owns the `Terminal`, handles setup/teardown and event polling.
//! Uses a blocking reader thread for crossterm events drained via
//! `try_recv` batches, mirroring the synchronous polling of the old
//! TUI to avoid mixing sync and async event readers.

use std::io;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::{Show, self as cursor};
use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
    EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event};
use crossterm::terminal::{DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::execute;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;
use tidev_types::message::BackendEvent;
use uuid::Uuid;

use crate::app::App;

/// Maximum crossterm events to process per render frame (mirrors old TUI's 32).
const MAX_CROSSTERM_EVENTS_PER_BATCH: usize = 32;

/// Maximum backend events to drain per frame (mirrors old TUI's 200).
const MAX_BACKEND_EVENTS_PER_BATCH: usize = 200;

/// Target frame budget for streaming-only updates (≈60 fps).
const FRAME_BUDGET: Duration = Duration::from_millis(16);

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
            cursor::Hide,
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub async fn run(&mut self, app: &mut App) -> Result<()> {
        // ── Dedicated crossterm reader thread ──────────────────────────
        //
        // Spawn a blocking thread that reads events synchronously and
        // forwards them through a channel.  This avoids the race condition
        // that would arise from mixing the async EventStream (which uses an
        // internal blocking reader) with sync poll/read calls on the same
        // event source.
        let (crossterm_tx, mut crossterm_rx) = mpsc::unbounded_channel::<Event>();
        tokio::task::spawn_blocking(move || {
            loop {
                match crossterm::event::read() {
                    Ok(event) => {
                        if crossterm_tx.send(event).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::error!("crossterm read error: {e}");
                        break;
                    }
                }
            }
        });

        // Take ownership of receivers so the borrow checker is happy.
        let mut request_rx = app.request_rx.take();
        let mut event_rx = app.event_rx.take();

        // Initial render.
        self.terminal.draw(|frame| app.draw(frame))?;
        let mut last_render = Instant::now();

        loop {
            // ── Phase 1: Drain all event sources (non-blocking) ────────
            let mut had_input = false;

            // 1a. Crossterm events (batch up to 32).
            let mut cc_count = 0;
            while cc_count < MAX_CROSSTERM_EVENTS_PER_BATCH {
                match crossterm_rx.try_recv() {
                    Ok(event) => {
                        match event {
                            Event::Key(key) => app.handle_key_event(key),
                            Event::Mouse(mouse) => app.handle_mouse_event(mouse),
                            Event::Resize(w, h) => app.handle_resize(w, h),
                            _ => {}
                        }
                        cc_count += 1;
                        had_input = true;
                    }
                    Err(_) => break,
                }
            }

            // 1b. Backend events (batch up to 200, coalesce deltas).
            //
            // Coalesce consecutive Delta and ReasoningDelta events from the
            // same request to reduce per-frame cache invalidation overhead
            // during LLM streaming (mirrors old TUI behaviour).
            let mut be_count = 0;
            let mut cd_delta: Option<Coalesced> = None;
            let mut cd_reasoning: Option<Coalesced> = None;

            'be: while let Some(ref mut rx) = event_rx {
                let event = match rx.try_recv() {
                    Ok(e) => e,
                    Err(_) => break 'be,
                };
                be_count += 1;

                if be_count > MAX_BACKEND_EVENTS_PER_BATCH {
                    // Overflow: flush coalesced, process directly, then
                    // leave remaining events for the next iteration.
                    flush_delta(&mut cd_delta, &mut cd_reasoning, app);
                    app.handle_backend_event(event);
                    break 'be;
                }

                match event {
                    BackendEvent::Delta { session_id, request_id, content } => {
                        coalesce_or_flush(&mut cd_delta, session_id, request_id, content, false, app);
                    }
                    BackendEvent::ReasoningDelta { session_id, request_id, content } => {
                        coalesce_or_flush(&mut cd_reasoning, session_id, request_id, content, true, app);
                    }
                    _ => {
                        // Non-delta: flush before processing to preserve ordering.
                        flush_delta(&mut cd_delta, &mut cd_reasoning, app);
                        app.handle_backend_event(event);
                    }
                }
            }
            flush_delta(&mut cd_delta, &mut cd_reasoning, app);

            // 1c. Tool permission requests.
            if let Some(ref mut rx) = request_rx {
                while let Ok(request) = rx.try_recv() {
                    app.handle_tui_request(request);
                }
            }

            if app.should_quit() {
                break;
            }

            // ── Phase 2: Throttled render ─────────────────────────────
            //
            // Render if:
            //   - the UI is dirty, AND
            //   - either we just handled input (immediate response),
            //     or enough time has passed since the last render (FPS cap).
            let now = Instant::now();
            if app.is_dirty() && (now - last_render >= FRAME_BUDGET || had_input) {
                self.terminal
                    .draw(|frame| app.draw(frame))
                    .context("failed to render frame")?;
                app.mark_clean();
                last_render = now;
            }

            // ── Phase 3: Idle wait ────────────────────────────────────
            //
            // If nothing was processed this iteration, wait for the next
            // event (or until the frame budget expires).
            if !had_input {
                tokio::select! {
                    Some(event) = crossterm_rx.recv() => {
                        match event {
                            Event::Key(key) => app.handle_key_event(key),
                            Event::Mouse(mouse) => app.handle_mouse_event(mouse),
                            Event::Resize(w, h) => app.handle_resize(w, h),
                            _ => {}
                        }
                    }
                    result = async {
                        match event_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(event) = result {
                            app.handle_backend_event(event);
                        }
                    }
                    result = async {
                        match request_rx.as_mut() {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        if let Some(request) = result {
                            app.handle_tui_request(request);
                        }
                    }
                    _ = tokio::time::sleep(FRAME_BUDGET) => {}
                }
            }
        }

        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableLineWrap,
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
            Show,
        );
    }
}

// ── Backend delta coalescing ──────────────────────────────────────────

/// Accumulated delta content for a single request.
struct Coalesced {
    session_id: Uuid,
    request_id: u64,
    content: String,
}

/// Coalesce `content` into `slot`, or flush the previous delta first.
fn coalesce_or_flush(
    slot: &mut Option<Coalesced>,
    session_id: Uuid,
    request_id: u64,
    content: String,
    is_reasoning: bool,
    app: &mut App,
) {
    // Try to extend the existing coalesced slot.
    if let Some(cd) = slot {
        if cd.session_id == session_id && cd.request_id == request_id {
            cd.content.push_str(&content);
            return;
        }
    }

    // Different request or empty slot: flush old, start new.
    if let Some(cd) = slot.take() {
        emit_delta(cd, is_reasoning, app);
    }
    *slot = Some(Coalesced { session_id, request_id, content });
}

/// Flush both coalesced slots.
fn flush_delta(
    delta: &mut Option<Coalesced>,
    reasoning: &mut Option<Coalesced>,
    app: &mut App,
) {
    if let Some(cd) = delta.take() {
        emit_delta(cd, false, app);
    }
    if let Some(cd) = reasoning.take() {
        emit_delta(cd, true, app);
    }
}

/// Emit a single coalesced delta through `handle_backend_event`.
fn emit_delta(cd: Coalesced, is_reasoning: bool, app: &mut App) {
    let session_id = cd.session_id;
    let request_id = cd.request_id;
    let content = cd.content;
    if is_reasoning {
        app.handle_backend_event(BackendEvent::ReasoningDelta { session_id, request_id, content });
    } else {
        app.handle_backend_event(BackendEvent::Delta { session_id, request_id, content });
    }
}
