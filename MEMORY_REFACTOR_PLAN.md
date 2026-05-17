# Memory System Refactoring Plan

## Objective

Remove the observation pipeline and embedding infrastructure. Replace each of their consumption points with lighter alternatives.

## Rationale

The observation pipeline (`observe → compress → embed → search`) adds significant complexity but delivers questionable value:

| Cost | Details |
|------|---------|
| LLM compression | Every tool call → LLM call (or synthetic fallback) |
| Dedup | In-memory SHA256 cache with TTL |
| Embedding | Every observation → vector embedding (LLM API call) |
| BM25 index | In-memory index, must be rebuilt on restart |
| Graph extraction | Per-observation node/edge creation |
| Circuit breaker | Compression failure handling |
| Backfill | Startup recovery + embedding backfill |

The `search` path had a bug — observation IDs were fused in hybrid search but silently dropped when resolved against the `memories` table, so observations were never searchable anyway.

### zstd compression constraints

| Data | Storage | FTS5? |
|------|---------|-------|
| `messages.content` | zstd-compressed BLOB | **No** — decompress overhead not worth it |
| `memories.content` | **Plain TEXT** (not compressed) | **Yes** — external content, just needs rebuild |
| `memories.title / tags / concepts / files` | Plain TEXT | Yes |
| `session_summaries.*` | Plain TEXT | **No** — LIKE is sufficient (hundreds of rows) |
| `sessions.context_summary` | Plain TEXT | No |

`memories.content` is stored as plain TEXT — confirmed in `remember.rs:90-98` (direct string INSERT, no `compress_text` call). The compression doc was outdated on this point.

## What Stays

| Component | Reason |
|-----------|--------|
| `memory` tool (`remember/search/list/read/forget`) | Core feature |
| `session_summaries` table + summary generation | Cross-session awareness |
| Consolidation pipeline (`consolidate.rs`) | Cross-session fact extraction |
| Reflection pipeline (`reflect.rs`) | Meta-insight synthesis |
| `memories` table + `memories_fts` FTS5 index | Memory storage + keyword search |
| `MemoryEntry` / `MemoryType` / `SessionSummary` types | Core data types |
| XML parsing utilities (`xml.rs`) | Session summary still uses XML output |
| `patterns.rs` co-change + error mining (rewritten) | Valuable — see replacement below |
| `graph.rs` knowledge graph (rewritten) | Valuable — see replacement below |

## What Goes

### Modules (remove entirely)

| File | Reason |
|------|--------|
| `src/memory/observe.rs` | Observation creation |
| `src/memory/compress.rs` | LLM + synthetic compression |
| `src/memory/compression_queue.rs` | Worker pool for async compression |
| `src/memory/dedup.rs` | SHA256 dedup for observations |
| `src/memory/hybrid_search.rs` | BM25 + vector RRF fusion |
| `src/memory/search_index.rs` -> `Bm25Index` only | In-memory BM25; keep `fts5_search_memories` |

### Types to remove

| Type | Used by |
|------|---------|
| `HookPayload` | Observation creation |
| `HookType` | Observation creation |
| `ObservationType` | Observation compression |
| `RawObservation` | Observation storage |
| `CompressedObservation` | Observation (search, graph, patterns) |
| `ObservationResult` | observe() return |
| `Modality` | Observation metadata |
| `HybridSearchResult` | Hybrid search fusion |
| `HybridScore` (private) | Hybrid search fusion |

### Fields to remove from `MemoryStore`

| Field | Reason |
|-------|--------|
| `dedup: DedupMap` | Observation dedup |
| `bm25: RwLock<Bm25Index>` | Observation BM25 index |
| `compression_enabled: AtomicBool` | Compression toggle |
| `llm_compression: AtomicBool` | LLM compression toggle |
| `compression_model: RwLock<Option<...>>` | Compression model config |
| `embedding_model: RwLock<Option<...>>` | Embedding model config |
| `compressor_tx` | Compression queue sender |
| `compression_cb_failures` | Circuit breaker state |
| `compression_cb_tripped_at` | Circuit breaker state |
| `hybrid_search: RwLock<HybridSearch>` | RRF fusion |

