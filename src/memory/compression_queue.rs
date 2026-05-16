use anyhow::Result;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, TryRecvError},
};
use std::thread;
use std::time::Duration;
use uuid::Uuid;

use super::engine::MemoryStore;

/// Bounded channel + N worker threads for async observation compression.
///
/// Replaces the per-observation `std::thread::spawn` pattern with a fixed pool
/// of consumer threads that pull from a shared channel. This provides:
/// - Bounded concurrency (default 4 simultaneous compressions)
/// - Natural backpressure (channel capacity 256)
/// - Clean shutdown (drains in-flight work)
pub struct CompressionQueue {
    /// The sender is wrapped in `Mutex<Option<>>` so `shutdown()` can drop
    /// it without consuming `self` (needed because this struct is behind `Arc`).
    sender: Mutex<Option<mpsc::SyncSender<Uuid>>>,
    /// Worker thread handles — never moved or joined; the OS cleans up on exit.
    _workers: Vec<thread::JoinHandle<()>>,
    /// Shared flag read by workers to exit early.
    shutdown: Arc<AtomicBool>,
}

impl CompressionQueue {
    /// Create a compression queue with N worker threads.
    ///
    /// Each worker polls the shared channel via `try_recv()` (without
    /// blocking other workers) and calls `store.compress()` on each
    /// observation ID received.
    pub fn start(store: Arc<MemoryStore>, concurrency: usize, shutdown: Arc<AtomicBool>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Uuid>(256);
        let rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(concurrency);

        for i in 0..concurrency {
            let store = store.clone();
            let rx = rx.clone();
            let shutdown = shutdown.clone();
            let handle = thread::Builder::new()
                .name(format!("compress-{}", i))
                .spawn(move || {
                    let rt = match tokio::runtime::Handle::try_current() {
                        Ok(h) => h,
                        Err(_) => {
                            crate::log_warn!("compression worker {}: no tokio runtime, exiting", i);
                            return;
                        }
                    };

                    loop {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }

                        // Hold the lock only long enough to try_recv.
                        // This prevents one worker from blocking others.
                        let id = {
                            let lock = rx.lock().unwrap();
                            match lock.try_recv() {
                                Ok(id) => id,
                                Err(TryRecvError::Empty) => {
                                    drop(lock);
                                    thread::sleep(Duration::from_millis(100));
                                    continue;
                                }
                                Err(TryRecvError::Disconnected) => break,
                            }
                        };

                        if let Err(e) = rt.block_on(store.compress(id)) {
                            crate::log_warn!("compression failed for {}: {}", id, e);
                        }
                    }
                })
                .expect("failed to spawn compression worker thread");
            workers.push(handle);
        }

        Self {
            sender: Mutex::new(Some(tx)),
            _workers: workers,
            shutdown,
        }
    }

    /// Get a clone of the sender for passing to `MemoryStore`.
    pub fn sender(&self) -> mpsc::SyncSender<Uuid> {
        self.sender
            .lock()
            .unwrap()
            .as_ref()
            .expect("compression queue sender not available")
            .clone()
    }

    /// Enqueue an observation for async compression.  Non-blocking.
    /// Returns `Ok(())` if the item was queued, or an error if the
    /// channel is full or disconnected.
    pub fn enqueue(&self, id: Uuid) -> Result<()> {
        let sender = self.sender.lock().unwrap();
        match sender.as_ref() {
            Some(s) => s.send(id)?,
            None => anyhow::bail!("compression queue is shut down"),
        }
        Ok(())
    }

    /// Signal shutdown. Workers will exit on their next poll after the
    /// sender is dropped (channel → Disconnected) or the shutdown flag is set.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        *self.sender.lock().unwrap() = None;
    }
}

/// Default number of concurrent compression workers.
pub const DEFAULT_COMPRESSION_CONCURRENCY: usize = 4;
