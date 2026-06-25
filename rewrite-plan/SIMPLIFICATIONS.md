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
9. [Phase 6: tidev-tui（未完成）](#9-phase-6-tidev-tui未完成)
10. [未移植的 tidev-engine 模块](#10-未移植的-tidev-engine-模块)

---

## 1. Phase 1: tidev-session

### 1.1 balance 模块未移植

| 项目 | 说明 |
|------|------|
| **原始位置** | `tidev-session/src/balance/` |
| **内容** | Token/fee 记账系统，用于跟踪 API 调用费用 |
| **简化** | 整个模块未复制到新 crate |
| **原因** | 归档中 balance 目录为空（无 .rs 文件），确认 git 也未跟踪该目录 |
| **影响** | 费用跟踪功能不可用 |
| **恢复** | 从原始 git history 恢复或重新实现 |

### 1.2 BackendEvent 结构变更（计划内）

| 项目 | 说明 |
|------|------|
| **变更** | 删除所有变体中的 `session_id` 字段 |
| **状态** | **计划内**，Per-Session Event Bus 架构需求 |
| **影响** | 所有 BackendEvent 消费者（TUI、web）需要更新匹配模式 |

### 1.3 Subagent 事件删除（计划内）

| 项目 | 说明 |
|------|------|
| **删除** | `SubagentStatus`, `SubagentToolResult`, `SubagentCompleted` |
| **状态** | **计划内**，前端直接订阅子 session 通道 |
| **影响** | TUI 和 web 前端中的对应处理逻辑需要删除 |

---

## 2. Phase 2: tidev-config

### 2.1 HooksConfig 内联占位

| 项目 | 说明 |
|------|------|
| **原始** | `PostToolUseHookConfig` 有完整字段和方法（matcher, command, extensions, timeout 等） |
| **当前** | `HooksConfig` 只有两个字段：`disable_all_hooks: bool`, `post_tool_use: Vec<serde_json::Value>` |
| **原因** | 避免依赖 `tidev-hooks` crate（Phase 3 之前不存在），使用 JSON Value 占位 |
| **影响** | 钩子系统配置不完整，无法在 config.toml 中精细定义钩子 |
| **恢复** | Phase 3 后从 `tidev-hooks` 重新导入完整 `HooksConfig` |

### 2.2 SyncConfig 内联占位

| 项目 | 说明 |
|------|------|
| **原始** | `SyncConfig` + `RemoteMachine`（含 `create_transport()`, `test_connection()` 等方法） |
| **当前** | `SyncConfig` 只有 `remotes: Vec<serde_json::Value>` 占位 |
| **原因** | 避免依赖 `tidev-sync` crate |
| **影响** | 配置中的远程机器无法验证连接 |
| **恢复** | 从 `tidev-sync::RemoteMachine` 替换占位 |

### 2.3 logging::init() 简化

| 项目 | 说明 |
|------|------|
| **原始** | `engine/logging.rs` — 完整的文件日志轮转系统（大小限制、文件数限制、异步刷新） |
| **当前** | 简单的 stderr 日志输出，使用 `fern` crate |
| **原因** | 原始实现包含约 100 行的复杂日志管理 |
| **影响** | 日志不会写入文件，只有 stderr 控制台输出 |
| **恢复** | 还原原始日志轮转实现 |

---

## 3. Phase 3: tidev-hooks

### 3.1 canonical_tool_name 搬入 hooks（可接受）

| 项目 | 说明 |
|------|------|
| **原始** | 位于 `engine/tooling/mod.rs` |
| **当前** | 位于 `tidev-hooks/src/canonical.rs`，同时 `tidev-tools/src/lib.rs` 也有独立副本 |
| **原因** | 提取 hooks 时无法引用还不存在的 tidev-tools |
| **影响** | 两个 crate 中重复定义，可能不同步 |
| **恢复** | 统一放到 `tidev-types` 或一个共享位置 |

### 3.2 HooksConfig 未使用完整类型

| 项目 | 说明 |
|------|------|
| **原始** | `PostToolUseHookConfig` — 完整的钩子定义（matcher, command, extensions, cwd, timeout, status_message, name） |
| **当前** | `tidev-config` 中使用 `Vec<serde_json::Value>` 占位 |
| **原因** | tidev-config 在 Phase 2 创建，tidev-hooks 在 Phase 3 创建 |
| **影响** | 见 2.1 |

---

## 4. Phase 3: tidev-instructions

⚠️ **无已知简化**。代码完整地从 `engine/instructions.rs` 提取，6 个测试全部通过。

---

## 5. Phase 3: tidev-snapshot

### 5.1 缺失 shared/undo.rs 中的类型

| 项目 | 说明 |
|------|------|
| **缺失** | `StepPatch`, `extract_patches_from_message`, `collect_patches_from_message`, `collect_patches_after_message` |
| **原始** | `engine/shared/undo.rs` — 约 200 行的撤销/重做补丁管理 |
| **原因** | 这些类型被 TUI 和 web 前端共享，属于 undo 系统的核心部分 |
| **影响** | TUI 中引用 `tidev_snapshot::StepPatch` 和 `collect_patches_after_message` 的地方编译失败 |
| **恢复** | 将 `shared/undo.rs` 的内容移植到 `tidev-snapshot` |

---

## 6. Phase 4: tidev-tools

### 6.1 encoding.rs 大幅简化

| 项目 | 说明 |
|------|------|
| **原始** | `engine/encoding.rs` — 使用 `encoding_rs` crate 做真实编码检测（UTF-8, GBK, Shift-JIS 等） |
| **当前** | `decode_command_output` 直接用 `String::from_utf8_lossy` |
| **原因** | 简化依赖，`encoding_rs` 增加编译时间 |
| **影响** | 非 UTF-8 输出（如中文 Windows GBK）可能乱码 |
| **恢复** | 添加 `encoding_rs` 依赖并还原原始检测逻辑 |

### 6.2 shell.rs 大幅简化

| 项目 | 说明 |
|------|------|
| **原始** | `engine/shell.rs` — 完整跨平台 shell 检测（Windows Git Bash/MSYS2/PowerShell auto-detect + 持久化），约 150 行 |
| **当前** | 只在 Linux/macOS 上尝试 `/bin/bash`, `/bin/zsh`, `/bin/sh` |
| **原因** | 简化跨平台逻辑 |
| **影响** | Windows 上 shell 检测完全不可用 |
| **恢复** | 将原始 `engine/shell.rs` 移植过来 |

### 6.3 shell::init 为空函数

| 项目 | 说明 |
|------|------|
| **原始** | 持久化用户配置的 Windows shell 选择到 config.toml |
| **当前** | 空函数，什么都不做 |
| **原因** | shell 初始化逻辑包含在原始 `shell.rs` 中，未移植 |
| **影响** | Windows 首次使用的 shell 选择不会被保存 |
| **恢复** | 随 shell.rs 一起移植 |

### 6.4 Task 工具中 AgentType 为内联定义

| 项目 | 说明 |
|------|------|
| **原始** | `engine/agent/AgentType` — 完整的 agent 类型枚举 |
| **当前** | `tidev-tools/src/agent.rs` 中内联定义，`tidev-agent/src/types.rs` 中也有独立定义 |
| **原因** | `task.rs` 引用 `crate::agent::AgentType`，但 tidev-agent 在 Phase 5 才创建 |
| **影响** | 两个副本可能不同步 |
| **恢复** | 统一放到 `tidev-agent`，tidev-tools 从那里导入 |

### 6.5 execute_tool_calls 返回占位结果

| 项目 | 说明 |
|------|------|
| **原始** | `tools.rs` 中的 `execute_tool_calls` → `execute_shell_tool_call` 路由到注册的工具处理函数 |
| **当前** | `AgentLoop::execute_tool_calls` 返回占位字符串 |
| **原因** | `ToolRegistry` 方法签名在新架构中变更，还未完全接入 |
| **影响** | AgentLoop 中的工具执行不会产生实际效果 |
| **恢复** | 将 `execute_tool_calls` 连接到 `ToolRegistry` |

---

## 7. Phase 4: tidev-mcp

### 7.1 ToolDefinition 为简化版

| 项目 | 说明 |
|------|------|
| **原始** | `engine/tooling/ToolDefinition` 含 `ToolOrigin` 枚举（`Local` / `Mcp { server_name, tool_name }`） |
| **当前** | `tidev-mcp::types::ToolDefinition` 为简化版，直接包含 `server_name` 和 `remote_tool_name` 字段 |
| **原因** | 避免引入 `ToolOrigin` 枚举依赖 |
| **影响** | 功能等价但类型不同，需要 `From` 转换 |
| **恢复** | 无需要，设计更干净 |

### 7.2 parse_tool 中 display_name 被省略

| 项目 | 说明 |
|------|------|
| **原始** | 从 MCP 工具定义中读取 `title` 字段作为 `display_name` |
| **当前** | display_name 在 `ToolDefinition::mcp()` 内部自动构建 |
| **原因** | 简化构造逻辑 |
| **影响** | MCP 工具自定义 display_name 不会被使用 |
| **恢复** | 在 `mcp()` 构造函数中检查并传递 display_name |

---

## 8. Phase 5: tidev-agent

这是**最大的简化区域**，因为 tidev-agent 是全新编写的代码（非提取）。

### 8.1 AgentLoop 工具执行为占位

| 项目 | 说明 |
|------|------|
| **原始** | `AgentRuntime::execute_tool_calls` → 通过 `ToolRegistry` 路由到 20+ 内置工具 + MCP |
| **当前** | `AgentLoop::execute_tool_calls` 返回占位字符串 `"Executed tool 'X' (standalone mode)"` |
| **原因** | 需要集成 `ToolRegistry`，但架构尚未确定如何连接 |
| **影响** | AgentLoop 无法真正执行任何工具 |
| **恢复** | 实现 `execute_tool_calls` → `ToolRegistry::execute` 路由 |

### 8.2 MCP 工具执行为 stub

| 项目 | 说明 |
|------|------|
| **当前** | `try_execute_mcp` 直接 `bail!("not implemented")` |
| **影响** | MCP 工具在 AgentLoop 中不可用 |
| **恢复** | 连接到 `tidev_mcp::McpManager::execute_call` |

### 8.3 工具审批流程未实现

| 项目 | 说明 |
|------|------|
| **原始** | `PendingToolApproval` → 用户确认 → `ApprovedTool` 的异步审批通道 |
| **当前** | 类型已定义（`PendingToolApproval`, `ApprovedTool`）但未接入 AgentLoop |
| **影响** | 所有工具调用自动执行，无用户确认 |
| **恢复** | 在 `execute_tool_calls` 中实现审批通道 |

### 8.4 子 agent 调度未实现

| 项目 | 说明 |
|------|------|
| **原始** | `SubagentConfig` → `run_subagent_inner` → 递归创建子 session |
| **当前** | `SessionManager::spawn` 支持 `parent_session_id`，但 task 工具未接入 |
| **影响** | task 工具返回占位字符串，不会真正创建子 agent |
| **恢复** | 在 `task.rs` 中调用 `SessionManager::spawn` |

### 8.5 Hook 执行未实现

| 项目 | 说明 |
|------|------|
| **原始** | `AgentRuntime` 在工具执行后调用 `HookEngine::run_post_tool_use_hooks` |
| **当前** | AgentLoop 中无钩子调用 |
| **影响** | post-tool-use 钩子不会触发 |
| **恢复** | 在 `execute_tool_calls` 后添加 `HookEngine` 调用 |

### 8.6 上下文压缩未实现

| 项目 | 说明 |
|------|------|
| **原始** | `ContextManager::compact_if_needed`, `compact`, `schedule_context_compaction` |
| **当前** | AgentLoop 中无压缩逻辑 |
| **影响** | 长对话不会自动压缩上下文 |
| **恢复** | 在 AgentLoop 循环中添加压缩检查 |

### 8.7 重试逻辑未实现

| 项目 | 说明 |
|------|------|
| **原始** | `stream_chat_with_retry` 在 tidev-llm 层实现（最多 MAX_RETRIES 次） |
| **当前** | tidev-llm 的带重试版本在提取后被保留，但 tidev-agent 的 AgentLoop 不处理 Failed 事件的重试 |
| **影响** | LLM 请求失败后直接抛出错误 |
| **恢复** | 在 `run_single_turn` 中添加重试循环 |

### 8.8 权限检查未实现

| 项目 | 说明 |
|------|------|
| **原始** | `PermissionConfig`, `ToolPermission::is_allowed_in` 控制 Plan/Build 模式 |
| **当前** | AgentLoop 不检查工具权限 |
| **影响** | Plan 模式下也可以执行写操作工具 |
| **恢复** | 在 `execute_tool_calls` 中添加权限检查 |

### 8.9 单元测试缺失

| 项目 | 说明 |
|------|------|
| **原始** | `agent/runtime/tests.rs` — 完整的 agent loop 测试 |
| **当前** | tidev-agent 中 0 个测试 |
| **原因** | 全新编写，尚未添加测试 |
| **恢复** | 编写 MockSessionStore + MockLlmClient 测试 |

---

## 9. Phase 6: tidev-tui（未完成）

TUI 移植**未完成**，当前编译状态：74 个错误。以下是已识别但未修复的问题：

### 9.1 未消除的 tidev-engine 依赖

| 类型 | 数量 |
|------|------|
| 缺失的类型导入（`StepPatch`, `NotificationManager` 等） | ~10 |
| BackendEvent session_id 字段残留 | ~12 |
| AgentRuntime → SessionManager 字段不匹配 | ~10 |
| SessionManager 缺失的方法调用 | ~5 |
| ApprovedTool / PendingToolApproval 字段变更 | ~8 |
| 其他类型不匹配 | ~30 |

### 9.2 关键的未移植依赖

| 类型 | 原始位置 | 需要移至 |
|------|---------|---------|
| `StepPatch` | `engine/shared/undo.rs` | `tidev-snapshot` |
| `collect_patches_after_message` | `engine/shared/undo.rs` | `tidev-snapshot` |
| `NotificationManager` | `engine/notifications.rs` | `tidev-hooks` |
| `QueuedUserMessage` | `engine/agent/runtime/types.rs` | `tidev-agent` |
| `AgentLoopConfig` | `engine/agent/runtime/types.rs` | `tidev-agent` |
| `NotificationConfig` | `engine/config/mod.rs` | `tidev-config`（部分已存在） |

### 9.3 初始化代码需重写

`core/run.rs` 中的 `App::new()` / `App::new_with_paths()` 创建 `AgentRuntime` 并手动设置其所有字段。需要重写为：

```rust
// 旧方式
let agent = AgentRuntime {
    workspace_root: ...,
    config_dir: ...,
    config_paths: ...,
    config: ...,
    auth: ...,
    store: ...,
    llm_client: ...,
    tools: ...,
    // ... 更多字段
};

// 新方式
let store = Arc::new(Mutex::new(store));
let llm = LlmClient::new(...);
let session_manager = SessionManager::new(store, llm);
let handle = session_manager.spawn(SessionConfig {
    model: active_model,
    tools: tool_definitions,
    store: store.clone(),
    // ...
}).await;
```

---

## 10. 未移植的 tidev-engine 模块

以下 tidev-engine 模块在整个重写中**未移植**到任何新 crate：

| 模块 | LOC | 功能 | 未移植原因 |
|------|-----|------|-----------|
| `memory/` | ~1,500 | 记忆/图谱/保留系统 | 非核心，架构不稳定 |
| `sandbox/` | ~800 | bwrap/landlock/seatbelt 沙箱 | 非核心，Linux only |
| `provider_setup/` | ~500 | API key 初始化流程 | 非核心 |
| `process.rs` | ~100 | 进程管理（kill children） | 可在需要时移植 |
| `notifications.rs` | ~200 | 桌面通知 | 可在需要时移植 |
| `shell.rs`（完整版） | ~150 | 跨平台 shell 检测 | 见 6.2 |
| `encoding.rs`（完整版） | ~100 | 编码检测 | 见 6.1 |
| `shared/undo.rs` | ~200 | 撤销/重做补丁 | 见 5.1 |
| `shared/file_search.rs` | ~400 | 文件搜索 | 已提取为 `tidev-search`，但部分辅助函数未移植 |

---

## 总结

| Phase | 简化程度 | 说明 |
|-------|---------|------|
| Phase 0（归档） | 无简化 | 纯文件移动 |
| Phase 1（types + session） | 计划内 | BackendEvent 变更是架构需求 |
| Phase 2（config + storage + llm） | **中** | HooksConfig/SyncConfig 占位，logging 简化 |
| Phase 3（5 个基础设施 crate） | **低** | hooks 中 canonical_tool_name 副本 |
| Phase 4（tools + mcp + context） | **中** | encoding/shell 简化，AgentType 副本 |
| Phase 5（agent） | **高** | 全新代码，工具执行/审批/子agent/hooks/压缩/重试均为 stub |
| Phase 6（tui） | **未完成** | 74 个编译错误待修复 |
| Phase 7（清理） | 未开始 | — |

最需要优先恢复的功能：
1. **Phase 5 AgentLoop** — 工具执行、审批流程
2. **Phase 2 tidev-config** — HooksConfig/SyncConfig 完整化
3. **Phase 4 tidev-tools** — encoding/shell 完整化
