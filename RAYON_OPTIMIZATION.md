# Rayon Parallel Optimization Analysis

This document analyzes potential opportunities for parallel optimization using the `rayon` crate in the TiDev codebase.

## Current Usage

### Already Parallelized

| Location | Description |
|----------|-------------|
| `src/app/render/render_chat.rs:847` | Tool call rendering with `par_iter()` for parallel card generation |

```rust
let tool_results_map = render_units
    .par_iter()
    .map(|unit| {
        // Parallel tool call rendering
    })
    .collect();
```

---

## Potential Optimization Opportunities

### Priority 1: High Impact

#### 1. @-mention File Search Scoring

**File:** `src/app/input/at_mention.rs:433-439`

**Current Implementation:**
```rust
let mut ranked = indexed_entries
    .iter()
    .filter_map(|entry| {
        score_entry(entry, query)
            .map(|candidate| (candidate.score, entry, candidate.matched_indices))
    })
    .collect::<Vec<_>>();
```

**Proposed Change:**
```rust
use rayon::prelude::*;

let mut ranked: Vec<_> = indexed_entries
    .par_iter()
    .filter_map(|entry| {
        score_entry(entry, query)
            .map(|candidate| (candidate.score, entry, candidate.matched_indices))
    })
    .collect();
```

**Impact:**
- Large workspaces can have 10,000+ indexed entries
- Each `score_entry` performs multiple string operations (exact match, prefix, contains, subsequence)
- Estimated speedup: 2-4x on 4-core systems

**Complexity:** Low
**Risk:** Low (pure computation, no shared mutable state)

---

#### 2. Grep Tool Parallel File Search

**File:** `src/tooling/builtin/search.rs:224-261`

**Current Implementation:**
```rust
for path in files {
    // Filter by include pattern
    // Search file content
    // Collect matches
    if searcher.search_path(matcher.clone(), &path, sink).is_err() {
        skipped += 1;
        continue;
    }
    matches.extend(file_hits);
}
```

**Proposed Change:**
```rust
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

let skipped = AtomicUsize::new(0);
let matches: Vec<SearchHit> = files
    .into_par_iter()
    .filter_map(|path| {
        // Filter by include pattern
        if let Some(include_matcher) = &include_matcher {
            // ... filter logic
        }

        let mut file_hits = Vec::new();
        // Note: Searcher is not thread-safe, need to create per-thread
        let searcher = SearcherBuilder::new().line_number(true).build();
        let sink = sinks::Lossy(|line_number, line| {
            file_hits.push(SearchHit { ... });
            Ok(true)
        });

        if searcher.search_path(matcher.clone(), &path, sink).is_err() {
            skipped.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        Some(file_hits)
    })
    .flatten()
    .collect();
```

**Impact:**
- Grep across large codebases can be 3-8x faster
- Most time spent in I/O and regex matching, both benefit from parallelism

**Complexity:** Medium
**Risk:** Medium
- `grep::Searcher` is not thread-safe, must create per-thread
- Need to handle `AtomicUsize` for skipped count

---

#### 3. Glob Tool Parallel File Matching

**File:** `src/tooling/builtin/search.rs:82-112`

**Current Implementation:**
```rust
for result in WalkBuilder::new(&search_root).build() {
    // Process each entry sequentially
    if glob_matches_path(&path, &search_root, &matcher, pattern) {
        matches.push(SearchHit::from_path(&path)?);
    }
}
```

**Proposed Change:**
```rust
use rayon::prelude::*;

let files: Vec<_> = WalkBuilder::new(&search_root)
    .build()
    .par_bridge()
    .filter_map(|result| {
        let entry = result.ok()?;
        if !entry.file_type()?.is_file() {
            return None;
        }
        let path = entry.into_path();
        if glob_matches_path(&path, &search_root, &matcher, pattern) {
            SearchHit::from_path(&path).ok()
        } else {
            None
        }
    })
    .collect();
```

**Impact:**
- Glob search in large directories benefits from parallel filesystem traversal
- `par_bridge()` enables parallel iteration over sequential iterator

**Complexity:** Low
**Risk:** Low

---

### Priority 2: Medium Impact

#### 4. Token Estimation for Messages

**File:** `src/context.rs:79-81`

**Current Implementation:**
```rust
pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
    messages.iter().map(Self::message_tokens).sum()
}
```

**Proposed Change:**
```rust
use rayon::prelude::*;

pub fn estimate_tokens_for_messages(messages: &[Message]) -> usize {
    messages.par_iter().map(Self::message_tokens).sum()
}
```

**Impact:**
- Only beneficial for very long conversations (100+ messages)
- Typical conversations have < 50 messages, limited benefit

**Complexity:** Very Low
**Risk:** Very Low

