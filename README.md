# tidev

> tidev is my personal project, I will adjust its functions according to my actual needs. Over the past two months I've added a lot of features that I don't use, making tidev bloated and difficult to use. I've aggressively removed several large features from tidev and plan to do a complete rewrite of tidev. The [`last-full`](https://github.com/Eslzzyl/tidev/tree/last-full) tag marks the last commit that contains a complete feature.

AI coding agent built in Rust. tidev reimplements the interaction model of [OpenCode](https://github.com/anomalyco/opencode) with a focus on performance and memory efficiency. It also introduced richer extended features.

> Disclaimer: This project is early in development. The database structure and UI may iterate quickly and produce incompatible changes. Some features may be unavailable yet or contain bugs. Run at your own risk.

---

## Screenshots

![welcome](https://github-imagebed.eslzzyl.eu.org/tidev/welcome.webp)
![chat](https://github-imagebed.eslzzyl.eu.org/tidev/chat.webp)

---

## Features

- **Multi-Provider LLM Support** -- Anthropic (Claude), OpenAI Chat Completions, and OpenAI Responses. Configurable presets with fallback providers and models.

- **Specialized Sub-Agents** -- Five agent roles for delegating tasks: General (default), Explorer (code search), Librarian (documentation), Oracle (architecture review), and Fixer (fast implementation). Each agent has a tailored system prompt and tool set.

- **Built-in Tool Set**
  - File operations: `read`, `write`, `edit`, `apply_patch`
  - Code search: `list`, `glob`, `grep`
  - Shell command execution with permission control
  - Session management: task delegation, todo tracking, question prompts
  - Web integration: search (Exa) and page fetching

- **Model Context Protocol (MCP) Support** (Experimental) -- Connect to MCP servers via child process or streamable HTTP. MCP tools are automatically discovered and available with namespaced permissions (`mcp:<server>:<tool>`).

- **Git Snapshot & Revert** -- Automatically track file changes in the workspace using git. View diff statistics and revert changes when needed.

- **Permission System** -- Granular tool permissions per session mode (Read, Search, Write, Edit, Execute, Session).

- **Usage Statistics** -- SQLite-based tracking of token consumption (input, output, cache) and request counts with time-bucket aggregation (hour, day, week, month).

- **API Balance Checking** -- Query account balances for DeepSeek and SiliconFlow providers directly from the UI.

- **Session Persistence** -- All conversations are stored in SQLite. Sessions, workspaces, messages, tool events, and reverts survive restarts. Large database columns are compressed using zstd at the application layer to ensure a stable increase in database file size.

---

## Performance

| Page            | OpenCode Memory | tidev Memory |
| --------------- | --------------- | ------------ |
| Welcome page    | ~470 MB         | ~17 MB       |
| simple session  | ~580 MB         | ~32 MB       |
| complex session | >1GB            | ~200 MB      |

| Metric      | OpenCode | tidev |
| ----------- | -------- | ----- |
| Binary size | 121 MB   | 26 MB |

---

## Gateway Mode

A gateway mode is under active development that runs tidev as a persistent server with bot integrations (like OpenClaw). Currently supported platforms include Telegram, QQ, Lark, and Discord. Both platforms can run simultaneously via a shared channel orchestrator.

Run with:

```
tidev gateway
```

> Note: Gateway mode is experimental. Configuration, reliability, and feature coverage are still evolving.

---

## WebUI

tidev also supports WebUI. Currently, WebUI is an experimental feature and may not be stable.

Run with:

```
tidev web
```

---

## How to Pronounce It?

I'd like to pronounce it as "tide-v", but "ti-dev" should also be fine.

---

## Install

### Via npm (recommended)

```bash
npm install -g tidev
```

### Via Homebrew

```bash
brew tap eslzzyl/tap
brew install tidev
```

### GitHub Releases

Download the pre-built binary for your platform from the [latest release](https://github.com/Eslzzyl/tidev/releases/latest).

### From source

Requires Rust stable toolchain, [pnpm](https://pnpm.io), and Node.js.

```bash
git clone https://github.com/Eslzzyl/tidev.git
cd tidev
cd web && pnpm install && pnpm build && cd ..
cargo install --path .
```

> **Note**: The `tidev` crate is no longer published on [crates.io](https://crates.io/crates/tidev). Use the methods above instead.

---

## Version Management

```bash
# Bump to a new version (updates Cargo.toml, Cargo.lock, and npm/package.json)
./scripts/bump-version.sh 0.6.0

# Push the tag to trigger CI release
git push origin master --tags
```

---

## Configuration

- Config file: `~/.config/tidev/config.toml`
- Auth file: `~/.local/share/tidev/auth.json`
- Session database: `~/.local/share/tidev/sessions.sqlite3`

Provider presets in the repository root (`presets.toml`) are merged with user configuration at runtime.
