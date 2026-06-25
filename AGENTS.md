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

This project is a Cargo workspace with 9 crates:

## Workspace Structure (multi-crate)

This project is a Cargo workspace with 6 crates:

| Crate             | Path                   | Description                                                                                                                                                                                                                                             |
| ----------------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **tidev** (root)  | `.`                    | Thin CLI dispatch (`src/main.rs` → `src/lib.rs`). Uses clap to delegate to subcrates.                                                                                                                                                                   |
| **tidev-types**   | `crates/tidev-types`   | Shared types & enums: types, prompts, reasoning levels, theme. Leaf crate, no internal deps.                                                                                                                                                            |
| **tidev-session** | `crates/tidev-session` | Core data model: session, conversation, message, tool call types, stats, balance, system info. Depends on `tidev-types`.                                                                                                                                |
| **tidev-storage** | `crates/tidev-storage` | SQLite persistence: `SessionStore` with separate read/write connections, schema (`schema.rs`), migrations (`migration.rs`), zstd compression. Depends on `tidev-types`, `tidev-session`.                                                                |
| **tidev-llm**     | `crates/tidev-llm`     | LLM provider abstraction: Anthropic, OpenAI chat, OpenAI Responses API, Google Gemini. Depends on `tidev-types`, `tidev-session`.                                                                                                                       |
| **tidev-engine**  | `crates/tidev-engine`  | **Core engine** — the largest crate. Contains agent runtime, tool registry, config loading, MCP, sandbox, memory/graph, snapshot, sync, instructions, logging, provider setup. Depends on `tidev-types`, `tidev-session`, `tidev-storage`, `tidev-llm`. |
| **tidev-tui**     | `crates/tidev-tui`     | Terminal UI (ratatui + crossterm). Depends on `tidev-engine`.                                                                                                                                                                                           |

## Entry Points

- `src/main.rs` → `pub fn run()` in `src/lib.rs` (CLI dispatch via clap)
- Root crate delegates to subcrates based on subcommand:
  - Default mode: `tidev-tui` (`crates/tidev-tui/`)
  - Export/Import: handled directly in root crate's `lib.rs`

## Architecture (key modules in each crate)

### tidev-engine (`crates/tidev-engine/src/`)

- `agent/` — agent loop (`runtime.rs`) and 6 agent types (General, Explorer, Librarian, Oracle, Designer, Fixer)
- `tooling/` — tool definitions, `ToolRegistry`, `ToolArgs` trait, `SkillCatalog`, `FileReadTracker`
- `config/` — config loading, auth storage, provider config, MCP config, sandbox config, reasoning/thinking levels
- `memory/` — memory/graph/retention system (graph nodes/edges, consolidation, eviction, lessons)
- `sandbox/` — sandbox execution (bwrap, landlock, seatbelt, process hardening)
- `snapshot/` — git snapshot/revert for workspace file tracking
- `mcp.rs` — Model Context Protocol (experimental); child process and streamable HTTP transports
- `instructions.rs` — instruction file resolution (AGENTS.md, CLAUDE.md, etc.) with upward directory walk
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
