# Configuration

tidev reads configuration from `~/.config/tidev/config.toml`. When a workspace root
is available, a project-local override at `<workspace_root>/.tidev/config.toml` is
loaded on top of the global file. The project-local config merges into the global
config with the following rules:

- Scalar fields (strings, booleans, numbers) from the project config take
  precedence over the global ones.
- Map fields such as `providers` and `mcp.servers` are overlaid: entries from the
  project config replace those from the global config at the same key.
- List fields such as `instructions`, `skills`, and `hooks.post_tool_use` are
  appended. Both the global and project entries are active, with project entries
  running or appearing after the global ones.
- Sub-config sections like `[ui]`, `[logging]`, and `[sandbox]` are replaced
  entirely when the project config explicitly contains that section. If the
  project config does not mention a section, the global value is preserved.
- `hooks.disable_all_hooks` from the project config wins.

The first invocation of tidev creates the default config file at the global path
with sensible defaults. Bundled provider presets ship with the binary and are
always available without needing to be declared in the user config.

## Data and authentication files

| File | Purpose |
|------|---------|
| `~/.config/tidev/config.toml` | Main configuration file |
| `~/.local/share/tidev/auth.json` | API keys and authentication tokens |
| `~/.local/share/tidev/sessions.sqlite3` | Main database |

API keys are stored separately from the config in `auth.json` to avoid accidental
commits. They can be managed through the TUI or by editing the file directly.

## Top-level keys

```
default_provider = "openai"
default_model = "gpt-4o-mini"
theme = "dark"
instructions = []
skills = []
```

### default_provider

The provider to use when no explicit provider is given. The value should match a
provider key in `providers` or in the bundled presets.

### default_model

The model to use when no explicit model is given. The value should match a model
key under the default provider.

### theme

The colour theme of the terminal UI. Supported values:

- `dark`
- `light`
- `nord`
- `one-dark`
- `catppuccin`
- `solarized`
- `orng`
- `github`
- `material`

### instructions

A list of instruction file paths or glob patterns. Each matching file is loaded
and included in the system prompt. Glob patterns are expanded relative to the
workspace root. Example:

```
instructions = ["docs/style.md", "packages/*/AGENTS.md"]
```

Additionally, tidev automatically discovers nearby instruction files named
`AGENTS.md`, `CLAUDE.md`, `.github/copilot-instructions.md`, or `CONTEXT.md` by
walking up from any file being read. When a file in a subdirectory is read, tidev
looks for these instruction files in the same directory and all ancestor
directories up to the workspace root. System-wide instruction files can be placed
in `~/.config/tidev/instructions/`, `<workspace_root>/.tidev/instructions/`, or
`<workspace_root>/.agents/`.

### skills

A list of skill sources. Each entry can be a local file path (relative to the
workspace root or absolute) or an HTTP/HTTPS URL pointing to a `SKILL.md` file.
Skills are also discovered automatically from the `.opencode/skills`,
`.claude/skills`, and `.agents/skills` directories relative to the workspace
root.

## [ui]

Controls aspects of the terminal interface. All settings have sensible
defaults, so you only need to add this section to `~/.config/tidev/config.toml`
if you want to override them.

```
[ui]
sidebar_width = 40
welcome_width = 90
max_input_lines = 6
scroll_speed = 3.0
external_editor = "code --wait"
tab_width = 4
```

| Key | Default | Description |
|-----|---------|-------------|
| `sidebar_width` | `40` | Width of the session sidebar in characters |
| `welcome_width` | `90` | Width of the welcome screen content |
| `max_input_lines` | `6` | Maximum number of lines for the input area |
| `scroll_speed` | `3.0` | Scroll speed multiplier for scrollable panes |
| `external_editor` | none | Command to launch an external editor (e.g. `code --wait`). Falls back to `$VISUAL`, then `$EDITOR`, then auto-detection |
| `tab_width` | `4` | Number of spaces a tab character expands to in diff views |

