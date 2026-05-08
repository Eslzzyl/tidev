# TiDev Performance Optimization Analysis

> A systematic analysis of performance bottlenecks in the TUI frontend and
> agent runtime, with specific code locations and suggested fix plans.
>
> Generated: 2026-05-08
> Scope: `src/agent/`, `src/tui/`, `src/storage/`, `src/tooling/`, `src/context.rs`

---

## Table of Contents

1. [Threading & Concurrency Model](#1-threading--concurrency-model)
2. [Message Rendering Pipeline](#2-message-rendering-pipeline)
3. [Text Rendering & Allocation Patterns](#3-text-rendering--allocation-patterns)
4. [Agent Runtime Loop](#4-agent-runtime-loop)
5. [Storage Layer](#5-storage-layer)
6. [Tool Execution](#6-tool-execution)
7. [Event Dispatch & Notifications](#7-event-dispatch--notifications)
8. [Sub-Agent / Delegation](#8-sub-agent--delegation)
9. [Priority Summary](#9-priority-summary)

---

## 1. Threading & Concurrency Model

**Current architecture:** The TUI run loop is single-threaded (synchronous event
loop). A companion `tokio::runtime::Runtime` handles async LLM streaming, tool
execution, and context compaction. The `App` struct owns everything — the tokio
runtime, the LLM client, the session store, etc.

### 1.1 — RefCell contention during render

**Location:** `src/tui/render/render.rs:67-81` (`App::render`) →
`src/tui/render/chat_render/mod.rs:73` (`render_chat`)

The render method borrows `self.message_render_cache` (a `RefCell<HashMap<..>>`)
and the layout index (`RefCell<MessageLayoutIndex>`). Multiple UI panels also
acquire `Mutex` locks (e.g. `balance_panel` at `render_balance.rs:58-65`).

If the render happens concurrently with a `BackendEvent` handler that mutates
shared state, the `RefCell` can **panic at runtime** (`already borrowed`).

**Plan:**
- Replace `RefCell<HashMap>` with a generation-counter approach: two copies
  (front/back) swapped atomically after each render batch, or use
  `std::cell::OnceCell` + immutability patterns.
- Replace `Arc<Mutex<Option<T>>>` panel states with `RwLock<Option<T>>` so
  renders can take a read lock without blocking writes.

### 1.2 — Global tokio runtime contention

**Location:** `src/tui/core/run.rs:23-28` (runtime construction)

All async work (LLM streaming, MCP, tool execution, storage writes) goes through
a single `tokio::runtime::Runtime`. A long-running bash command or an MCP refresh
can starve the LLM streaming task of runtime cycles.

**Plan:**
- Split into two runtimes: a **foreground runtime** (for LLM streaming and UI
  responsiveness) and a **background runtime** (for bash, MCP, storage).
- Use `tokio::task::spawn_blocking` for CPU-heavy storage operations.

---

## 2. Message Rendering Pipeline

This is the heaviest per-frame work. `render_chat` processes every visible
message, computes block data in parallel via `rayon`, then renders visible lines.

### 2.1 — Overly coarse cache invalidation

**Location:** `src/tui/core/run.rs:494-499` (`clear_message_render_cache`)
**Usage sites:** ~15 call sites across `src/tui/mod.rs`, `src/tui/input/event/`

Every Delta event (10–50 ms during streaming) triggers
`invalidate_active_message_render_cache_for`. This removes the affected
message from the cache and **invalidates the entire layout index**
(`.valid = false`).

On the next frame, `render_chat` rebuilds the full `MessageLayoutIndex`
and recomputes all block data — even though only a few characters changed
at the end of the streaming message.

**Plan:**
- **Incremental layout updates.** Instead of setting `.valid = false` globally,
  update the index in-place: recompute only the last `MessageBlock` (the one
  being streamed) and adjust `total_lines`.
- **Fine-grained cache keys.** Add `content_length` to `MessageRenderCacheKey`.
  When a Delta arrives, bump the generation counter for that message ID only,
  rather than invalidating the entire cache.

### 2.2 — Layout index rebuilt every frame

**Location:** `src/tui/render/chat_render/mod.rs:1347-1443` (index building)

The `MessageLayoutIndex` is rebuilt from scratch every frame unless the cache
was invalidated (which happens on every Delta). The rebuild iterates all
messages, groups them into blocks, then calls `par_iter()` to compute line
counts.

**Plan:**
- Add a **dirty generation counter** to `App`:
  ```rust
  struct RenderState {
      message_generation: u64,   // bumped on any message mutation
      layout_generation: u64,    // bumped when layout is rebuilt
      rendered_width: usize,     // width for last layout
  }
  ```
  Skip the rebuild if `message_generation == layout_generation &&
  rendered_width == current_width`.
- On Delta, bump `message_generation` only for the affected message ID,
  allowing incremental block update.

### 2.3 — `par_iter()` overhead on every frame

**Location:** `src/tui/render/chat_render/mod.rs:1399-1412`
(`blocks_info.par_iter().map(compute_block_data)`)

Even with few messages, `rayon::par_iter` has ~10 µs of dispatch overhead.
At 60 fps that is ~0.6 ms/frame of wasted CPU time.

**Plan:**
- Add a **minimum batch size** heuristic: only use `par_iter` if
  `blocks_info.len() > 4`. For small message sets, use sequential iteration.
- Cache `BlockComputation` results in a `HashMap<MessageId, ComputedBlock>`
  so messages whose content hash hasn't changed skip recomputation entirely.

### 2.4 — Full markdown re-parse on every frame

**Location:** `src/tui/render/chat_render/content.rs:306-497`
(`compute_block_data`) → `src/markdown_render/mod.rs:39-52`
(`render_markdown_text_with_width_and_cwd`)

Every frame re-parses the full markdown text (`pulldown_cmark`) and then
re-highlights code blocks (`syntect`). For a long assistant response with a
100-line code block, this takes 500 µs–2 ms per block.

**Plan:**
- **Content-hash based cache.** Cache keyed by `(blake3::hash(content),
  body_width)`. On render, skip re-parsing if the hash matches the current
  content and width.
- **Incremental streaming render.** During a Delta stream, only re-render the
  incremental suffix. Pre-parse once, then on each Delta, patch the last
  `RenderedBlock` by locating the truncation point in the parsed event stream.
- **Syntax-highlight cache.** Key = `(file_extension, code_text_hash)`. Reuse
  highlighted lines for identical code blocks across the same session.

> **⚠️ Dynamic table sizing concern (terminal resize) — fully preserved.**
>
> Table column widths are computed dynamically at render time in
> `src/markdown_render/table.rs:71-110` (`TableState::render(wrap_width)`):
>
> 1. `Line 77-78` — `available_width = wrap_width - prefix_width`
> 2. `Line 101` — `measure_column_widths()` scans all cell content at the
>    current `available_width` to produce per-column widths.
> 3. `Line 102-110` — if terminal is too narrow, the renderer falls back
>    to a **stacked rows layout** (each cell on its own line).
>
> Because the cache key **includes `body_width`** (derived from the current
> terminal width), a terminal resize triggers a cache miss and the table is
> fully re-parsed and laid out at the new width. This is exactly what happens
> today with the uncapped re-render — no regression.
>
> Similarly, word wrapping (`src/markdown_render/wrap.rs` `RtOptions.width`)
> is driven by `body_width`, which is part of the cache key. When the user
> resizes the terminal, wrapped content reflows correctly.
>
> The optimization only skips re-computation when **both content AND width
> are unchanged** — which is the common case between frames during streaming.

---

## 3. Text Rendering & Allocation Patterns

### 3.1 — Scrollbar allocation per frame

**Location:** `src/tui/render/render.rs:12-56` (`render_scrollbar`)

Allocates a `Vec<Line>` with one `Line` per terminal row (40–60 rows), each
containing a `Span`. At 60 fps this is ~3 000 allocations/second.

**Plan:**
- Pre-render the scrollbar as a `Vec<Line>` in `App`. Only recompute it when
  `(scroll, content_height, height)` changes.
- Use a `Cell<(usize, usize, usize, Vec<Line>)>` for the cached scrollbar.

### 3.2 — `decorate_card_lines` clones every span

**Location:** `src/tui/render/render.rs:740-758` (`decorate_card_lines`)

Clones the `Line`'s `.spans` and patches each `Span`'s background style.
Creates O(lines × spans) new `Span` allocations per frame.

**Plan:**
- Pre-compute decorated card lines at block-computation time (when the cache
  is populated), not during final rendering.
- Store `decorated_lines: Vec<Line<'static>>` in the render cache value so
  they are reused on cache hits.

### 3.3 — `shorten_single_line` double allocation

**Location:** `src/tui/render/render.rs:767-770` (`shorten_single_line`)

```rust
let single_line = value.replace('\n', " ").replace('\r', "");
```

This allocates **two temporary `String`** values per call. Called hundreds
of times per frame across tool-result rendering and card previews.

**Plan:**
- Single-pass iteration:
  ```rust
  fn shorten_single_line(value: &str, max_chars: usize) -> String {
      let single: String = value.chars()
          .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
          .collect();
      shorten(&single, max_chars)
  }
  ```

### 3.4 — Color mixing per line frame

**Location:** `src/tui/render/chat_render/utils.rs:12-100`
(`render_reasoning_markdown_lines`)

`theme::mix_colors()` is called for each line (once for the label style and
once per span for the background blend). For a 50-line reasoning block this
is 100+ RGBA mix calls per frame.

**Plan:**
- Pre-compute the two blended colors (`dimmed_color`, `body_dimmed_color`)
  once in `RenderContext`. Use them as cached values throughout the render
  function.
- Move the span-blending loop into the cache computation so it's only done
  when the content actually changes.

---

## 4. Agent Runtime Loop

### 4.1 — Retry clones every request

**Location:** `src/llm/mod.rs:97-108` (`stream_chat_with_retry`)

```rust
messages.clone(),
tools.clone(),
```

On each retry attempt the full message list and tool definition list is
deep-cloned. For sessions with 50+ messages and file attachments this is
10 KB + per clone. With 3 retries = 3× allocation.

**Plan:**
- Use `Arc<[Message]>` for messages and `Arc<[ToolDefinition]>` for tools.
  Clone is a cheap ref-count bump. Only materialize owned copies if a retry
  is actually needed.
- Alternatively, move the clone into the retry branch only (after the error
  is deemed retryable).

### 4.2 — `build_request_messages` O(n) per request

**Location:** `src/context.rs:128-260` (`build_request_messages`)

Traverses all visible messages, tracking orphaned tool calls via a
`HashMap`. Each `MessageRole::Assistant` check and tool-call mapping
insertion is O(1), but the full scan is still O(n) per LLM request.

For a 200-message conversation this is negligible (~2 µs), but the
method **allocates a new `Vec<Message>`** each time, deep-cloning each
message's `content` and `reasoning` fields.

**Plan:**
- Consider **reference-based message building**:
  `fn build_request_messages(&self) -> Vec<Cow<'_, Message>>` so the
  provider layer can decide which messages to own vs. reference.
- Alternatively, reuse a `Vec<Message>` buffer that's cleared and filled
  in-place (with careful capacity management to avoid reallocation).

### 4.3 — System prompt composed every turn

**Location:** `src/agent/runtime.rs:141-185` (`compose_system_prompt`)

Composes the system prompt from: base prompt, mode reminder, instruction
files (read from disk), workspace boundary hints, and tool descriptions.
Even though the result is assigned to a model clone (so the original stays
intact for prefix caching), the composition itself does disk I/O for
instruction files on every turn.

**Plan:**
- **System prompt cache** keyed by `(session_id, mode, agent_type,
  config_etag)`. Bump the etag when config files or AGENTS.md change.
- Instruction files can be cached in-memory with a file-watcher-based
  invalidation (`notify` crate).

### 4.4 — Context compaction blocks the agent loop

**Location:** `src/context.rs:276-...` (`compact`)

```rust
pub async fn compact(&mut self, llm: &LlmClient, ...) -> Result<bool>
```

Compaction makes a synchronous-looking LLM call (`complete_with_messages`)
that takes 1–5 seconds. During this time `run_agent_loop` is blocked
and cannot process new user messages.

The TUI tracks compacting sessions via `compacting_sessions: HashSet<Uuid>`
(`src/tui/mod.rs:153`) and skips starting new agent loops while compacting.

**Plan:**
- **Spawn compaction as a separate background task.** The agent loop yields
  control, the task runs on the background runtime, and when it finishes it
  sends `BackendEvent::ContextCompacted` to wake the UI.
- While compaction is in-flight, new user messages are queued (already the
  case via `queued_messages`) and processed once compaction completes.
- Consider a **read-only minimal loop** that can serve tool-call-free
  assistant turns while compaction runs on the rest of the context.

### 4.5 — Tool execution is sequential

**Location:** `src/agent/runtime.rs:497-730` (`execute_tool_calls`)

Tools are executed in a loop, one after another. For independent tools
(e.g., two `read` calls, or a `read` + `grep`), this wastes parallelism.

**Plan:**
- Add a `parallel_safe: bool` field to `ToolDefinition`. Tools that are
  read-only and don't produce side-effects can be executed concurrently
  via `tokio::join!` or `futures::future::join_all`.
- Default `read`, `list`, `grep`, `glob` to `parallel_safe = true`.

---

## 5. Storage Layer

### 5.1 — `append_message` single-row insert per call

**Location:** `src/storage/mod.rs:440-489` (`append_message`)

Each tool call result and each assistant message triggers a separate SQL
`INSERT`. In a tool-heavy turn (10+ tools), this is 10+ individual commits
with zstd compression + JSON serialization each.

**Plan:**
- Introduce `append_messages(&self, session_id, messages: &[Message])`
  that wraps all inserts in a single transaction:
  ```sql
  BEGIN;
  INSERT INTO messages ... VALUES (...);
  INSERT INTO messages ... VALUES (...);
  ...
  COMMIT;
  ```
- Use `rusqlite::Transaction` to auto-rollback on failure.

### 5.2 — Eager decompression of all messages

**Location:** `src/storage/mod.rs:976-1020` (`load_messages`)

Each row is decompressed (zstd) and JSON-deserialized immediately. For a
session with 200 messages and ~300 KB of compressed text, this means
300 KB of decompression + 2 MB+ of `Message` allocations on session load.

**Plan:**
- **Lazy message loading.** Store the compressed blob alongside an index
  (message ID → byte offset). Messages are decompressed on-demand when
  first accessed (by ID or by range).
- For the TUI, only the visible window of ~20 messages needs to be fully
  deserialized at any time. Background messages stay compressed.
- Introduce a `LazyMessage` type that wraps `(session_id, message_id)`
  and decompresses on first `.content()` / `.reasoning()` call.

### 5.3 — `read_blob_maybe_text` type-inference overhead

**Location:** `src/storage/mod.rs:1948-1957` (`read_blob_maybe_text`)

```rust
match row.get::<_, Vec<u8>>(idx) {
    Ok(bytes) => Ok(decompress_text(&bytes)),
    Err(_) => { /* fallback: read as TEXT */ },
}
```

Every column read tries `Vec<u8>` first, then falls back to `String`.
After a schema migration (`TEXT → BLOB`) all rows are BLOB, so the
fallback is dead code — but it still adds a branch that the CPU must
speculate.

**Plan:**
- Run a one-time migration that converts any remaining TEXT rows to BLOB.
- Remove the fallback path once all databases are migrated.
- Add a `schema_version` check at `SessionStore::open` time and skip
  the fallback for version ≥ 2.

### 5.4 — `Clone` for `SessionStore` re-opens connections

**Location:** `src/storage/mod.rs:36-41` (`Clone` impl)

```rust
impl Clone for SessionStore {
    fn clone(&self) -> Self {
        Self::open(&self.path).expect("...")  // re-opens SQLite file!
    }
}
```

Every clone (used when passing the store between `AgentRuntime` and TUI)
opens a **new pair of SQLite connections**. This means wasted file
descriptors and repeated WAL setup.

**Plan:**
- Use `Arc<SessionStore>` instead of `Clone`. SQLite in WAL mode supports
  concurrent readers sharing the same connection pool.
- Add a reference-counted connection pool instead of cloning connections.

---

## 6. Tool Execution

### 6.1 — `FileReadTracker` mutex contention

**Location:** `src/tooling/file_read_tracker.rs` (underlying data structure)

`FileReadTracker` uses `Arc<StdMutex<HashMap>>`. The render thread reads
file-read stamps while the agent runtime writes them. Under heavy tool
activity, this causes spin-wait contention.

**Plan:**
- Replace `StdMutex<HashMap>` with `dashmap::DashMap` for lock-free
  concurrent reads and fine-grained sharded writes.

### 6.2 — `canonical_tool_name` called repeatedly

**Location:** `src/tui/render/chat_render/tool.rs:34-46`,
`src/tui/render/chat_render/content.rs`, `src/tooling/`

`canonical_tool_name` is called on every tool render, sometimes multiple
times for the same tool call. It does a string lookup in a static slice.

**Plan:**
- Use `once_cell::sync::LazyLock<HashMap<&'static str, &'static str>>`
  for the canonical name mapping.
- Or add a `canonical_name: &'static str` field directly to `ToolCall`
  so it's resolved once on construction.

### 6.3 — Workspace path resolution without cache

**Location:** `src/tooling/builtin/file.rs:281-295` (`read_path`),
`src/tooling/builtin/utils.rs` (`resolve_workspace_path`)

Each file tool resolves the workspace-relative path via `Path::join` +
`canonicalize` + existence check. Repeated for every `read`, `write`,
`edit` call — even if the same file is accessed multiple times.

**Plan:**
- Add a **path resolution cache** — `SimpleLruCache<PathBuf, PathBuf, 256>`.
- Invalidate entries when files are created/deleted (via `notify` events
  or when `write`/`edit` tools write to a cached path).
- Skip `canonicalize` when the path is already absolute and exists.

### 6.4 — bash tool spawns a process per call

**Location:** `src/tooling/builtin/exec.rs:89-308` (`run_shell_inner`)

Each bash invocation spawns a new shell process. For rapid tool-use
patterns (e.g., "run test, see output, edit, run test"), this adds
~10 ms per spawn just for process setup.

**Plan:**
- Consider a **long-lived shell session** (`portable-pty` already a
  dependency) for quick commands. Reuse a persistent PTY for multiple
  bash calls within the same turn.
- Only spawn a fresh process for long-running or isolated commands.

---

## 7. Event Dispatch & Notifications

### 7.1 — Channel saturation during high-output tools

**Location:** `src/agent/runtime.rs:970-1040` (main streaming event loop),
`src/tui/mod.rs:580-1220` (`handle_backend_event`)

`BackendEvent::Delta` events are sent for every LLM content chunk. During
bash output streaming, `ShellOutput` events also flood the channel. The
TUI main loop consumes via `try_recv()` in a loop — if the event loop is
blocked on rendering, the channel can grow to 1000+ events.

**Plan:**
- **Event coalescing.** For `Delta` events, if the same `(session_id,
  request_id, message_id)` already has a pending event in the channel,
  merge the content strings rather than pushing a new event.
- **Batched consumption.** In the main loop, drain the channel in batches
  of up to 50 events, then render once.
- Use `tokio::sync::mpsc::channel` (bounded) instead of `unbounded_channel`
  to apply backpressure. Drop stale events when the channel is full.

### 7.2 — Full-frame pull rendering

**Location:** `src/tui/render/render.rs:67` (`App::render`), called from
`src/tui/core/run.rs` event loop.

The render method always rebuilds the entire UI tree. ratatui's
`Terminal::draw()` uses a `Diff` backend to minimise terminal escape codes,
but the computation cost of building the full tree is still incurred.

**Plan:**
- **Dirty-region tracking.** Track which panel(s) actually changed since
  last frame. Only render dirty panels.
- For streaming, only the message viewport and scrollbar need updates;
  the sidebar, composer, and panels remain static.

---

## 8. Sub-Agent / Delegation

### 8.1 — HashMap lookups on every delegation

**Location:** `src/delegate/mod.rs:62-76` (`check_depth`),
`src/delegate/mod.rs:81-102` (`track_task`)

Every delegation checks depth and pending-task limits via `HashMap::get`.
The maps are small, so this is negligible — but the **depth_cache is
never pruned**. Long-running sessions accumulate stale entries.

**Plan:**
- Add periodic pruning of `depth_cache` (e.g., remove entries for sessions
  that have completed).
- Use `LruCache` for `depth_cache` to bound memory growth.

### 8.2 — Sub-agent sessions are full-weight SQLite sessions

**Location:** `src/agent/runtime.rs:760-870` (sub-agent creation)

Each sub-agent delegation creates a new session in the SQLite database with
full metadata persistence. Many sub-agents are transient (<30 s lifetime)
and are discarded without ever persisting.

**Plan:**
- **Lightweight in-memory sub-agent storage.** Sub-agent messages stay
  in a `Vec<Message>` behind `Arc<RwLock<>>`. Only persist to SQLite if
  the user explicitly forks or the sub-agent result is referenced later.
- Reduces I/O from O(N-sub-agents) to O(1) for ephemeral tasks.

---

## 9. Priority Summary

| Priority | ID | Area | Bottleneck | Impact | Effort |
|----------|----|------|------------|--------|--------|
| **P0** | 2.1 | Render | Coarse cache invalidation bins rendering on every Delta | 60→120 fps, smooth streaming | 2–3 d |
| **P0** | 2.4 | Render | Full markdown re-parse every frame | 0.5–2 ms saved per frame | 3–5 d |
| **P1** | 7.1 | Events | Channel saturation during tool output | Stable UI under load | 1–2 d |
| **P1** | 4.4 | Agent | Compaction blocks agent loop | Unblocks user input | 3 d |
| **P1** | 5.2 | Storage | Eager decompression of all messages | Faster session load, less RAM | 2 d |
| **P2** | 3.1–3.4 | Render | Allocation patterns (scrollbar, cards, color mixing) | ~1 ms/frame reduction × N items | 2 d |
| **P2** | 5.1 | Storage | Single-row inserts per tool result | Batch writes: 10× fewer commits | 1 d |
| **P2** | 6.1 | Tooling | `FileReadTracker` mutex contention | Smoother concurrent read/write | 0.5 d |
| **P3** | 4.1 | Agent | LLM retry clones expensive | Modest memory savings | 0.5 d |
| **P3** | 6.3 | Tooling | Path resolution without cache | Faster repeated file access | 1 d |
| **P3** | 8.2 | Delegate | Sub-agent full SQLite persistence | Reduced I/O for ephemeral tasks | 1 d |
| **P3** | 5.3 | Storage | `read_blob_maybe_text` fallback path | Marginal CPU win post-migration | 0.5 d |

### Legend

- **P0** — Directly observable frame drops/stutter on every interaction.
- **P1** — Measurable latency under common workloads (long sessions, tool output).
- **P2** — Optimization of hot paths; noticeable under sustained use.
- **P3** — Cleanup / hardening; small cumulative benefit.

### Recommended implementation order

1. **P0 items first** — they affect every user on every frame.
2. **P1 storage + events** — compound impact as sessions grow long.
3. **P2 allocation patterns** — incremental wins on the rendering hot path.
4. **P3 hardening** — when the dust settles.
