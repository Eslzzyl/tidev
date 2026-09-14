# D-006: MCP Client Placement

**Date**: 2026-08-03
**Status**: Superseded by the target roadmap

## Previous decision

The 2026-07-03 rewrite plan skipped MCP because the feature had no verified
users and the archived implementation had not been validated.

## Current decision

The target roadmap supersedes that decision. MCP is implemented as a generic
client and registry in `tidev-agent`, using `rmcp`. `tidev-core` retains only
the product integration layer: configuration mapping, workspace path
resolution, permission mapping, and TUI-facing connection state.

`tidev-agent` exposes MCP tools through the generic `Tool` and `ToolRegistry`
interfaces. `tidev-core` exposes fixed `mcp_list`, `mcp_search`, and `mcp_call`
tools to models, then resolves each `mcp_call` target from the current catalog. A
disconnected or failed server leaves the model-facing definitions unchanged and
causes only calls targeting that server to be unavailable.

## Scope boundary

The rewrite still does not add MCP-specific product policy to the agent crate.
Hosts decide when to connect, which permissions to expose, and how MCP errors
are presented to users.
