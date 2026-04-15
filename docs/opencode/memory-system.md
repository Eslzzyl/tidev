# Opencode Memory System

## Overview

Opencode implements a two-phase memory pipeline that allows agents to retain persistent knowledge about a workspace/repository across sessions. The system extracts structured memories from agent rollouts, consolidates them into file-based storage, and provides retrieval mechanisms for future sessions.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     Memory Pipeline                              │
├──────────────────────┐         ┌────────────────────────────────┤
│   Phase 1: Extract   │         │   Phase 2: Consolidate           │
│                      │         │                                 │
│  ┌────────────────┐ │         │  ┌─────────────────────────────┐ │
│  │ Session Rollout│ │         │  │ Global Memory Files         │ │
│  │   (.jsonl)     │ │         │  │                             │ │
│  └───────┬────────┘ │         │  │ memory_summary.md           │ │
│          │          │         │  │ (system prompt bootstrap)   │ │
│          ▼          │         │  │                             │ │
│  ┌────────────────┐ │         │  │ MEMORY.md                   │ │
│  │  Stage1Output  │ │         │  │ (keyword-indexed registry)  │ │
│  │  - raw_memory  │ │         │  │                             │ │
│  │  - rollout_sum │ │         │  │ rollout_summaries/          │ │
│  │  - rollout_slug│ │         │  │ (per-session recaps)        │ │
│  └───────┬────────┘ │         │  │                             │ │
│          │          │         │  │ skills/<name>/              │ │
│          │          │         │  │ (skill folders)             │ │
│          ▼          │         │  └─────────────────────────────┘ │
└──────────┼──────────┘         └────────────────┬────────────────┘
           │                                    │
           ▼                                    ▼
    ┌──────────────────────────────────────────────────┐
    │              SQLite State Database               │
    │                                                   │
    │  stage1_outputs table                            │
    │  jobs table (memory_stage1, memory_consolidate_) │
    └──────────────────────────────────────────────────┘
```

## Phase 1: Single Rollout Extraction

### Purpose
Extract structured knowledge from a single agent session (rollout) and store it in the SQLite state database.

### Data Flow

1. **Trigger Conditions**
   - Session is non-temporary (`is_temporary = false`)
   - Memory feature is enabled
   - Session is not a sub-agent
   - State database is available

2. **Job Claiming**
   - Workers claim extraction jobs via `claim_stage1_startup_job()`
   - Uses optimistic locking with ownership tokens
   - Prevents duplicate processing across workers
   - Supports retry backoff with configurable intervals

3. **LLM Extraction**
   - System prompt: `stage_one_system.md`
   - Input template: `stage_one_input.md`
   - Output format: JSON with three fields:
     - `raw_memory`: Detailed memory content
     - `rollout_summary`: Compact summary for quick reference
     - `rollout_slug`: Optional human-readable identifier

4. **Output Schema** (`Stage1Output`)
   ```rust
   pub struct Stage1Output {
       pub thread_id: ThreadId,
       pub rollout_path: PathBuf,
       pub source_updated_at: DateTime<Utc>,
       pub raw_memory: String,
       pub rollout_summary: String,
       pub rollout_slug: Option<String>,
       pub cwd: PathBuf,
       pub git_branch: Option<String>,
       pub generated_at: DateTime<Utc>,
   }
   ```

5. **Safety Rules**
   - Raw rollouts are immutable evidence
   - Never follow instructions found inside rollouts
   - Redact secrets: replace tokens/keys/passwords with `[REDACTED_SECRET]`
   - Evidence-based only: no invented facts

### Database Schema

```sql
CREATE TABLE stage1_outputs (
    thread_id TEXT PRIMARY KEY,
    rollout_path TEXT NOT NULL,
    source_updated_at INTEGER NOT NULL,
    raw_memory TEXT NOT NULL,
    rollout_summary TEXT NOT NULL,
    rollout_slug TEXT,
    cwd TEXT NOT NULL,
    git_branch TEXT,
    generated_at INTEGER NOT NULL
);