### Methods to remove from `MemoryStore`

- `observe()`
- `compress()`
- `schedule_compression()`
- `schedule_embedding_backfill()`
- `recover_uncompressed()`
- `backfill_embeddings()`
- `backfill_embedding()`
- `set_compression_enabled()`
- `set_llm_compression()`
- `set_compression_sender()`
- `resolve_compression_llm()`
- `set_embedding_model()`
- `load_recent_compressed_observations()`
- `list_recent_observations()`
- `graph_extract_from_observation()`
- `search_hybrid_with()` (directly)
- `search_hybrid()` (if it calls hybrid)

### Methods requiring LLM resolver update (not removal)

| Method | Current resolver | New resolver |
|--------|-----------------|-------------|
| `summarize_session` | `resolve_summarization_llm` | same, drop compression_model fallback |
| `run_consolidation` | `resolve_compression_llm` | rename to `resolve_consolidation_llm` (uses active_model or summarization_model) |
| `run_reflect` | `resolve_compression_llm` | rename to `resolve_reflection_llm` (uses active_model or summarization_model) |

### Database tables to remove from writes

| Table | Reason |
|-------|--------|
| `compressed_observations` | Observation storage — stop writing, leave rows orphaned |
| `vec_obs_map` | Observation UUID → vec0 rowid |
| `vec_observations` (vec0 virtual) | Observation embeddings |

**Note:** Tables stay in schema for backward compatibility. No drop migration. Just stop writing to them.

### Config fields to remove

| Config field | Location |
|--------------|----------|
| `memory.compression_enabled` | `config.toml` + `Config` struct |
| `memory.llm_compression` | `config.toml` + `Config` struct |
| `memory.compression_model` | `config.toml` + `Config` struct |
| `memory.embedding_model` | `config.toml` + `Config` struct |

### Hook engine changes

- Remove `store.observe(&payload)` from `on_post_tool_use()` (hooks/engine.rs:~167-180)
- Remove `store.observe(&payload)` from `on_pre_tool_use()` (hooks/engine.rs:~189-201)
- Remove `store.observe(&payload)` from `on_post_tool_failure()` (hooks/engine.rs:~281-299)
- Remove (or no-op) `record_observation()` (hooks/engine.rs:~303-324)

### TUI changes

- Remove observations panel from `tui/ui/memory_panel.rs`
- Remove observations rendering from `tui/render/chat_dialog/panels.rs`
- Remove compression toggle from `tui/ui/settings_panel.rs` (lines 73-85, `SettingKey::CompressionEnabled` + `SettingKey::LlmCompression`)
- Remove `set_compression_enabled` / `set_llm_compression` calls from `tui/input/event/actions.rs` (lines 441-451, `close_settings_panel`)
- Remove `compression_model`/`embedding_model` resolution + `set_embedding_model`/`set_compression_enabled`/`set_llm_compression` from `tui/core/run.rs` (lines 84-113)
- Remove `compression_queue` field, `BackgroundTasks` storage, and shutdown logic from `tui/core/run.rs`
- Remove `CompressionEnabled` / `LlmCompression` variants from `SettingKey` enum in `tui/ui/settings_panel.rs`

### Gateway/web changes

- `web/mod.rs:102-106` — remove `set_compression_enabled`, `set_llm_compression`
- `gateway/mod.rs:95-97, 169-171` — remove `set_compression_enabled`, `set_llm_compression` (2 channels)
- Both already pass `None` for compression/summarization model overrides, so no model resolution change needed

## What Gets Rewritten

### 1. Session summary (`sessions.rs`)

**Before:** Loads compressed_observations → LLM → summary
**After:** Reuses compaction's approach — feeds messages to LLM → summary

**Design:**
- `SessionService::summarize_session` builds its prompt by calling the same message-building helper that compaction uses (e.g. `ContextManager::build_request_messages()` or equivalent), producing:
  - System prompt
  - All user/assistant/tool messages verbatim
  - A `"Please summarize this session"` user message
- The output format stays the same XML structure (title, narrative, decisions, files, concepts — parsed via `xml.rs`)
- `resolve_summarization_llm` falls back to `active_model` directly (remove compression_model from chain)

