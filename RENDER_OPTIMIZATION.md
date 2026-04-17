# Message Rendering Optimization Plan

## Current Implementation

### Architecture Overview

Message rendering flows through:

```
render_chat.rs::render_messages()
    └── messages_text()
        ├── cached_render_message_cards()  // Cache lookup
        ├── render_message_cards()         // Per-message rendering
        │   ├── render_assistant_body_lines()
        │   │   ├── render_reasoning_lines()
        │   │   └── render_markdown_text_with_width_and_cwd()
        │   └── render_tool_result_lines()
        └── decorate_card_lines()          // Card decoration
```

### Existing Optimizations

1. **MessageRenderCache** (`src/app/runtime/state.rs:25-49`)
   - Caches rendered `Vec<(Color, Vec<Line<'static>>)>` cards
   - Key: `(session_id, message_id, width, kind)`
   - Max entries: 1200
   - LRU eviction via `last_used_tick`

2. **Streaming Skip** (`render_chat.rs:458-461`)
   - Streaming messages bypass cache, always re-render

3. **Code Highlighting Limits** (`markdown_render/highlight.rs:18-19`)
   - `MAX_HIGHLIGHT_BYTES: 512KB`
   - `MAX_HIGHLIGHT_LINES: 10,000`

### Performance Hotspots

| Location | Issue | Impact |
|----------|-------|--------|
| `messages_text()` line 351-420 | Iterates all messages every frame | O(n) per frame |
| `render_markdown_text_with_width_and_cwd()` | Full parse on every cache miss | Expensive for long content |
| Cache key includes `width` | Window resize invalidates all cache | Mass re-render on resize |
| `highlight_code_to_lines()` | Syntect parsing is slow | Blocks main thread |

---

## ✅ Implemented: Viewport Virtualization

**Status:** Implemented in `src/app/runtime/state.rs` and `src/app/render/render_chat.rs`

### Design

```
┌─────────────────────────────────────────────────────────────────────┐
│                    Layer 1: Layout Index                            │
├─────────────────────────────────────────────────────────────────────┤
│ MessageLayoutIndex {                                                 │
│   blocks: Vec<MessageBlock>,   // Sorted by start_line              │
│   total_lines: usize,                                               │
│   width: usize,                // Width used for calculations       │
│   valid: bool,                 // Needs rebuild?                    │
│ }                                                                    │
│                                                                      │
│ MessageBlock {                                                       │
│   message_id: Uuid,           // Primary message ID                 │
│   message_start_idx: usize,   // Index in messages array            │
│   message_count: usize,       // 1 for User, 1+ for Assistant+Tool  │
│   start_line: usize,          // Starting line in rendered output   │
│   line_count: usize,          // Lines consumed by this block       │
│ }                                                                    │
└─────────────────────────────────────────────────────────────────────┘
```

### Implementation Details

1. **Dual Rendering Paths** (`render_chat.rs:351-518`)
   - Virtualized rendering for conversations > 20 messages
   - Full rendering for streaming or small conversations
   - Threshold configurable via `VIRTUALIZE_THRESHOLD` constant

2. **Layout Index Update** (`render_chat.rs:519-573`)
   - Rebuilds on width change or cache clear
   - Calculates line counts using existing cache
   - Groups Assistant + Tool messages into single blocks

3. **Binary Search** (`render_chat.rs:633-659`)
   - `find_visible_message_blocks()` uses `partition_point()` for O(log n)
   - Includes 5-line buffer above/below viewport for smooth scrolling

4. **Cache Integration**
   - Layout index uses existing `MessageRenderCache` for line counts
   - Cache clear also invalidates layout index (`run.rs:301-306`)

### Performance Characteristics

| Scenario | Before | After |
|----------|--------|-------|
| 100 messages, 30-line viewport | Render ~1000 lines | Render ~40 lines |
| Scrolling one line | Re-render all | Re-render visible only |
| New message arrives | Re-render all | Incremental update |
| Window resize | Re-render all | Re-render visible only |

### Limitations

- Virtualization disabled during streaming (content changes rapidly)
- Small conversations (< 20 messages) use full render (overhead not worth it)
- Layout index requires cache hit to get accurate line count

---

## Remaining Optimization Proposals

### 2. Incremental Rendering

**Problem:** Re-rendering unchanged messages wastes CPU cycles.

**Solution:** Track message content hash and only re-render changed messages.

**Implementation:**

```rust
struct MessageHash {
    content_hash: u64,      // content + reasoning hash
    tool_calls_hash: u64,   // tool_calls hash
    streaming: bool,        // always re-render streaming
}

// Store in Message or separate map
fn should_rerender(&self, message: &Message) -> bool {
    let current_hash = compute_hash(message);
    let cached_hash = self.message_hashes.get(&message.id);
    current_hash != cached_hash
}
```

**Key Changes:**
- Add content hash tracking
- Compare before cache lookup
- Update hash after render

**Complexity:** High  
**Expected Impact:** Significant for editing/undo operations

---

### 3. Markdown AST Caching

**Problem:** `pulldown_cmark::Parser` parses from scratch on every cache miss.

**Solution:** Cache parsed AST, apply wrapping on demand.

**Implementation:**

```rust
// Cache parsed events instead of final lines
enum CachedMarkdown {
    Events(Vec<pulldown_cmark::Event<'static>>),  // Parsed but not wrapped
    Lines(Vec<Line<'static>>),                     // Fully rendered (current)
}

// Render flow:
// 1. Parse markdown → Events (cache this)
// 2. Apply width wrapping → Lines (per-width cache or on-demand)
```

