# AgentRuntime 迁移

## 目标

将 TUI、Web、Gateway 共用的 LLM ↔ 工具执行循环逻辑提取到共享的 `AgentRuntime`（`src/agent/runtime.rs`），消除三处重复实现，确保行为一致。

---

## ✅ 全部完成

### ✅ Web 后端 — 已完成

Web 是第一个完全迁移的消费者。`src/web/routes/messages.rs` 直接调用 `agent.run_agent_loop()`，不再有任何重复的 LLM 循环、工具执行、消息持久化代码。

| 文件 | 变更 |
|---|---|
| `src/agent/mod.rs` | 添加 `pub mod runtime;` |
| `src/agent/runtime.rs` | 核心 AgentRuntime（13 个方法） |
| `src/tooling/registry.rs` | 添加 `memory_store()`, `execute_call_spawned()`, `is_read_only_call()` |
| `src/web/state.rs` | `AppState` 新增 `agent` 字段 |
| `src/web/mod.rs` | 初始化 AgentRuntime |
| `src/web/routes/messages.rs` | 改用 `agent.run_agent_loop()` |

### ✅ Gateway QQ — 已完成

原始代码有 4 个方法（~195 行）完全重复 `run_agent_loop`/`execute_tool_calls` 的逻辑。

| 删除的方法 | 替换方案 |
|---|---|
| `llm_completion_turn()` (~60 行) | `run_agent_loop` 内部处理 |
| `run_single_streaming_turn()` (~40 行) | `run_agent_loop` 内部处理 |
| `execute_tool_calls()` (~40 行) | `run_agent_loop` 内部 + `execute_call_spawned` |
| `run_agent_with_tools()` (~55 行) | 新实现：创建 CancellationToken → 设置 context → 调用 `run_agent_loop()` → 读取最终消息发送 |
| `cancellation_flags: HashMap<String, Arc<AtomicBool>>` | `cancellation_tokens: HashMap<String, CancellationToken>` |

### ✅ Gateway Telegram — 已完成

原始代码有 3 个方法（~310 行）。

| 删除的方法 | 替换方案 |
|---|---|
| `run_single_streaming_turn()` (~145 行) | `run_agent_loop` 内部 streaming + spawned event handler 做 draft 编辑 |
| `execute_tool_calls()` (~60 行) | 删除；tool result 通过 spawned event handler 实时推送 |
| `run_agent_with_tools()` (~100 行) | 新实现：创建事件通道 → 启动 event handler → 调用 `run_agent_loop()` → 处理最终响应 |
| `cancellation_flags: HashMap<i64, Arc<AtomicBool>>` | `cancellation_tokens: HashMap<i64, CancellationToken>` |
| `TelegramBot` | 添加 `#[derive(Clone)]`，使 event handler 可独立持有 bot 引用 |

### ✅ 工具执行基础设施 — 全部完成

`execute_call_spawned()` — 所有前端统一使用的 panic-safe 执行方法。

| 改动 | 文件 | 说明 |
|---|---|---|
| `execute_call_spawned()` | `src/tooling/registry.rs` | `spawn_blocking` + `catch_unwind`，panic 转为错误消息 |
| `execute_tool_calls()` 改造 | `src/agent/runtime.rs` | **只读工具（Read/Search）并行**，**写入工具（Write/Edit/Execute）串行**；全部有 catch_unwind |
| `is_read_only_call()` | `src/tooling/registry.rs` | 基于 `ToolPermission` 判断只读/写入 |
| `persist_tool_result()` | `src/agent/runtime.rs` | 封装 "创建 Message → 持久化 DB → 发送 ToolCompleted 事件" |

### ✅ TUI — 已完成

TUI **已迁移到 `run_agent_loop`**。新增**权限审批通道（Permission Approval Channel）**机制，使 TUI 的交互式权限对话框与 `run_agent_loop` 的解耦共存。

#### 架构

```
submit_prompt_now
  ├─ persist user msg to DB + snapshot
  ├─ spawn_agent_loop ───────────────────────────────┐
  │   ├─ 创建 permission channel (mpsc)              │
  │   └─ spawn run_agent_loop_with_permission_channel │
  │       ├─ LLM stream → BackendEvent               │
  │       ├─ persist assistant message                │
  │       └─ 有 tool calls？──→ PendingToolApproval   │
  │                           └─ 等待审批              │
  └─ process_backend_events ◄── BackendEvent          │
      ├─ Delta / ToolCallUpdated / Finished / etc.    │
      └─ pending_permission_rx ◄── PendingToolApproval│
          ├─ 权限审批流程                              │
          │   ├─ can_execute mode 过滤                 │
          │   ├─ workspace boundary 检查               │
          │   ├─ question dialog                       │
          │   ├─ confirmation dialog                   │
          │   └─ auto-approve                          │
          └─ send_permission_approval ────────────────►│
              └─ ApprovedTool[]                        │
                                                       │
              run_agent_loop 收到审批后:                │
              ├─ 执行 approved 工具                     │
              │   └─ persist_tool_result → ToolCompleted│
              ├─ 持久化 rejected 工具的结果              │
              └─ 继续下一轮 LLM 循环                    │
```

