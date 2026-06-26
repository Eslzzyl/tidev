## Build & Verify

```sh
cargo check          # faster than cargo build
cargo clippy         # linting (no rustfmt config in root — uses defaults)
cargo clippy --fix   # auto-fix issues
cargo test           # >200 test functions, 8 async; uses tempfile crate
```

## Key CLI Commands

| Command                         | Description                                                   |
| ------------------------------- | ------------------------------------------------------------- |
| `tidev` (no subcommand)         | Terminal TUI (default)                                        |
| `tidev db migrate`              | Apply pending schema migrations                               |
| `tidev db status`               | Show current vs. latest schema version                        |
| `tidev export --session <UUID>` | Export session(s) to plain SQLite (no zstd)                   |
| `tidev export --all`            | Export all sessions                                           |
| `tidev import <path>`           | Import sessions from an exported SQLite database              |
| `tidev tmp list`                | List tidev temp files in /tmp                                 |
| `tidev tmp clean`               | Clean old temp files (`--dry-run` to preview)                 |
| `tidev sync`                    | Sync sessions with remote machines via SSH                    |

## Workspace Structure (multi-crate)

This project is a Cargo workspace with 18 crates:

| Crate             | Path                   | Description                                                                                                                                                                                                                                             |
| ----------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **tidev** (root)  | `.`                    | Thin CLI dispatch (`src/main.rs` → `src/lib.rs`). Uses clap to delegate to subcrates.                                                                                                                                                                   |
| **tidev-types**   | `crates/tidev-types`   | Shared types & enums: `AgentType`, prompts, reasoning levels, theme, `ToolSchema`. Leaf crate, no internal deps.                                                                                                                                        |
| **tidev-session** | `crates/tidev-session` | Core data model: session, conversation, message, tool call types, `BackendEvent`, stats, balance, system info. Depends on `tidev-types`.                                                                                                                |
| **tidev-storage** | `crates/tidev-storage` | SQLite persistence: `SessionStore` with separate read/write connections, schema (`schema.rs`), migrations (`migration.rs`), zstd compression. Depends on `tidev-types`, `tidev-session`.                                                                |
| **tidev-config**  | `crates/tidev-config`  | Config loading, auth storage, provider config, MCP config, reasoning/thinking levels.                                                                                                                                                                   |
| **tidev-llm**     | `crates/tidev-llm`     | LLM provider abstraction: Anthropic, OpenAI chat, OpenAI Responses API, Google Gemini. Depends on `tidev-types`, `tidev-session`.                                                                                                                       |
| **tidev-hooks**   | `crates/tidev-hooks`   | PostToolUse hook engine with configurable command matchers.                                                                                                                                                                                              |
| **tidev-instructions** | `crates/tidev-instructions` | Instruction file resolution (AGENTS.md, CLAUDE.md, etc.) with upward directory walk.                                                                                                                                |
| **tidev-snapshot** | `crates/tidev-snapshot` | Git snapshot/revert for workspace file tracking.                                                                                                                                                                                                        |
| **tidev-sync**    | `crates/tidev-sync`    | Session sync over SSH.                                                                                                                                                                                                                                  |
| **tidev-search**  | `crates/tidev-search`  | File search utilities (grep, glob wrappers).                                                                                                                                                                                                            |
| **tidev-mcp**     | `crates/tidev-mcp`     | Model Context Protocol (experimental); child process and streamable HTTP transports.                                                                                                                                                                     |
| **tidev-tools**   | `crates/tidev-tools`   | Tool definitions, `ToolRegistry`, `ToolArgs` trait, `SkillCatalog`, `FileReadTracker`, 20+ builtin tools.                                                                                                                                               |
| **tidev-context** | `crates/tidev-context` | Context management: `ContextManager`, `CompactionConfig`, compaction/summarization logic.                                                                                                                                                                |
| **tidev-agent**   | `crates/tidev-agent`   | **Agent runtime** — `AgentLoop` (core LLM ↔ tool execution loop with Per-Session Event Bus), `SessionManager` (session lifecycle), `ControlEvent`, prompts, factories, persistence.                                                                     |
| **tidev-notification** | `crates/tidev-notification` | Desktop notification support.                                                                                                                                                                                                                           |
| **tidev-tui**     | `crates/tidev-tui`     | Terminal UI (ratatui + crossterm). Depends on `tidev-agent`, `tidev-tools`, `tidev-config`, etc.                                                                                                                                                        |

## Entry Points

- `src/main.rs` → `pub fn run()` in `src/lib.rs` (CLI dispatch via clap)
- Root crate delegates to subcrates based on subcommand:
  - Default mode: `tidev-tui` (`crates/tidev-tui/`)
  - Export/Import: handled directly in root crate's `lib.rs`

## Architecture (key modules in each crate)

### tidev-agent (`crates/tidev-agent/src/`)

- `agent_loop.rs` — `AgentLoop`: core LLM ↔ tool execution loop with retry, hooks, context compaction, subagent delegation
- `session_manager.rs` — `SessionManager`: 3-field struct (store, llm, active) + control channel for parent-child coordination
- `types.rs` — `ApprovedTool`, `PendingToolApproval`, `QueuedUserMessage`, `AgentDefinition`, `ControlEvent`, `SessionHandle`, `SharedAgentState`, `compose_static_system_prompt()`
- `prompts.rs` — System prompts for all 6 built-in agent types (General, Explorer, Librarian, Oracle, Designer, Fixer)
- `factories.rs` — `AgentOverride`, `create_agent()`, `create_all_agents()`, `create_sub_agents()`
- `persistence.rs` — Message persistence helpers (`persist_message`, `persist_tool_result`, `persist_assistant_message`)

