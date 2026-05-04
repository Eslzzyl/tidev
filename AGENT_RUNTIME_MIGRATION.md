# AgentRuntime 迁移

## 目标

将 TUI、Web、Gateway 共用的 LLM ↔ 工具执行循环逻辑提取到共享的 `AgentRuntime`（`src/agent/runtime.rs`），消除三处重复实现，确保行为一致。

---

## 现状

### ✅ Web 后端 — 已完成

Web 是第一个完全迁移的消费者。`src/web/routes/messages.rs` 直接调用 `agent.run_agent_loop()`，不再有任何重复的 LLM 循环、工具执行、消息持久化代码。

| 文件 | 变更 |
|---|---|
| `src/agent/mod.rs` | 添加 `pub mod runtime;` |
| `src/agent/runtime.rs` | 核心 AgentRuntime（9 个方法） |
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

### ⚠️ TUI — 部分完成

**已完成的点级委托：**

| 改动 | 文件 | 说明 |
|---|---|---|
| `build_request_messages()` + `tool_definitions()` 改用 `agent` | `src/app/mod.rs:437-440` | 消除与 ContextManager 的直接耦合 |
| `persist_message()` 改用 `agent.persist_message()` | `src/app/mod.rs:1203-1206` | 统一持久化路径 |
| `compose_system_prompt()` 已委托给 `agent` | `src/app/mod.rs:498-503` | 之前已完成 |
| 全部 5 个 `execute_call` 路径替换为 `execute_call_spawned` | `permission.rs`, `workspace_boundary.rs`, `subagent.rs` | 所有工具执行有 catch_unwind 保护 |

**未完成的架构层迁移：**

TUI **没有使用 `run_agent_loop()`**。TUI 的 `start_assistant_turn()`（`src/app/mod.rs:420-467`）仍然手动管理整个 LLM 交互周期：

```
TUI 当前流程（手动管理）:
  start_assistant_turn
    ├─ 手动 compose_system_prompt
    ├─ 手动 build_request_messages
    ├─ 手动获取 tool_definitions
    ├─ 手动 spawn LLM stream
    ├─ 事件循环处理每个 BackendEvent（~500 行 match）
    ├─ finish_assistant_turn
    │   ├─ 手动检测 turn_mode
    │   ├─ 手动设置 token 用量
    │   ├─ 手动持久化 assistant message
    │   ├─ 手动处理 tool calls → begin_tool_execution
    │   └─ 手动 context compaction + drain queued prompts
    └─ begin_tool_execution
        ├─ 手动权限检查链（~400 行）
        ├─ 手动调度执行
        └─ 手动收结果 → record_tool_result

Web/Gateway 流程（AgentRuntime 自动管理）:
  run_agent_loop
    ├─ 自动 compose/build/tools
    ├─ 自动 run_single_turn（内部 spawn LLM + 收集事件）
    ├─ 自动 persist_assistant_message
    ├─ 自动 execute_tool_calls（读并行/写串行 + catch_unwind）
    └─ 自动循环直到完成 + context compaction
```

---

## 尚未完成的工作

### 🔴 TUI 接入 `run_agent_loop`（收益最大）

**问题：** TUI 的 `start_assistant_turn` + `finish_assistant_turn` + `begin_tool_execution` 三块逻辑（合计 ~600 行）与 `run_agent_loop()` 完全重复。

**迁移障碍：**
1. TUI 的事件流是单通道（`backend_tx`/`backend_rx`），`run_agent_loop` 要求传入 `event_tx`
2. TUI 有**交互式权限检查**（对话框、工作区边界、问题工具），这是写入工具特有的、`run_agent_loop` 不提供的
3. TUI 需要实时更新 UI（状态面板、loading 提示），`run_agent_loop` 的事件通道需要映射到 TUI 的事件循环
4. TUI 的 token 用量跟踪（`UsageStats` → `ContextUsage`）在 AgentRuntime 中未捕获
5. TUI 的子代理（subagent）执行是异步的（通过独立会话），与 `run_agent_loop` 的同步风格不同

**建议方案：** 不是简单替换 `start_assistant_turn`，而是让 `run_agent_loop` 作为轮询内核，TUI 在其上包装权限检查和 UI 更新层。

### 🟠 `record_tool_result` → `agent.persist_tool_result()`

`record_tool_result()`（`permission.rs:~30 行`）的 DB 持久化 + 事件发送部分可以委托给 `agent.persist_tool_result()`，保留 TUI 特有的 in-memory conversation 更新、cache 清理等。

### 🟡 测试