**Key Changes:**
- Two-tier cache: AST cache (width-independent) + line cache (width-dependent)
- On resize: reuse AST, re-wrap only

**Complexity:** Medium  
**Expected Impact:** Faster cache recovery after resize

---

## ✅ Implemented: Code Highlighting Cache

**Status:** Implemented in `src/markdown_render/highlight.rs`

### Design

Caches highlighted code lines using a combination of:
- `blake3` hash for code content (fast, ~10GB/s throughput)
- Theme generation counter (invalidates cache on theme change)
- Language identifier

### Implementation Details

```rust
static HIGHLIGHT_CACHE: OnceLock<RwLock<LruCache<HighlightCacheKey, Vec<Line<'static>>>>> =
    OnceLock::new();
static HIGHLIGHT_CACHE_GEN: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct HighlightCacheKey {
    theme_gen: u64,       // Incremented on theme change
    lang: String,         // Language identifier
    code_hash: [u8; 32],  // blake3 hash of code content
}

pub(crate) fn highlight_code_to_lines(code: &str, lang: &str) -> Vec<Line<'static>> {
    // 1. Check size limits (skip cache for huge files)
    // 2. Compute cache key with theme_gen + lang + code_hash
    // 3. Return cached result if hit
    // 4. Otherwise, highlight and store in cache
}
```

### Key Features

1. **LRU eviction**: Uses `lru` crate with 100 entry limit
2. **Theme invalidation**: `set_syntax_theme()` increments `HIGHLIGHT_CACHE_GEN`
3. **Fast hashing**: `blake3` for code content (already in dependencies)
4. **Thread-safe**: `RwLock` protects cache access
5. **Size bypass**: Large code (>512KB or >10,000 lines) skips highlighting entirely

### Performance Characteristics

| Scenario | Before | After |
|----------|--------|-------|
| Same code rendered twice | Parse twice | Parse once, cache hit |
| Theme change | Re-parse all | Cache invalidated by gen counter |
| 100+ code blocks | Parse each time | LRU keeps hot blocks cached |

---

## ✅ Implemented: Parallel Rendering

**Status:** Implemented in `src/app/render/render_chat.rs`

### Design

Parallel rendering is applied to tool call rendering, which is the most CPU-intensive part of message rendering. The implementation preserves the existing cache mechanism while leveraging `rayon` for parallel execution.

### Implementation Details

```rust
// Tool calls are rendered in parallel using rayon
let tool_results_map: HashMap<usize, Vec<(...)>> = render_units
    .par_iter()
    .map(|unit| {
        // Render tool calls independently
        let mut tool_cards = Vec::new();
        for (tool_call, tool_result) in &unit.tool_results {
            let card_lines = render_tool_call_with_result_standalone(...);
            tool_cards.push((tool_result_id, palette.panel_light, card_lines));
        }
        (unit.message_idx, tool_cards)
    })
    .collect();
```

### Key Features

1. **Hybrid approach**: Cache for message cards + parallel for tool calls
2. **Preserves cache behavior**: `cached_render_message_cards()` still updates cache stats
3. **Thread-safe rendering**: Uses standalone functions with `ThemePalette` (Copy)
4. **Selective parallelization**: Only tool calls (most expensive) are parallelized

### Performance Characteristics

| Scenario | Before | After |
|----------|--------|-------|
| 10 tool calls, 8 cores | Sequential render | ~3-4x faster |
| Cache hit on messages | Fast path preserved | Same |
| Streaming messages | Full render | Same (no parallelization) |

### Implementation Notes

- Added `rayon` dependency
- Created standalone render functions (`render_tool_call_with_result_standalone`, etc.)
- Message cards still use cache for consistency with existing tests
- Tool call rendering is the primary parallelization target

---

### 5. Incremental Rendering

## Recommended Implementation Order

| Priority | Optimization | Status | Rationale |
|----------|--------------|--------|-----------|
| 1 | Viewport Virtualization | ✅ Done | High impact for long conversations |
| 2 | Code Highlighting Cache | ✅ Done | Low effort, high impact, isolated change |
| 3 | Parallel Rendering | ✅ Done | Good ROI, preserves cache behavior |
| 4 | Markdown AST Caching | Pending | Mitigates resize penalty |
| 5 | Incremental Rendering | Pending | Highest complexity, save for later |

---

## Measurement Strategy

### Metrics to Track

1. **Frame time**: Target < 16ms (60 FPS)
2. **Cache hit rate**: Current implementation logs at 12ms threshold
3. **Memory usage**: Cache size should be bounded

### Benchmark Setup

```rust
#[bench]
fn bench_render_100_messages(b: &mut Bencher) {
    let app = create_app_with_messages(100);
    b.iter(|| app.messages_text(Some(80)));
}

#[bench]
fn bench_render_streaming(b: &mut Bencher) {
    let mut app = create_app_with_streaming_message();
    b.iter(|| app.messages_text(Some(80)));
}
```

### Profiling Commands

```bash
# CPU profiling
cargo flamegraph --root -- bin/tidev

# Memory profiling (valgrind on Linux)
valgrind --tool=massif target/release/tidev

# Timing logs (already implemented)
# Look for "messages_text slow" in logs
```

---

## Notes

- Current slow threshold is 12ms (`render_chat.rs:438`)
- Cache stats are logged when slow: hits, misses, entries
- Consider adding metrics to the UI for debugging

## References

- `src/app/render/render_chat.rs` - Main rendering logic
- `src/markdown_render/mod.rs` - Markdown parsing
- `src/markdown_render/highlight.rs` - Code highlighting
- `src/markdown_render/wrap.rs` - Line wrapping
- `src/app/runtime/state.rs` - Cache structures