### tidev-tools (`crates/tidev-tools/src/`)

- `registry.rs` — `ToolRegistry`: tool definition storage, model-aware filtering, `execute_call` routing
- `builtin/` — 20+ built-in tool implementations (read, write, edit, grep, glob, bash, websearch, etc.)
- `agent.rs` — Re-exports `AgentType` from `tidev-types` (was duplicate, now unified)
- `SkillCatalog` — Reusable skill management

### tidev-config (`crates/tidev-config/src/`)

- `lib.rs` — `AppConfig`, `ActiveModel`, `ConfigPaths`, `SharedConfig`
- `auth.rs` — `AuthStore`, `From<ActiveModel> for LlmProviderConfig`
- `reasoning.rs` — `ThinkingLevelType`

### tidev-tui (`crates/tidev-tui/src/`)

- `core/run.rs` — TUI entry point, App construction, SessionManager initialization
- `input/` — keyboard/mouse input handling
- `render/` — ratatui rendering
- `ui/` — UI components (panels, dialogs)
- `markdown/` — markdown rendering
- `theme/` — styling
- `llm_bridge.rs` — bridge between engine and `tidev-llm` crate
- `provider_setup/` — API key / provider initialization flow
- `sync/` — session sync over SSH
- `hooks/` — lifecycle hooks
- `notifications.rs` — desktop notification support

### tidev-types (`crates/tidev-types/src/`)

- `types.rs` — core types
- `prompts.rs` — session modes: `Plan` (read-only) and `Build` (full tools); system prompts
- `reasoning.rs` — reasoning/thinking levels
- `theme.rs` — color theme definitions

### tidev-session (`crates/tidev-session/src/`)

- `session.rs` — `Conversation`, `Message`, `MessageRole`, `ToolCall`, etc.
- `balance/` — token/fee accounting
- `stats/` — usage statistics, granularity
- `system_info.rs` — system metadata collection
- `utils.rs` — utilities

### tidev-storage (`crates/tidev-storage/src/`)

- `database.rs` — `SessionStore` implementation (read/write connections)
- `schema.rs` — `SCHEMA_SQL`, `EXPORT_SCHEMA_SQL`, `SCHEMA_VERSION`
- `migration.rs` — `MIGRATIONS` list
- `compression.rs` — zstd compress/decompress helpers

### tidev-llm (`crates/tidev-llm/src/`)

- `anthropic.rs` — Anthropic API provider
- `openai.rs` — OpenAI chat completions provider
- `responses.rs` — OpenAI Responses API provider
- `gemini.rs` — Google Gemini provider
- `turn.rs` — turn/round management
- `tool_call_format.rs` — tool call formatting
- `think_parser.rs` — thinking tag extraction
- `attachments.rs`, `debug.rs`, `error.rs`, `types.rs`

### tidev-tui (`crates/tidev-tui/src/`)

- `core/run.rs` — TUI entry point
- `input/` — keyboard/mouse input handling
- `render/` — ratatui rendering
- `ui/` — UI components
- `markdown/` — markdown rendering
- `theme/` — styling
- `commands.rs`, `panel_launcher.rs`

## Storage Locations

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Database Schema

Tables: `meta`, `sessions`, `session_workspaces`, `session_instruction_sources`, `session_reverts`, `messages`, `tool_events`, `todos`, `tool_permissions`, `graph_nodes`, `graph_edges`, `retention_scores`.

### Schema changes

1. Append a `Migration` entry to `MIGRATIONS` in `crates/tidev-storage/src/migration.rs`
2. Update `SCHEMA_SQL` in `crates/tidev-storage/src/schema.rs` for fresh installs
3. Bump `SCHEMA_VERSION` in `crates/tidev-storage/src/schema.rs`
4. Run `cargo test` to verify

### Squashing old migrations

1. Update `SCHEMA_SQL` to cumulative squashed state
2. Remove squashed entries from `MIGRATIONS`
3. Update `meta` row `('schema_version', '<new_baseline_version>')` for existing databases

## Build System

- Release profile optimizes for binary size: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`
- Use `cargo build -p tidev-tui` to build a specific crate in isolation

## Provider Presets

`presets.toml` in the repo root is merged with user config at runtime.

## Submodules

| Path                                | Upstream                                                                         |
| ----------------------------------- | -------------------------------------------------------------------------------- |
| `opencode/`                         | [anomalyco/opencode](https://github.com/anomalyco/opencode)                      |
| `codex/`                            | [openai/codex](https://github.com/openai/codex)                                  |
| `zeroclaw/`                         | [zeroclaw-labs/zeroclaw](https://github.com/zeroclaw-labs/zeroclaw)              |
| `opencode-dynamic-context-pruning/` | [Opencode-DCP](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) |

## Release

Triggered by pushing a `v*` tag. CI workflow (`.github/workflows/release.yml`):

- Builds for 5 platforms (Linux x64/ARM64, macOS x64/ARM64, Windows x64)
- Requires `libdbus-1-dev` on Linux
- Runs `cargo build --release --locked`
- Creates GitHub Release with checksum manifest

## Important Conventions

- `cargo clippy` for linting; no root-level rustfmt config (uses defaults)
- Database columns with large content (messages, tool events) are zstd-compressed at the application layer
- Emoji is STRICTLY FORBIDDEN at ANY code in this project
- NEVER automatically simplify the implementation of a plan. If you believe simplification is necessary, stop and solicit feedback from users.
- ALWAYS use Simplified Chinese commit message.
- When building/testing a single crate, use `-p <crate-name>` (e.g., `cargo test -p tidev-storage`)