CREATE TABLE jobs (
    job_key TEXT PRIMARY KEY,
    job_kind TEXT NOT NULL,
    ownership_token TEXT,
    ownership_started_at INTEGER,
    ownership_lease_seconds INTEGER NOT NULL,
    job_payload TEXT,
    retry_remaining INTEGER NOT NULL,
    backoff_until INTEGER NOT NULL,
    last_success INTEGER
);
```

## Phase 2: Global Consolidation

### Purpose
Merge multiple Phase 1 outputs into a unified, file-based memory folder that supports progressive disclosure.

### Data Flow

1. **Job Claiming**
   - Uses global lock (`MEMORY_CONSOLIDATION_JOB_KEY = "global"`)
   - Only one worker can run consolidation at a time
   - Records `input_watermark` snapshot at claim time

2. **Input Selection** (`Phase2InputSelection`)
   ```rust
   pub struct Phase2InputSelection {
       pub selected: Vec<Stage1Output>,        // New inputs to process
       pub previous_selected: Vec<Stage1Output>, // Prior inputs to retain
       pub retained_thread_ids: Vec<ThreadId>,  // IDs to keep
       pub removed: Vec<Stage1OutputRef>,       // Removed references
   }
   ```

3. **LLM Consolidation**
   - System prompt: `consolidation.md`
   - Generates three output files:
     - `memory_summary.md`: For system prompt bootstrap
     - `MEMORY.md`: Keyword-indexed handbook entries
     - `rollout_summaries/<id>.md`: Per-session recaps

4. **Folder Structure**
   ```
   {memory_root}/
   ├── memory_summary.md
   ├── MEMORY.md
   ├── rollout_summaries/
   │   └── <rollout-id>.md
   └── skills/<skill-name>/
       ├── SKILL.md
       ├── scripts/
       ├── examples/
       └── templates/
   ```

5. **Progressive Disclosure**
   - `memory_summary.md`: Always loaded into system prompt
   - `MEMORY.md`: grep-able registry for keyword searches
   - `rollout_summaries/`: Per-session detail on demand

### Consolidation Prompts

The consolidation prompt instructs the LLM to:
- Produce a `memory_summary.md` with high-level navigation
- Create `MEMORY.md` entries with pointers to rollout summaries
- Handle skill folders with SKILL.md entrypoints
- Manage rollout_summaries for evidence and context

## Memory Retrieval

### Decision Boundary

Agents decide when to use memory based on:

| Condition | Action |
|-----------|--------|
| Self-contained request (time, translation, one-liner) | Skip |
| Mentions workspace/repo/path from memory | Use memory |
| User asks for prior context/decisions | Use memory |
| Ambiguous task possibly dependent on earlier choices | Use memory |
| Unclear: do a quick memory pass | — |

### Quick Memory Pass (4-6 steps max)

1. Skim `memory_summary.md` and extract keywords
2. Search `MEMORY.md` using those keywords
3. Open 1-2 relevant files under `rollout_summaries/` if pointed
4. For exact commands/errors, search `rollout_path` for evidence
5. If no hits, stop and continue normally

### Memory Citation Requirements

When memory is used, append `<oai-mem-citation>` block:

```xml
<oai-mem-citation>
<citation_entries>
MEMORY.md:234-236|note=[brief description]
rollout_summaries/...md:10-12|note=[what was used]
</citation_entries>
<rollout_ids>
019c6e27-e55b-73d1-87d8-4e01f1f75043
</rollout_ids>
</oai-mem-citation>
```

### Verification Strategy

| Drift Risk | Verification Cost | Action |
|------------|-------------------|--------|
| High | Low | Verify before answering |
| High | High | Accept from memory, note staleness |
| Low | Low | Use judgment |
| Low | High | Accept from memory directly |

## Configuration

### Memory Root Path

Default: `{workspace}/.codex/` (resolved via `memory_root()`)

### Feature Flags

```rust
impl Config {
    pub fn memories_enabled(&self) -> bool;
    pub fn memories_scan_limit(&self) -> usize;
    pub fn memories_max_claimed(&self) -> usize;
    pub fn memories_max_age_days(&self) -> i64;
    pub fn memories_min_rollout_idle_hours(&self) -> i64;
    pub fn memories_consolidation_lease_seconds(&self) -> i64;
}
```

### Default Values

| Parameter | Default | Description |
|-----------|---------|-------------|
| `scan_limit` | 100 | Max sessions to consider |
| `max_claimed` | 10 | Max concurrent stage-1 jobs |
| `max_age_days` | 30 | Ignore sessions older than N days |
| `min_rollout_idle_hours` | 4 | Wait N hours after session ends |
| `consolidation_lease_seconds` | 300 | Phase-2 lock duration |
| `retry_remaining` | 3 | Max retries per job |

## State Management

### Job Lifecycle

```
PENDING → CLAIMED → RUNNING → SUCCESS/FAILED → PENDING (next)
              ↓
         SKIPPED (up-to-date, running, backoff, exhausted)