## [logging]

tidev can write log files for debugging purposes.

```
[logging]
enabled = false
level = "INFO"
max_size_mb = 10
max_files = 5
console = false
```

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable file logging |
| `level` | `"INFO"` | Log level: `DEBUG`, `INFO`, `WARN`, `ERROR` |
| `max_size_mb` | `10` | Maximum log file size in megabytes before rotation |
| `max_files` | `5` | Number of rotated log files to retain |
| `console` | `false` | Also write log output to stderr |

## [notifications]

Desktop notifications for completed operations.

```
[notifications]
enabled = true
method = "auto"
condition = "unfocused"
```

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable terminal notifications |
| `method` | `"auto"` | Notification method. `"auto"` chooses the best available; `"osc9"` uses the iTerm2 OSC 9 escape sequence; `"bel"` uses the terminal bell |
| `condition` | `"unfocused"` | When to notify. `"unfocused"` notifies only when the terminal is not focused; `"always"` notifies on every operation |

## [permissions]

Tool permissions are grouped by mode. tidev has two modes: `plan` (for
discussion, research, and planning) and `build` (for active development). Each
mode has its own set of tool permissions.

```
[permissions.plan]
read = true
search = true
write = false
edit = false
execute = true
session = true

[permissions.build]
read = true
search = true
write = true
edit = true
execute = true
session = true
```

| Key | Plan default | Build default | Description |
|-----|-------------|---------------|-------------|
| `read` | `true` | `true` | Allow reading files (`read`, `websearch`, `webfetch`) |
| `search` | `true` | `true` | Allow searching (`glob`, `grep`) |
| `write` | `false` | `true` | Allow writing files (`write`) |
| `edit` | `false` | `true` | Allow editing files (`edit`, `apply_patch`) |
| `execute` | `true` | `true` | Allow executing shell commands (`bash`) |
| `session` | `true` | `true` | Allow session management (`task`, `question`, `skill`, `memory`, `todowrite`) |

## [sandbox]

Controls how shell commands are sandboxed during execution. The sandbox restricts
filesystem access for shell commands while network access remains unrestricted.

```
[sandbox]
mode = "workspace-write"
writable_roots = ["/some/additional/path"]
```

| Key | Default | Description |
|-----|---------|-------------|
| `mode` | `"workspace-write"` | Sandbox mode: `"workspace-write"` (read access everywhere, write restricted to workspace and `/tmp`), `"read-only"` (read-only access to the entire filesystem), or `"danger-full-access"` (no filesystem restrictions) |
| `writable_roots` | `[]` | Additional absolute directories where writes are permitted in `workspace-write` mode |

The implementation uses OS-level sandbox mechanisms: Seatbelt on macOS and
Bubblewrap or Landlock (as fallback) on Linux. Sandbox is not available on Windows yet.

## [tmp]

Manages tidev temporary files.

```
[tmp]
auto_cleanup = false
max_age_hours = 24
```

| Key | Default | Description |
|-----|---------|-------------|
| `auto_cleanup` | `false` | Automatically remove known tidev temp files on startup |
| `max_age_hours` | `24` | Maximum age in hours for temp files to be kept. Files older than this are removed during cleanup |

## [mcp]

tidev supports the Model Context Protocol (MCP) for extending tool capabilities.
MCP servers can use three transport kinds: `stdio`, `http` (streamable HTTP), and
`sse` (Server-Sent Events).

```
[mcp.servers.my_server]
kind = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
cwd = "/some/path"
env = { MY_VAR = "value" }

[mcp.servers.remote]
kind = "http"
url = "https://example.com/mcp"

[mcp.servers.events]
kind = "sse"
url = "https://example.com/sse"
```

