//! Tui — terminal layer.
//!
//! Owns the `Terminal`, handles setup/teardown and event polling.
//! Uses a dedicated OS thread for `crossterm::event::poll` + `read`
//! that forwards events through a `tokio::sync::mpsc` channel.  This
//! mirrors the proven architecture of the v0.6.x TUI (synchronous
//! event reading in a single execution context) while keeping the
//! main event loop async for multiplexing with backend/request events.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::cursor::{Show, self as cursor};
use crossterm::event::{DisableBracketedPaste, DisableFocusChange, DisableMouseCapture,
    EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, Event, KeyEventKind};
use crossterm::terminal::{DisableLineWrap, EnableLineWrap, EnterAlternateScreen,
    LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use crossterm::execute;
use ratatui::backend::{Backend, ClearType, CrosstermBackend};
use ratatui::Terminal;
use tidev_types::message::BackendEvent;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::app::App;

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
        let mut terminal = Terminal::new(backend)?;

        // Clear the alternate screen and reset the internal double-
        // buffer so the first `draw()` produces a full redraw.
        //
        // NOTE: we use `clear_region` + `swap_buffers` instead of
        // `terminal.clear()` to avoid `cursor::position()` (which
        // involves a blocking DSR query-response round-trip that
        // can hang on slow terminals or piped input).
        terminal
            .backend_mut()
            .clear_region(ClearType::All)
            .context("failed to clear terminal")?;
        terminal.swap_buffers();

        Ok(Self { terminal })
    }

    pub async fn run(&mut self, app: &mut App) -> Result<()> {
        // Take ownership of receivers so the borrow checker is happy.
        let mut request_rx = app.request_rx.take();
        let mut event_rx = app.event_rx.take();

        // ── Dedicated crossterm reader thread ─────────────────────────
        //
        // A single OS thread that synchronously calls `poll()` + `read()`
        // and forwards every event through an mpsc channel.  This mirrors
        // the v0.6.x architecture (one thread, one event source, no async
        // wrappers) and avoids the cross-thread / cross-task contention
        // that plagued earlier async-based approaches.
        //
        // The thread is started *before* the initial render so that any
        // keystroke arriving during startup is guaranteed to be captured
        // (the kernel buffers stdin, and the reader thread begins polling
        // immediately).
        let reader_running = Arc::new(AtomicBool::new(true));
        let (crossterm_tx, mut crossterm_rx) = mpsc::unbounded_channel::<Event>();

        let running = reader_running.clone();
        let tx = crossterm_tx.clone();
        let reader_handle = thread::Builder::new()
            .name("crossterm-reader".into())
            .spawn(move || {
                // The poll interval balances responsiveness against
                // CPU usage.  50 ms matches the v0.6.x behaviour.
                let poll_interval = Duration::from_millis(50);

                while running.load(Ordering::SeqCst) {
                    match crossterm::event::poll(poll_interval) {
                        Ok(true) => {
                            let event = match crossterm::event::read() {
                                Ok(event) => event,
                                Err(e) => {
                                    log::error!("crossterm read error: {e}");
                                    break;
                                }
                            };

                            // Mirror v0.6.x: only forward Press/Repeat
                            // key events.  Release events are filtered
                            // out at the source.
                            let should_send = match &event {
                                Event::Key(key) => matches!(
                                    key.kind,
                                    KeyEventKind::Press | KeyEventKind::Repeat
                                ),
                                _ => true,
                            };

                            if should_send && tx.send(event).is_err() {
                                break;
                            }
                        }
                        Ok(false) => {}
                        Err(e) => {
                            log::error!("crossterm poll error: {e}");
                            break;
                        }
                    }
                }

                log::info!("crossterm reader thread exited");
            })
            .context("failed to spawn crossterm reader thread")?;

        // ── Initial render ───────────────────────────────────────────
        self.terminal
            .draw(|frame| app.draw(frame))
            .context("failed to render initial frame")?;
        app.mark_clean();
        let mut last_render = Instant::now();

        loop {
            // ── Phase 1: Wait for and process the next event ─────────

            let mut processed = false;

            tokio::select! {
                result = crossterm_rx.recv() => {
                    match result {
                        Some(event) => {
                            app.handle_crossterm_event(event);
                            processed = true;
                        }
                        None => {
                            // Reader thread exited (channel closed).
                            break;
                        }
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
                        processed = true;
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
                        processed = true;
                    }
                }
                _ = tokio::time::sleep(FRAME_BUDGET) => {}
            }

            if app.should_quit() {
                break;
            }

            // ── Phase 2: Drain remaining events (non-blocking) ───────

            // 2a. Backend events (batch up to 200, coalesce deltas).
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

            // 2c. Tool permission requests.
            if let Some(ref mut rx) = request_rx {
                while let Ok(request) = rx.try_recv() {
                    app.handle_tui_request(request);
                }
            }

            // ── Phase 2.5: Fallback flush for pending prompts ─────────
            //
            // Per-frame: flush queued prompts for any session whose loop
            // has finished since the last frame.
            if app.has_pending_prompts() {
                app.flush_pending_prompt_queue();
            }

            // ── Phase 2.6: Refresh active-session list for the UI ──────
            let active = app.runtime.active_sessions();
            app.set_active_sessions(active);

            if app.should_quit() {
                break;
            }

            // ── Phase 3: Throttled render ────────────────────────────
            //
            // Per-frame: auto-scroll when dragging a selection near the edge
            // of the message content area or composer input area (mirrors old TUI behaviour).
            app.update_mouse_selection_auto_scroll();
            app.update_input_area_auto_scroll();

            // Spinner wake-up: during a pending request (active but not yet
            // streaming) or compaction, re-dirty the message list whenever the
            // ASCII spinner frame advances so the footer keeps animating.
            if app.has_active_request() || app.is_compacting() {
                let frame = (app.spinner_elapsed().as_millis() / 100) as u64;
                if frame != app.last_spinner_frame
                    && let Some(ml) = &mut app.message_list {
                        ml.dirty = true;
                    }
            }

            // Render if:
            //   - we just handled input (immediate response), OR
            //   - the UI is dirty AND enough time has passed (FPS cap).
            let now = Instant::now();
            if processed || (app.is_dirty() && now - last_render >= FRAME_BUDGET) {
                self.terminal
                    .draw(|frame| app.draw(frame))
                    .context("failed to render frame")?;
                app.mark_clean();
                app.last_spinner_frame = (app.spinner_elapsed().as_millis() / 100) as u64;
                last_render = now;
            }
        }

        // ── Cleanup ──────────────────────────────────────────────
        //
        // Signal the reader thread to stop.  It will notice the flag
        // within the next poll interval (≤ 50 ms) and exit.
        reader_running.store(false, Ordering::SeqCst);
        let handle = reader_handle;
        // Join the thread in a blocking task so we don't leave a
        // dangling thread during shutdown.
        if let Err(e) = tokio::task::spawn_blocking(move || {
            if let Err(e) = handle.join() {
                log::error!("crossterm reader thread panicked: {e:?}");
            }
        })
        .await
        {
            log::error!("failed to join crossterm reader thread: {e}");
        }

        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // Order matters: first leave the alternate screen (so the
        // terminal driver's output processing is still in raw mode
        // while we write the escape sequence), then disable raw mode.
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableLineWrap,
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
            Show,
        );
        let _ = disable_raw_mode();
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
    if let Some(cd) = slot
        && cd.session_id == session_id && cd.request_id == request_id {
            cd.content.push_str(&content);
            return;
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
