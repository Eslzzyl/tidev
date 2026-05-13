# Hook Configuration

tidev supports post-tool-use hooks that run shell commands after certain tools
complete. Hooks are configured under the `[hooks]` section of `config.toml`.

## Enabling and disabling hooks

The entire hook system can be disabled with a top-level flag:

```
[hooks]
disable_all_hooks = false
```

When `disable_all_hooks` is `true`, no hooks run regardless of individual hook
definitions. This is useful for temporarily suppressing hooks during debugging
or for specific workspaces via the project-local config override.

## Post-tool-use hooks

Post-tool-use hooks run after a tool finishes execution. Each hook is defined
as a `[[hooks.post_tool_use]]` entry:

```
[[hooks.post_tool_use]]
matcher = "write|edit|apply_patch"
command = "my-formatter {filepath}"
extensions = [".py", ".rs"]
timeout_sec = 30
status_message = "Formatting"
cwd = "/project"
name = "python-formatter"
```

| Key | Required | Default | Description |
|-----|----------|---------|-------------|
| `matcher` | Yes | | Tool name pattern. Pipe-separated list of tool names to match, or `"*"` to match all tools |
| `command` | Yes | | Shell command to execute. Supports template variables |
| `extensions` | No | `[]` | File extension filter. When non-empty, the hook only runs if the tool result has a filepath with a matching extension (e.g. `[".py", ".rs"]`) |
| `timeout_sec` | No | `30` | Command timeout in seconds |
| `status_message` | No | `""` | Message shown in the TUI while the hook runs |
| `cwd` | No | Workspace root | Working directory for the command |
| `name` | No | None | Human-readable label for the hook |

### Matcher patterns

The `matcher` field determines which tools trigger the hook. It uses simple
pipe-separated OR logic:

- `"write"` matches only the `write` tool.
- `"write|edit|apply_patch"` matches `write`, `edit`, or `apply_patch`.
- `"*"` matches every tool.

Whitespace around pipe characters is tolerated. The matcher uses the canonical
tool name, so aliases like `write_file` or `shell` are resolved to their
canonical forms (`write` and `bash` respectively).

The canonical tool names are:

| Canonical name | Aliases | Description |
|----------------|---------|-------------|
| `read` | `read_file` | Read file contents |
| `write` | `write_file` | Write file contents |
| `edit` | | Edit a file |
| `apply_patch` | | Apply a patch to a file |
| `glob` | | Search for files by pattern |
| `grep` | | Search file contents |
| `bash` | `shell` | Execute a shell command |
| `task` | | Delegate to a sub-agent |
| `question` | | Ask the user a question |
| `todowrite` | `todo` | Update the todo list |
| `skill` | | Load a skill |
| `memory` | | Read or write workspace memories |
| `websearch` | | Search the web |
| `webfetch` | | Fetch a web page |

### Template variables in commands

The `command` string supports these template variables that are substituted at
runtime:

| Variable | Description |
|----------|-------------|
| `{filepath}` | Absolute path to the file that was modified by the tool, if applicable. Empty string if no file was involved |
| `{workspace_root}` | Absolute path to the workspace root |
| `{tool_name}` | Name of the tool that triggered the hook (the canonical name) |

For example, a hook that runs a linter on the modified file:

```
[[hooks.post_tool_use]]
matcher = "write|edit|apply_patch"
command = "ruff check --fix {filepath}"
extensions = [".py"]
```

### Extension filtering

When `extensions` is non-empty, the hook only runs if the tool result includes
a filepath and that filepath has one of the listed extensions. The extension
values should include the leading dot.

Examples:

- `extensions = [".py"]` -- only for Python files
- `extensions = [".rs", ".toml"]` -- only for Rust and TOML files
- `extensions = []` -- no filtering (run for all matching tools)

### Command execution

The hook command is executed via `sh -c` with the configured timeout. By
default it runs in the workspace root directory; the `cwd` field can override
this. Environment variables from the tidev process are inherited by the hook
command.

If a hook command fails (non-zero exit code), the error is captured and
included in the tool result output shown to the LLM model. Successful hook
output is also included if non-empty.

### Multiple hooks

Multiple `[[hooks.post_tool_use]]` entries can be defined. They run in the
order they appear in the config file. Each hook is evaluated independently: a
hook runs if its matcher matches the tool and its extension filter passes,
regardless of whether other hooks matched.

When using project-local config overlay, hooks from both the global and project
configs are active. Project hooks run after global hooks.

### TUI output

While a hook is running, tidev displays the `status_message` in the TUI. After
the hook completes, its outcome (`Ran` or `Failed`) is appended to the tool
result output along with any stdout or error text. The outcome is visible to
the LLM model in subsequent turns.

## Use cases

**Code formatting on write:**

```
[[hooks.post_tool_use]]
matcher = "write|edit|apply_patch"
command = "dprint fmt {filepath}"
extensions = [".rs", ".js", ".ts", ".json"]
timeout_sec = 10
status_message = "Formatting with dprint"
name = "dprint"
```

**Linting after file modifications:**

```
[[hooks.post_tool_use]]
matcher = "write|edit|apply_patch"
command = "eslint --fix {filepath}"
extensions = [".js", ".jsx", ".ts", ".tsx"]
timeout_sec = 15
status_message = "Running ESLint"
name = "eslint"
```

**Running tests after specific file changes:**

```
[[hooks.post_tool_use]]
matcher = "write|edit|apply_patch"
command = "cargo test --test integration_tests"
extensions = [".rs"]
timeout_sec = 120
status_message = "Running integration tests"
name = "integration-tests"
```

**Logging all tool usage:**

```
[[hooks.post_tool_use]]
matcher = "*"
command = "echo \"[{tool_name}] {filepath}\" >> /tmp/tidev-tools.log"
timeout_sec = 5
name = "tool-logger"
```

**Hooks in project-local config:**

By placing a `.tidev/config.toml` file in the workspace root with hook
definitions, each project can have its own hooks without affecting the global
configuration. Project hooks run after global hooks, which is useful when both
configurations define format-on-write hooks: the global formatter runs first,
then the project-specific one.