#### 权限通道机制

`AgentRuntime` 与 TUI 之间的工具审批通过 **`mpsc + oneshot` 通道**实现：

```rust
struct PendingToolApproval {
    tool_calls: Vec<ToolCall>,
    mode: SessionMode,
    response_tx: oneshot::Sender<Vec<ApprovedTool>>,
}

struct ApprovedTool {
    tool_call: ToolCall,
    rejection: Option<ToolExecutionResult>,  // None = 批准执行
}
```

流程：
1. `run_agent_loop_with_permission_channel` 收到 LLM 返回的 tool calls
2. 发送 `PendingToolApproval` 到 `permission_tx`（mpsc）
3. 等待 `response_rx`（oneshot）
4. TUI 事件循环在 `process_backend_events` 中轮询 `pending_permission_rx`
5. 执行原有的权限审批逻辑（对话框、workspace boundary、question tool 等）
6. 通过 `response_tx` 发送审批结果
7. 运行时继续执行已批准的 tools

#### 非交互式权限过滤（所有前端共享）

`AgentRuntime::execute_tool_calls()` 新增两个检查：

| 检查 | 说明 | 适用范围 |
|---|---|---|
| `can_execute` mode 过滤 | Plan 模式下拒绝写入工具 | 所有前端 |
| `needs_confirmation` + `auto_approve_permissions` | TUI 设 `true`（自己管），Web/Gateway 设 `false`（拒绝） | 所有前端 |

#### 变更概要

| 文件 | 变更 |
|---|---|
| `src/agent/runtime.rs` | 新增 `PendingToolApproval`, `ApprovedTool`, `auto_approve_permissions`, `permission_tx` 参数, `can_execute` 检查 |
| `src/app/mod.rs` | 新增 `spawn_agent_loop()`, `pending_permission_rx`, `pending_permission_response`, `pending_rejected_tools`；简化 `finish_assistant_turn` |
| `src/app/runtime/run.rs` | 初始化新字段；TUI 的 AgentRuntime 设 `auto_approve_permissions: true` |
| `src/app/ui/permission.rs` | `process_pending_tool_execution` 支持通道模式；新增 `send_permission_approval`；`record_tool_result` 跳过运行时流的 DB 重复写入 |
| `src/app/ui/question.rs` | `resolve_question_dialog` 支持通道模式 |
| `src/web/mod.rs` | 添加 `auto_approve_permissions: false` |
| `src/gateway/qq.rs` | 添加 `auto_approve_permissions: false` |
| `src/gateway/telegram/channel.rs` | 添加 `auto_approve_permissions: false` |

### 消息排队（Message Queue）—— 显示队列 + 运行时队列

`AgentRuntime` 内置消息排队机制。前端通过 `agent.queue_user_message(QueuedUserMessage)` 推送消息，`run_agent_loop` 在每个 turn 完成后自动拾取。

TUI 保留了 `pending_prompt_queue` 作为**显示专用队列**。当用户在 agent 运行时输入新消息：
1. `queue_prompt()` 调用 `agent.queue_user_message()` 投递到运行时队列
2. 同时推送到 `pending_prompt_queue` 供 UI 渲染（"QUEUE" 面板 + 状态栏计数）
3. `spawn_agent_loop()` 启动时清空显示队列（因为运行时会拾取所有排队的消息）
4. 旧 `drain_queued_prompts()` 已移除

---

## 架构关系图

```
┌─────────────────────────────────────────────────────────────┐
│                    AgentRuntime                              │
│              (src/agent/runtime.rs)                          │
│                                                              │
│  compose_system_prompt()  ← TUI / Web / Gateway              │
│  build_request_messages() ← TUI / Web / Gateway              │
│  tool_definitions()       ← TUI / Web / Gateway              │
│  run_single_turn()        ← Web                              │
│  execute_tool_calls()     ← Web / Gateway / TUI              │
│    • can_execute mode 过滤  ✅ 新增                           │
│    • auto_approve_permissions  ✅ 新增                        │
│    • 读并行 / 写串行                                          │
│    • catch_unwind 保护                                       │
│    • spawn_blocking 不阻塞 async 任务                         │
│  run_agent_loop()         ← Web / Gateway / TUI  ✅           │
│    • CancellationToken 支持                                   │
│    • Context compaction                                      │
│    • PendingToolApproval 通道  ✅ 新增                        │
│  persist_assistant_message()                                  │
│  persist_message()        ← TUI (thin wrapper)               │
│  persist_tool_result()                                        │
└─────────────────────────┬───────────────────────────────────┘
                          │
         ┌────────────────┼─────────────────┐
         │                │                 │
    ┌────▼────┐      ┌────▼──────┐    ┌─────▼─────┐
    │  TUI    │      │   Web     │    │ Gateway   │
    │  ✅     │      │   ✅      │    │ (Tg, QQ)  │
    │         │      │           │    │   ✅      │
    │ run_agent│      │ run_agent │    │ run_agent │
    │ _loop() │      │  _loop()  │    │  _loop()  │
    │  + perm │      │           │    │           │
    │ channel │      │           │    │           │
    └─────────┘      └───────────┘    └──────────┘
```