#### ⚠️ Critical constraint: messages must be append-only

When building the LLM request for session summary, messages MUST be fed to the provider exactly as stored — **no filtering, no rewriting, no truncation of individual messages**.

The reason is **provider prefix caching**: the same message prefix (system prompt + early messages) appears in both compaction requests and session summary requests. If the prefix is byte-for-byte identical, the provider reuses its cached KV cache from the compaction call, saving significant cost and latency. Any modification — even trimming trailing whitespace or reordering fields — invalidates the cache.

**Concretely:**
- OK: append a new summary instruction at the end
- OK: skip the entire summarization if session is too short
- **NOT OK**: trim/edit/reformat individual message content
- **NOT OK**: remove tool result messages (breaks tool call → result pairing)
- **NOT OK**: reorder messages or insert synthetic messages

### 2. Pattern mining (`patterns.rs`)

**co_change patterns:**
- **Before:** Reads `files` field from compressed_observations
- **After:** Reads tool result messages with `tool_name IN ('write', 'edit', 'edit_and_apply')`, extracts file paths from the message content/metadata

Implementation concerns: messages.content is zstd-compressed. The pattern miner needs to decompress tool result content to extract file paths. This is acceptable because pattern mining runs once per session (consolidation frequency), not per request.

**error_repeat patterns:**
- **Before:** Reads `obs_type = 'error'` from compressed_observations
- **After:** Reads tool result messages where content matches error patterns (non-zero exit, error/fail/panic markers). Same decompression trade-off as co-change.

### 3. Knowledge graph extraction (`graph.rs`)

**Before:** `extract_from_observation` creates nodes from `obs.concepts` + `obs.files`
**After:** `extract_from_session_summary` or `extract_from_memory_entry` — creates nodes from:
- `session_summaries.files_modified` and `session_summaries.concepts` (plain TEXT, no decompression needed)
- Or `MemoryEntry.files` and `MemoryEntry.concepts` from consolidation output (plain TEXT)

### 4. Session summary injection from `compose_dynamic_context`

- Remove `## Recent Key Observations` section from `agent/runtime.rs`
- This section loaded `load_recent_compressed_observations(&session_id, 8, 5)` and injected into system-reminder
- It was redundant — model already has full messages in context

### 5. `start_background_tasks` in `memory/mod.rs`

Current tasks:

| Task | Lines | Interval | Action |
|------|-------|----------|--------|
| Compression queue startup | 52-63 | — | **Remove** |
| `recover_uncompressed(50)` | 65-70 | startup | **Remove** |
| `backfill_embeddings(50)` | 72-77 | startup | **Remove** |
| Periodic eviction (`run_eviction`) | 79-98 | 3600s | **Keep** |
| Periodic consolidation (`run_consolidation`) | 100-136 | 1800s poll | **Keep**, but rename LLM resolver |
| Periodic reflection (`run_reflect`) | 138-174 | 1800s poll | **Keep**, but rename LLM resolver |

Both `run_consolidation` (engine.rs:951) and `run_reflect` (engine.rs:1548) currently call `resolve_compression_llm()`. After removing compression model, they need a new resolver: either reuse `resolve_summarization_llm` (chain: `summarization_model → active_model`) or add a dedicated one.

`BackgroundTasks` struct (mod.rs:33-36) holds `compression_queue: Arc<CompressionQueue>`, used by TUI for shutdown (`run.rs:460`). After refactor, remove the struct entirely — `start_background_tasks` returns nothing.

## What Gets Added

### 1. `memories_fts` rebuild on startup

`memories_fts` is already defined as an external content FTS5 table referencing `memories`. It was never populated — no `'rebuild'` command exists in the codebase.

**Fix:** Add at startup (in `MemoryStore::new()` or `start_background_tasks()`):

```rust
db.execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", [])?;
```

This reads all rows from `memories`, tokenizes the columns (title + content + tags + concepts + files — all plain TEXT), and builds the FTS5 inverted index. Subsequent `SELECT ... FROM memories_fts WHERE memories_fts MATCH ?` will work correctly.

