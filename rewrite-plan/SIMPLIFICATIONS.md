# Tidev 重写 — 简化记录

本文档记录了 `tidev` 重写过程中所有与原始 `v0.6.x` 代码库相比出现的**功能简化**。
每个条目说明：简化了什么、为什么简化、恢复计划。

---

## 目录

1. [Phase 1: tidev-session](#1-phase-1-tidev-session)
2. [Phase 2: tidev-config](#2-phase-2-tidev-config)
3. [Phase 3: tidev-hooks](#3-phase-3-tidev-hooks)
4. [Phase 3: tidev-instructions](#4-phase-3-tidev-instructions)
5. [Phase 3: tidev-snapshot](#5-phase-3-tidev-snapshot)
6. [Phase 4: tidev-tools](#6-phase-4-tidev-tools)
7. [Phase 4: tidev-mcp](#7-phase-4-tidev-mcp)
8. [Phase 5: tidev-agent](#8-phase-5-tidev-agent)
9. [Phase 6: tidev-tui（73 个编译错误）](#9-phase-6-tidev-tui73-个编译错误)
10. [未移植的 tidev-engine 模块](#10-未移植的-tidev-engine-模块)
11. [恢复路线图](#11-恢复路线图)

---

## 1. Phase 1: tidev-session

### 1.2 BackendEvent 结构变更（计划内）

| 项目     | 说明                                                 |
| -------- | ---------------------------------------------------- |
| **变更** | 删除所有变体中的 `session_id` 字段                   |
| **状态** | **计划内**，Per-Session Event Bus 架构需求           |
| **影响** | 所有 BackendEvent 消费者（TUI、web）需要更新匹配模式 |

### 1.3 Subagent 事件删除（计划内）

| 项目     | 说明                                                        |
| -------- | ----------------------------------------------------------- |
| **删除** | `SubagentStatus`, `SubagentToolResult`, `SubagentCompleted` |
| **状态** | **计划内**，前端直接订阅子 session 通道                     |
| **影响** | TUI 和 web 前端中的对应处理逻辑需要删除                     |

---

## 2. Phase 2: tidev-config

### 2.1 HooksConfig 内联占位 — ✅ 已修复

| 项目     | 说明                                                                                           |
| -------- | ---------------------------------------------------------------------------------------------- |
| **原始** | `PostToolUseHookConfig` 有完整字段和方法（matcher, command, extensions, timeout 等），位于 `tidev-engine/src/hooks/config.rs` |
| **当前** | `tidev-config` 已改为从 `tidev-hooks` 重新导入完整 `HooksConfig`                                |
| **状态** | ✅ **已完成** — 2026-06-26 替换为 `pub use tidev_hooks::config::HooksConfig`                    |

### 2.2 SyncConfig 内联占位 — ✅ 已修复

| 项目     | 说明                                                                                             |
| -------- | ------------------------------------------------------------------------------------------------ |
| **原始** | `tidev-engine/src/sync/mod.rs` — `SyncConfig { remotes: Vec<RemoteMachine> }` 完整类型，含 `RemoteMachine { name, host, tidev_path, last_sync_at }` |
| **当前** | `tidev-config` 已改为从 `tidev-sync` 重新导入完整 `SyncConfig`                                    |
| **影响** | TUI 中 `config.sync.remotes[0].name` / `.host` / `.last_sync_at` 等字段现在编译通过                |
| **状态** | ✅ **已完成** — 2026-06-26 替换为 `pub use tidev_sync::SyncConfig`                                |

### 2.3 logging::init() 简化

| 项目     | 说明                                                                           |
| -------- | ------------------------------------------------------------------------------ |
| **原始** | `engine/logging.rs` — 完整的文件日志轮转系统（大小限制、文件数限制、异步刷新） |
| **当前** | 简单的 stderr 日志输出，使用 `fern` crate                                      |
| **原因** | 原始实现包含约 100 行的复杂日志管理                                            |
| **影响** | 日志不会写入文件，只有 stderr 控制台输出                                       |
| **恢复** | 还原原始日志轮转实现                                                           |

---

## 3. Phase 3: tidev-hooks

### 3.1 canonical_tool_name 搬入 hooks（可接受）

| 项目     | 说明                                                                            |
| -------- | ------------------------------------------------------------------------------- |
| **原始** | 位于 `engine/tooling/mod.rs`                                                    |
| **当前** | 位于 `tidev-hooks/src/canonical.rs`，同时 `tidev-tools/src/lib.rs` 也有独立副本 |
| **原因** | 提取 hooks 时无法引用还不存在的 tidev-tools                                     |
| **影响** | 两个 crate 中重复定义，可能不同步                                               |
| **恢复** | 统一放到 `tidev-types` 或一个共享位置                                           |

### 3.2 HooksConfig 未使用完整类型 — ✅ 已修复

| 项目     | 说明                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------ |
| **原始** | `PostToolUseHookConfig` — 完整的钩子定义（matcher, command, extensions, cwd, timeout, status_message, name） |
| **当前** | `tidev-config` 已改为从 `tidev-hooks` 重新导入完整 `HooksConfig`                                                          |
| **状态** | ✅ **已完成** — 随 2.1 一起修复                                                                                           |

---

## 4. Phase 3: tidev-instructions

⚠️ **无已知简化**。代码完整地从 `engine/instructions.rs` 提取，6 个测试全部通过。

---

## 5. Phase 3: tidev-snapshot

### 5.1 缺失 shared/undo.rs 中的类型 — ✅ 已修复

| 项目     | 说明                                                                                                         |
| -------- | ------------------------------------------------------------------------------------------------------------ |
| **缺失** | `StepPatch`, `extract_patches_from_message`, `collect_patches_from_message`, `collect_patches_after_message` |
| **原始** | `engine/shared/undo.rs` — 约 200 行的撤销/重做补丁管理                                                       |
| **恢复** | 将 `shared/undo.rs` 的内容移植到 `tidev-snapshot`                                                            |
| **状态** | ✅ **已完成** — 2026-06-26 移植到 `crates/tidev-snapshot/src/lib.rs`，`cargo test -p tidev-snapshot` 通过  |

---

## 6. Phase 4: tidev-tools

### 6.1 encoding.rs 大幅简化

| 项目     | 说明                                                                                       |
| -------- | ------------------------------------------------------------------------------------------ |
| **原始** | `engine/encoding.rs` — 使用 `encoding_rs` crate 做真实编码检测（UTF-8, GBK, Shift-JIS 等） |
| **当前** | `decode_command_output` 直接用 `String::from_utf8_lossy`                                   |
| **原因** | 简化依赖，`encoding_rs` 增加编译时间                                                       |
| **影响** | 非 UTF-8 输出（如中文 Windows GBK）可能乱码                                                |
| **恢复** | 添加 `encoding_rs` 依赖并还原原始检测逻辑                                                  |

### 6.2 shell.rs 大幅简化

| 项目     | 说明                                                                                                           |
| -------- | -------------------------------------------------------------------------------------------------------------- |
| **原始** | `engine/shell.rs` — 完整跨平台 shell 检测（Windows Git Bash/MSYS2/PowerShell auto-detect + 持久化），约 150 行 |
| **当前** | 只在 Linux/macOS 上尝试 `/bin/bash`, `/bin/zsh`, `/bin/sh`                                                     |
| **原因** | 简化跨平台逻辑                                                                                                 |
| **影响** | Windows 上 shell 检测完全不可用                                                                                |
| **恢复** | 将原始 `engine/shell.rs` 移植过来                                                                              |

### 6.3 shell::init 为空函数

| 项目     | 说明                                              |
| -------- | ------------------------------------------------- |
| **原始** | 持久化用户配置的 Windows shell 选择到 config.toml |
| **当前** | 空函数，什么都不做                                |
| **原因** | shell 初始化逻辑包含在原始 `shell.rs` 中，未移植  |
| **影响** | Windows 首次使用的 shell 选择不会被保存           |
| **恢复** | 随 shell.rs 一起移植                              |

### 6.4 Task 工具中 AgentType 为内联定义

| 项目     | 说明                                                                             |
| -------- | -------------------------------------------------------------------------------- |
| **原始** | `engine/agent/AgentType` — 完整的 agent 类型枚举                                 |
| **当前** | `tidev-tools/src/agent.rs` 中内联定义，`tidev-agent/src/types.rs` 中也有独立定义 |
| **原因** | `task.rs` 引用 `crate::agent::AgentType`，但 tidev-agent 在 Phase 5 才创建       |
| **影响** | 两个副本可能不同步                                                               |
| **恢复** | 统一放到 `tidev-agent`，tidev-tools 从那里导入                                   |

### 6.5 execute_tool_calls 返回占位结果

| 项目     | 说明                                                                                      |
| -------- | ----------------------------------------------------------------------------------------- |
| **原始** | `tools.rs` 中的 `execute_tool_calls` → `execute_shell_tool_call` 路由到注册的工具处理函数 |
| **当前** | `AgentLoop::execute_tool_calls` 返回占位字符串                                            |
| **原因** | `ToolRegistry` 方法签名在新架构中变更，还未完全接入                                       |
| **影响** | AgentLoop 中的工具执行不会产生实际效果                                                    |
| **恢复** | 将 `execute_tool_calls` 连接到 `ToolRegistry`                                             |

### 6.6 task.rs 占位执行

| 项目     | 说明                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------- |
| **原始** | `engine/tooling/builtin/task.rs` — `execute_tool_call` 调用 `AgentRuntime::run_subagent()` 创建子 session，运行完整的子 agent 循环 |
| **当前** | `tidev-tools/src/builtin/task.rs` — `execute_tool_call()` 返回占位字符串 `"Started {agent_type} subagent task '{description}'"`，不实际创建子 agent，参数校验完整但无副作用 |
| **原因** | task 工具需要调用 `SessionManager::spawn()`，但架构连接未完成（互为依赖）                                |
| **影响** | task 工具不会真正执行子 agent，LLM 得到虚假的成功响应                                                   |
| **恢复** | 在 task 工具中调用 `SessionManager::spawn()`，传递正确的 AgentType、会话 ID 和工具列表                  |

### 6.7 MCP 工具执行为 stub

| 项目     | 说明                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------- |
| **原始** | `engine/mcp.rs` — `McpManager::execute_call` 通过 MCP 协议调用远程工具                                    |
| **当前** | `tidev-mcp/src/lib.rs` — `try_execute_mcp` 直接 `bail!("not implemented")`                              |
| **影响** | MCP 工具在 AgentLoop 中不可用                                                                            |
| **恢复** | 连接到 `tidev_mcp::McpManager::execute_call`（McpManager 中的 `list_tools` 和 `call_tool` 已实现，113 行，但未暴露给 AgentLoop） |

---

## 7. Phase 4: tidev-mcp

### 7.1 ToolDefinition 为简化版

| 项目     | 说明                                                                                               |
| -------- | -------------------------------------------------------------------------------------------------- |
| **原始** | `engine/tooling/ToolDefinition` 含 `ToolOrigin` 枚举（`Local` / `Mcp { server_name, tool_name }`） |
| **当前** | `tidev-mcp::types::ToolDefinition` 为简化版，直接包含 `server_name` 和 `remote_tool_name` 字段     |
| **原因** | 避免引入 `ToolOrigin` 枚举依赖                                                                     |
| **影响** | 功能等价但类型不同，需要 `From` 转换                                                               |
| **恢复** | 无需要，设计更干净                                                                                 |

### 7.2 parse_tool 中 display_name 被省略

| 项目     | 说明                                                  |
| -------- | ----------------------------------------------------- |
| **原始** | 从 MCP 工具定义中读取 `title` 字段作为 `display_name` |
| **当前** | display_name 在 `ToolDefinition::mcp()` 内部自动构建  |
| **原因** | 简化构造逻辑                                          |
| **影响** | MCP 工具自定义 display_name 不会被使用                |
| **恢复** | 在 `mcp()` 构造函数中检查并传递 display_name          |

---

## 8. Phase 5: tidev-agent

这是**最大的简化区域**，因为 tidev-agent 是全新编写的代码（非提取）。

### 8.1 AgentLoop 工具执行为占位

| 项目     | 说明                                                                                   |
| -------- | -------------------------------------------------------------------------------------- |
| **原始** | `AgentRuntime::execute_tool_calls` → 通过 `ToolRegistry` 路由到 20+ 内置工具 + MCP     |
| **当前** | `AgentLoop::execute_tool_calls` 返回占位字符串 `"Executed tool 'X' (standalone mode)"` |
| **原因** | 需要集成 `ToolRegistry`，但架构尚未确定如何连接                                        |
| **影响** | AgentLoop 无法真正执行任何工具                                                         |
| **恢复** | 实现 `execute_tool_calls` → `ToolRegistry::execute` 路由                               |

### 8.2 工具审批流程未实现 + 类型字段不匹配

| 项目     | 说明                                                                                                                                                                                    |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **原始** | `PendingToolApproval { tool_calls: Vec<ToolCall>, mode: SessionMode, response_tx: oneshot::Sender<Vec<ApprovedTool>> }` — 异步审批通道，TUI 通过 `response_tx` 返回用户决定               |
| **当前** | `tidev-agent::types::PendingToolApproval { session_id, request_id, tool_call, tool_definition }` — 结构完全不同；`ApprovedTool { tool_call, tool_definition }` — 缺失审批相关字段         |
| **影响** | TUI 编译错误：`PendingToolApproval` 无 `tool_calls`/`response_tx`/`mode` 字段；`ApprovedTool` 无 `rejection`/`child_session_id`/`allow_outside`/`sensitive_file_approved` 字段（约 12 个错误） |
| **恢复** | 统一类型定义：将 `PendingToolApproval` 恢复为包含 `tool_calls: Vec<ToolCall>` + `mode` + `response_tx` 的完整审批结构；在 `ApprovedTool` 中添加所有缺失字段                              |

### 8.3 SessionManager API 不匹配

| 项目     | 说明                                                                                                                                                                                                                                   |
| -------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **原始** | TUI 期望 SessionManager 有 ~12 个可直接访问的字段和方法：`workspace_root`、`config_dir`、`config_paths`、`config`、`auth`、`llm_client`、`tools`、`instructions`、`instruction_content_cache`、`queued_messages`、`hooks`、`auto_approve_permissions`、`store` |
| **当前** | `crates/tidev-agent/src/session_manager.rs` 的 `SessionManager` 只有 3 个字段（`store`、`llm`、`active`）和 5 个方法（`spawn`、`cancel`、`list_active`、`is_active`、`active_count`）                                                          |
| **影响** | TUI 中约 25 个编译错误：struct `SessionManager` has no field named `workspace_root` / `config_dir` / `tools` 等；no method named `queue_user_message` / `compose_static_system_prompt` / `run_agent_loop_with_permission_channel`      |
| **恢复** | 两种方案：(A) 在 SessionManager 中添加缺失的访问器方法；(B) 重写 TUI 中直接访问 SessionManager 内部字段的代码，改为通过 `tidev-config`、`tidev-agent` 公共 API 获取。推荐方案 B                                                     |

### 8.4 子 agent 调度未实现

| 项目     | 说明                                                                 |
| -------- | -------------------------------------------------------------------- |
| **原始** | `SubagentConfig` → `run_subagent_inner` → 递归创建子 session         |
| **当前** | `SessionManager::spawn` 支持 `parent_session_id`，但 task 工具未接入 |
| **影响** | task 工具返回占位字符串，不会真正创建子 agent                        |
| **恢复** | 在 `task.rs` 中调用 `SessionManager::spawn`                          |

### 8.5 Hook 执行未实现

| 项目     | 说明                                                                  |
| -------- | --------------------------------------------------------------------- |
| **原始** | `AgentRuntime` 在工具执行后调用 `HookEngine::run_post_tool_use_hooks` |
| **当前** | AgentLoop 中无钩子调用                                                |
| **影响** | post-tool-use 钩子不会触发                                            |
| **恢复** | 在 `execute_tool_calls` 后添加 `HookEngine` 调用                      |

### 8.6 上下文压缩未实现

| 项目     | 说明                                                                          |
| -------- | ----------------------------------------------------------------------------- |
| **原始** | `ContextManager::compact_if_needed`, `compact`, `schedule_context_compaction` |
| **当前** | AgentLoop 中无压缩逻辑                                                        |
| **影响** | 长对话不会自动压缩上下文                                                      |
| **恢复** | 在 AgentLoop 循环中添加压缩检查                                               |

### 8.7 重试逻辑未实现

| 项目     | 说明                                                                                       |
| -------- | ------------------------------------------------------------------------------------------ |
| **原始** | `stream_chat_with_retry` 在 tidev-llm 层实现（最多 MAX_RETRIES 次）                        |
| **当前** | tidev-llm 的带重试版本在提取后被保留，但 tidev-agent 的 AgentLoop 不处理 Failed 事件的重试 |
| **影响** | LLM 请求失败后直接抛出错误                                                                 |
| **恢复** | 在 `run_single_turn` 中添加重试循环                                                        |

### 8.8 权限检查未实现

| 项目     | 说明                                                                     |
| -------- | ------------------------------------------------------------------------ |
| **原始** | `PermissionConfig`, `ToolPermission::is_allowed_in` 控制 Plan/Build 模式 |
| **当前** | AgentLoop 不检查工具权限                                                 |
| **影响** | Plan 模式下也可以执行写操作工具                                          |
| **恢复** | 在 `execute_tool_calls` 中添加权限检查                                   |

### 8.9 单元测试缺失

| 项目     | 说明                                              |
| -------- | ------------------------------------------------- |
| **原始** | `agent/runtime/tests.rs` — 完整的 agent loop 测试 |
| **当前** | tidev-agent 中 0 个测试                           |
| **原因** | 全新编写，尚未添加测试                            |
| **恢复** | 编写 MockSessionStore + MockLlmClient 测试        |

---

## 9. Phase 6: tidev-tui（73 个编译错误 → ✅ 已全部修复）

> **状态**：2026-06-26 所有 73 个编译错误已清除。`cargo check -p tidev-tui` 通过，整个工作区 `cargo check --workspace` 通过。

TUI 移植过程中共出现 73 个编译错误，以下按类别分类记录。

### 9.1 SessionManager 内部字段直接访问（~15 个错误）

TUI 代码大量直接访问 `SessionManager` 的内部字段，但新版 `SessionManager` 只有 `store`、`llm`、`active` 三个字段：

| 错误模式 | 原因 | 影响范围 |
|---------|------|---------|
| `no field 'tools' on SessionManager` | TUI 读取 `session_manager.tools` | `core/run.rs`, `commands.rs` |
| `no field 'llm_client' on SessionManager` | TUI 读取 `session_manager.llm_client` | `core/run.rs` |
| `no field 'workspace_root' on SessionManager` | TUI 读取 `session_manager.workspace_root` | `core/run.rs`, `input/event/mod.rs` |
| `no field 'config' on SessionManager` | TUI 读取 `session_manager.config` | `core/run.rs` |
| `no field 'auth' on SessionManager` | TUI 读取 `session_manager.auth` | `core/run.rs` |
| `no field 'hooks' on SessionManager` | TUI 读取 `session_manager.hooks` | `core/run.rs` |
| `no field 'config_dir' / 'config_paths' on SessionManager` | TUI 读取配置路径 | `core/run.rs` |
| `no field 'instructions' / 'instruction_content_cache' on SessionManager` | TUI 读取指令缓存 | `core/run.rs` |
| `no field 'queued_messages' on SessionManager` | TUI 读取消息队列 | `core/run.rs` |
| `no field 'auto_approve_permissions' on SessionManager` | TUI 读取自动审批配置 | `core/run.rs` |
| `no field 'store' on SessionManager` (类型不匹配) | TUI 以特定方式访问 store | `core/run.rs` |

**恢复**：TUI 初始化代码需要从 `AgentRuntime` 手动构造改为使用 `SessionManager::spawn()` API。将 App 持有的配置字段改为在构造时直接注入，而非从 SessionManager 读取。

### 9.2 SessionManager 缺失的方法调用（~8 个错误）

TUI 调用的方法在新版 SessionManager 中不存在：

| 错误 | 原始代码 | 恢复 |
|------|---------|------|
| `no method 'queue_user_message'` | `session_manager.queue_user_message(...)` | 在 SessionManager 中添加消息队列支持 |
| `no method 'compose_static_system_prompt'` | `session_manager.compose_static_system_prompt(...)` | 在 SessionManager 中添加系统提示组合方法 |
| `no method 'run_agent_loop_with_permission_channel'` | `session_manager.run_agent_loop_with_permission_channel(...)` | 实现审批通道并暴露方法 |
| `no method 'session_id'` on BackendEvent | `event.session_id()` | BackendEvent 已删除 `session_id`，改为从事件通道上下文获取 |

**恢复**：在 `SessionManager` 中添加缺失的方法；或重构 TUI 逻辑不再依赖这些方法。

### 9.3 BackendEvent 模式匹配残留（~5 个错误）

TUI 处理事件时仍使用旧版带 `session_id` 字段的模式匹配：

```rust
// TUI 中的旧代码：
BackendEvent::Delta { content, session_id, .. } => { ... }
BackendEvent::ReasoningDelta { content, session_id, .. } => { ... }
BackendEvent::ShellOutput { content, finished, exit_code, session_id, .. } => { ... }
```

新版 BackendEvent 所有变体已删除 `session_id`，只需移除模式中的 `session_id` 字段即可。

### 9.4 ApprovedTool / PendingToolApproval 字段不匹配（~12 个错误）

| 错误 | 原因 | 受影响的代码位置 |
|------|------|----------------|
| `no field 'rejection' on ApprovedTool` | TUI 检查 `approved_tool.rejection` | `ui/permission.rs` |
| `no field 'child_session_id' on ApprovedTool` | TUI 读取 `approved_tool.child_session_id` | `ui/permission.rs` |
| `no field 'allow_outside' on ApprovedTool` | TUI 设置 `approved_tool.allow_outside` | `ui/permission.rs` |
| `no field 'sensitive_file_approved' on ApprovedTool` | TUI 设置 `approved_tool.sensitive_file_approved` | `ui/permission.rs` |
| `no field 'tool_calls' on PendingToolApproval` | TUI 读取 `pending.tool_calls` | `ui/permission.rs` |
| `no field 'mode' on PendingToolApproval` | TUI 读取 `pending.mode` | `ui/permission.rs` |
| `no field 'response_tx' on PendingToolApproval` | TUI 写入 `pending.response_tx` | `ui/permission.rs` |

**恢复**：统一 `tidev-agent::types` 中的 `ApprovedTool` 和 `PendingToolApproval` 定义，匹配 TUI 期望的完整字段集。

### 9.5 未移植的类型导入（~8 个错误）

| 错误 | 缺失类型 | 原始位置 | 需要移至 |
|------|---------|---------|---------|
| `cannot find 'NotificationManager' in 'tidev_hooks'` | `NotificationManager` | `engine/notifications.rs` | ✅ 已移植到 `tidev-notification` |
| `cannot find struct 'QueuedUserMessage'` | `QueuedUserMessage` | `engine/agent/runtime/types.rs` | `tidev-agent` |
| `cannot find struct 'AgentLoopConfig'` | `AgentLoopConfig` | `engine/agent/runtime/types.rs` | `tidev-agent` |
| `cannot find 'StepPatch' in 'tidev_snapshot'` | `StepPatch` | `engine/shared/undo.rs` | `tidev-snapshot` |
| `cannot find function 'collect_patches_after_message'` | `collect_patches_after_message` | `engine/shared/undo.rs` | `tidev-snapshot` |
| `cannot find 'tooling' in 'tidev_tools'` | 模块路径 `tidev_tools::tooling` | 旧模块结构 | 更新导入路径 |

### 9.6 tidev-config 内联 JSON Value 占位（~8 个错误）

| 错误 | 受影响的代码 | 原因 |
|------|------------|------|
| `no field 'name' on type '&Value'` | `ui/sync_panel.rs` | TUI 访问 `sync_config.remotes[0].name` |
| `no field 'host' on type '&Value'` | `ui/sync_panel.rs` | TUI 访问 `sync_config.remotes[0].host` |
| `no field 'last_sync_at' on type '&Value'` | `ui/sync_panel.rs` | TUI 访问 `sync_config.remotes[0].last_sync_at` |

**恢复**：见 2.2 — 用 `tidev-sync::SyncConfig & RemoteMachine` 完整类型替换 JSON Value。

### 9.7 AgentType 非迭代器（~3 个错误）

| 错误 | 受影响的代码 | 原因 |
|------|------------|------|
| `tidev_agent::AgentType is not an iterator` | `ui/agents_panel.rs` | TUI 枚举所有 agent 类型以渲染面板 |
| `tidev_agent::AgentType is not an iterator` | input/event | TUI 枚举类型做快捷键映射 |

**恢复**：为 `AgentType` 添加 `variants()` 关联函数返回 `&'static [AgentType]`。

### 9.8 shell::init 参数类型不匹配（~2 个错误）

| 错误 | 受影响的代码 | 原因 |
|------|------------|------|
| `expected &Path, found &ConfigPaths` | `core/run.rs:39` | `tidev_tools::shell::init()` 签名期望 `Option<&Path>` 但 TUI 传入 `Option<&ConfigPaths>` |

**恢复**：更新 `shell::init` 签名为接受 `Option<&ConfigPaths>`，或修复调用处转换。

### 9.9 其他类型不匹配（~12 个错误）

包括：`SessionManager` 构造参数类型不匹配、`ToolRegistry` 方法签名变化、导入路径变更等零散类型错误，分布在整个 TUI crate 中。

**恢复**：随上述主要类别修复后逐一解决。

### 9.10 初始化流程需重写

`core/run.rs` 中的 `App::new()` / `App::new_with_paths()` 需要从旧式构造迁移：

```rust
// 旧方式 — 设置 AgentRuntime 的所有字段
let agent = AgentRuntime { workspace_root, config_dir, config_paths, config, auth,
    store, llm_client, tools, instructions, ... };

// 新方式 — 使用 SessionManager API
let session_manager = SessionManager::new(store, llm);
let handle = session_manager.spawn(SessionConfig { model, tools, ... }).await;
let app = App { session_manager, config, workspace_root, ... };
```

---

## 10. 未移植的 tidev-engine 模块

以下 tidev-engine 模块在整个重写中**未移植**到任何新 crate：

| 模块                    | LOC    | 功能                         | 未移植原因                                    |
| ----------------------- | ------ | ---------------------------- | --------------------------------------------- |
| `memory/`               | ~1,500 | 记忆/图谱/保留系统           | 非核心，架构不稳定                            |
| `sandbox/`              | ~800   | bwrap/landlock/seatbelt 沙箱 | 非核心，Linux only                            |
| `provider_setup/`       | ~500   | API key 初始化流程           | 非核心                                        |
| `process.rs`            | ~46    | 进程管理（restart_self）     | 可在需要时移植                                |
| `notifications.rs`      | ~329   | 桌面通知（OSC 9 / BEL）      | ✅ 已移植到 `tidev-notification` |
| `shell.rs`（完整版）    | ~240   | 跨平台 shell 检测            | 见 6.2                                        |
| `encoding.rs`（完整版） | ~246   | 编码检测                     | 见 6.1                                        |
| `shared/undo.rs`        | ~200   | 撤销/重做补丁（StepPatch）   | 见 5.1                                        |
| `tmp.rs`（扫描/清理）    | ~175   | 扫描/清理 /tmp 中临时文件    | 仅 TmpConfig 已提取，实际逻辑未移植           |

## 11. 恢复路线图

### 修复编译错误的推荐顺序（10 步）

以下是按依赖关系排列的、清除 73 个 TUI 编译错误的逐步计划：

| 步骤 | 操作 | 修复约 | 依赖 | 状态 |
|------|------|--------|------|------|
| 1 | **移植 `StepPatch` + `collect_patches_after_message` 到 `tidev-snapshot`** | 3 个 | 无 | ✅ 完成 |
| 2 | **修复 `SyncConfig` / `HooksConfig` — 用完整类型替换 JSON Value 占位** | 8 个 | 无 | ✅ 完成 |
| 3 | **移植 `NotificationManager` 到 `tidev-notification`** | 3 个 | 无 | ✅ 完成 |
| 4 | **统一 `ApprovedTool` / `PendingToolApproval` — 添加缺失字段** | 12 个 | 无 | ✅ 完成 |
| 5 | **移植缺失类型** — `QueuedUserMessage`, `AgentLoopConfig`, `SubagentConfig`, `AgentType` 方法 | 9 个 | 步骤 1 | ✅ 完成 |
| 6 | **删除 BackendEvent 模式匹配/构造中的 `session_id`** | 7 个 | 无 | ✅ 完成 |
| 7 | **为 `AgentType` 添加缺失方法** — `description`, `is_read_only`, `default_tool_restrictions`, `default_temperature`, `all` | 4 个 | 无 | ✅ 完成 |
| 8 | **修复 `shell::init` 参数类型** — 修改调用处使用 `None` | 2 个 | 无 | ✅ 完成 |
| 9 | **修复 TUI 导入路径** — `tooling::` → `tidev_tools::`, `tidev_tools::config` → `tidev_config` | 5 个 | 无 | ✅ 完成 |
| 10 | **补全 SessionManager 公开字段和方法** — 添加 TUI 期望的 12 个字段 + 3 个方法 | 24 个 | 步骤 5,9 | ✅ 完成 |

> **总计**：所有 10 步完成后，编译错误从 **73 降至 0**。整个工作区 `cargo check --workspace` 通过。

### 编译后需恢复的核心功能

清除编译错误后，按此顺序恢复 AgentLoop 的核心功能：

| 优先级 | 功能 | 影响 | 工作量估计 |
|--------|------|------|-----------|
| P0 | **AgentLoop 工具执行** — 连接 `ToolRegistry` 实现真正的工具路由 | AgentLoop 产生实际效果 | 中 |
| P1 | **工具审批流程** — 实现 `PendingToolApproval` → 用户确认 → `ApprovedTool` 异步通道 | 用户控制工具执行 | 中 |
| P2 | **Subagent 调度** — 在 task.rs 中调用 `SessionManager::spawn()` | 子 agent 真正工作 | 小 |
| P3 | **Hook 执行** — 在 AgentLoop 循环中调用 `HookEngine` | post-tool-use 钩子触发 | 小 |
| P4 | **上下文压缩** — 在 AgentLoop 中调用 `ContextManager::compact_if_needed()` | 长对话自动压缩 | 小 |
| P5 | **LLM 重试** — 在 `run_single_turn` 中处理 `Failed` 事件 | 容错性 | 小 |
| P6 | **权限/模式检查** — 在 `execute_tool_calls` 中添加 Plan/Build 模式判断 | 安全限制 | 小 |
| P7 | **跨平台 shell/encoding** — 移植 Windows shell 检测和编码转换 | Windows 兼容性 | 中 |

---

## 总结

| Phase                             | 简化程度   | 说明                                                     |
| --------------------------------- | ---------- | -------------------------------------------------------- |
| Phase 0（归档）                   | 无简化     | 纯文件移动                                               |
| Phase 1（types + session）        | 计划内     | BackendEvent 变更是架构需求                              |
| Phase 2（config + storage + llm） | **中** | HooksConfig/SyncConfig 已完整化，logging 简化（待恢复）    |
| Phase 3（5 个基础设施 crate）     | **低** | StepPatch 已移植，canonical_tool_name 副本仍存在            |
| Phase 4（tools + mcp + context）  | **中** | encoding/shell 待完整化，task/MCP 仍为 stub                |
| Phase 5（agent）                  | **高** | 工具执行/审批/子agent/hooks/压缩/重试均为 stub，类型已就绪 |
| Phase 6（tui）                    | **✅ 已修复** | 73 个编译错误已全部清除                                    |
| Phase 7（清理）                   | 未开始     | —                                                        |

最需要优先恢复的功能：

1. **AgentLoop 工具执行** — 连接 `ToolRegistry` 实现真正的工具路由
2. **工具审批流程** — 实现 `PendingToolApproval` → 用户确认 → `ApprovedTool` 异步通道
3. **Subagent 调度** — 在 task.rs 中调用 `SessionManager::spawn()`
4. **Hook 执行** — 在 AgentLoop 循环中调用 `HookEngine`
5. **上下文压缩** — 在 AgentLoop 中调用 `ContextManager::compact_if_needed()`
6. **跨平台 shell/encoding** — 移植 Windows shell 检测和编码转换
