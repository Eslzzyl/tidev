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

/// A task for the compression queue worker pool.
pub enum QueueTask {
    /// Full compression + embedding for a new observation.
    CompressAndEmbed(Uuid),
    /// Embedding-only for an already-compressed observation
    /// that is missing a vector embedding.
    EmbedBackfill(Uuid),
}

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
    sender: Mutex<Option<mpsc::SyncSender<QueueTask>>>,
    /// Worker thread handles — never moved or joined; the OS cleans up on exit.
    _workers: Vec<thread::JoinHandle<()>>,
    /// Shared flag read by workers to exit early.
    shutdown: Arc<AtomicBool>,
}

impl CompressionQueue {
    /// Create a compression queue with N worker threads.
    ///
    /// Each worker polls the shared channel via `try_recv()` (without
    /// blocking other workers) and dispatches to the appropriate
    /// `MemoryStore` method based on [`QueueTask`] variant.
    pub fn start(store: Arc<MemoryStore>, concurrency: usize, shutdown: Arc<AtomicBool>) -> Self {
        let (tx, rx) = mpsc::sync_channel::<QueueTask>(256);
        let rx = Arc::new(Mutex::new(rx));
        let mut workers = Vec::with_capacity(concurrency);

        for i in 0..concurrency {
            let store = store.clone();
            let rx = rx.clone();
            let shutdown = shutdown.clone();
            let handle = thread::Builder::new()
                .name(format!("compress-{}", i))
                .spawn(move || {
                    let worker_rt = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(rt) => rt,
                        Err(e) => {
                            crate::log_warn!(
                                "compression worker {}: failed to create runtime: {}",
                                i,
                                e
                            );
                            return;
                        }
                    };

                    loop {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }

                        let task = {
                            let lock = rx.lock().unwrap();
                            match lock.try_recv() {
                                Ok(task) => task,
                                Err(TryRecvError::Empty) => {
                                    drop(lock);
                                    thread::sleep(Duration::from_millis(100));
                                    continue;
                                }
                                Err(TryRecvError::Disconnected) => break,
                            }
                        };

                        match task {
                            QueueTask::CompressAndEmbed(id) => {
                                if let Err(e) = worker_rt.block_on(store.compress(id)) {
                                    crate::log_warn!("compression failed for {}: {}", id, e);
                                }
                            }
                            QueueTask::EmbedBackfill(id) => {
                                if let Err(e) = worker_rt.block_on(store.backfill_embedding(id)) {
                                    crate::log_warn!(
                                        "embedding backfill failed for {}: {}",
                                        id,
                                        e
                                    );
                                }
                            }
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
    pub fn sender(&self) -> mpsc::SyncSender<QueueTask> {
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
            Some(s) => s.send(QueueTask::CompressAndEmbed(id))?,
            None => anyhow::bail!("compression queue is shut down"),
        }
        Ok(())
    }

    /// Enqueue an embedding-only backfill task.  Non-blocking.
    pub fn enqueue_embedding_backfill(&self, id: Uuid) -> Result<()> {
        let sender = self.sender.lock().unwrap();
        match sender.as_ref() {
            Some(s) => s.send(QueueTask::EmbedBackfill(id))?,
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
