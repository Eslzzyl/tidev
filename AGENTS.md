# TiDev

A terminal AI coding assistant built with Rust and ratatui.

## Build & Run

```sh
cargo build --release
./target/release/tidev
cargo run --release
```

## Architecture

- `src/app.rs` - TUI main loop, screen state (Welcome/Chat), event routing
- `src/app/connect.rs` - Provider creation wizard UI
- `src/app/render.rs` - Terminal rendering (ratatui)
- `src/llm.rs` - OpenAI-compatible HTTP client (streaming + non-streaming)
- `src/tools.rs` - Tool registry and execution
- `src/config.rs` - Config loading, provider/model config (XDG-based)
- `src/storage.rs` - SQLite session persistence (rusqlite, bundled)
- `src/session.rs` - Conversation/message types
- `src/prompts.rs` - Built-in prompt presets
- `src/commands.rs` - Slash command registry
- `src/context.rs` - Context manager
- `src/provider_setup.rs` - Provider onboarding UI

## Storage Locations (XDG)

- Config: `~/.config/tidev/config.toml`
- Auth: `~/.local/share/tidev/auth.json`
- DB: `~/.local/share/tidev/sessions.sqlite3`

## Built-in Commands

`/connect` (login), `/model` (models), `/theme`, `/clear` (new), `/help`, `/quit` (exit, q)

## Tools

`read_file`, `write_file`, `list_dir`, `shell` - all paths are resolved against workspace root.

## Prompt Presets

`tidev_default`, `plan`, `review`, `apply_patch`, `compact`, `provider_setup`.

## LLM Client

OpenAI-compatible. Sends `system` prompt first, then conversation history. Supports streaming via SSE (`data: [DONE]` terminator).

## Database Schema

Tables: `meta`, `sessions`, `messages`, `tool_events`. Foreign keys enforced. Messages ordered by `created_at, rowid`.

## Submodule

`opencode/` is a git submodule pointing to `https://github.com/anomalyco/opencode.git`.

## Testing

No tests currently exist in the repo.