### 2. Incremental FTS5 update on `remember()`

After the initial rebuild, new memories need to be indexed incrementally. In `RememberService::remember()`, after the INSERT into `memories`:

```rust
db.execute(
    "INSERT INTO memories_fts(rowid, title, content, tags, concepts, files)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    rusqlite::params![last_insert_rowid(), title, content, tags_json, concepts_json, files_json],
)?;
```

This keeps the FTS5 index in sync without a full rebuild on every startup.

### 3. `resolve_summarization_llm` cleanup

Remove `compression_model` from the fallback chain. After this refactor, the chain becomes:
```rust
summarization_model → active_model
```

## Startup Initialization

Three entry points configure the MemoryStore at startup: TUI (`tui/core/run.rs`), web (`web/mod.rs`), and gateway (`gateway/mod.rs`, 2 channels). After refactoring, all three converge on the same simplified pattern:

### Current sequence (for each mode)

| Step | TUI | Web | Gateway |
|------|-----|-----|---------|
| 1. DB open + schema | `Database::open()` → executes `SCHEMA_SQL` | same | same |
| 2. Create MemoryStore | `open_with_shared_write()` — initializes dedup/bm25/compression/embedding/hybrid fields | same | same (x2) |
| 3. Set LLM + models | `set_models(llm, active, compression_override, summarization_override)` | `set_models(llm, default, None, None)` | same (x2) |
| 4. Compression flags | `set_compression_enabled()`, `set_llm_compression()` | same | same (x2) |
| 5. Embedding | `set_embedding_model()` (creates vec0 table lazily) | **not called** | **not called** |
| 6. Background tasks | `start_background_tasks()` — queue + recover + backfill + evict + consolidate + reflect | same (discards return) | same (x2, discards return) |

### After refactor

| Step | All modes |
|------|-----------|
| 1. DB open + schema | Unchanged. Deprecated tables (`compressed_observations`, `vec_obs_map`, `vec_observations`) remain but are never written to. |
| 2. Create MemoryStore | Constructor removes all observation/embedding/compression fields. Keep only `llm`, `active_model`, `summarization_model`. |
| 3. Set LLM + models | `set_models(llm, active, summarization)` — only 2 model params. Remove compression_model parameter. |
| 4. Compression flags | Removed entirely. No `set_compression_enabled`, `set_llm_compression`, `set_embedding_model`. |
| 5. Background tasks | Simplified to eviction + consolidation + reflection only. No return value. |
| 6. FTS5 rebuild | New step: `db.execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")` |

### `set_models` signature change

```rust
// Before
pub fn set_models(&self, llm: LlmClient, active: ActiveModel,
                   compression: Option<ActiveModel>, summarization: Option<ActiveModel>)

// After
pub fn set_models(&self, llm: LlmClient, active: ActiveModel,
                   summarization: Option<ActiveModel>)
```

### `MemoryStore` constructor field changes

```rust
// Before (open_with_shared_write)
dedup: Mutex::new(DedupMap::new()),                      // REMOVE
bm25: RwLock::new(Bm25Index::new()),                     // REMOVE
llm: RwLock::new(None),                                  // KEEP
active_model: RwLock::new(None),                         // KEEP
compression_model: RwLock::new(None),                    // REMOVE
summarization_model: RwLock::new(None),                  // KEEP
embedding_model: RwLock::new(None),                      // REMOVE
hybrid_search: RwLock::new(HybridSearch::new()),         // REMOVE
compression_enabled: AtomicBool::new(true),              // REMOVE
llm_compression: AtomicBool::new(false),                 // REMOVE
compression_cb_failures: AtomicU32::new(0),              // REMOVE
compression_cb_tripped_at: RwLock::new(None),            // REMOVE
compression_sender: RwLock::new(None),                   // REMOVE
```

### Caller-specific startup changes

