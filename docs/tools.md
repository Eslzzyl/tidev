# Basic Tools

TiDev exposes a small set of local tools that are safe by default and stay inside the workspace root.

## Current tools

- `read_file`: read a text file inside the workspace.
- `write_file`: write a text file inside the workspace, creating parent directories when needed.
- `list_dir`: list the entries in a directory inside the workspace.
- `shell`: run a shell command in the workspace root.

## Safety

- Paths are resolved against the workspace root and rejected if they escape it.
- Long outputs are truncated before they can flood the terminal.
- Shell commands run in the workspace root so relative paths behave predictably.

## Implementation note

The current tool layer is intentionally small, but the registry is already structured so more tools can be added later without changing the TUI shape.