| Key | Description |
|-----|-------------|
| `kind` | Transport kind: `"stdio"`, `"http"`, or `"sse"` |
| `command` | Executable command (stdio only, required) |
| `args` | Command arguments (stdio only, optional) |
| `cwd` | Working directory for the server process (stdio only, optional) |
| `env` | Environment variables for the server process (stdio only, optional) |
| `url` | Server URL (http and sse only, required) |

## [gateway]

Gateway mode allows tidev to run as a service accessible through messaging
platforms. The gateway shares provider configuration with the TUI mode but uses
its own default provider and model settings that fall back to the global defaults
if empty. Gateway sessions can be persisted to the SQLite database and restored
on restart.

```
[gateway]
default_provider = ""
default_model = ""
session_persistence = true

[gateway.telegram]
enabled = false
allowlist = []
poll_timeout_secs = 30

[gateway.qq]
enabled = false
allowlist = []
sandbox = false
```

**Telegram gateway:**

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable Telegram polling |
| `allowlist` | `[]` | Allowed Telegram user or chat identifiers |
| `poll_timeout_secs` | `30` | Long-poll timeout in seconds for `getUpdates` |

**QQ gateway:**

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable QQ Channel gateway |
| `allowlist` | `[]` | Allowed QQ user or channel identifiers |
| `sandbox` | `false` | Use the QQ sandbox environment |

### [gateway.scheduler]

The task scheduler enables cron-based periodic job execution. Jobs can run shell
commands or AI agent prompts. Results can be delivered to gateway channels
(Telegram, QQ, Discord, Lark) when `delivery` is configured.

The scheduler is **disabled by default** and must be explicitly enabled.

```
[gateway.scheduler]
enabled = true
poll_secs = 15
max_concurrent = 3
max_tasks = 10
max_run_history = 100

[gateway.scheduler.jobs.daily_report]
name = "Daily Report"
job_type = "shell"
schedule = { kind = "cron", expr = "0 9 * * *", tz = "Asia/Shanghai" }
command = "echo hello"
enabled = true
delivery = { mode = "announce", channel = "telegram", to = "123456789" }
```

**Scheduler settings:**

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `false` | Enable the task scheduler |
| `poll_secs` | `15` | Polling interval in seconds (minimum 5) |
| `max_concurrent` | `3` | Maximum number of jobs to execute concurrently |
| `max_tasks` | `10` | Maximum number of due jobs to fetch per poll cycle |
| `max_run_history` | `100` | Number of run history entries retained per job |

**Job definition keys** (`[gateway.scheduler.jobs.<alias>]`):

| Key | Default | Description |
|-----|---------|-------------|
| `name` | `None` | Human-readable job name |
| `job_type` | `"shell"` | Job type: `"shell"` or `"agent"` |
| `schedule` | (required) | Schedule specification (see below) |
| `command` | `None` | Shell command (required for `job_type = "shell"`) |
| `prompt` | `None` | Agent prompt (required for `job_type = "agent"`) |
| `enabled` | `true` | Whether the job is active |
| `model` | `None` | Model override for agent jobs |
| `allowed_tools` | `None` | Tool allowlist for agent jobs (e.g. `["read", "grep"]`) |
| `uses_memory` | `true` | Whether to inject memory context for agent jobs |
| `session_target` | `None` | Session target: `"isolated"` (default) or `"main"` |
| `delivery` | `None` | Delivery configuration (see below) |

**Schedule specification:**

The `schedule` field uses a tagged union with three variants:

```
# Every 5 minutes (cron expression)
schedule = { kind = "cron", expr = "*/5 * * * *" }

# With timezone
schedule = { kind = "cron", expr = "0 9 * * 1-5", tz = "America/New_York" }

# One-shot at a specific time
schedule = { kind = "at", at = "2026-06-01T09:00:00Z" }

# Fixed interval in milliseconds
schedule = { kind = "every", every_ms = 3600000 }
```

**Delivery configuration:**

When a job completes with `delivery.mode = "announce"`, the result is sent to
the specified gateway channel.

