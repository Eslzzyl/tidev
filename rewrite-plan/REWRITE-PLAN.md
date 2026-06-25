# Tidev Workspace Rewrite Plan

This document describes the target workspace structure for the crate decomposition
rewrite. It is the result of a comprehensive analysis of the current 6-crate workspace
(66,328 lines of Rust) and proposes a 15-crate structure that eliminates the "God
Crate" problem, removes internal circular dependencies, and enforces strict
dependency layering.

Companion document: [ARCHITECTURE.md](./ARCHITECTURE.md) — per-session event bus
design for the agent runtime.

---

## Table of Contents

1. [Current Problems](#1-current-problems)
2. [Design Principles](#2-design-principles)
3. [New Workspace Structure](#3-new-workspace-structure)
4. [Crate-by-Crate Specification](#4-crate-by-crate-specification)
5. [Dependency Graph](#5-dependency-graph)
6. [Key Type Placement](#6-key-type-placement)
7. [TUI Refactoring](#7-tui-refactoring)
8. [Migration Strategy](#8-migration-strategy)

---

## 1. Current Problems

### 1.1 God Crate — `tidev-engine` (19,564 lines, 68 files)

`tidev-engine`承担了过多职责: configuration, agent runtime, tool system, MCP, hooks,
snapshot, sync, instructions, context management, logging, notifications, shell
detection, encoding, and temp file management. Almost everything except TUI rendering
lives in this single crate. This makes the dependency graph opaque and the crate
difficult to reason about or test in isolation.

### 1.2 God Module — `tidev-tui` App struct (33,547 lines, 82 files)

The `App` struct in `tidev-tui/src/lib.rs` owns 70+ fields covering everything from
engine runtime internals to every panel's UI state. Business logic (permission checks,
workspace boundary validation, undo management) is interleaved with rendering code.

### 1.3 Leaky Abstractions

The TUI reaches deep into engine internals rather than going through public APIs:

| Leak | Count | Examples |
|------|-------|---------|
| `tooling::builtin::*` imports | 7 | `resolve_workspace_path`, `is_path_sensitive`, `kill_all_children` |
| `agent::runtime::*` types | 4 | `ApprovedTool`, `PendingToolApproval`, `QueuedUserMessage` |
| `shared::*` internals | 2 | `StepPatch`, `FileSearch` |
| Direct tool schema knowledge | 3 | TUI parses `TaskArgs`, `TodoItem` JSON for rendering |

### 1.4 Internal Soft Cycles

Two circular dependency paths exist at the module level within `tidev-engine`:

- `tooling ↔ mcp`: `ToolRegistry` holds `McpManager`; `McpManager` returns `ToolDefinition`
- `tooling ↔ instructions`: `file.rs` calls `resolve_nearby_instructions()`; `instructions.rs` calls `canonicalize_display`

### 1.5 Duplicate Types

`tidev-llm::types::ToolDefinition` (4 fields) and `tidev_engine::tooling::ToolDefinition`
(6+ fields) are structurally similar types bridged by `llm_bridge.rs` conversions.

### 1.6 Undocumented Internal Cycles

Two additional module-level dependency issues exist that were not caught in the
initial analysis:

- **`agent ↔ tooling`**: `agent/runtime/mod.rs` imports `ToolRegistry`, while
  `tooling/builtin/task.rs` imports `AgentType`. This creates a module-level cycle
  between the agent runtime and the tooling system.
- **`snapshot → tooling` (inverted dependency)**: `snapshot/mod.rs` calls
  `canonicalize_display()` from `tooling::builtin::utils`. The utility function
  is used by four subsystems (snapshot, instructions, agent runtime, tooling itself)
  but lives in a `tooling::builtin` submodule — the wrong place and visibility level.

### 1.7 `task` Tool: Stub Implementation with Fragile String Dispatch

The `task` tool (`tooling/builtin/task.rs`, 62 lines) **does not actually execute
subagents**. It only validates arguments and returns a magic string:

```rust
Ok(format!(
    "Started {agent_type} subagent task '{description}'",
    agent_type = agent_type.display_name()
))
```

The agent loop (`agent_loop.rs`) must parse this magic string to decide whether to
actually dispatch `run_subagent()`. This is:

- **Fragile**: Changing the output string silently breaks subagent dispatch
- **Misleading API**: `execute_tool_call("task", ...)` appears to execute the task
  but returns a placeholder
- **Hidden coupling**: The agent loop must understand the task tool's internal contract
- **Anti-pattern**: String parsing as control flow mechanism

**Fix**: Treat task delegation as a first-class agent loop action (e.g., via
`BackendEvent::SubtaskRequested`), not as a tool that returns a magic string.

### 1.8 Performance Concerns

| Issue | Location | Severity |
|-------|----------|----------|
| `ToolDefinition` deep clone on every tool lookup and execution (clones `serde_json::Value` + 4 `String`s) | `registry.rs:250`, `mcp.rs` | **High** |
| Two linear scans per tool lookup (builtin definitions, then MCP tools) | `registry.rs:250-268` | Medium |
| Global `Arc<Mutex<()>>` serializing ALL snapshot operations across sessions | `snapshot/mod.rs` | Medium |
| `truncate()` allocates new `String` even when input is already under limit | `context.rs:413` | Low |
| `filepath.exists()` stat call on every post-tool-use hook execution | `hooks/engine.rs:121` | Low |
| `BackendEvent::session_id()` 18-arm match extracting session_id on every event | `session.rs:831` | Low |

### 1.9 Visibility and Encapsulation Issues

The entire `tidev-engine` module tree is `pub`, exposing many internal details:

| Module | Problem |
|--------|---------|
| `pub mod shared` | Named "shared" implying internal use, but publicly exposed to all consumers |
| `tooling::builtin::*` | Entire builtin implementation tree is `pub`; should be `pub(crate)` with limited re-exports |
| `config::reasoning` | `ThinkingLevelType` parsing methods used directly by TUI (deep structural coupling) |
| `tooling::builtin::utils` | 534-line mixed utility bag; used by 4 different subsystems; should be promoted to `crate::util::path` |

### 1.10 TUI-Specific Problems

Beyond the leaky abstractions documented in 1.3:

- **`use super::*` proliferation**: 22+ files use `use super::*`, importing the entire
  `lib.rs` namespace into every module
- **Inline test code**: 12+ `mod tests` blocks embedded in production files instead
  of dedicated `tests.rs` files (engine has 20 inline test modules)
- **Monolithic event handler**: `process_backend_events()` is a 500+ line match statement
  handling 18+ event variants with complex session-aware conditional logic
- **Triple subagent event dispatch**: `SubagentStatus` / `SubagentToolResult` /
  `SubagentCompleted` three aggregated events + `running_subagent_executions` cache
  in TUI — all eliminated by per-session event channels
- **Direct `AgentRuntime` access**: TUI directly calls `self.agent.run_agent_loop()`,
  `self.agent.run_subagent()`, etc.

### 1.11 Code Size Distribution Imbalance

```
tidev-types:       959  lines (1.4%)
tidev-session:   2,062  lines (3.1%)
tidev-llm:       5,758  lines (8.7%)
tidev-storage:   4,438  lines (6.7%)
tidev-engine:   19,564  lines (29.5%)   ← God Crate
tidev-tui:      33,547  lines (50.6%)   ← God Module
─────────────────────────────────
Total:          66,328  lines
```

**Two crates account for 80% of all code.** The TUI alone has more lines than all
other crates combined (33,547 vs 32,781).

### 1.12 Feature Flag Simplicity (Post-cleanup)

Prior to rewrite, the sole feature flag `tui` (now removed) controlled only the TUI
dependency. Web and gateway crates were previously implemented and fully removed.
The workspace has no optional features remaining.

---

## 2. Design Principles

| # | Principle | Rationale |
|---|-----------|-----------|
| 1 | **One clear responsibility per crate** | Reduces cognitive load, clarifies boundaries |
| 2 | **DAG-only dependencies (no cycles)** | Eliminates all circular dependency paths |
| 3 | **Types sink to the lowest layer** | Shared types in `tidev-types`; eliminates conversion bridges |
| 4 | **Logic strictly separated from UI** | Business logic never depends on rendering frameworks |
| 5 | **Minimal public API surface** | Each crate exposes only what consumers need |
| 6 | **No God Modules** | Target: every source file < 500 lines |

---

## 3. New Workspace Structure

```
                              ┌────────────┐
                              │   tidev    │  Layer 8: CLI dispatch
                              └─────┬──────┘
                                    │
                              ┌─────┴──────┐
                              │  tidev-tui │  Layer 7: Terminal UI
                              └─────┬──────┘
                                    │
                              ┌─────┴──────┐
                              │ tidev-agent│  Layer 6: Agent runtime
                              └─────┬──────┘
                          ┌─────────┼──────────┐
                    ┌─────┴───┐ ┌──┴───┐ ┌────┴─────┐
                    │tidev-   │ │tidev-│ │tidev-    │
                    │context  │ │ mcp  │ │tools     │
                    └─────────┘ └──────┘ └────┬─────┘
                    Layer 5                    │
                                              │
         ┌──────┬───────┬─────────┬───────┬───┴───┐
         │hooks │instr- │snapshot │ sync  │search │
         │      │uctions│         │       │       │
         └──┬───┘──┬────┘───┬────┘──┬────┘───────┘
            │      │        │       │        Layer 3-4
         ┌──┴──────┴───┬────┴───┬───┴────────┐
         │ tidev-config│tidev-  │tidev-llm   │
         │             │storage │            │
         └──────┬──────┴───┬────┴────────────┘
                │          │            Layer 2
         ┌──────┴──────────┴──────┐
         │     tidev-session      │        Layer 1
         └──────────┬─────────────┘
                ┌───┴──────────────┐
                │   tidev-types    │        Layer 0 (leaf)
                └──────────────────┘
```

**14 sub-crates + 1 root = 15 total** (up from 6 + 1 = 7).

---

## 4. Crate-by-Crate Specification

### Layer 0 — Type Foundation

#### `tidev-types` (~1,500 lines)

Pure type definitions with no business logic. Zero internal dependencies.

| Content | Source | Notes |
|---------|--------|-------|
| Core types (ModelId, ProviderId, etc.) | `tidev-types/types.rs` | Already here |
| **ApiType** enum | `tidev-llm/types.rs` | **Move here** to avoid config → llm dependency |
| **ToolSchema** (name, description, parameters) | **New** | Replaces `tidev-llm::types::ToolDefinition` |
| ToolPermission, PermissionMode | `tidev-types/types.rs` | Already here |
| Prompts (base_instruction, system prompts) | `tidev-types/prompts.rs` | Already here |
| ReasoningLevel, ThinkingLevelType | `tidev-types/reasoning.rs` | Already here |
| ThemePalette | `tidev-types/theme.rs` | Already here |
| SessionMode | `tidev-types/prompts.rs` | Already here |

**New type — `ToolSchema`:**

```rust
/// The LLM-facing tool interface. Minimal — only what providers need.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

This replaces `tidev-llm::types::ToolDefinition` and eliminates the
`llm_bridge.rs` conversion entirely.

**External dependencies:** `serde`, `serde_json`, `uuid`, `chrono`, `log`

---

### Layer 1 — Core Data

#### `tidev-session` (~2,000 lines)

Session data model, conversation types, message types, statistics.

| Content | Source |
|---------|--------|
| Conversation, Message, MessageRole | `tidev-session/session.rs` |
| ToolCall, ToolExecutionResult, ToolMetadata | `tidev-session/session.rs` |
| BackendEvent, AssistantTurn | `tidev-session/session.rs` |
| TokenUsage, Balance | `tidev-session/balance/` |
| UsageSummary, ModelUsageEntry, etc. | `tidev-session/stats/` |
| SystemInfo | `tidev-session/system_info.rs` |

**Dependencies:** `tidev-types`

---

### Layer 2 — Storage, Config & LLM

#### `tidev-config` (~2,000 lines)

Configuration loading, TOML parsing, auth management, provider catalog, model
resolution. The single source of truth for all runtime configuration.

| Content | Source | Notes |
|---------|--------|-------|
| AppConfig (split into sub-structs) | `engine/config/mod.rs` (1,436 lines) | **Needs decomposition into ~6 files** |
| AuthStore, ProviderAuth, ActiveModel | `engine/config/auth.rs` | |
| ProviderConfig, ModelConfig, ProviderSource | `engine/config/provider.rs` | |
| ConfigPaths | `engine/config/paths.rs` | |
| LogConfig | `engine/config/logging.rs` | |
| TidevLogger | `engine/logging.rs` | Move here (configures from LogConfig) |
| NotificationConfig | `engine/config/mod.rs` (inline) | |
| WebSearchConfig | `engine/config/mod.rs` (inline) | |
| GatewayConfig | `engine/config/mod.rs` (inline) | |
| McpConfig, McpServerConfig | `engine/config/mcp.rs` | |
| SnapshotConfig | `engine/config/snapshot.rs` | |
| TmpConfig | `engine/config/tmp.rs` | |
| UiConfig | `engine/config/ui.rs` | |
| ThinkingMatcher | `engine/config/reasoning.rs` | |
| Bundled provider catalog | `engine/config/mod.rs` | |
| Config file loading (TOML) | `engine/config/mod.rs` | |

**Dependencies:** `tidev-types` (+ `tidev-llm` only if ApiType is NOT moved;
see Section 6)

#### `tidev-storage` (~4,400 lines)

SQLite persistence, schema management, migrations, zstd compression,
session export/import.

**Dependencies:** `tidev-types`, `tidev-session`

*(Already exists. No structural changes needed.)*

#### `tidev-llm` (~5,800 lines)

LLM provider implementations. Streaming chat completions. Think-block parsing.

| Content | Source |
|---------|--------|
| LlmClient | `tidev-llm/lib.rs` |
| LlmProviderConfig | `tidev-llm/types.rs` |
| Anthropic provider | `tidev-llm/anthropic.rs` |
| OpenAI Chat provider | `tidev-llm/openai.rs` |
| OpenAI Responses provider | `tidev-llm/responses.rs` |
| Gemini provider | `tidev-llm/gemini.rs` |
| Turn management | `tidev-llm/turn.rs` |
| Think parser | `tidev-llm/think_parser.rs` |
| Attachments | `tidev-llm/attachments.rs` |

**Changes:**
- **Remove** `tidev-llm::types::ToolDefinition` (replaced by `tidev-types::ToolSchema`)
- **Remove** `tidev-llm::types::ApiType` (moved to `tidev-types`)
- **Update** all provider functions to accept `Vec<ToolSchema>` instead of
  `Vec<ToolDefinition>`

**Dependencies:** `tidev-types`, `tidev-session`

---

### Layer 3 — Infrastructure Services

#### `tidev-hooks` (~400 lines)

Post-tool-use hook engine. Self-contained; no dependencies on other internal crates
beyond `tidev-session`.

| Content | Source |
|---------|--------|
| HookEngine | `engine/hooks/engine.rs` |
| HooksConfig, PostToolUseHookConfig | `engine/hooks/config.rs` |
| Hook matcher | `engine/hooks/matcher.rs` |
| Hook runner | `engine/hooks/runner.rs` |

**Dependencies:** `tidev-session`

#### `tidev-instructions` (~600 lines)

Instruction file discovery (AGENTS.md, CLAUDE.md, etc.) and system prompt assembly.

| Content | Source |
|---------|--------|
| resolve_nearby_instructions() | `engine/instructions.rs` |
| system_paths() | `engine/instructions.rs` |
| system_prompt_and_sources_with_cache() | `engine/instructions.rs` |
| APPENDABLE_INSTRUCTION_SOURCES | `engine/instructions.rs` |

**Dependencies:** `tidev-types` (for prompt constants)

**Note:** Remove dependency on `canonicalize_display` from engine's tooling. This
function should become a public utility in `tidev-types` or `tidev-tools`.

#### `tidev-snapshot` (~1,800 lines)

Git-based file snapshots and undo/redo support.

| Content | Source | Notes |
|---------|--------|-------|
| SnapshotService | `engine/snapshot/mod.rs` | |
| GitSnapshot | `engine/snapshot/git.rs` | |
| Patch, FileDiff, EMPTY_TREE_HASH | `engine/snapshot/mod.rs` | |
| StepPatch, undo/redo helpers | `engine/shared/undo.rs` | **Merge from shared/** |

**Dependencies:** `tidev-types`

#### `tidev-sync` (~400 lines)

SSH-based session synchronization.

| Content | Source |
|---------|--------|
| SyncManager | `engine/sync/mod.rs` |
| RemoteMachine, SyncConfig | `engine/sync/mod.rs` |
| SshTransport | `engine/sync/transport/ssh.rs` |

**Dependencies:** `tidev-storage`

#### `tidev-search` (~900 lines)

File indexing and fuzzy path search. Used by TUI for @-mention autocomplete.

| Content | Source |
|---------|--------|
| FileIndex, FileSuggestion | `engine/shared/file_search.rs` |
| fuzzy_path_search() | `engine/shared/file_search.rs` |
| FileSearch, current_at_fragment() | `engine/shared/file_search.rs` |

**Dependencies:** `tidev-types` (minimal — just type imports if any)

---

### Layer 4 — Tool System

#### `tidev-tools` (~5,500 lines)

Tool definitions, registry, skill catalog, and all builtin tool implementations.

| Content | Source | Notes |
|---------|--------|-------|
| **ToolDefinition** (extends ToolSchema) | `engine/tooling/mod.rs` | New richer type |
| canonical_tool_name() | `engine/tooling/mod.rs` | |
| ToolRegistry | `engine/tooling/registry.rs` | **Remove MCP coupling** |
| SkillCatalog, SkillInfo | `engine/tooling/skills.rs` | |
| ToolArgs trait + macros | `engine/tooling/tools.rs` | |
| FileReadTracker | `engine/tooling/file_read_tracker.rs` | |
| **file** tool (read/write/edit) | `engine/tooling/builtin/file.rs` | |
| **exec** tool (bash) | `engine/tooling/builtin/exec.rs` | |
| Shell detection | `engine/shell.rs` | **Merge into this crate** |
| Encoding detection | `engine/encoding.rs` | **Merge into this crate** |
| **search** tools (glob/grep) | `engine/tooling/builtin/search.rs` | |
| **web** tools (websearch/fetch) | `engine/tooling/builtin/web/` | |
| **apply_patch** tool | `engine/tooling/builtin/apply_patch/` | |
| task, todo, question, skill | `engine/tooling/builtin/*.rs` | |
| sudo tool | `engine/tooling/builtin/sudo.rs` | |
| Sensitive file detection | `engine/tooling/builtin/sensitive.rs` | |
| Tool execution dispatch | `engine/tooling/builtin/mod.rs` | |
| Bundled skills | `engine/tooling/bundled_skills/` | |

**New `ToolDefinition` design:**

```rust
/// Rich tool definition with metadata for the tooling system.
pub struct ToolDefinition {
    /// The LLM-facing schema (name, description, parameters).
    pub schema: ToolSchema,
    /// Human-readable display name.
    pub display_name: String,
    /// Required permissions for this tool.
    pub permissions: Vec<ToolPermission>,
    /// Where this tool came from (builtin, MCP, skill).
    pub origin: ToolOrigin,
}

impl ToolDefinition {
    /// Convert to the minimal LLM-facing schema.
    pub fn to_schema(&self) -> ToolSchema {
        self.schema.clone()
    }
}
```

**Key change — remove MCP coupling:**

Currently `ToolRegistry` directly owns an `McpManager`. In the new design:
- `ToolRegistry` is a pure tool registry with no MCP knowledge
- It exposes `register_external_definitions(defs: Vec<ToolDefinition>)` for MCP tools
- `tidev-agent` wires MCP into the registry at startup

**Dependencies:** `tidev-types`, `tidev-session`, `tidev-config`, `tidev-storage`,
`tidev-instructions`, `tidev-search`, `tidev-snapshot`

---

### Layer 5 — Agent Components

#### `tidev-mcp` (~650 lines)

Model Context Protocol client. Server discovery, tool bridging, status tracking.

| Content | Source |
|---------|--------|
| McpManager | `engine/mcp.rs` |
| McpServerState, McpConnectionStatus | `engine/mcp.rs` |
| McpServerSummary | `engine/mcp.rs` |

**Dependencies:** `tidev-types` (for `ToolSchema`, MCP config types),
`tidev-session`

**Note:** Uses `ToolSchema` (not `ToolDefinition`) to avoid depending on
`tidev-tools`. The agent runtime bridges the two.

#### `tidev-context` (~800 lines)

Conversation context window management, compaction, and pruning.

| Content | Source |
|---------|--------|
| ContextManager | `engine/context.rs` |
| CompactionConfig | `engine/context.rs` |

**Dependencies:** `tidev-types`, `tidev-config`, `tidev-tools` (for `ToolSchema`),
`tidev-llm` (for `LlmClient`)

---

### Layer 6 — Agent Runtime

#### `tidev-agent` (~3,000 lines)

The central orchestrator. Agent runtime, main loop, subagent scheduling, tool
execution pipeline, persistence.

| Content | Source | Notes |
|---------|--------|-------|
| AgentType, AgentDefinition | `engine/agent/mod.rs` | |
| AgentOverride, create_agent | `engine/agent/factories.rs` | |
| System prompts per agent type | `engine/agent/prompts.rs` | |
| AgentRuntime struct | `engine/agent/runtime/mod.rs` | |
| run_single_turn, run_agent_loop | `engine/agent/runtime/agent_loop.rs` | |
| execute_tool_calls | `engine/agent/runtime/agent_loop.rs` | |
| run_subagent | `engine/agent/runtime/subagent.rs` | |
| persist_message, persist_tool_result | `engine/agent/runtime/persistence.rs` | |
| QueuedUserMessage, AgentLoopConfig | `engine/agent/runtime/types.rs` | |
| ApprovedTool, PendingToolApproval | `engine/agent/runtime/types.rs` | **Public API types** |
| Desktop notifications | `engine/notifications.rs` | Move here (329 lines, small) |
| LLM bridge (From impls) | `engine/llm_bridge.rs` | **Remove** — use ToolSchema directly |

**New `AgentRuntime` design:**

```rust
pub struct AgentRuntime {
    pub workspace_root: PathBuf,
    pub config: AppConfig,                // from tidev-config
    pub auth: AuthStore,                  // from tidev-config
    pub store: Arc<Mutex<SessionStore>>,  // from tidev-storage
    pub llm_client: LlmClient,            // from tidev-llm
    pub tools: ToolRegistry,              // from tidev-tools
    pub mcp: McpManager,                  // from tidev-mcp (injected)
    pub context: ContextManager,          // from tidev-context
    pub hooks: HookEngine,                // from tidev-hooks
    pub instructions: Vec<String>,
    // ...
}
```

**Public API for consumers (TUI):**

```rust
impl AgentRuntime {
    pub async fn new(...) -> Result<Self>;
    pub async fn run(&mut self) -> Result<()>;
    pub fn send_message(&self, msg: QueuedUserMessage);
    pub fn cancel(&self);
    pub fn pending_approvals(&self) -> Receiver<PendingToolApproval>;
}
```

**Dependencies:** `tidev-types`, `tidev-session`, `tidev-config`, `tidev-storage`,
`tidev-llm`, `tidev-tools`, `tidev-context`, `tidev-mcp`, `tidev-hooks`,
`tidev-snapshot`, `tidev-instructions`

---

### Layer 7 — Application

#### `tidev-tui` (~25,000 lines estimated after cleanup)

Terminal UI. See [Section 7](#7-tui-refactoring) for the detailed restructuring plan.

**Dependencies:** `tidev-agent`, `tidev-config`, `tidev-tools`, `tidev-types`,
`tidev-session`, `tidev-storage`, `tidev-search`, `tidev-snapshot`, `tidev-context`,
`tidev-mcp`, `tidev-sync`, `tidev-llm`

**Note:** The TUI still has a wide dependency surface, but every import goes through
public APIs rather than internal implementation details.

---

### Layer 8 — CLI Root

#### `tidev` (~500 lines)

CLI dispatch, subcommand handling, temporary file management.

| Content | Source | Notes |
|---------|--------|-------|
| CLI argument parsing (clap) | `src/main.rs`, `src/lib.rs` | |
| Subcommand dispatch | `src/lib.rs` | |
| export / import | `src/lib.rs` | |
| tmp list / clean | `engine/tmp.rs` | **Move here** |
| process restart | `engine/process.rs` | **Move here** (46 lines) |
| DB migrate / status | `src/lib.rs` | |
| sync command | `src/lib.rs` | |

**Dependencies:** `tidev-tui`, `tidev-agent`, `tidev-config`, `tidev-storage`,
`tidev-types`

---

## 5. Dependency Graph

### Edge List

| # | Depends On | Required By |
|---|-----------|-------------|
| 1 | tidev-types | tidev-session, tidev-config, tidev-storage, tidev-llm, tidev-hooks, tidev-instructions, tidev-snapshot, tidev-search, tidev-mcp, tidev-tools, tidev-context, tidev-agent, tidev-tui, tidev |
| 2 | tidev-session | tidev-storage, tidev-llm, tidev-hooks, tidev-mcp, tidev-tools, tidev-agent, tidev-tui |
| 3 | tidev-config | tidev-tools, tidev-context, tidev-agent, tidev-tui, tidev |
| 4 | tidev-storage | tidev-sync, tidev-tools, tidev-agent, tidev-tui, tidev |
| 5 | tidev-llm | tidev-context, tidev-agent, tidev-tui |
| 6 | tidev-hooks | tidev-agent |
| 7 | tidev-instructions | tidev-tools, tidev-agent |
| 8 | tidev-snapshot | tidev-tools, tidev-agent, tidev-tui |
| 9 | tidev-sync | tidev-tui |
| 10 | tidev-search | tidev-tools, tidev-tui |
| 11 | tidev-mcp | tidev-agent, tidev-tui |
| 12 | tidev-tools | tidev-context, tidev-agent, tidev-tui |
| 13 | tidev-context | tidev-agent, tidev-tui |
| 14 | tidev-agent | tidev-tui, tidev |

### Layer Summary

| Layer | Crates | Internal Deps |
|-------|--------|---------------|
| 0 (Foundation) | tidev-types | 0 |
| 1 (Core Data) | tidev-session | 1 |
| 2 (Storage/LLM) | tidev-config, tidev-storage, tidev-llm | 1-2 each |
| 3 (Infrastructure) | tidev-hooks, tidev-instructions, tidev-snapshot, tidev-sync, tidev-search | 0-1 each |
| 4 (Tools) | tidev-tools, tidev-mcp | 3-6 each |
| 5 (Agent Parts) | tidev-context | 4 |
| 6 (Agent Runtime) | tidev-agent | 11 |
| 7 (Application) | tidev-tui | 12 |
| 8 (CLI) | tidev | 5 |

**Zero circular dependencies. Every edge is unidirectional.**

---

## 6. Key Type Placement

| Type | Current Location | New Location | Rationale |
|------|-----------------|-------------|-----------|
| ApiType | `tidev-llm::types` | **`tidev-types`** | Needed by tidev-config; avoids config → llm dependency |
| ToolSchema | (new) | **`tidev-types`** | LLM-facing interface; eliminates bridge conversions |
| ToolDefinition | `engine::tooling` | **`tidev-tools`** | Rich definition with permissions, origin |
| ToolPermission | `engine::tooling` | **`tidev-types`** | Shared across crates (config, tools, agent) |
| ToolOrigin | `engine::tooling` | **`tidev-tools`** | Tooling-specific, not needed elsewhere |
| LlmProviderConfig | `tidev-llm::types` | **`tidev-llm`** | LLM-specific |
| ActiveModel | `engine::config::auth` | **`tidev-config`** | Config-specific |
| ApprovedTool | `engine::agent::runtime::types` | **`tidev-agent`** | Agent-specific, public API |
| PendingToolApproval | `engine::agent::runtime::types` | **`tidev-agent`** | Agent-specific |
| StepPatch | `engine::shared::undo` | **`tidev-snapshot`** | Snapshot-specific |
| FileIndex, FileSearch | `engine::shared::file_search` | **`tidev-search`** | Search-specific |
| McpManager | `engine::mcp` | **`tidev-mcp`** | MCP-specific |
| HookEngine | `engine::hooks` | **`tidev-hooks`** | Hook-specific |
| ContextManager | `engine::context` | **`tidev-context`** | Context-specific |
| canonicalize_display | `engine::tooling::builtin::utils` | **`tidev-types`** or **`tidev-tools`** | Used by instructions, snapshot, TUI |
| ResolvedShell | `engine::shell` | **`tidev-tools`** | Only used by exec tool |
| decode_command_output | `engine::encoding` | **`tidev-tools`** | Only used by exec tool |

---

## 7. TUI Refactoring

### 7.1 Current Structure Problems

The `App` struct in `tidev-tui/src/lib.rs` (2,257 lines) is a monolith containing:
- Engine runtime references (AgentRuntime, ToolRegistry, ContextManager)
- Session state (Conversation, message caches)
- UI state for every panel (15+ panel state structs)
- Input state (Composer, AtMention, Snippet, MouseSelection)
- Event channel state

Every module in the crate does `use super::*` via `lib.rs`, effectively importing
everything.

### 7.2 Proposed Structure

```
tidev-tui/src/
├── lib.rs                  — App struct (slimmed to ~200 lines)
├── app/
│   ├── mod.rs              — App definition, field declarations
│   ├── init.rs             — Initialization, engine wiring
│   ├── event.rs            — Top-level event dispatch
│   └── state.rs            — AppState (minimal)
├── core/
│   ├── run.rs              — Event loop
│   ├── permissions.rs      — Tool approval pipeline (from ui/permission.rs)
│   ├── questions.rs        — Question workflow (from ui/question.rs)
│   ├── workspace.rs        — Workspace boundary checks (from ui/workspace_boundary.rs)
│   └── undo.rs             — Undo/redo management (from core/undo.rs)
├── render/
│   ├── chat.rs             — Message rendering
│   ├── tool_cards.rs       — Tool call card rendering
│   ├── diff.rs             — Unified diff rendering
│   ├── panels.rs           — Panel widget rendering
│   └── dialogs.rs          — Dialog widget rendering
├── input/
│   ├── keyboard.rs         — Keyboard event handling
│   ├── mouse.rs            — Mouse event handling
│   └── composer.rs         — Input composition
├── markdown/               — Markdown → ratatui
├── theme/                  — Color themes
├── panels/                 — Per-panel state + logic
│   ├── session.rs
│   ├── model.rs
│   ├── settings.rs
│   ├── mcp.rs
│   ├── sync.rs
│   ├── skills.rs
│   └── ...
└── widgets/                — Reusable UI components
```

### 7.3 Eliminating Leaky Abstractions

| Current Leak | Fix |
|-------------|-----|
| TUI imports `tooling::builtin::utils::*` | Move path utilities to `tidev-tools` public API |
| TUI imports `tooling::builtin::sensitive::*` | Expose via `ToolRegistry::check_sensitive_path()` |
| TUI directly handles `ApprovedTool` | Import from `tidev-agent` public API |
| TUI uses `shared::undo::StepPatch` | Import from `tidev-snapshot` public API |
| TUI uses `shared::file_search` | Import from `tidev-search` public API |
| TUI calls `builtin::kill_all_children` | Expose as `ToolRegistry::kill_all_children()` |

### 7.4 Separation of Concerns

| Category | Current | Target |
|----------|---------|--------|
| Pure rendering (no engine deps) | 42% | 50%+ (render/, markdown/, theme/) |
| Business logic (no ratatui deps) | 36% | 30% (core/, panels/) |
| Mixed (UI + logic) | 22% | <20% (app lifecycle only) |

---

## 8. Migration Strategy

The migration is broken into 5 phases. Each phase results in a compilable, testable
workspace. No "big bang" rewrite.

### Phase 1 — Foundation Crates (Low Risk)

Extract independent modules from `tidev-engine` that have no or minimal internal
dependencies.

1. Move `ApiType` from `tidev-llm` to `tidev-types`
2. Create `tidev-config` (from `engine/config/`, `engine/logging.rs`)
3. Create `tidev-hooks` (from `engine/hooks/`, ~400 lines, self-contained)
4. Create `tidev-snapshot` (from `engine/snapshot/` + `engine/shared/undo.rs`)
5. Create `tidev-sync` (from `engine/sync/`, self-contained)
6. Create `tidev-search` (from `engine/shared/file_search.rs`)
7. Create `tidev-instructions` (from `engine/instructions.rs`)
8. Create `tidev-mcp` (from `engine/mcp.rs`)

**Verification:** `cargo check && cargo clippy && cargo test` after each extraction.

### Phase 2 — Tool System (Medium Risk)

9. Create `tidev-tools` (from `engine/tooling/`, `engine/shell.rs`, `engine/encoding.rs`)
10. Redesign `ToolRegistry` to remove MCP coupling
11. Introduce `ToolSchema` in `tidev-types`, update `tidev-llm` to use it
12. Delete `llm_bridge.rs` (conversions no longer needed)
13. **Redesign `task` tool**: Convert from stub returning magic string to
    first-class agent loop action via `BackendEvent::SubtaskRequested`. The tool
    definition remains for LLM-facing schema, but execution produces a structured
    event consumed by `SessionManager` rather than a parsed string.
14. **Performance: Optimize `ToolRegistry` lookup**: Switch from linear scan to
    `HashMap<String, ToolDefinition>` for O(1) tool lookup. Avoid cloning
    `ToolDefinition` on every execution by using `Arc<ToolSchema>` sharing.
15. **Extract `canonicalize_display`** from `tooling::builtin::utils` into a
    shared utility location (e.g., `tidev-types` or new `tidev-util`)

**Verification:** All tool tests pass, `execute_tool_call` dispatch works,
`task` tool produces `SubtaskRequested` event instead of magic string.

### Phase 3 — Agent System (Medium Risk)

13. Create `tidev-context` (from `engine/context.rs`)
14. Create `tidev-agent` (from `engine/agent/`, `engine/notifications.rs`)
15. Wire agent runtime to use new crate APIs instead of engine internals

**Verification:** Agent loop runs end-to-end, subagent spawning works.

### Phase 4 — TUI Refactoring (High Risk)

16. Restructure `tidev-tui` App monolith into `app/`, `core/`, `render/`, etc.
17. Separate business logic from rendering
18. Replace all leaky abstractions with public API calls
19. Eliminate `use super::*` pattern — each file imports only what it needs
20. **Separate inline tests**: Move `mod tests` blocks from production files into
    dedicated `tests.rs` files
21. **Simplify event handling**: `process_backend_events()` decomposition into
    focused handler methods; per-session event channels eliminate session_id
    checking in every handler
22. **Remove subagent aggregation events**: Delete `SubagentStatus`,
    `SubagentToolResult`, `SubagentCompleted` variants — frontend subscribes to
    child session channels directly

**Verification:** Full TUI manual testing, all TUI tests pass, event handling is
session-agnostic (no `is_active_request()` checks).

### Phase 5 — Cleanup

20. Delete `tidev-engine` (all content extracted)
21. Remove `engine/` directory
22. Update root crate to use new sub-crates
23. Update `AGENTS.md` to reflect new workspace structure
24. Final `cargo clippy --all-targets && cargo test` pass

---

## Appendix A — Current vs. Proposed Dependency Count

| Crate | Current Internal Deps | New Internal Deps |
|-------|-----------------------|-------------------|
| tidev-types | 0 | 0 |
| tidev-session | 1 | 1 |
| tidev-config | 0 (was inside engine) | 1 (tidev-types) |
| tidev-storage | 2 | 2 |
| tidev-llm | 2 | 2 |
| tidev-hooks | 0 (was inside engine) | 1 (tidev-session) |
| tidev-instructions | 0 (was inside engine) | 1 (tidev-types) |
| tidev-snapshot | 0 (was inside engine) | 1 (tidev-types) |
| tidev-sync | 0 (was inside engine) | 1 (tidev-storage) |
| tidev-search | 0 (was inside engine) | 0-1 |
| tidev-mcp | 0 (was inside engine) | 2 (tidev-types, tidev-session) |
| tidev-tools | 0 (was inside engine) | 7 |
| tidev-context | 0 (was inside engine) | 4 |
| tidev-agent | 0 (was inside engine) | 11 |
| tidev-tui | 5 | 12 (all via public APIs) |
| tidev (root) | 4 | 5 |

## Appendix B — External Dependency Migration

Key external dependencies that move between crates during decomposition:

| External Crate | Current Owner | New Owner(s) |
|---------------|---------------|--------------|
| `rmcp` | tidev-engine | tidev-mcp |
| `ignore`, `notify` | tidev-engine | tidev-search (+ tidev-tools for glob/grep) |
| `blake3` | tidev-engine | tidev-snapshot |
| `globset` | tidev-engine | tidev-instructions, tidev-tools |
| `shlex` | tidev-engine | tidev-tools |
| `toml` | tidev-engine | tidev-config |
| `diffy` | tidev-engine | tidev-snapshot (+ tidev-tui for rendering) |
| `reqwest` | tidev-llm, tidev-engine | tidev-llm, tidev-instructions, tidev-tools |
| `pulldown-cmark`, `html2md` | tidev-engine, tidev-tui | tidev-tools, tidev-tui |
| `syntect`, `two-face` | tidev-tui | tidev-tui (no change) |