| Entry point | Changes |
|-------------|---------|
| `tui/core/run.rs:83-113` | Remove `compression_model` resolution (lines 84-88). Keep `summarization_model` resolution (lines 89-93). Remove `set_compression_enabled`, `set_llm_compression` (lines 100-101). Remove `set_embedding_model` block (lines 107-113). Simplify `set_models` call to 3 args. |
| `web/mod.rs:102-106` | Remove `set_compression_enabled`, `set_llm_compression`. `set_models` already passes `None, None` → becomes `None`. |
| `gateway/mod.rs:95-97, 169-171` | Same as web, applied to both Telegram and QQ channels. |

## Search Architecture

After refactoring:

```
memory::search(query):
  ┌─ memories_fts (FTS5) ──────────── 模型主动 remember 的事实
  │   (startup rebuild + incremental update on remember)
  │
  └─ session_summaries (LIKE) ─────── 会话级 narrative + concepts
      WHERE title LIKE ? OR narrative LIKE ? OR concepts LIKE ?
      (几百行数据，LIKE 足够)
```

- No in-memory state
- No LLM API calls
- No embedding latency

## File Changes Summary

### Files to delete

| File | Lines |
|------|-------|
| `src/memory/observe.rs` | ~79 |
| `src/memory/compress.rs` | ~887 |
| `src/memory/compression_queue.rs` | ~186 |
| `src/memory/dedup.rs` | ~65 |
| `src/memory/hybrid_search.rs` | ~79 |

### Files to significantly modify

| File | Changes |
|------|---------|
| `src/memory/engine.rs` | Remove ~500 lines (observe, compress, embed, BM25, hybrid search, backfill, graph_extract). Remove fields: dedup, bm25, compression_model, embedding_model, compression_enabled, llm_compression, cb_*, compression_sender, hybrid_search. Remove methods listed above. Rename LLM resolvers for consolidate/reflect. Simplify `search()` to FTS5 + LIKE only. |
| `src/memory/sessions.rs` | Rewrite `summarize_session` to use messages instead of observations |
| `src/memory/patterns.rs` | Rewrite `mine_co_change` and `mine_error_repeats` to read messages (with decompression) |
| `src/memory/graph.rs` | Rewrite `extract_from_observation` to use session_summaries or memories |
| `src/memory/remember.rs` | Add FTS5 incremental write after INSERT |
| `src/memory/mod.rs` | Remove module decls (compress, compression_queue, dedup, hybrid_search, observe). Remove `pub use` for CompressionQueue/DedupMap/Bm25Index. Remove BackgroundTasks struct. Simplify start_background_tasks to only eviction/consolidation/reflection, no return value. |
| `src/memory/search_index.rs` | Remove `Bm25Index`, add `fts5_rebuild_memories()` helper |
| `src/memory/types.rs` | Remove ~150 lines of observation-related types (HookPayload, HookType, ObservationType, RawObservation, CompressedObservation, ObservationResult, Modality, HybridSearchResult, HybridScore) |
| `src/hooks/engine.rs` | Remove 4x `store.observe()` calls + `record_observation` method |
| `src/agent/runtime.rs` | Remove `format_compressed_observations` + `## Recent Key Observations` injection |
| `src/config/mod.rs` | Remove `compression_enabled`, `llm_compression`, `compression_model`, `embedding_model` fields |
| `src/storage/schema.rs` | Keep deprecated tables, add startup rebuild logic |
| `src/tui/ui/memory_panel.rs` | Remove observations panel |
| `src/tui/render/chat_dialog/panels.rs` | Remove observations rendering |
| `src/tui/ui/settings_panel.rs` | Remove Compression/LLM Compression entries, SettingKey variants, apply_to_config branches |
| `src/tui/input/event/actions.rs` | Remove `set_compression_enabled`/`set_llm_compression` from close_settings_panel |
| `src/tui/core/run.rs` | Remove compression_model/embedding_model resolution, set_embedding_model/set_compression_enabled/set_llm_compression calls, compression_queue field + shutdown, simplify start_background_tasks call |
| `src/web/mod.rs` | Remove `set_compression_enabled`, `set_llm_compression` |
| `src/gateway/mod.rs` | Remove `set_compression_enabled`, `set_llm_compression` (2 channels) |

## Implementation Order

### Phase 1 — Remove observation write path ✅

