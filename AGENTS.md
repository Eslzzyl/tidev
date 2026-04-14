# TiDev

A terminal AI coding assistant built with Rust and ratatui.

## Check

Prefer `cargo check` over `cargo build` for faster verification.

```sh
cargo check
cargo clippy
cargo test
```

## Architecture

- `src/app.rs` — main run loop entrypoint (`app::run()`)
- `src/storage.rs` — SQLite session persistence
- `src/llm.rs` — LLM provider abstraction
- `src/context.rs` — conversation context management
- `src/tooling.rs` — agent tool definitions
- `src/instructions.rs` — instruction file handling
- `src/workspace_snapshot.rs` — file tree snapshots

## Storage Locations

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Database Schema

Current schema version: 6 (table `meta`).

Tables: `meta`, `sessions`, `session_workspaces`, `session_reverts`, `messages`, `tool_events`, `todos`, `tool_permissions`.

Do not add runtime migration code in `src/storage.rs`. If the schema changes, update `SCHEMA_SQL` in `src/storage.rs` directly and ask user to recreate the database.

## Bundled Provider Presets

`presets.toml` in the repo root is merged with user config at runtime. Bundled providers: `deepseek`. Do not put user credentials in this file. Do not modify this file to add new provider unless user ask you to do so.

## Submodules

- `opencode/` — git submodule pointing to `https://github.com/anomalyco/opencode.git` (shallow)
- `codex/` — git submodule pointing to `https://github.com/openai/codex.git` (shallow)