| Key | Default | Description |
|-----|---------|-------------|
| `mode` | `"none"` | Delivery mode: `"announce"` or `"none"` |
| `channel` | `None` | Target channel: `"telegram"`, `"qq"`, `"discord"`, `"lark"` |
| `to` | `None` | Recipient identifier (chat ID, user ID, channel ID) |
| `thread_id` | `None` | Optional thread/conversation identifier |
| `best_effort` | `true` | Don't fail the job if delivery fails |

**Shell jobs** execute the command via `sh -c <command>` with a 120-second
timeout. **Agent jobs** create an isolated session, run the prompt through the
full LLM + tool loop, and return the final assistant output.

## [agent]

The multi-agent subsystem allows tidev to delegate specialised subtasks to
sub-agents. Each sub-agent type can be assigned a different model, or sub-agents
can inherit the parent session's model. The delegation depth and the number of
concurrent sub-agent sessions per parent can be configured.

```
[agent]
enabled = true
default_subagent_model = "deepseek-v4-flash"
default_subagent_provider = ""
max_depth = 3
max_sessions_per_agent = 5

[agent.models]
explorer = "deepseek-v4-flash"
oracle = "deepseek-v4-pro"
fixer = "minimax-m-2-7"

[agent.thinking_levels]
explorer = "deepseek:Off"
fixer = "deepseek:High"
```

| Key | Default | Description |
|-----|---------|-------------|
| `enabled` | `true` | Enable the multi-agent delegation system |
| `default_subagent_model` | `""` | Default model for sub-agents. If empty, sub-agents inherit the parent session's model |
| `default_subagent_provider` | `""` | Default provider for sub-agents. If empty, uses the global default provider |
| `max_depth` | `3` | Maximum delegation chain depth (orchestrator can delegate to a sub-agent, which can delegate further) |
| `max_sessions_per_agent` | `5` | Maximum concurrent sub-agent tasks per parent session |
| `models` | `{}` | Per-agent model overrides. Keyed by agent type name (`explorer`, `oracle`, `fixer`, `librarian`). Value can be a plain model ID or `"provider/model_id"` format |
| `thinking_levels` | `{}` | Per-agent thinking level overrides. Value format matches the thinking level string representation, e.g. `"deepseek:Off"`, `"deepseek:High"`, `"qwen:On"`, `"glm:On"`. Overrides the auto-detected thinking level for the agent's model |

## Reasoning and thinking

Certain models support extended reasoning or thinking. tidev auto-
detects the correct thinking level for a model based on its identifier and makes
it configurable at the agent level.

The auto-detection rules are:

- Models containing both `"deepseek"` and `"4"` in their lowercased ID use the
  DeepSeek V4 thinking level (Off, High, Max).
- Models containing both `"qwen"` and `"3."` in their lowercased ID use the Qwen
  3.5/3.6 thinking level (Off, On).
- Models containing `"glm"` in their lowercased ID use the GLM thinking level
  (Off, On).
- All other models have no thinking support.

Agent-level thinking configuration overrides the auto-detected level. The TUI
also allows toggling the thinking level at runtime for the active session.

## Context management

tidev automatically prunes conversation context to stay within the model's
context window. The compaction thresholds are hard-coded and not user-
configurable through the TOML file, but the following values apply:

- Prune threshold: 24,000 tokens (or 75% of the model's context window,
  whichever is smaller, with a 4,000-token minimum)
- Retain recent tokens: 12,000 tokens (the most recent portion kept verbatim)
- Summary cap: 8,000 characters

When the total estimated tokens exceed the prune threshold, older messages are
compressed into a summary. The summarised portion is determined by subtracting
the retain budget from the prune threshold.

## Provider and model configuration

Provider and model configuration has its own dedicated section. See
`providers.md` in this directory.

## Hook configuration

Post-tool-use hooks have their own dedicated section. See `hooks.md` in this
directory.
