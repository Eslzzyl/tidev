# tidev

A terminal-based AI coding assistant built in Rust. Tidev reimplements the interaction model of [OpenCode](https://github.com/anomalyco/opencode) with a focus on performance and memory efficiency.

> Disclaimer: This project is early in development. The database structure and UI may iterate quickly and produce incompatible changes. Run at your own risk.

---

## Features

- **Multi-Provider LLM Support** -- Anthropic (Claude), OpenAI Chat Completions, and OpenAI Responses. Configurable presets with fallback providers and models.

- **Specialized Sub-Agents** -- Six agent roles for delegating tasks: General (default), Explorer (code search), Librarian (documentation), Oracle (architecture review), Designer (UI/UX), and Fixer (fast implementation). Each agent has a tailored system prompt and tool set.

- **Built-in Tool Set**
  - File operations: `read`, `write`, `edit`, `apply_patch`
  - Code search: `list`, `glob`, `grep`
  - Shell command execution with permission control
  - Session management: task delegation, todo tracking, question prompts
  - Web integration: search (Exa) and page fetching

- **Model Context Protocol (MCP) Support** -- Connect to MCP servers via child process or streamable HTTP. MCP tools are automatically discovered and available with namespaced permissions (`mcp:<server>:<tool>`).

- **Git Snapshot & Revert** -- Automatically track file changes in the workspace using git. View diff statistics and revert changes when needed.

- **Permission System** -- Granular tool permissions per session mode (Read, Search, Write, Edit, Execute, Session).

- **Usage Statistics** -- SQLite-based tracking of token consumption (input, output, cache) and request counts with time-bucket aggregation (hour, day, week, month).

- **API Balance Checking** -- Query account balances for DeepSeek and SiliconFlow providers directly from the UI.

- **Session Persistence** -- All conversations are stored in SQLite. Sessions, workspaces, messages, tool events, and reverts survive restarts.

---

## Performance

Significantly lower memory usage and binary size compared to OpenCode:

| Page | OpenCode Memory | tidev Memory |
|------|----------------|--------------|
| Welcome page | ~470 MB | ~16 MB |
| Chat page (simple session) | ~580 MB | ~30 MB |

| Metric | OpenCode | tidev |
|--------|----------|-------|
| Binary size | 121 MB | 14 MB |

---

## Screenshots

![welcome](https://github-imagebed.eslzzyl.eu.org/tidev/welcome.webp)
![chat](https://github-imagebed.eslzzyl.eu.org/tidev/chat.webp)
![stat](https://github-imagebed.eslzzyl.eu.org/tidev/stat.webp)
![models](https://github-imagebed.eslzzyl.eu.org/tidev/models.webp)

---

## Experimental: Gateway Mode

A gateway mode is under active development that runs tidev as a persistent server with bot integrations (like OpenClaw):

- **Telegram** -- Polling-based bot with file attachment support and user allowlist.
- **QQ** -- Bot integration using the QQ channel API with sandbox mode.

Both platforms can run simultaneously via a shared channel orchestrator. Gateway mode uses a dedicated system prompt and manages separate chat-to-session mappings.

Run with:

```
tidev gateway
```

> Note: Gateway mode is experimental. Configuration, reliability, and feature coverage are still evolving.

---

## How to Pronounce It?

I'd like to pronounce it as "tide-v", but "ti-dev" should also be fine.

---

## Install from crates.io

1. `cargo install tidev`
1. Run `tidev` in your working directory.

## Install from Source

1. Ensure the latest Rust stable toolchain is installed. https://rust-lang.org/
2. Clone the repository and `cd tidev`.
3. `cargo install --path .`
4. Run `tidev` in your working directory.

---

## Configuration

- Config file: `~/.config/tidev/config.toml`
- Auth file: `~/.local/share/tidev/auth.json`
- Session database: `~/.local/share/tidev/sessions.sqlite3`

Provider presets in the repository root (`presets.toml`) are merged with user configuration at runtime.
