---
name: inspect-tidev-db
description: Debug tidev's database by searching session titles, exporting decompressed copies, and inspecting messages. Use when investigating session data, diagnosing storage issues, or auditing conversation history.
---

# Inspect Tidev Database

A structured workflow for reading and debugging tidev's session database. The main database at `~/.local/share/tidev/sessions.sqlite3` stores certain columns (e.g. `messages.content`) as zstd-compressed BLOBs, so you cannot read message content directly. This skill explains how to work around that.

## Prerequisites

- `sqlite3` installed
- `tidev` binary in `$PATH`
- Tidev database exists at `~/.local/share/tidev/sessions.sqlite3`
- (Optional) `zstd` CLI if you want to manually inspect compressed blobs

## Database Overview

### Tables with plain TEXT (readable directly from main DB)

| Table | Important columns |
|---|---|
| `sessions` | `id` (UUID), `title`, `created_at`, `updated_at` |
| `session_workspaces` | `session_id`, `workspace_root` |
| `session_instruction_sources` | `session_id`, `source` |
| `meta` | `key`, `value` (key-value config store) |

### Tables with zstd-compressed BLOB columns (must export to read)

| Table | Compressed columns | Notes |
|---|---|---|
| `messages` | `content`, `reasoning` | Main conversation content |
| `messages` | `patch_files`, `file_diffs` | File patches (may be NULL) |
| `session_reverts` | `redo_snapshot` | Snapshot for undo |
| `todos` | `content` | Task list items |
| `memories` | `content` | Memory store |

## Workflow

### Step 1 — Find candidate sessions by title

Query the `sessions` table directly (it's plain TEXT):

```bash
sqlite3 ~/.local/share/tidev/sessions.sqlite3 \
  "SELECT id, title, created_at FROM sessions \
   WHERE title LIKE '%<keyword>%' \
   ORDER BY updated_at DESC;"
```

To see recent sessions:

```bash
sqlite3 ~/.local/share/tidev/sessions.sqlite3 \
  "SELECT id, substr(title,1,80) AS title, created_at \
   FROM sessions ORDER BY updated_at DESC LIMIT 20;"
```

The output includes the session **UUID** (first column) — note it for Step 2.

### Step 2 — Export the session to a decompressed SQLite file

```bash
tidev export --session <UUID> --output /tmp/tidev-debug-XXXXX.db
```

> **Tip**: Use `mktemp` to generate a safe temp path:
> ```bash
> OUTFILE=$(mktemp /tmp/tidev-debug-XXXXXX.db)
> tidev export --session <UUID> --output "$OUTFILE"
> ```

If you need multiple sessions in one export:
```bash
tidev export --session <UUID1> --session <UUID2> --output /tmp/tidev-export.db
```

### Step 3 — Inspect exported data with sqlite3

**List all messages in a session (chronological):**
```bash
sqlite3 "$OUTFILE" \
  "SELECT role, substr(content,1,200) AS preview FROM messages ORDER BY created_at;"
```

**Find messages containing a specific phrase:**
```bash
sqlite3 "$OUTFILE" \
  "SELECT role, substr(content,1,300) AS snippet FROM messages \
   WHERE content LIKE '%<search-term>%' ORDER BY created_at;"
```

**Count messages by role:**
```bash
sqlite3 "$OUTFILE" \
  "SELECT role, COUNT(*) AS count FROM messages GROUP BY role;"
```

**Get full content of a specific message (by ID):**
```bash
sqlite3 "$OUTFILE" \
  "SELECT content FROM messages WHERE id = '<message-id>';"
```

**Show reasoning content (chain-of-thought) if present:**
```bash
sqlite3 "$OUTFILE" \
  "SELECT substr(reasoning,1,500) FROM messages WHERE reasoning IS NOT NULL AND reasoning != '';"
```

### Step 4 — Clean up

Remove the temp file when done:

```bash
rm -f "$OUTFILE"
```

## Advanced: Manual Decompression (without export)

If you need to inspect a single compressed value directly from the main DB without exporting an entire session:

```bash
# Extract a compressed blob to a file
sqlite3 ~/.local/share/tidev/sessions.sqlite3 \
  "SELECT content FROM messages WHERE id = '<msg-id>'" | tail -1 > /tmp/blob.bin

# Decompress with zstd
zstd -d /tmp/blob.bin -o /tmp/decompressed.txt 2>/dev/null || \
  echo "Not valid zstd (maybe it's legacy uncompressed data)"
```

This is useful when you want a quick peek without the export step, but the export approach is generally preferred because it handles schema complexity and multi-blob decompression automatically.

## Tips

- The exported database is a **plain SQLite file** — you can use any SQLite tool (DBeaver, sqlitebrowser, etc.) to open it interactively.
- The `sessions.title` often contains the user's first message or a summary — search keywords matching the conversation topic.
- If `tidev export --session` fails with "invalid session UUID", ensure the UUID includes hyphens (e.g., `550e8400-e29b-41d4-a716-446655440000`).
- For bulk analysis of all sessions, use `tidev export --all --output ./all-sessions.db` (may be slow for large databases).