## 已知问题

### 🟡 子代理（Subagent）

`run_subagent()` 和 `run_subagent_inner()` 已添加到 `AgentRuntime`。

| 问题 | 描述 | 原因 |
|------|------|------|
| 🔴 **串行执行** | 多个 subagent 顺序执行而非并行 | `run_subagent()` 的 future 不满足 `Send`，无法用于 `tokio::spawn`。 |
| 🔴 **独立循环** | `run_subagent_inner` 复制了 `run_agent_loop_with_tools` 的循环体 | 避免 async 递归类型检测。 |
| 🟡 **无嵌套子代理** | `task` 工具在子 session 中返回 "nesting too deep" 错误 | 同上，独立循环不包含 `task` 处理逻辑。 |

**修复方向：**
1. 排查 `AgentRuntime` 或其子组件中导致 future 非 `Send` 的字段（候选：`McpManager`、`LlmClient` 的 `http::Client`）
2. 修复后，`run_subagent_inner` 可改用 `run_agent_loop_with_tools`（通过 `Box::pin` 阻断递归检测）
3. 子 agent 可改为 `tokio::spawn` + `join_all` 并行执行

### 🟡 测试

| 测试 | 状态 |
|---|---|
| `execute_call_spawned` 单元测试（panic 捕获 + 错误处理） | ❌ |
| `execute_tool_calls` 集成测试（并行/串行调度 + 持久化） | ❌ |
| `run_agent_loop` 集成测试（Mock LLM） | ❌ |
| `PendingToolApproval` 通道集成测试 | ❌ |
| `run_subagent` 集成测试 | ❌ |
| Gateway 端到端测试 | ❌ |

### 注意事项

1. **TUI 事件流是双通道**：`BackendEvent` 走 `backend_rx`，审批请求走 `pending_permission_rx`。两者在 `process_backend_events` 中统一轮询。

2. **交互式权限是 TUI 独有的**：workspace boundary、permission dialog、question tool 通过权限通道与 AgentRuntime 解耦。Web/Gateway 不设 permission_tx，工具直接进入 `execute_tool_calls` 接受非交互检查。

3. **`execute_call_spawned` 的 SessionStore 线程安全**：每个 spawned 任务持有独立的 `SessionStore` 克隆（各自打开新的 SQLite 连接）。SQLite WAL 模式处理并发读写。

4. **`catch_unwind` 的限制**：`AssertUnwindSafe` 抑制编译器的 unwind-safety 检查。在 `execute_call_spawned` 中所有参数都是 clone 的 owned 值，安全。

5. **`auto_approve_permissions` 策略**：TUI 设 `true`（自身通过权限通道处理交互审批），Web/Gateway 设 `false`（安全默认，需要确认的工具被拒绝）。

## 下一步

### ✅ 已完成 — TUI 遗留清理

- **移除 `start_assistant_turn`**：所有调用点已迁移到 `spawn_agent_loop`
- **移除 `drain_queued_prompts`**：消息排队由 `agent.queue_user_message()` 处理
- **保留 `pending_prompt_queue` 仅用于 UI 渲染**：`queue_prompt` 同时推送到运行时队列和显示队列
- **旧 flow 出口路径**（`try_start_parallel_execution` / `process_pending_tool_execution` / `resolve_workspace_boundary_dialog` 中的旧路径）已替换为 `self.pending_request = false`

### 下一步

### 1. 🟡 测试覆盖（中等优先级）

最优先的测试：

| 测试 | 原因 |
|---|---|
| `execute_tool_calls` + `can_execute` 过滤 | 安全关键：确保 Plan 模式下写入工具被拒绝 |
| `PendingToolApproval` 通道流程 | 核心架构：确保审批→执行→继续循环正确 |
| `record_tool_result` 跳过 DB 写入 | 数据完整：确保不产生重复消息 |

### 2. 🟡 子代理修复（低优先级）

- 排查 future 非 `Send` 的来源（`McpManager`、`http::Client`）
- 修复后可用 `Box::pin` 解决递归类型问题
- 子代理可改为并行执行

### 3. 🔴 Gateway 端到端测试（低优先级）

当前无端到端测试。Web/Gateway 依赖手动测试。
