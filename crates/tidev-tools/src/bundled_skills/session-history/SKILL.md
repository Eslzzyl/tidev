---
name: session-history
description: Inspect tidev session history and retrieve selected messages from the current or another session.
---

# Session history

Use tidev's read-only session commands when the current context summary does
not contain a detail needed for the task. The commands read the stored
session directly and do not start the TUI.

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

## Search other sessions

Use the session list to identify a related session, then inspect that session
with the same commands. Prefer the current workspace when looking for related
work, and use the exact session ID or a unique prefix before reading messages.
