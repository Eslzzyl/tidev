## Build & Verify

```sh
cargo check          # faster than cargo build
cargo clippy         # linting (no rustfmt config in root — uses defaults)
cargo clippy --fix   # auto-fix issues
cargo test           # >200 test functions, 8 async; uses tempfile crate
```

## Key CLI Commands

| Command | Description |
|---------|-------------|
| `tidev` (no subcommand) | Terminal TUI (default) |
| `tidev gateway` | Start gateway server (Telegram + QQ bots) |
| `tidev web` | Start web UI server |
| `tidev web --dev-fs` | Serve web frontend from `web/dist` instead of embedded assets |
| `tidev db migrate` | Apply pending schema migrations |
| `tidev db status` | Show current vs. latest schema version |
| `tidev export --session <UUID>` | Export session(s) to plain SQLite (no zstd) |
| `tidev export --all` | Export all sessions |
| `tidev tmp list` | List tidev temp files in /tmp |
| `tidev tmp clean` | Clean old temp files (`--dry-run` to preview) |

## Entry Points

- `src/main.rs` → `pub fn run()` in `src/lib.rs` (CLI dispatch via clap)
- Default mode: Terminal UI (`src/tui/`)
- Gateway mode: `tidev gateway` → `gateway::run()` (Telegram + QQ)
- Web mode: `tidev web` → `web::run()` (axum HTTP+WS, frontend in `web/`)
- Session export: `tidev export --session <UUID>`

## Architecture (key modules)

- `src/agent/runtime.rs` — **shared agent loop** (LLM ↔ tool execution) used by all frontends. Contains `run_agent_loop()` and `run_subagent()`.
- `src/agent/mod.rs` — 6 agent types: General, Explorer, Librarian, Oracle, Designer, Fixer
- `src/tui/` — terminal UI (ratatui + crossterm); `src/tui/core/run.rs` is the TUI entry point
- `src/web/` — axum web server; `routes/`, `event_bus.rs` (WebSocket events), `state.rs`
- `src/gateway/` — Telegram (`telegram/`) and QQ (`qq.rs`) bot integrations via shared channel orchestrator
- `src/storage/` — SQLite persistence (`SessionStore` with separate read/write connections); schema in `schema.rs`, migrations in `migration.rs`
- `src/llm/` — LLM provider abstraction: Anthropic (`anthropic.rs`), OpenAI chat (`openai.rs`), OpenAI Responses API (`responses.rs`), Google Gemini (`gemini.rs`)
- `src/config/` — config loading, auth storage, provider config, MCP config, sandbox config, reasoning/thinking levels
- `src/tooling/` — tool definitions, `ToolRegistry`, `ToolArgs` trait, `SkillCatalog`, `FileReadTracker`
- `src/memory/` — memory/graph/retention system (graph nodes/edges, consolidation, eviction, lessons)
- `src/sandbox/` — sandbox execution (bwrap, landlock, seatbelt, process hardening)
- `src/snapshot/` — git snapshot/revert for workspace file tracking
- `src/mcp.rs` — Model Context Protocol (experimental); child process and streamable HTTP transports
- `src/instructions.rs` — instruction file resolution (AGENTS.md, CLAUDE.md, etc.) with upward directory walk
- `src/prompts.rs` — session modes: `Plan` (read-only) and `Build` (full tools); system prompts

## Storage Locations

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Database Schema

Tables: `meta`, `sessions`, `session_workspaces`, `session_instruction_sources`, `session_reverts`, `messages`, `tool_events`, `todos`, `tool_permissions`, `graph_nodes`, `graph_edges`, `retention_scores`.

### Schema changes

1. Append a `Migration` entry to `MIGRATIONS` in `src/storage/migration.rs`
2. Update `SCHEMA_SQL` in `src/storage/schema.rs` for fresh installs
3. Bump `SCHEMA_VERSION` in `src/storage/schema.rs`
4. Run `cargo test` to verify

### Squashing old migrations

1. Update `SCHEMA_SQL` to cumulative squashed state
2. Remove squashed entries from `MIGRATIONS`
3. Update `meta` row `('schema_version', '<new_baseline_version>')` for existing databases

## Build System

- `build.rs` builds web frontend (`pnpm build` in `web/`) and embeds assets via `include_bytes!`
- If `pnpm` is not available, the web frontend is skipped and `tidev web` shows a placeholder page (TUI works fine)
- Release profile optimizes for binary size: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`

## Provider Presets

`presets.toml` in the repo root is merged with user config at runtime.

## Submodules

| Path | Upstream |
|------|----------|
| `opencode/` | [anomalyco/opencode](https://github.com/anomalyco/opencode) |
| `codex/` | [openai/codex](https://github.com/openai/codex) |
| `zeroclaw/` | [zeroclaw-labs/zeroclaw](https://github.com/zeroclaw-labs/zeroclaw) |
| `opencode-dynamic-context-pruning/` | [Opencode-DCP](https://github.com/Opencode-DCP/opencode-dynamic-context-pruning) |

## Web Frontend

See `web/AGENTS.md`. Uses **pnpm** (not npm/yarn). Build commands:

```bash
cd web && pnpm install && pnpm build
```

Dev server: `pnpm dev` (Vite, serves from filesystem). TypeScript 6.0 + React 19 + Tailwind CSS 4.

## npm Package

`npm/tidev/` publishes the binary via GitHub release artifacts (`npm install -g tidev`). Installation downloads the correct platform binary in `scripts/install.js`.

## Release

Triggered by pushing a `v*` tag. CI workflow (`.github/workflows/release.yml`):
- Builds for 5 platforms (Linux x64/ARM64, macOS x64/ARM64, Windows x64)
- Requires `libdbus-1-dev` on Linux
- Builds web frontend, then `cargo build --release --locked`
- Creates GitHub Release with checksum manifest
- Publishes to npm (`npm/tidev/`)

## Important Conventions

- `cargo clippy` for linting; no root-level rustfmt config (uses defaults)
- Database columns with large content (messages, tool events) are zstd-compressed at the application layer
- Emoji is STRICTLY FORBIDDEN at ANY code in this project
- NEVER automatically simplify the implementation of a plan. If you believe simplification is necessary, stop and solicit feedback from users.
- ALWAYS use English commit message.
