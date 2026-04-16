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

## Optimization Proposals

### 1. Viewport Virtualization

**Problem:** Currently renders all messages even when off-screen.

**Solution:** Only render messages within the visible viewport plus a small buffer.

**Implementation:**

```rust
// In messages_text(), calculate visible range
fn calculate_visible_message_range(
    messages: &[Message],
    scroll_offset: usize,
    viewport_height: usize,
) -> Range<usize> {
    // Binary search to find first visible message
    // Include buffer of 2-3 messages above/below for smooth scrolling
}
```

**Key Changes:**
- Add `message_line_offsets: Vec<usize>` to track each message's starting line
- Use binary search to find visible range
- Render only visible messages

**Complexity:** Medium  
**Expected Impact:** 50-90% reduction for long conversations

---

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

### 4. Code Highlighting Cache

**Problem:** Syntect parsing is CPU-intensive, repeated for same code.

**Solution:** Cache highlighted lines by content hash.

**Implementation:**

```rust
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

static HIGHLIGHT_CACHE: Lazy<RwLock<LruCache<u64, Vec<Line<'static>>>>> = 
    Lazy::new(|| RwLock::new(LruCache::new(NonZeroUsize::new(100).unwrap())));

fn highlight_code_to_lines_cached(code: &str, lang: &str) -> Vec<Line<'static>> {
    let hash = compute_content_hash(code, lang);
    if let Some(cached) = HIGHLIGHT_CACHE.read().get(&hash) {
        return cached.clone();
    }
    let lines = highlight_code_to_lines(code, lang);
    HIGHLIGHT_CACHE.write().put(hash, lines.clone());
    lines
}
```

**Key Changes:**
- Add global LRU cache for highlighted code
- Use `xxhash` or `ahash` for fast hashing
- Limit cache size (e.g., 100 blocks)

**Complexity:** Low  
**Expected Impact:** Significant for code-heavy conversations

---

### 5. Parallel Rendering

**Problem:** Message rendering is single-threaded.

**Solution:** Use `rayon` for parallel message rendering.

**Implementation:**

```rust
use rayon::prelude::*;

fn render_all_messages(&self, messages: &[Message], width: usize) -> Vec<Line<'static>> {
    messages
        .par_iter()
        .flat_map(|message| self.render_message_cards(message, width))
        .collect()
}
```

**Key Changes:**
- Add `rayon` dependency
- Parallelize `messages_text()` loop
- Ensure thread-safety (palette clone, etc.)

**Complexity:** Low  
**Expected Impact:** 2-4x speedup on multi-core systems

**Caveats:**
- ratatui `Line<'static>` is thread-safe, but styles must be `Copy`
- Need to profile to ensure overhead doesn't dominate

---

## Recommended Implementation Order

| Priority | Optimization | Rationale |
|----------|--------------|-----------|
| 1 | Code Highlighting Cache | Low effort, high impact, isolated change |
| 2 | Viewport Virtualization | High impact for long conversations |
| 3 | Markdown AST Caching | Mitigates resize penalty |
| 4 | Parallel Rendering | Good ROI, but needs profiling |
| 5 | Incremental Rendering | Highest complexity, save for later |

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