**Recommendation:** Skip unless profiling shows this as bottleneck

---

#### 5. Code Highlighting Batch Processing

**File:** `src/markdown_render/highlight.rs`

**Current State:** Already has LRU cache for individual code blocks

**Potential Enhancement:**
If rendering multiple code blocks simultaneously (e.g., during initial message load), could parallelize:

```rust
fn highlight_multiple_blocks(blocks: &[(String, String)]) -> Vec<Vec<Line<'static>>> {
    blocks
        .par_iter()
        .map(|(code, lang)| highlight_code_to_lines(code, lang))
        .collect()
}
```

**Impact:**
- Only useful during bulk message loading
- Cache already handles most cases

**Complexity:** Medium
**Risk:** Low (cache is thread-safe with RwLock)

**Recommendation:** Profile first, may not be needed

---

### Priority 3: Low Impact / Not Recommended

#### 6. Snapshot Git Operations

**File:** `src/snapshot/git.rs`

**Analysis:**
- Git commands are inherently serial (spawn processes)
- Most operations already minimal (single git command)
- `find_changed_files`, `stage_files` etc. run single git process

**Recommendation:** Not suitable for rayon parallelization

---

#### 7. Storage Layer

**File:** `src/storage/mod.rs`

**Analysis:**
- SQLite operations should be single-threaded for consistency
- Batch operations already optimized with single SQL statements
- Connection is not thread-safe

**Recommendation:** Not suitable for rayon parallelization

---

#### 8. Markdown Rendering

**File:** `src/markdown_render/mod.rs`

**Analysis:**
- Single markdown text processed sequentially
- Parser state is stateful (indent stack, link state, etc.)
- Would require significant refactoring

**Recommendation:** Not worth the complexity

---

## Implementation Priority

| Priority | Feature | Complexity | Impact | Risk |
|----------|---------|------------|--------|------|
| 1 | @-mention scoring | Low | High | Low |
| 2 | Grep tool | Medium | High | Medium |
| 3 | Glob tool | Low | Medium | Low |
| 4 | Token estimation | Very Low | Low | Very Low |

---

## General Guidelines for Rayon Usage

### When to Use Parallel Iterators

1. **Sufficient work per item** - Each iteration should do meaningful work
2. **Large collection** - 100+ items typically needed for benefit
3. **No shared mutable state** - Or use thread-safe primitives
4. **CPU-bound work** - I/O bound may not benefit as much

### Thread Safety Considerations

```rust
// BAD: Shared mutable state
let mut counter = 0;
items.par_iter().for_each(|item| {
    counter += 1; // Data race!
});

// GOOD: Use atomic types
use std::sync::atomic::{AtomicUsize, Ordering};
let counter = AtomicUsize::new(0);
items.par_iter().for_each(|item| {
    counter.fetch_add(1, Ordering::Relaxed);
});

// GOOD: Use reduce/collect
let count = items.par_iter().count();
```

### Creating Thread-Local Resources

```rust
// BAD: Non-thread-safe type shared across threads
let searcher = SearcherBuilder::new().build();
items.par_iter().map(|item| {
    searcher.search(item) // searcher is not thread-safe
});

// GOOD: Create per-thread
items.par_iter().map(|item| {
    let searcher = SearcherBuilder::new().build(); // One per thread
    searcher.search(item)
});
```

---

## Benchmarking Strategy

Before and after implementing any parallel optimization:

1. Use `criterion` crate for micro-benchmarks
2. Test with realistic data sizes:
   - Small: 10-100 items
   - Medium: 100-1,000 items
   - Large: 1,000-10,000+ items
3. Measure on different core counts
4. Profile with `perf` or `instruments`

```rust
#[cfg(test)]
mod benches {
    use super::*;
    use std::time::Instant;

    #[test]
    fn benchmark_search_scoring() {
        let entries: Vec<IndexedEntry> = /* generate 10,000 entries */;
        let query = "test";

        let start = Instant::now();
        let results = rank_indexed_entries(&entries, query, 0);
        let serial_duration = start.elapsed();

        // Compare with parallel version
        let start = Instant::now();
        let results = rank_indexed_entries_parallel(&entries, query, 0);
        let parallel_duration = start.elapsed();

        println!("Serial: {:?}", serial_duration);
        println!("Parallel: {:?}", parallel_duration);
    }
}
```

---

## Summary

The most impactful rayon optimizations for TiDev are:

1. **@-mention scoring** - Easy win, high impact for large workspaces
2. **Grep tool** - Medium complexity, high impact for code search
3. **Glob tool** - Low complexity, medium impact

The existing tool call rendering parallelization in `render_chat.rs` is a good pattern to follow for additional optimizations.
