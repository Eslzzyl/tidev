---
name: session-history
description: Inspect tidev session history and retrieve selected messages from the current or another session.
---

# Session history

Use tidev's read-only session commands when the current context summary does
not contain a detail needed for the task. The commands read the stored
session directly and do not start the TUI.

## Search history

Search the message fields in every session, including child sessions:

```bash
tidev session search "keyword" --format json
```

Use `--field all` to include session metadata, textual attachment fields, and
retained tool output. Image bytes are not keyword-searchable and are skipped.
Use a short or complete session ID to scope the search:

```bash
tidev session search "keyword" --field all --session a1b2c3d4e5f6 --format json
```

Each result contains `session_id`, `message_id`, `sequence`, `role`, `field`,
and a context `snippet`. Search results can be filtered or sorted by the shell
or by `jq`:

```bash
tidev session search "keyword" --field all --format json |
  jq '.[] | select(.role == "assistant") | {session_id, message_id, field, snippet}'
```

## Find sessions

List recent sessions, including child sessions:

```bash
tidev session list --limit 50 --format json
```

Filter the JSON metadata before selecting a session:

```bash
tidev session list --limit 100 --format json |
  jq '.[] | {session_id, parent_session_id, workspace_root, title, updated_at}'
```

The session ID may be a complete UUID or a unique UUID prefix.

## Inspect messages

Inspect a session as JSON and project only the fields needed:

```bash
tidev session show <SESSION_ID_OR_PREFIX> --format json |
  jq '.messages[] | {sequence, id: .message.id, role: .message.role, content: .message.content}'
```

Inspect one message:

```bash
tidev session show <SESSION_ID_OR_PREFIX> --message-id <MESSAGE_UUID> --format json
```

Useful fields include `.message.content`, `.message.reasoning`,
`.message.tool_calls`, `.app_data`, and `.tool_output`.

If a message has a retained full tool output, pass its output ID to:

```bash
tidev tool-output <TOOL_OUTPUT_ID>
```

The full output may have expired; the message itself remains available.

## PowerShell

Use `Out-String` before `ConvertFrom-Json` so the complete native-command
output is parsed as one JSON document:

```powershell
$workspaceRoot = (Get-Location).Path
$sessions = (tidev session list --limit 100 --format json | Out-String) | ConvertFrom-Json
$sessions |
  Where-Object { $_.workspace_root -eq $workspaceRoot } |
  Select-Object session_id, title, updated_at
```

```powershell
$view = (tidev session show $sessionId --format json | Out-String) | ConvertFrom-Json
$view.messages |
  Where-Object { $_.message.role -eq 'assistant' } |
  Select-Object sequence,
    @{n='id';e={$_.message.id}},
    @{n='content';e={$_.message.content}}
```

```powershell
$hits = (tidev session search 'keyword' --field all --format json | Out-String) |
  ConvertFrom-Json
$hits |
  Where-Object { $_.role -eq 'assistant' } |
  Sort-Object created_at |
  Select-Object session_id, message_id, field, snippet
```

## Search other sessions

Omit `--session` to search the whole database. Add `--workspace` when related
history should be limited to one workspace.

```bash
tidev session search "keyword" --field all --workspace "$PWD" --format json
```

After selecting a result, inspect the exact message with its full session ID
and message ID. Use `tidev tool-output` with `tool_output_id` for retained full
tool output.
