# D-005: tidev-agent Runtime Boundary

**Date**: 2026-08-03
**Status**: Adopted and implemented in stages; v1 boundary finalized

## Background

The old `AgentRuntime` owned tidev product state and was cloned wholesale for
subagents. The rewritten architecture separates the reusable agent mechanism
from tidev product policy while still providing a usable default runtime.

## Decision

`tidev-agent` owns the generic protocol-level runtime:

```
tidev-agent
├── AgentContext + run_agent_loop
├── AgentRuntime + MessageStore
├── ContextManager + MessageBuffer
├── Tool + ToolContext + ToolRegistry
├── MCP client + McpRegistry
└── AgentEvent

tidev-core
├── CoreContext and BackendEvent
├── approval and Mode policy
├── application message data and persistence
├── snapshots and instruction injection
└── subagent/session orchestration
```

The generic `AgentContext` has seven methods: `tools`, `event_tx`,
`stream_turn`, `execute_tools`, `save_messages`, `workspace_root`, and
`load_messages`. The loop does not know about approvals, modes, snapshots,
application metadata, or subagent sessions.

`AgentRuntime` provides the default implementation for products that need no
approval policy. It owns protocol messages in a `MessageBuffer`, delegates
persistence through `MessageStore`, builds request views through
`ContextManager`, streams `LlmEvent` values as `AgentEvent`, and executes
read-only tools concurrently while keeping write-tool execution serial.

Approvals and subagents remain host policies. A product that needs either can
implement `AgentContext::execute_tools` itself, as tidev-core does. No generic
`ApprovalHandler` or tidev session type is added to the agent crate.

For v1, this also resolves the optional `SubagentHost` design point: no generic
`SubagentHost` trait is added. `AgentRuntime` does not inspect or dispatch a
`task` tool. tidev-core keeps subagent session creation, model selection,
approval inheritance, cancellation, event association, and result synthesis in
its `CoreContext::execute_tools` implementation while reusing the generic
`run_agent_loop` for child sessions.

## Dependencies

The agent crate depends on `tidev-llm` for protocol and provider types. MCP
support uses the external `rmcp` client. It has no dependency on
`tidev-core`, `tidev-config`, `tidev-storage`, or `tidev-tools`.

## Reasons

1. A product can use `AgentRuntime` without importing tidev application policy.
2. Core-specific approval, snapshot, and subagent behavior stays testable at
   the host boundary.
3. The loop and context construction can be reused without changing the bytes
   of protocol messages sent to an LLM.