| 测试 | 状态 |
|---|---|
| `execute_call_spawned` 单元测试（panic 捕获 + 错误处理） | ❌ |
| `execute_tool_calls` 集成测试（并行/串行调度 + 持久化） | ❌ |
| `run_agent_loop` 集成测试（Mock LLM） | ❌ |
| Gateway 端到端测试 | ❌ |

### 代码行数（大概）

| 文件 | 变更 |
|---|---|
| `src/agent/runtime.rs` | ~870 行 |
| `src/tooling/registry.rs` | +55 行（execute_call_spawned, is_read_only_call） |
| `src/web/state.rs` | +6 行 |
| `src/web/mod.rs` | +45 行 |
| `src/web/routes/messages.rs` | ~-140 行净减少 |
| `src/gateway/telegram/channel.rs` | ~-180 行净减少 |
| `src/gateway/telegram/bot.rs` | +1 行（Clone derive） |
| `src/gateway/qq.rs` | ~-140 行净减少 |
| `src/app/mod.rs` | -5 行 |
| `src/app/ui/permission.rs` | ~0 行（替换为 execute_call_spawned） |
| `src/app/ui/workspace_boundary.rs` | ~0 行（同上） |
| `src/app/runtime/subagent.rs` | ~0 行（同上） |
| `src/app/runtime/run.rs` | +12 行（AgentRuntime 初始化） |

### 架构关系图

```
┌─────────────────────────────────────────────────────────────┐
│                    AgentRuntime                              │
│              (src/agent/runtime.rs)                          │
│                                                              │
│  compose_system_prompt()  ← TUI / Web / Gateway              │
│  build_request_messages() ← TUI / Web / Gateway              │
│  tool_definitions()       ← TUI / Web / Gateway              │
│  run_single_turn()        ← Web                              │
│  execute_tool_calls()     ← Web / Gateway                    │
│    • 读并行 / 写串行                                          │
│    • catch_unwind 保护                                       │
│    • spawn_blocking 不阻塞 async 任务                         │
│  run_agent_loop()         ← Web / Gateway (TUI ❌)           │
│    • CancellationToken 支持                                   │
│    • Context compaction                                      │
│  persist_assistant_message()                                  │
│  persist_message()        ← TUI (thin wrapper)               │
│  persist_tool_result()                                        │
└─────────────────────────┬───────────────────────────────────┘
                          │
         ┌────────────────┼─────────────────┐
         │                │                 │
    ┌────▼────┐      ┌────▼──────┐    ┌─────▼─────┐
    │  TUI    │      │   Web     │    │ Gateway   │
    │         │      │           │    │ (Tg, QQ)  │
    │         │      │           │    │           │
    │ □ compose  │  │ run_agent │    │ run_agent │
    │ □ build   │   │  _loop()  │    │  _loop()  │
    │ □ persist │   │    ✅     │    │    ✅     │
    │         │      │           │    │           │
    │ ──── 工具执行─│ ──── 工具执行│ ──── 工具执行─│
    │ 🟢 全部工具  │ 🟢 并行 +  │ 🟢 并行 +   │
    │   catch_   │    spawn_  │    spawn_   │
    │   unwind    │    block + │    block +  │
    │            │    catch_  │    catch_   │
    │            │    unwind   │    unwind   │
    │            │            │            │
    │ ──── LLM 循环─│ ──── LLM 循环─│ ──── LLM 循环─│
    │ 🔴 手动管理  │ 🟢 agent   │ 🟢 agent   │
    │    ~600 行  │  _loop()   │  _loop()   │
    │    重复逻辑  │            │            │
    └──────────┘   └────────────┘ └───────────┘
     ✅ = 已使用 AgentRuntime
     🟢 = 改造完成
     🔴 = 尚未改造
     □ = 点级委托完成，架构未变
```

### 注意事项

1. **TUI 事件流是单通道**：TUI 通过 `backend_tx` / `backend_rx` 接收所有 `BackendEvent`。`run_agent_loop` 要求传入 `event_tx`。如果 TUI 改用 `run_agent_loop`，需要将 `backend_tx` 作为 `event_tx` 传入。

2. **交互式权限是 TUI 独有的**：`allow_outside` 工作区边界检查、权限对话框、问题工具这些都是 TUI 特有的，不应该进入 AgentRuntime。如果 TUI 要用 `run_agent_loop`，需要在 `run_agent_loop` 之外先完成权限检查，然后把已批准的 tool calls 传入。

3. **`execute_call_spawned` 的 SessionStore 线程安全**：每个 spawned 任务持有独立的 `SessionStore` 克隆（各自打开新的 SQLite 连接）。SQLite WAL 模式处理并发读写。

4. **`catch_unwind` 的限制**：`AssertUnwindSafe` 抑制编译器的 unwind-safety 检查。在 `execute_call_spawned` 中所有参数都是 clone 的 owned 值，安全。
