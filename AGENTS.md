# TiDev

A terminal AI coding assistant built with Rust.

## Build & Verify

```sh
cargo check          # faster than cargo build
cargo clippy         # linting
cargo clippy --fix   # auto-fix issues, then manually edit remaining ones
cargo test
```

## Entry Points

- `src/main.rs` → `pub fn run()` in `src/lib.rs`
- Default mode: `app::run()` (terminal TUI)
- Gateway mode: `tidev gateway telegram` runs `gateway::run()`

## Architecture (key modules)

- `src/app.rs` — main TUI run loop
- `src/storage.rs` — SQLite session persistence (`SessionStore`)
- `src/llm.rs` — LLM provider abstraction
- `src/context.rs` — conversation context management
- `src/tooling.rs` — agent tool definitions
- `src/instructions.rs` — instruction file handling
- `src/snapshot.rs` — file tree snapshots

## Storage Locations

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Database Schema

Tables: `meta`, `sessions`, `session_workspaces`, `session_reverts`, `messages`, `tool_events`, `todos`, `tool_permissions`.

Do not add runtime migration code. If the schema changes, update `SCHEMA_SQL` in `src/storage/schema.rs` directly and require the user to recreate the database.

## Provider Presets

`presets.toml` in the repo root is merged with user config at runtime. Do not put user credentials in this file.

## Submodules

- `codex/`
- `opencode/`
- `rtk/`
- `nanobot/`
- `zeroclaw/`

## Web frontend

web frontend in ./web/

use pnpm as package manager.
