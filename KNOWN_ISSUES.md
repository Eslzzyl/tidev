# Known Technical Debt & Design Issues

## 1. Workspace boundary tools bypass the runtime

**File:** `src/tui/ui/workspace_boundary.rs:153-175`

When a user approves a tool to access files outside the workspace, the tool is
**executed directly by the TUI** (via `execute_call_spawned` + `block_on`),
instead of being sent back to the agent runtime.

**Consequences:**
- The runtime has no knowledge of this tool execution.
- The tool result is **not persisted** to the database — `record_tool_result`
  only updates the in-memory conversation.
- On session reload, the tool result is lost.
- This predates the `tool_events` cleanup; the issue existed before (the old
  `record_tool_result` also skipped persistence in the runtime flow).

**Suggested fix:** Send the approved tool call back to the runtime rather than
executing it synchronously in the TUI. The `workspace_boundary_approved` map
already tracks which tools were allowed, and `send_permission_approval`
propagates `allow_outside` to the runtime.

---

## 2. `cancel_requested` flag is never read by the runtime

**Files:**
- `src/tui/ui/permission.rs:93,152` — field definition
- `src/tui/input/event/request.rs:33,77,116` — written on cancel

The `cancel_requested: Arc<AtomicBool>` flag on `RunningToolExecution` and
`RunningSubagentExecution` is set to `true` on cancellation, but the agent
runtime never checks it. The runtime uses `CancellationToken` for cancellation
instead.

**Possible cleanup:** Remove the flag and simplify the structs.

---

## 3. `RunningStatus` / `_status` is dead code

**File:** `src/tui/ui/permission.rs:83-95,107`

The `RunningStatus` enum and `_status` field on `RunningToolExecution` are
annotated with `#[allow(dead_code)]` and are never read. They were likely
intended for UI tracking but never used.

---

## 4. `parse_minimax_invoke_calls` is missing

**File:** `src/agent/runtime.rs:1107`

The function `parse_minimax_invoke_calls` is called but not defined anywhere
in the codebase. This is a pre-existing build error in the working tree
(unrelated to any cleanup in this document).
