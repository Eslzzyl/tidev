pub mod consolidate;
pub mod engine;
pub mod evict;
pub mod graph;
pub mod graph_retrieval;
pub mod lessons;
pub mod patterns;
pub mod reflect;
pub mod remember;
pub mod retention;
pub mod search_index;
pub mod sessions;
pub mod slots;
pub mod types;
pub mod xml;

pub use engine::MemoryStore;
pub use search_index::fts5_search_memories;
pub use types::*;

use crate::config::MemoryConfig;
use std::sync::Arc;
use std::time::Duration;

/// Start all memory background tasks: periodic eviction, consolidation,
/// and reflection.
///
/// Must be called after models are configured on `store` (via
/// [`MemoryStore::set_models`]).
///
/// Only spawns tasks when `config.enabled && config.auto_learn` is true.
///
/// Call once per session lifecycle, from within a tokio runtime context.
pub fn start_background_tasks(
    store: Arc<MemoryStore>,
    runtime: &tokio::runtime::Handle,
    workspace_root: &str,
    config: &MemoryConfig,
) {
    if !config.enabled || !config.auto_learn {
        crate::log_info!("memory: background tasks disabled by config");
        return;
    }

    let _ws = workspace_root.to_string();

    // ── Periodic eviction (every 3600s) ────────────────────────────────
    let evict_store = store.clone();
    runtime.spawn(async move {
        // Run once on startup
        let _t_evict = std::time::Instant::now();
        if let Err(e) = evict_store.run_eviction() {
            crate::log_warn!("memory: initial eviction failed: {}", e);
        } else {
            crate::log_info!("memory: initial eviction completed in {:?}", _t_evict.elapsed());
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
            if elapsed.is_none_or(|d| d >= 1800) {
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
            if elapsed.is_none_or(|d| d >= 1800) {
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
}
