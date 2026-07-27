# tidev 适配 ACP（Agent Client Protocol）可行性分析报告

## 一、概述

本文档分析 tidev 适配 ACP（Agent Client Protocol）的可行性、潜在挑战和解决方法。

**结论**：tidev 完全可以适配 ACP，预计工作量 2.5-4 周。ACP 协议覆盖 tidev 约 85% 的核心功能，未覆盖部分可通过 Agent 内部透明化处理。

## 二、ACP 协议简介

ACP 是一个基于 JSON-RPC 2.0 的开放协议，标准化了代码编辑器（Client）和编码 Agent 之间的通信。当前稳定版本为 v1，v2 处于 Draft 阶段。

- 官方仓库：https://github.com/agentclientprotocol/agent-client-protocol
- 官方 Rust SDK：[`agent-client-protocol`](https://crates.io/crates/agent-client-protocol)（已到 1.0）
- 文档站点：https://agentclientprotocol.com

### 2.1 协议生命周期

```
initialize → session/new → session/prompt → session/update (notifications) → idle
                                    ↑                                        │
                                    └────────────────────────────────────────┘
```

### 2.2 Agent 必须实现的方法

| 方法 | 方向 | 说明 |
|------|------|------|
| `initialize` | Client→Agent | 协商协议版本和能力 |
| `new_session` | Client→Agent | 创建会话（传入 cwd + MCP servers） |
| `load_session` | Client→Agent | 恢复已有会话 |
| `session/prompt` | Client→Agent | 发送用户消息 |
| `session/cancel` | Client→Agent | 取消当前任务 |
| `session/close` | Client→Agent | 关闭会话 |
| `session/update` | Agent→Client | 推送状态更新（核心通知） |
| `session/request_permission` | Agent→Client | 请求用户授权工具执行 |

### 2.3 Agent 可选实现的方法

| 方法 | 说明 |
|------|------|
| `authenticate` / `auth/logout` | 认证（仅当 advertise authMethods 时） |
| `set_session_mode` | 设置会话模式（如 plan/build） |
| `elicitation/create` | 向用户请求结构化信息（v2 新增） |

### 2.4 `session/update` 通知类型

| 类型 | 说明 |
|------|------|
| `user_message` | 用户消息确认 |
| `agent_message` | Agent 消息 upsert（完整替换语义） |
| `agent_message_chunk` | Agent 消息流式 chunk |
| `tool_call_update` | 工具调用状态更新（pending→in_progress→completed/error） |
| `tool_call_content_chunk` | 工具调用输出流式 chunk |
| `plan_update` | 执行计划更新（item-based） |
| `state_update` | 会话状态（idle/running/requires_action） |
| `cost_update` | 费用信息 |
| `context_update` | 上下文窗口使用情况 |

### 2.5 传输层

ACP 使用 JSON-RPC over **stdio**（subprocess 模式）。Client 启动 Agent 作为子进程，通过 stdin/stdout 通信。Streamable HTTP 仍在 draft 中。

## 三、tidev 当前架构

### 3.1 Crate 结构

```
tidev (binary)
├── tidev-types     — 共享类型（Message, ToolDefinition, SessionMode 等）
├── tidev-agent     — Agent 循环骨架（run_agent_loop, AgentContext trait）
├── tidev-core      — 运行时编排（Runtime, SessionManager, ContextManager, CoreContext）
├── tidev-tui       — 终端 UI（ratatui，~96 个源文件）
├── tidev-llm       — LLM 客户端
├── tidev-tools     — 工具实现（shell, file, search, task, todo, web 等）
├── tidev-storage   — SQLite 存储
├── tidev-config    — 配置管理
├── tidev-snapshot  — undo/redo 快照
├── tidev-search    — 文件搜索（@mention autocomplete）
├── tidev-instructions — 指令注入（AGENTS.md 等）
├── tidev-logging   — 日志
└── tidev-utils     — 工具函数
```

### 3.2 依赖方向

```
tidev-tui → tidev-core → tidev-agent → tidev-types
                       → tidev-llm
                       → tidev-tools
                       → tidev-storage
                       → tidev-config
                       → tidev-snapshot
                       → tidev-instructions
                       → tidev-search
```

**关键**：`tidev-core` 不依赖 `tidev-tui`。TUI 是 core 的消费者，不是提供者。这意味着 Runtime 可以脱离 TUI 独立运行。

### 3.3 核心通信模型

Runtime 通过两个 tokio channel 与外部通信：

- `event_tx: UnboundedSender<BackendEvent>` — 向前端推送实时事件
- `request_tx: UnboundedSender<TuiRequest>` — 向前端发送权限请求

TUI 消费这两个 channel。ACP adapter 可以替代 TUI 成为新的消费者。

### 3.4 BackendEvent 完整变体

定义在 `crates/tidev-types/src/message.rs:528-652`，共 20 个变体：

| 变体 | 说明 |
|------|------|
| `Delta` | LLM 流式文本输出 |
| `ReasoningDelta` | LLM 流式推理输出 |
| `ToolCallUpdated` | 工具调用参数流式更新 |
| `ToolStarting` | 工具开始执行 |
| `ToolCompleted` | 工具执行完成 |
| `Finished` | LLM turn 完成 |
| `Failed` | LLM turn 失败 |
| `Retrying` | LLM 请求重试 |
| `TurnStarting` | 新 turn 开始 |
| `StreamEnd` | 流式输出结束 |
| `UsageStats` | token 使用统计 |
| `SubagentStatus` | 子 agent 状态更新 |
| `SubagentCompleted` | 子 agent 完成 |
| `UserMessageCreated` | 用户消息已创建 |
| `InstructionsLoaded` | 指令文件已加载 |
| `ContextCompacted` | 上下文压缩完成 |
| `UndoCompleted` | undo 完成 |
| `SidebarSnapshotReady` | 侧边栏快照就绪 |
| `ShellOutput` | shell 命令流式输出 |
| `MessagesTruncated` | 消息已截断 |

### 3.5 权限系统

**当前流程**（`crates/tidev-core/src/agent_ctx.rs:576-747`）：

```
request_tool_approval(tool_calls, mode)
  ├─ 1. can_execute() — 模式级权限检查
  ├─ 2. DB remembered permission — 用户记住的权限
  ├─ 3. workspace_boundary_violation — 工作区边界检查
  ├─ 4. sensitive_file_violation — 敏感文件检查
  ├─ 5. auto-approve if no violations
  └─ 6. send TuiRequest to TUI — 等待用户决策
```

**简化后**（用户计划）：

```
request_tool_approval(tool_calls, mode)
  ├─ 1. can_execute() — 硬编码模式检查，直接 reject
  ├─ 2. workspace_boundary_violation — 工作区边界检查
  ├─ 3. sensitive_file_violation — 敏感文件检查
  └─ 4. question tool — 强制路由到前端
```

删除：DB remembered permission、PermissionDialog、ToolCallWithViolations 中的 permission_key/permission_label/needs_confirmation。

### 3.6 MCP 集成

Runtime 通过 `McpManager`（`crates/tidev-core/src/mcp.rs`）管理 MCP 服务器：

- 从 `AppConfig` 读取 MCP 服务器配置
- `McpManager::refresh_all()` 连接所有服务器并发现工具
- `McpManager::upsert_server()` 支持动态添加服务器
- 已支持 stdio 和 HTTP transport（`rmcp` crate）
- MCP 工具与 built-in 工具合并到 `ToolRegistry`

### 3.7 子 Agent 机制

tidev 支持多 agent 类型（General、Explorer、Librarian、Oracle、Fixer），通过 `task` 工具委托子任务：

- 子 agent 有独立的 session（child_session_id）
- 子 agent 有独立的 tool 集合（根据 agent_type 过滤）
- 子 agent 可以使用不同的 model
- 子 agent 执行期间 parent 处于等待状态
- 子 agent 不能嵌套（`task` 工具被过滤掉）
- 实现位于 `crates/tidev-core/src/agent_ctx.rs:1307-1597`

## 四、ACP 适配方案

### 4.1 推荐架构

新增 `tidev-acp` crate，tidev 同时支持两种模式：

```
tidev --tui          # 独立 TUI 模式（现有行为不变）
tidev --acp-stdio    # ACP Agent 模式（作为子进程被 Client 驱动）
```

### 4.2 核心映射关系

| ACP 概念 | tidev 对应 |
|----------|-----------|
| `initialize` | 构建 Runtime，返回 capabilities |
| `session/new` | `Runtime::create_default_session()` + 连接 MCP servers |
| `session/resume` | 加载已有 session，重建 buffer + context |
| `session/prompt` | `Runtime::submit_prompt()` |
| `session/cancel` | `Runtime::cancel_session()` |
| `session/close` | 清理 session 资源 |
| `session/update` (agent_message) | `BackendEvent::Delta` + `BackendEvent::Finished` |
| `session/update` (tool_call_update) | `BackendEvent::ToolStarting` + `BackendEvent::ToolCompleted` |
| `session/update` (tool_call_content_chunk) | `BackendEvent::ShellOutput` |
| `session/update` (state_update) | idle/running 状态追踪 |
| `session/request_permission` | `TuiRequest`（boundary/sensitive/question） |
| `set_session_mode` | 切换 `SessionMode::Plan` / `SessionMode::Build` |
| `plan_update` | `BackendEvent` 中的 todo 信息 |

### 4.3 BackendEvent → ACP session/update 映射

| BackendEvent | ACP session/update | 复杂度 |
|---|---|---|
| `Delta` | `agent_message_chunk` | ⭐ 简单 |
| `ReasoningDelta` | `agent_message_chunk` (reasoning) | ⭐ 简单 |
| `ToolCallUpdated` | `tool_call_update` (参数更新) | ⭐ 简单 |
| `ToolStarting` | `tool_call_update { status: in_progress }` | ⭐ 简单 |
| `ToolCompleted` | `tool_call_update { status: completed }` + `tool_call_content_chunk` | ⭐⭐ 中等 |
| `Finished` | 无需单独映射 — 由各 chunk 累积 | — |
| `Failed` | error notification | ⭐ 简单 |
| `ShellOutput` | `tool_call_content_chunk` | ⭐⭐ 中等 |
| `SubagentStatus` | `tool_call_update` (内部透明化) | ⭐⭐ 中等 |
| `SubagentCompleted` | `tool_call_update { status: completed }` | ⭐⭐ 中等 |
| `UserMessageCreated` | `user_message` | ⭐ 简单 |
| `UsageStats` | `context_update` | ⭐ 简单 |
| 其余事件 | 忽略（TUI 专属或内部优化） | — |

### 4.4 权限映射

简化后的权限模型完美匹配 ACP 的 `session/request_permission`：

| 触发场景 | ACP 处理 |
|----------|----------|
| workspace boundary violation | `session/request_permission`（描述包含越界路径）→ approve/deny |
| sensitive file violation | `session/request_permission`（描述包含敏感文件路径）→ approve/deny |
| question tool | ACP v2: `elicitation/create`；v1 fallback: `session/request_permission` |

### 4.5 MCP 集成

ACP Client 通过 `session/new` 传入 MCP servers。合并策略：

1. 从 `NewSessionRequest.mcp_servers` 获取 Client 的 servers
2. 转换为 `McpServerConfig` 格式
3. 调用 `McpManager::upsert_server()` 添加/更新
4. 连接并发现 tools
5. 与 tidev 自身配置的 servers 合并（Client 的优先级更高）

### 4.6 Session ID 映射

tidev 使用 `Uuid`，ACP 使用 `String`。直接用 `Uuid::to_string()` 作为 ACP session ID，无需额外映射表。

### 4.7 Subagent 处理

推荐**透明化方案**：子 agent 的执行完全在 Agent 内部完成，ACP Client 只看到最终结果。

- `tool_call_update` 报告 `task` 工具调用
- 子 agent 执行期间 parent 处于 `running` 状态
- 子 agent 完成后，结果作为 `tool_call_content_chunk` 推送
- Client 看到的是一个普通的工具调用，不知道内部有 subagent

理由：
1. 实现最简单 — 子 agent 执行逻辑不变
2. 符合 ACP 设计哲学 — Agent 内部实现细节不暴露给 Client
3. 其他 ACP agent 也有类似内部机制，对 Client 透明

## 五、功能兼容性评估

### 5.1 ✅ 完全支持

| tidev 功能 | ACP 对应 |
|---|---|
| LLM 对话 | `session/prompt` + `agent_message` |
| 流式输出 | `agent_message_chunk` |
| 工具调用 | `tool_call_update` + `tool_call_content_chunk` |
| 工具权限审批（简化后） | `session/request_permission` |
| 取消任务 | `session/cancel` |
| 会话管理 | `new_session` / `load_session` / `session/close` |
| MCP 工具集成 | `capabilities.session.mcp` |
| 图片输入 | `ContentBlock::Image` |
| Plan 模式 | `set_session_mode` + `plan_update` |
| 文件 Diff 展示 | `Diff` content block |
| 结构化 Todo | `plan_update` (item-based) |
| 推理/思考 | `agent_message_chunk` (reasoning) |

### 5.2 ⚠️ 部分支持（需要适配）

| tidev 功能 | ACP 对应 | 适配方案 |
|---|---|---|
| Subagent | 无直接对应 | 透明化处理 |
| Question 工具 | `elicitation/create` (v2) | v2 直接映射；v1 用 permission request fallback |
| Undo/Redo | 无直接对应 | Agent 内部处理，不暴露给 Client |
| Context Compaction | 无直接对应 | Agent 内部优化 |
| Shell 输出流 | `tool_call_content_chunk` | 翻译为 tool call content |

### 5.3 ❌ 无法支持（ACP 协议限制）

| tidev 功能 | 说明 |
|---|---|
| 多 session 并发 | ACP 是单 session 模式 — 一个连接一个 session |
| Sidebar 快照 | TUI 专属功能 |
| TUI 主题/渲染 | TUI 专属 |
| 文件搜索索引 (@mention) | TUI 专属功能 |

### 5.4 功能覆盖率

**约 85%** 的 tidev 核心功能可通过 ACP 直接或间接支持。未覆盖部分主要是 TUI 专属功能和多 session 并发（ACP 协议限制）。

## 六、挑战与解决方法

### 6.1 高难度挑战

#### 挑战 1：CoreContext 的 request_tx 绑定

`CoreContext` 构造时硬编码了 `request_tx: UnboundedSender<TuiRequest>`。

**解决方法（方案 B — bridge task）**：在 ACP adapter 中运行一个额外的 task，消费 `request_rx`，将 `TuiRequest` 翻译为 ACP permission request，然后将 Client 的响应通过 `response_tx` 回传。不需要修改 `CoreContext`。

#### 挑战 2：BackendEvent → ACP session/update 的完整翻译

20 个 BackendEvent 变体中，约 10 个需要翻译为 ACP session/update 通知。

**解决方法**：实现 `AcpEventTranslator` struct，内部维护：
- `message_id` 计数器（ACP 要求每个消息有唯一 ID）
- `tool_call_id` 映射
- state 追踪（idle/running/requires_action）

### 6.2 中等难度挑战

#### 挑战 3：ACP prompt 的异步语义

ACP 的 `session/prompt` 在 Agent 接受消息后立即返回，然后通过 `session/update` 推送后续内容。tidev 的 `submit_prompt` 也是 fire-and-forget。

**解决方法**：两者语义匹配。在 ACP adapter 中：
1. 调用 `submit_prompt()`
2. 立即返回 `PromptResponse`
3. 后续 events 通过 BackendEvent channel 接收并转发

#### 挑战 4：MCP servers 合并

Client 传入的 MCP servers 需要与 tidev 配置文件中的合并。

**解决方法**：在 `new_session` handler 中，将 Client 的 servers 通过 `McpManager::upsert_server()` 动态添加，与 tidev 自身配置合并。

#### 挑战 5：message_id 分配

ACP 要求每个消息有唯一 `messageId`，tidev 的消息 ID 是 `Uuid`。

**解决方法**：在 ACP adapter 层维护一个 `message_id` 计数器。每次 `TurnStarting` 时分配新的 `messageId`。

### 6.3 低难度挑战

| 挑战 | 解决方法 |
|------|---------|
| Session ID 类型差异（Uuid vs String） | 直接用 `Uuid::to_string()` |
| stdio transport | ACP SDK 的 `AgentSideConnection` 已封装 |
| capability 声明 | 根据 tidev 实际能力返回 |

## 七、实现路径

### Phase 1：最小可用 ACP Agent（~1 周）

**目标**：tidev 能作为 ACP agent 被 Zed/VS Code 等 Client 驱动，支持基本对话。

新增 crate：`tidev-acp`

实现内容：
1. `tidev-acp` crate 骨架 + `agent-client-protocol` SDK 依赖
2. 实现 `acp::Agent` trait：`initialize`、`new_session`、`prompt`、`cancel`
3. BackendEvent → ACP session/update 翻译器（Delta、Finished、Failed）
4. stdio transport 启动入口
5. CLI 子命令 `tidev --acp-stdio`

### Phase 2：权限和 MCP（~1 周）

实现内容：
1. TuiRequest bridge task（boundary/sensitive → ACP permission request）
2. Client MCP servers 合并
3. 完整的工具调用报告（ToolCallUpdate、ToolCallContent）

### Phase 3：完整功能（~1 周）

实现内容：
1. `session/resume` + 历史重放
2. `session/close`
3. Subagent 透明化处理
4. `elicitation/create` 支持（question tool）
5. Plan 模式 + `plan_update`
6. Error handling 和 edge cases

## 八、工作量估算

| 模块 | 工作内容 | 预估时间 |
|------|---------|---------|
| `tidev-acp` crate 骨架 | 新 crate，依赖 ACP SDK，实现 agent trait | 1-2 天 |
| stdio transport | stdio JSON-RPC 消息读写 | 1 天 |
| initialize / 能力协商 | 实现 initialize handler | 1 天 |
| session 生命周期 | session/new、session/resume | 2-3 天 |
| prompt → Runtime 桥接 | session/prompt → submit_prompt() | 1-2 天 |
| BackendEvent → session/update 翻译 | 最复杂的部分 | 5-7 天 |
| 权限请求桥接 | TuiRequest → ACP permission request | 1 天 |
| MCP 集成 | Client MCP servers 合并 | 1-2 天 |
| Subagent 透明化 | 子 agent 结果翻译 | 1-2 天 |
| CLI 入口适配 | --acp-stdio 启动模式 | 1 天 |
| 测试与集成 | 端到端测试，与真实 Client 联调 | 3-5 天 |
| **总计** | | **~2.5-4 周** |

## 九、风险评估

| 风险 | 影响 | 缓解措施 |
|------|------|---------|
| ACP v2 Draft 可能变更 | 中 | 先基于 v1 实现，v2 稳定后迁移 |
| 子 agent 的 ACP 映射 | 低 | 透明化方案简单可靠 |
| ACP SDK 版本依赖 | 低 | 已到 1.0 稳定版 |
| 多 session 并发不支持 | 低 | ACP 协议限制，可接受 |
| Question 工具在 v1 的映射 | 低 | 用 permission request fallback |

## 十、附录

### A. ACP Agent trait 接口

```rust
#[async_trait(?Send)]
pub trait Agent {
    async fn initialize(&self, args: InitializeRequest) -> Result<InitializeResponse, Error>;
    async fn authenticate(&self, args: AuthenticateRequest) -> Result<AuthenticateResponse, Error>;
    async fn new_session(&self, args: NewSessionRequest) -> Result<NewSessionResponse, Error>;
    async fn load_session(&self, args: LoadSessionRequest) -> Result<LoadSessionResponse, Error>;
    async fn set_session_mode(&self, args: SetSessionModeRequest) -> Result<SetSessionModeResponse, Error>;
    async fn prompt(&self, args: PromptRequest) -> Result<PromptResponse, Error>;
    async fn cancel(&self, args: CancelNotification) -> Result<(), Error>;
    async fn close_session(&self, args: CloseSessionNotification) -> Result<(), Error>;
    // ...
}
```

### B. 关键源文件索引

| 文件 | 说明 |
|------|------|
| `crates/tidev-types/src/message.rs:528-652` | BackendEvent 完整定义 |
| `crates/tidev-agent/src/context.rs:57-114` | ApprovedTool / TuiRequest / TuiResponse |
| `crates/tidev-agent/src/context.rs:125-220` | AgentContext trait |
| `crates/tidev-core/src/agent_ctx.rs:186-236` | CoreContext 结构体 |
| `crates/tidev-core/src/agent_ctx.rs:576-747` | request_tool_approval 实现 |
| `crates/tidev-core/src/agent_ctx.rs:1307-1597` | Subagent 支持 |
| `crates/tidev-core/src/runtime.rs:75-140` | Runtime 结构体 |
| `crates/tidev-core/src/runtime.rs:386-465` | submit_prompt / submit_prompt_with_attachments |
| `crates/tidev-core/src/runtime.rs:584-670` | start_agent_loop |
| `crates/tidev-core/src/mcp.rs:119-213` | McpManager |
| `crates/tidev-core/src/registry.rs:32-68` | ToolRegistry |
| `crates/tidev-tui/src/lib.rs:20-34` | TUI 入口（Runtime 创建） |
| `crates/tidev-tui/src/app/tools.rs:20-44` | handle_tui_request |
| `src/main.rs:176-237` | CLI 入口 |
