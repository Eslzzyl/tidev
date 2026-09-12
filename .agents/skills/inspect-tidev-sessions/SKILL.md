---
name: inspect-tidev-sessions
description: Inspect tidev sessions and messages directly through the CLI without creating an intermediate database export; use SQLite or JSONL export only for advanced queries, scripting, or legacy binaries.
---

# Inspect Tidev Sessions

Use tidev's session CLI to find and inspect conversation data. The normal
workflow does not start the TUI and does not create a temporary export file.
The CLI reads compressed message columns through the storage layer, so do not
query compressed message content from the main SQLite database directly.

## Prerequisites

- tidev is available in PATH.
- The database exists at ~/.local/share/tidev/sessions.sqlite3.
- jq is optional for filtering JSON inspection output.
- sqlite3 is only needed for advanced fallback queries.

First verify that the direct inspection commands are available:

~~~bash
tidev session --help
~~~

If the installed binary does not have session list/show commands, use the
legacy export fallback below.

## Normal CLI workflow

### 1. Find a session

List recent sessions in a human-readable table:

~~~bash
tidev session list
~~~

Use filters or machine-readable output when needed:

~~~bash
tidev session list --limit 20 --format json
tidev session list --query 'keyword' --format json
~~~

The JSON output includes the exact session_id. The list command includes child
sessions as well as root sessions.

### 2. Show all messages in a session

~~~bash
tidev session show <SESSION_UUID>
tidev session show <SESSION_UUID> --format json
~~~

The command displays session metadata, system prompt, context summary,
messages, protocol metadata, application data, and retained full tool output.
It reads the database directly, does not export an intermediate file, and does
not start the TUI.

### 3. Show one message

Use message_id from the session JSON or text output:

~~~bash
tidev session show <SESSION_UUID> --message-id <MESSAGE_UUID>
tidev session show <SESSION_UUID> --message-id <MESSAGE_UUID> --format json
~~~

The single-message JSON includes the protocol message, app_data such as mode,
snapshot, diff, or child-session information, and tool_output when it is still
retained. A null tool_output means that the output may have expired or was
never retained; persisted message content is still shown.

### 4. Filter JSON output

~~~bash
tidev session show <SESSION_UUID> --format json |
  jq '.messages[] | {sequence, role: .message.role, content: .message.content}'

tidev session show <SESSION_UUID> --format json |
  jq '.messages[] | select(.message.role == "assistant")'
~~~

### 5. Search message history across sessions

Search the default message fields across every session, including child
sessions:

~~~bash
tidev session search 'keyword' --format json
~~~

Use `--field all` to include session metadata, textual attachment fields, and
retained tool output. Image bytes are skipped. Scope the search to a session
with its complete UUID or a unique prefix:

~~~bash
tidev session search 'keyword' --field all --session a1b2c3d4e5f6 --format json
~~~

Each result includes the session ID, message ID, sequence, role, matched field,
match count, and a context snippet. Use `--format jsonl` when a streaming
one-result-per-line format is more convenient for shell pipelines.

PowerShell can parse the JSON result and apply its own filtering and sorting:

~~~powershell
$hits = (tidev session search 'keyword' --field all --format json | Out-String) |
  ConvertFrom-Json
$hits |
  Where-Object { $_.role -eq 'assistant' } |
  Sort-Object created_at |
  Select-Object session_id, message_id, field, snippet
~~~

## Export fallback and advanced workflows

Use export when you need a portable file, arbitrary SQL, an external SQLite
tool, bulk script processing, or an older tidev binary:

~~~bash
tidev export --format sqlite --session <SESSION_UUID> --output /tmp/tidev-export.db
tidev export --format jsonl --session <SESSION_UUID> --output /tmp/tidev-export.jsonl
~~~

SQLite export remains the default when format is omitted. JSONL writes one
message per line and is intended for streaming or scripting; it is not a full
replacement for the SQLite schema. For example:

~~~bash
sqlite3 /tmp/tidev-export.db \
  "SELECT role, substr(content, 1, 200) AS preview
   FROM messages ORDER BY created_at, rowid;"
~~~

For multiple selected sessions, repeat the session option. Use the all-sessions
option when a complete root-session export is needed. Remove temporary files
after inspection.

## Legacy fallback

If the direct session commands are unavailable, locate a session with SQLite,
export it, and inspect the decompressed copy:

~~~bash
sqlite3 ~/.local/share/tidev/sessions.sqlite3 \
  "SELECT id, title, created_at, updated_at FROM sessions
   WHERE title LIKE '%keyword%' ORDER BY updated_at DESC;"

tidev export --session <SESSION_UUID> --output /tmp/tidev-export.db
sqlite3 /tmp/tidev-export.db \
  "SELECT id, role, content FROM messages ORDER BY created_at, rowid;"
rm -f /tmp/tidev-export.db
~~~

The direct CLI path is preferred because it handles zstd decompression, legacy
uncompressed values, message ordering, application data, and retained tool
outputs through the same read path used by tidev.
