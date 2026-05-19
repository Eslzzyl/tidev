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
- `src/storage/migration.rs` — schema migration runner
- `src/llm.rs` — LLM provider abstraction
- `src/context.rs` — conversation context management
- `src/tooling.rs` — agent tool definitions
- `src/instructions.rs` — instruction file handling
- `src/snapshot.rs` — file tree snapshots

## Storage Locations

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Database Schema & Migrations

Tables: `meta`, `sessions`, `session_workspaces`, `session_reverts`, `messages`, `tool_events`, `todos`, `tool_permissions`.

Schema changes are handled by the migration system in `src/storage/migration.rs`.

### How to add a migration

1. **Append** a `Migration` entry to `MIGRATIONS` in `src/storage/migration.rs`:
   ```rust
   Migration {
       version: 33,
       description: "Add collapsed column to messages",
       sql: "ALTER TABLE messages ADD COLUMN collapsed INTEGER NOT NULL DEFAULT 0;",
   }
   ```
2. **Update `SCHEMA_SQL`** in `src/storage/schema.rs` so fresh installations get the complete schema.
3. **Bump `SCHEMA_VERSION`** in `src/storage/schema.rs`.
4. Run `cargo test` to verify.

Migrations auto-apply on startup. You can also manually run `tidev db migrate` or check status with `tidev db status`.

### Squashing old migrations

When many small migrations have accumulated:
1. Update `SCHEMA_SQL` to represent the cumulative state of the squashed migrations.
2. Remove the squashed entries from `MIGRATIONS` in `migration.rs`.
3. For each existing database, update `meta` row `('schema_version', '<new_baseline_version>')`.

### CLI commands

- `tidev db migrate` — apply pending migrations
- `tidev db status` — show current vs. latest schema version

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