1. ✅ Delete `observe.rs`, `compress.rs`, `compression_queue.rs`, `dedup.rs`
2. ✅ Remove hook engine observation recording (4x `store.observe()`, `record_observation`)
3. ✅ Remove observation-related types from `types.rs`
4. ✅ Remove MemoryStore fields/methods for observation, compression (see field table above)
5. ✅ Remove config fields (`compression_enabled`, `llm_compression`, `compression_model`)
6. ✅ Remove `set_compression_sender`, `compression_enabled`, `llm_compression`, `embedding_model` from all callers
7. ✅ Remove TUI settings entries (Compression/LLM Compression toggles)
8. ✅ Remove observations panel + rendering (memory_panel.rs, panels.rs)
9. ✅ Remove `format_compressed_observations` + `## Recent Key Observations` injection (runtime.rs)
10. ✅ Simplify `start_background_tasks` (remove CompressionQueue, recover/backfill, BackgroundTasks struct)
11. ✅ Simplify `run_consolidation` / `run_reflect` to use `resolve_summarization_llm`
12. ✅ Simplify `set_models` to 3 params (llm, active, summarization)
13. ✅ Simplify MemoryStore constructors
14. ✅ `cargo check` + `cargo clippy` + `cargo test` pass

### Phase 2 — Remove embedding + search infrastructure, simplify constructor ✅

1. ✅ Delete `hybrid_search.rs`
2. ✅ Remove `Bm25Index` from `search_index.rs`
3. ✅ `embedding_model` config field removed from `MemoryConfig`; `resolve_embedding_model` removed; `memory_model_label`/`memory_model_display`/`set_memory_model` stripped of "embedding" cases
4. ✅ Simplify MemoryStore constructors (remove dedup, bm25, compression_model, embedding_model, hybrid_search, cb_* fields) — done in Phase 1
5. ✅ Simplify `MemoryStore::search()` to FTS5 + LIKE (no hybrid fallback) — done in Phase 1
6. ✅ Remove `search_hybrid_with()` and `search_hybrid()` — already removed
7. ✅ Add `memories_fts` startup rebuild (`rebuild_fts5_if_needed()`)
8. ✅ Add incremental FTS5 write on `remember()`

### Phase 3 — Rewrite background tasks + startup callers + consumption points ✅

1. ✅ Simplify `set_models` signature: remove `compression` parameter, rename to `(llm, active, summarization)` — done in Phase 1
2. ✅ Simplify all startup callers (run.rs, web.rs, gateway.rs) — done in Phase 1
3. ✅ Simplify `start_background_tasks()` in `mod.rs` — done in Phase 1
4. ✅ Rename LLM resolvers in `engine.rs` — done in Phase 1
5. ✅ Rewrite `sessions.rs` to use messages → LLM summary (reads from `messages` table, decompresses zstd content)
6. ✅ Rewrite `patterns.rs` to use tool messages → co-change + error patterns (reads `file_diffs` and content from tool result messages)
7. ✅ Rewrite `graph.rs` to use session_summaries/memories → graph nodes/edges (added `extract_from_session_summary`, `extract_from_memory_entry`, integrated in consolidation pipeline)
8. ✅ Remove observations from `compose_dynamic_context` (runtime.rs) — done in Phase 1
9. ✅ Remove TUI compression queue field + shutdown logic (run.rs) — done in Phase 1

### Phase 4 — Final cleanup ✅

1. ✅ Remove observations panel (memory_panel.rs, panels.rs) — done in Phase 1
2. ✅ Remove orphaned `CompressedObservation` references from TUI render code — done in Phase 1
3. ✅ `cargo clippy` + `cargo test` pass

## Existing Data

Existing `compressed_observations` rows become orphaned. No need to migrate — they were never searchable (due to the hybrid search bug), and session summaries are already persisted in `session_summaries` independently.

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Session summary quality drops | Medium | Messages contain richer context — quality should improve |
| Pattern mining loses accuracy | Medium | Co-change from writes-only data is actually more accurate than read+write observations |
| `memories_fts` rebuild reveals bugs | Low | Test on a small development database first |
| Decompression in pattern mining is slow | Low | Consolidation runs every 30 min; acceptable latency |