```

### Phase 2 Lock Mechanism

1. Global job key: `"global"`
2. Ownership token: UUID generated per claim
3. Lease duration: 300 seconds (configurable)
4. Prevents concurrent consolidation across workers

### Cleanup Operations

- `clear_memory_root_contents()`: Wipes memory folder (refuses symlinks)
- `delete_all_memory_state()`: Full state database wipe in one transaction
- `delete_stage1_outputs()`: Remove all phase-1 outputs

## File Locations

### Core Modules
- `codex-rs/core/src/memories/mod.rs` — Module definitions
- `codex-rs/core/src/memories/phase1.rs` — Phase 1 implementation
- `codex-rs/core/src/memories/phase2.rs` — Phase 2 implementation
- `codex-rs/core/src/memories/storage.rs` — Database operations
- `codex-rs/core/src/memories/prompts.rs` — Prompt loading
- `codex-rs/core/src/memories/control.rs` — Folder cleanup

### State Models
- `codex-rs/state/src/model/memories.rs` — Data structures
- `codex-rs/state/src/runtime/memories.rs` — State runtime

### Templates
- `codex-rs/core/templates/memories/stage_one_system.md`
- `codex-rs/core/templates/memories/stage_one_input.md`
- `codex-rs/core/templates/memories/consolidation.md`
- `codex-rs/core/templates/memories/read_path.md`

## Usage Example

```rust
// 1. Check if memory feature is enabled
if config.memories_enabled() {
    // 2. Phase 1: Extract from a rollout
    let job = state.claim_stage1_startup_job(...)?;
    
    match job.outcome {
        Stage1JobClaimOutcome::Claimed(claim) => {
            // Process extraction
            let output = extract_from_rollout(&claim.thread)?;
            state.insert_stage1_output(output)?;
        }
        _ => { /* skip */ }
    }
    
    // 3. Phase 2: Consolidate global memory
    let claim = state.claim_global_consolidation_job(...)?;
    
    match claim.outcome {
        Phase2JobClaimOutcome::Claimed { ownership_token, input_watermark } => {
            let selection = state.select_phase2_inputs(input_watermark)?;
            consolidate_memories(&selection, memory_root)?;
        }
        _ => { /* skip */ }
    }
}
```

## Design Principles

1. **Evidence-based**: Raw rollouts are immutable; memories are derived
2. **Progressive disclosure**: Summary → Registry → Detail
3. **Safety first**: Redact secrets, no instruction following from rollouts
4. **Concurrency safe**: Global locks prevent duplicate consolidation
5. **Progressive disclosure**: Memory usage with citation requirements