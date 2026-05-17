pub mod compress;
pub mod compression_queue;
pub mod consolidate;
pub mod dedup;
pub mod engine;
pub mod evict;
pub mod graph;
pub mod graph_retrieval;
pub mod hybrid_search;
pub mod lessons;
pub mod observe;
pub mod patterns;
pub mod reflect;
pub mod remember;
pub mod retention;
pub mod search_index;
pub mod sessions;
pub mod slots;
pub mod types;

pub use compression_queue::{CompressionQueue, QueueTask, DEFAULT_COMPRESSION_CONCURRENCY};
pub use dedup::DedupMap;
pub use engine::MemoryStore;
pub use search_index::Bm25Index;
pub use types::*;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Handles returned from starting memory background tasks.
pub struct BackgroundTasks {
    /// The compression queue, for shutdown signalling.
    pub compression_queue: Arc<CompressionQueue>,
}

/// Start all memory background tasks: compression queue, periodic eviction,
/// consolidation, and reflection.
///
/// Must be called after models are configured on `store` (via
/// [`MemoryStore::set_models`], [`MemoryStore::set_embedding_model`], etc.).
///
/// Call once per session lifecycle, from within a tokio runtime context.
pub fn start_background_tasks(
    store: Arc<MemoryStore>,
    runtime: &tokio::runtime::Handle,
    workspace_root: &str,
) -> BackgroundTasks {
    let _ws = workspace_root.to_string();

    // ── Compression queue ──────────────────────────────────────────────
    let shutdown = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(CompressionQueue::start(
        store.clone(),
        DEFAULT_COMPRESSION_CONCURRENCY,
        shutdown,
    ));
    store.set_compression_sender(queue.sender());
    crate::log_info!(
        "memory: compression queue started ({} workers)",
        DEFAULT_COMPRESSION_CONCURRENCY,
    );

    // ── Recover uncompressed observations ──────────────────────────────
    match store.recover_uncompressed(50) {
        Ok(0) => crate::log_info!("memory: no uncompressed observations to recover"),
        Ok(n) => crate::log_info!("memory: queued {} uncompressed observations for compression", n),
        Err(e) => crate::log_warn!("memory: recovery of uncompressed observations failed: {}", e),
    }

    // ── Backfill embeddings ────────────────────────────────────────────
    match store.backfill_embeddings(50) {
        Ok(0) => crate::log_info!("memory: no observations need embedding backfill"),
        Ok(n) => crate::log_info!("memory: queued {} observations for embedding backfill", n),
        Err(e) => crate::log_warn!("memory: backfill of embeddings failed: {}", e),
    }

    // ── Periodic eviction (every 3600s) ────────────────────────────────
    let evict_store = store.clone();
    runtime.spawn(async move {
        // Run once on startup
        if let Err(e) = evict_store.run_eviction() {
            crate::log_warn!("memory: initial eviction failed: {}", e);
        } else {
            crate::log_info!("memory: initial eviction completed");
        }
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            crate::log_info!("memory: running eviction");
            if let Err(e) = evict_store.run_eviction() {
                crate::log_warn!("memory: eviction failed: {}", e);
            } else {
                crate::log_info!("memory: eviction completed");
            }
        }
    });

    // ── Periodic consolidation (60s poll, DB-timestamp) ────────────────
    let cons_store = store.clone();
    let cons_ws = _ws.clone();
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let last_run = cons_store
                .meta_get("consolidation_last_run")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<i64>().ok());
            let elapsed = last_run.map(|ts| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                now - ts
            });
            if elapsed.map_or(true, |d| d >= 1800) {
                crate::log_info!(
                    "memory: running consolidation (last run: {}s ago)",
                    elapsed.unwrap_or(-1)
                );
                if let Err(e) = cons_store.run_consolidation(&cons_ws).await {
                    crate::log_warn!("memory: consolidation failed: {}", e);
                } else {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = cons_store.meta_set("consolidation_last_run", &now.to_string());
                    crate::log_info!("memory: consolidation completed");
                }
            }
        }
    });

    // ── Periodic reflection (60s poll, DB-timestamp) ───────────────────
    let refl_store = store.clone();
    let refl_ws = _ws;
    runtime.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let last_run = refl_store
                .meta_get("reflection_last_run")
                .ok()
                .flatten()
                .and_then(|s| s.parse::<i64>().ok());
            let elapsed = last_run.map(|ts| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                now - ts
            });
            if elapsed.map_or(true, |d| d >= 1800) {
                crate::log_info!(
                    "memory: running reflection (last run: {}s ago)",
                    elapsed.unwrap_or(-1)
                );
                if let Err(e) = refl_store.run_reflect(&refl_ws).await {
                    crate::log_warn!("memory: reflection failed: {}", e);
                } else {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let _ = refl_store.meta_set("reflection_last_run", &now.to_string());
                    crate::log_info!("memory: reflection completed");
                }
            }
        }
    });

    BackgroundTasks { compression_queue: queue }
}
