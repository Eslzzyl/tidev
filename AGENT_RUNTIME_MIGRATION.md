# AgentRuntime 迁移计划

## 目标

将 TUI、Web、Gateway 三者共用的 LLM ↔ 工具执行循环逻辑提取到共享的 `AgentRuntime`（`src/agent/runtime.rs`），消除三处重复实现，确保行为一致。

---

## 已完成的工作

### 1. 创建共享 AgentRuntime

**`src/agent/runtime.rs`** — 核心文件，包含：

| 方法 | 功能 | 使用方 |
|---|---|---|
| `compose_system_prompt()` | 合并 base prompt + 指令文件 + 模式提醒 + 环境信息 + 工作区记忆 | TUI / Web / Gateway |
| `build_request_messages()` | 从 DB 消息构建预处理后的 LLM 请求消息（回退点过滤、孤立 tool call/result 处理、模式切换注入） | Web / Gateway |
| `tool_definitions()` | 返回 `ToolRegistry::all_definitions()`（15+ 内置工具 + MCP 工具 + 模型适配过滤） | Web / Gateway |
| `run_single_turn()` | 流式调用 LLM，实时转发 `BackendEvent`，返回最终的 `AssistantTurn` | Web |
| `execute_tool_calls()` | 执行一组 tool call，持久化结果到 DB，发送 `ToolCompleted` 事件 | Web / Gateway |
| `persist_assistant_message()` | 持久化 assistant 消息到 DB | —（供各前端调用） |
| `run_agent_loop()` | 完整 agent 循环：`load → compose → stream → (execute + loop)`，含 CancellationToken + context compaction | Web |

### 2. Web 后端接入 AgentRuntime

**不再重复的代码：**

| 之前的问题 | 现在的方案 |
|---|---|
| `tools = vec![]` — LLM 看不到工具 | `ToolRegistry::all_definitions()` — 全部内置 + MCP 工具 |
| 仅使用 `model_config.system_prompt` | `compose_system_prompt()` — 含指令文件、环境信息、记忆 |
| 手动在 `BackendEvent::Finished` 时持久化 assistant | AgentRuntime 内部自动持久化 |
| 无工具执行 (LLM 返回 tool call 后无反应) | `run_agent_loop` 自动执行工具并回环直到完成 |
| 不使用 `ContextManager::build_request_messages()` | AgentRuntime 调用 `build_request_messages()` 做消息预处理 |

**修改的文件：**

| 文件 | 变更 |
|---|---|
| `src/agent/mod.rs` | 添加 `pub mod runtime;` |
| `src/agent/runtime.rs` | 新文件 |
| `src/tooling/registry.rs` | 添加 `pub fn memory_store()` |
| `src/web/state.rs` | `AppState` 新增 `agent: AgentRuntime` |
| `src/web/mod.rs` | 初始化 AgentRuntime（ToolRegistry + MemoryStore + McpManager） |
| `src/web/routes/messages.rs` | `send_message` 改用 `agent.run_agent_loop()` |

---

## 还需要做的工作

以下是在当前迁移基础上可以继续改进的方向，按优先级排列。

---

### 🔴 0. 工具执行架构改造 — panic-safe 异步执行（基础设施层）

这是所有后续优化的基础。当前三套前端有**三种不同的工具执行模型**，而且都没有 `catch_unwind` 保护。

#### 现状：三套执行模型

```
┌─────────────────────────────────────────────────────────┐
│                   工具执行现状                             │
├──────────────┬──────────────────┬───────────────────────┤
│    TUI       │     Web          │     Gateway           │
├──────────────┼──────────────────┼───────────────────────┤
│ Phase 1:     │ AgentRuntime     │ AgentRuntime          │
│ spawn_blocking│ execute_calls() │ execute_calls()       │
│ (bash/read)  │ sync inline      │ sync inline           │
│              │                  │                       │
│ Phase 2:     │ 🔴 阻塞 async    │ 🔴 阻塞 async 线程   │
│ 🔴 阻塞 UI   │    任务线程      │                       │
│    线程      │ 🔴 无 panic      │ 🔴 无 panic 保护      │
│ 🔴 无 panic  │     保护         │                       │
│     保护     │                  │                       │
└──────────────┴──────────────────┴───────────────────────┘
```

#### 风险分析：完全没有任何 `catch_unwind` 边界

| 执行路径 | 线程 | Panic 后果 |
|---|---|---|
| **TUI Phase 1** (`spawn_blocking`) | Blocking 线程 | 🔴 Panic 被 tokio 吞掉，工具永不返回，UI 卡死 |
| **TUI Phase 2** (内联同步) | **UI 事件循环线程** | 🔴 **Panic 直接炸掉整个 TUI 进程** |
| **AgentRuntime** (顺序同步) | Async 任务线程 | 🔴 Panic 传播到 `JoinHandle`，如果被 drop 则吞掉 |
| **MCP 工具** (`runtime.block_on`) | 调用者线程 | 🔴 同调用者 |

你的 `edit` 工具偶尔 panic 的情况就是 Phase 2 路径——`edit_file` 本身没有 `unwrap()`，但 `line_slice()` 的 `lines[..start]` 或 `apply_patch_contents()` 的 `&line_fragments[cursor..]` 如果索引越界就会 panic，**没有任何保护**，直接炸掉进程。

#### 核心改动：`ToolRegistry::execute_call_spawned()`

在 `ToolRegistry` 层添加 `catch_unwind` + `spawn_blocking` 方法，所有前端统一使用。**不改动串行/并行的调度策略**——写入工具仍然串行执行，这由调用方决定：

```rust
// 调用方决定执行策略：
//
// 并行执行（Web/Gateway 的 execute_tool_calls，TUI 的 read-only 工具）：
for call in tool_calls {
    handles.push(self.tools.execute_call_spawned(...));
}
for handle in handles { /* await all */ }

// 串行执行（TUI 的 write/edit/apply_patch）：
for call in tool_calls {
    let result = self.tools.execute_call_spawned(...).await;
    // 处理结果
}```

```rust
// src/tooling/registry.rs — 新增
impl ToolRegistry {
    /// Execute a tool call on a blocking thread with panic protection.
    /// Never panics — caught panics become `ToolExecutionResult`.
    pub fn execute_call_spawned(
        &self,
        runtime_handle: &tokio::runtime::Handle,
        store: &SessionStore,
        session_id: Uuid,
        call: &ToolCall,
        mode: SessionMode,
        allow_outside: bool,
    ) -> JoinHandle<ToolExecutionResult> {
        let registry = self.clone();  // ToolRegistry is Clone
        let store = store.clone();
        let call = call.clone();
        let runtime_handle = runtime_handle.clone();

        tokio::task::spawn_blocking(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                registry.execute_call(&runtime_handle, &store, session_id, &call, mode, allow_outside)
            }));
            match result {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => ToolExecutionResult::new(format!("Error: {e}")),
                Err(panic) => {
                    let msg = /* extract panic message */;
                    ToolExecutionResult::new(format!("Tool panicked: {msg}"))
                }
            }
        })
    }
}
```

#### 联动改动

| 改动 | 文件 | 说明 | 优先级 |
|---|---|---|---|
| **0a.** 新增 `execute_call_spawned()` | `src/tooling/registry.rs` | 基础：catch_unwind + spawn_blocking | 🔴 最高 |
| **0b.** `AgentRuntime::execute_tool_calls()` 改用 `execute_call_spawned()` | `src/agent/runtime.rs` | 工具并行执行 + 不阻塞 async 任务 | 🔴 |
| **0c.** TUI Phase 2 改用 `execute_call_spawned()` | `src/app/ui/permission.rs` | **串行执行保留**，但改为 spawn_blocking → 不阻塞 UI 线程 + panic 不崩溃 | 🔴 |
| **0d.** TUI Phase 1 统一为 `execute_call_spawned()` | `src/app/ui/permission.rs` | 消除 Phase 1/Phase 2 的执行方式差异（但仍保留串行/并行策略差异） | 🟠 |

> **设计说明**：TUI Phase 2（write/edit/apply_patch 等）的**串行执行是有意设计的**——防止并发的文件写入冲突。改造的目标不是改为并行，而是将执行从 UI 线程移到 blocking 线程 + 加上 panic 保护。串行/并行的调度决策仍由调用方（各前端）自行决定。

#### 联动改动的效果

| 前端 | 当前 | 改后 |
|---|---|---|
| **TUI — 写入工具** (write/edit/apply_patch) | Phase 2: 串行阻塞 UI 线程 / 无 panic 保护 | **串行保留**，在 blocking 线程执行 / UI 不再卡顿 / panic 被捕获 |
| **TUI — 只读工具** (bash/read/list/glob/grep/web) | Phase 1: 并行 spawn_blocking / panic 被吞掉 | 统一使用 `execute_call_spawned()` / panic 被捕获转为错误消息 |
| **Web** | 顺序同步执行 async 任务 / 无 panic 保护 | **所有工具并行执行** / 不阻塞 async 任务 / panic 被捕获 |
| **Gateway** | 同 Web | 同 Web |

#### 路径依赖

```
改动 0a: ToolRegistry::execute_call_spawned()    ← 基础设施
    ├─ 改动 0b: AgentRuntime::execute_tool_calls() 用它  ← Web/Gateway
    └─ 改动 0c: TUI Phase 2 用它                      ← TUI
    └─ 改动 0d: TUI Phase 1 统一用它                   ← TUI 执行方式统一
```

### 🔴 1. TUI — 用 `agent.build_request_messages()` + `agent.tool_definitions()`

**当前状态：** TUI 仍然直接调用 `context_manager.build_request_messages(&self.conversation, self.mode)` 和 `self.tools.all_definitions()`（`src/app/mod.rs:437-440`）。

**AgentRuntime 已提供：** `build_request_messages(&messages, &context_manager, mode)` 和 `tool_definitions()`，前者额外处理孤立的 tool call/result。

**改动量：** ~3 行代码修改。

**风险：** 无。`agent.build_request_messages()` 内部逻辑与 `ContextManager::build_request_messages()` 几乎相同，增加孤儿 tool call 处理。

---

### 🔴 2. TUI — 用 `agent.persist_assistant_message()` 替代手动 DB 写入

**当前状态：** TUI 在 `finish_assistant_turn()`（`src/app/mod.rs:1203-1206`）手动调用 `self.store.append_message()` 持久化 assistant 消息。`AgentRuntime::persist_assistant_message()` 做同样的工作（`runtime.rs:372-387`）。

**差异点：** TUI 额外设置 `input_tokens`/`output_tokens`/`total_tokens`/`cache_read_tokens`/`cache_write_tokens`/`model_id`/`completed_at` 这些 token 用量字段（从 `self.context_usage` 获取）。这些是 AgentRuntime 未设置的。

**建议方案：** 让 `persist_assistant_message()` 接受一个可选的 token usage 参数，或让 TUI 在调用 `agent.persist_assistant_message()` 之前先设置好 in-memory message 上的 token 字段。

---

### 🟠 3. TUI — 用 `agent.execute_tool_calls()` 简化工具执行链（低优先级）

**当前状态：** `src/app/ui/permission.rs` 有完整的权限检查 → 执行链（~800 行）。其中工具执行部分（`start_readonly_tool_execution`、`start_parallel_execution` 中的同步路径）与 `AgentRuntime::execute_tool_calls()` 重复。

**为什么不能简单替换：** TUI 的工具执行链是**交互式**的——每个 tool call 都需要经过：
1. 模式可用性检查
2. 已记住的权限检查
3. 工作区边界检查
4. 权限对话框等待用户输入
5. 问题工具的特殊处理
6. 子任务的特殊处理

而 `AgentRuntime::execute_tool_calls()` 是**非交互式**的——执行所有 tool call 并立即返回。

**建议方案：** 在 `AgentRuntime` 上新增 `persist_tool_result(tool_call, result)` 方法，封装 `Message::tool_result()` + `store.append_message()` + `ToolCompleted` 事件的公共模式。这样 `permission.rs` 中的 `record_tool_result()` 调用可以委托给共享实现。

---

### 🟠 4. Gateway — QQ `llm_completion_turn()` 替换为 `agent.run_single_turn()`

**当前状态：** `src/gateway/qq.rs:514-574`（60 行）完全重复 `AgentRuntime::run_single_turn()`。
**风险：** 低。
**额外发现：** QQ 使用 `Uuid::new_v4()` 作为 session_id（而非 `conversation.session_id`），且 `request_id` 固定为 `1`。这是**潜在的 bug**——事件无法关联到正确的 session。使用 `agent.run_single_turn()` 会自动修复。

---

### 🟠 5. Gateway — Telegram `run_single_streaming_turn()` 替换为 `agent.run_single_turn()`

**当前状态：** `src/gateway/telegram/channel.rs:408-553` — 145 行代码，其中事件收集逻辑与 `run_single_turn()` 重复。

**迁移障碍：** Draft 编辑混在 `Delta` 事件的处理中（约 40 行）。

**建议方案：**
```rust
// 1. 创建事件通道
let (event_tx, mut event_rx) = unbounded_channel();

// 2. 用 AgentRuntime 获取 turn
let turn = self.agent.run_single_turn(
    session_id, request_id, model,
    request_messages, tool_definitions,
    thinking_level, &event_tx,
).await?;

// 3. 消费事件做 draft 编辑
while let Ok(Some(event)) = event_rx.try_recv() {
    match event {
        BackendEvent::Delta { content, .. } => { /* update_draft(...) */ }
        BackendEvent::ToolCallUpdated { .. } => { /* update_draft with tool info */ }
        _ => {}
    }
}

// 4. 最终的 draft 编辑在 get_turn 后完成
finalize_draft(draft_message_id, &turn.content)?;
```

**注意：** 事件可能在 `run_single_turn` 返回后仍有剩余（因为事件通道是异步的），需要处理 `try_recv` 循环或使用 `tokio::spawn` 在后台消费。

---

### 🔴 6. Gateway — QQ `run_agent_with_tools()` 简化

**当前状态：** `src/gateway/qq.rs:415-470` — 50 行循环代码，结构与 `run_agent_loop()` 几乎相同。

**建议方案：** 替换为：
```rust
let (event_tx, _event_rx) = unbounded_channel();
self.agent.run_agent_loop(
    conversation.session_id,
    active_model.clone(),
    &mut context_manager,
    SessionMode::Build,
    thinking_level,
    event_tx,
    Some(cancel_token),  // 用 CancellationToken 替代 8 轮硬限制
).await?;
```

---

### 🔴 7. Gateway — Telegram `run_agent_with_tools()` 简化

**当前状态：** `src/gateway/telegram/channel.rs:304-406` — 100 行循环代码。

**建议方案：** 同 QQ，但 Telegram 需要额外的 draft 管理：
- 在调用 `run_agent_loop` 前发送 initial draft
- 通过 `event_rx` 消费事件更新 draft 进度
- Loop 结束后根据是否有 tool calls 决定删除 draft 或 finalize

---

### 🟡 8. 新方法 `AgentRuntime::persist_tool_result()`

**动机：** 消除 TUI `permission.rs` 中 `record_tool_result()`（~30 行）与 `AgentRuntime::execute_tool_calls()` 中 DB 持久化 + 事件发射逻辑（~15 行）的重复。

**建议接口：**
```rust
pub async fn persist_tool_result(
    &self,
    session_id: Uuid,
    request_id: u64,
    tool_call: &ToolCall,
    result: ToolExecutionResult,
    event_tx: &UnboundedSender<BackendEvent>,
) -> Result<()> {
    let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
    { let store = self.store.lock().await; store.append_message(session_id, &tool_msg)?; }
    let _ = event_tx.send(BackendEvent::ToolCompleted { ... });
    Ok(())
}
```

---

### 🔴 1. TUI — 用 `agent.build_request_messages()` + `agent.tool_definitions()`

**当前状态：** TUI 仍然直接调用 `context_manager.build_request_messages(&self.conversation, self.mode)` 和 `self.tools.all_definitions()`（`src/app/mod.rs:437-440`）。

**AgentRuntime 已提供：** `build_request_messages(&messages, &context_manager, mode)` 和 `tool_definitions()`，前者额外处理孤立的 tool call/result。

**改动量：** ~3 行代码修改。

**风险：** 无。`agent.build_request_messages()` 内部逻辑与 `ContextManager::build_request_messages()` 几乎相同，增加孤儿 tool call 处理。

---

### 🔴 2. TUI — 用 `agent.persist_assistant_message()` 替代手动 DB 写入

**当前状态：** TUI 在 `finish_assistant_turn()`（`src/app/mod.rs:1203-1206`）手动调用 `self.store.append_message()` 持久化 assistant 消息。`AgentRuntime::persist_assistant_message()` 做同样的工作（`runtime.rs:372-387`）。

**差异点：** TUI 额外设置 `input_tokens`/`output_tokens`/`total_tokens`/`cache_read_tokens`/`cache_write_tokens`/`model_id`/`completed_at` 这些 token 用量字段（从 `self.context_usage` 获取）。这些是 AgentRuntime 未设置的。

**建议方案：** 让 `persist_assistant_message()` 接受一个可选的 token usage 参数，或让 TUI 在调用 `agent.persist_assistant_message()` 之前先设置好 in-memory message 上的 token 字段。

---

### 🟠 3. TUI — 用 `execute_call_spawned()` 简化工具执行链

**当前状态：** `src/app/ui/permission.rs` 有完整的权限检查 → 执行链（~800 行）。其中工具执行部分（`start_readonly_tool_execution`、Phase 2 内联执行）与 `AgentRuntime::execute_tool_calls()` 重复。

**依赖：** 需要先完成 **改动 0**（`execute_call_spawned` 基础设施）。

**建议方案：**
1. Phase 2（write/edit/apply_patch 等）改用 `execute_call_spawned()` → 不再阻塞 UI 线程
2. Phase 1 也统一使用 `execute_call_spawned()` → 消除 Phase 1/2 区分
3. 权限检查链保持不变（这个是 TUI 特有的，不应该进入 AgentRuntime）

---

### 🟠 4. Gateway — QQ `llm_completion_turn()` 替换为 `agent.run_single_turn()`

**当前状态：** `src/gateway/qq.rs:514-574`（60 行）完全重复 `AgentRuntime::run_single_turn()`。
**风险：** 低。
**额外发现：** QQ 使用 `Uuid::new_v4()` 作为 session_id（而非 `conversation.session_id`），且 `request_id` 固定为 `1`。这是**潜在的 bug**——事件无法关联到正确的 session。使用 `agent.run_single_turn()` 会自动修复。

---

### 🟠 5. Gateway — Telegram `run_single_streaming_turn()` 替换为 `agent.run_single_turn()`

**当前状态：** `src/gateway/telegram/channel.rs:408-553` — 145 行代码，其中事件收集逻辑与 `run_single_turn()` 重复。

**迁移障碍：** Draft 编辑混在 `Delta` 事件的处理中（约 40 行）。

**建议方案：**
```rust
// 1. 创建事件通道
let (event_tx, mut event_rx) = unbounded_channel();

// 2. 用 AgentRuntime 获取 turn
let turn = self.agent.run_single_turn(
    session_id, request_id, model,
    request_messages, tool_definitions,
    thinking_level, &event_tx,
).await?;

// 3. 消费事件做 draft 编辑
while let Ok(Some(event)) = event_rx.try_recv() {
    match event {
        BackendEvent::Delta { content, .. } => { /* update_draft(...) */ }
        BackendEvent::ToolCallUpdated { .. } => { /* update_draft with tool info */ }
        _ => {}
    }
}

// 4. 最终的 draft 编辑在 get_turn 后完成
finalize_draft(draft_message_id, &turn.content)?;
```

**注意：** 事件可能在 `run_single_turn` 返回后仍有剩余（因为事件通道是异步的），需要处理 `try_recv` 循环或使用 `tokio::spawn` 在后台消费。

---

### 🔴 6. Gateway — QQ `run_agent_with_tools()` 简化

**当前状态：** `src/gateway/qq.rs:415-470` — 50 行循环代码，结构与 `run_agent_loop()` 几乎相同。

**建议方案：** 替换为：
```rust
let (event_tx, _event_rx) = unbounded_channel();
self.agent.run_agent_loop(
    conversation.session_id,
    active_model.clone(),
    &mut context_manager,
    SessionMode::Build,
    thinking_level,
    event_tx,
    Some(cancel_token),  // 用 CancellationToken 替代 8 轮硬限制
).await?;
```

---

### 🔴 7. Gateway — Telegram `run_agent_with_tools()` 简化

**当前状态：** `src/gateway/telegram/channel.rs:304-406` — 100 行循环代码。

**建议方案：** 同 QQ，但 Telegram 需要额外的 draft 管理：
- 在调用 `run_agent_loop` 前发送 initial draft
- 通过 `event_rx` 消费事件更新 draft 进度
- Loop 结束后根据是否有 tool calls 决定删除 draft 或 finalize

---

### 🟡 8. 新方法 `AgentRuntime::persist_tool_result()`

**动机：** 消除 TUI `permission.rs` 中 `record_tool_result()`（~30 行）与 `AgentRuntime::execute_tool_calls()` 中 DB 持久化 + 事件发射逻辑（~15 行）的重复。

**建议接口：**
```rust
pub async fn persist_tool_result(
    &self,
    session_id: Uuid,
    request_id: u64,
    tool_call: &ToolCall,
    result: ToolExecutionResult,
    event_tx: &UnboundedSender<BackendEvent>,
) -> Result<()> {
    let tool_msg = Message::tool_result(&tool_call.id, &tool_call.name, result.clone());
    { let store = self.store.lock().await; store.append_message(session_id, &tool_msg)?; }
    let _ = event_tx.send(BackendEvent::ToolCompleted { ... });
    Ok(())
}
```

---

### 🟡 9. 测试补充

| 测试 | 当前状态 | 方法 |
|---|---|---|
| `compose_system_prompt` 单元测试 | ❌ 未覆盖 | 使用 tempfile 创建临时指令文件 |
| `execute_call_spawned` 单元测试 | ❌ 未覆盖 | Mock 工具验证 panic 捕获 + 错误处理 |
| `execute_tool_calls` 单元测试 | ❌ 未覆盖 | 集成测试验证并行执行 + 持久化 |
| `run_agent_loop` 集成测试 | ❌ 未覆盖 | Mock LLM 客户端返回预设的 `AssistantTurn` |
| `persist_tool_result` 单元测试 | ❌ 未覆盖 | 同 `execute_tool_calls` |
| Gateway 端到端测试 | ❌ 未覆盖 | Mock Telegram/QQ API |

---

### ⚪ 10. 低优先级 / 未来方向

| 方向 | 说明 | 原因 |
|---|---|---|
| **TUI `schedule_context_compaction_for_session` 清理** | AgentRuntime 已内置 compaction，TUI 的独立调度可能冗余 | 低优先 — TUI 的调度方式不同（异步 vs 同步） |
| **Web 取消支持增强** | 将 `abort_request` 连接到 `CancellationToken` | 目前只移除 request 跟踪，不真正中断 LLM 请求 |
| **Gateway Telegram 改用完整 `run_agent_loop`** | 同上第 7 项 | 需要 draft 编辑重构，收益大但工作量大 |
| **QQ session_id/request_id 修复** | QQ 使用随机 session_id 和固定 request_id | 不影响功能但影响事件关联，替换为 `run_agent_loop` 自动修复 |
---

### 风险与注意事项

1. **TUI 的事件流是单通道**：TUI 通过 `backend_tx` 接收所有 `BackendEvent`。如果改用 `agent.run_single_turn()`，事件会通过 `event_tx` 发送，TUI 需要换个方式接收。目前的 `backend_tx` 可以直接作为 `event_tx` 传入，`run_single_turn` 会转发事件到 `event_tx`，TUI 可以通过 `backend_rx` 继续接收。

2. **Telegram draft 编辑的时间敏感性**：Draft 编辑需要实时（每 1200ms 或内容超过 4096 字符）。如果使用 `run_single_turn()`，事件异步到达，需要确保 draft 更新在 turn 完成期间持续进行。

3. **QQ 的 8 轮硬限制 vs CancellationToken**：QQ 限制工具循环最多 8 轮，Telegram 无限。使用 `run_agent_loop` 的 `CancellationToken` 可以统一为外部触发的中止，消除硬限制差异。

4. **`persist_assistant_message` 的 token 用量**：TUI 从 `self.context_usage` 设置 token 用量。AgentRuntime 不跟踪这个。如果统一，需要在 AgentRuntime 中也捕获 `BackendEvent::UsageStats`。

5. **会话一致性**：Gateway 维护内存中的 `conversation`（在 `handle_message` 中加载，in-place 更新），而 `run_agent_loop` 从 DB 重新加载。改用 `run_agent_loop` 意味着 Gateway 需要调整对内存 conversation 的依赖。

6. **⚠️ `execute_call_spawned()` 的 `SessionStore` 线程安全**：`SessionStore` 内部使用 `rusqlite::Connection`，**不是** `Send` + `Sync`。但当前 `execute_call` 已经通过 `&self`（非 `&mut`）传递 `store` 引用，SQLite 连接在只读访问时是线程安全的。`execute_call_spawned` 中 clone `store` 需要确认每个 clone 持有独立的连接（`SessionStore::open` 返回新连接），或者使用 `Arc<Mutex<SessionStore>>` 封装。

7. **⚠️ `catch_unwind` 的限制**：`AssertUnwindSafe` 会抑制编译器的 unwind-safety 检查。需要确保被捕获的闭包不持有 `&Mut` 引用或 `Guard` 等破坏 unwind-safety 的类型。在 `execute_call_spawned` 中，所有参数都是 clone 的 owned 值，安全。

8. **`edit` 工具的真实 panic 根因**：分析显示 `edit_file` 本身的错误处理是完整的（全部用 `?`）。panic 更可能来自 `line_slice()` 的 `lines[..start]` 越界（当前调用者都传递合法索引，但后续修改可能引入越界）或 `apply_patch_contents:217` 的 `&line_fragments[cursor..]` 越界（malformed patch）。`catch_unwind` 可以将这些 panic 转为用户可见的错误消息，而非进程崩溃。
---

## 架构关系图

```
┌─────────────────────────────────────────────────────────────┐
│                    AgentRuntime                              │
│              (src/agent/runtime.rs)                          │
│                                                              │
│  ┌──────────────────────────────────────────────────┐        │
│  │ ✅ compose_system_prompt()   ← TUI/Web/Gateway   │        │
│  │ ✅ build_request_messages()  ← Web/Gateway       │        │
│  │    ❌ TUI 仍直接调用 ContextManager               │        │
│  │ ✅ tool_definitions()        ← Web/Gateway       │        │
│  │    ❌ TUI 仍直接调用 all_definitions()             │        │
│  │ ✅ run_single_turn()         ← Web               │        │
│  │    ❌ Telegram/QQ 各有一份重复实现 ~145+60 行     │        │
│  │ ❌ execute_tool_calls()      ← 顺序同步阻塞       │        │
│  │    ✨ 目标: 改用 execute_call_spawned() 并行执行   │        │
│  │ ✅ run_agent_loop()          ← Web               │        │
│  │    ❌ Telegram/QQ 各有一份重复循环 ~100+50 行     │        │
│  │ ✅ persist_assistant_message()                    │        │
│  │    ❌ TUI 手动调用 store.append_message()          │        │
│  │                                                  │        │
│  │ ✨ 新增功能:                                      │        │
│  │   • CancellationToken (run_agent_loop)           │        │
│  │   • Context compaction (maybe_compact)           │        │
│  │   • 返回 ToolExecutionResult (execute_tool_calls)│        │
│  │   • 🔜 execute_call_spawned (panic-safe + async) │        │
│  └──────────────────────────────────────────────────┘        │
│                                                              │
│  依赖注入:                                                   │
│  - LlmClient / ToolRegistry / ContextManager                 │
│  - SessionStore / MemoryStore                                │
└─────────────────────────┬───────────────────────────────────┘
                          │
        ┌─────────────────┼─────────────────┐
        │                 │                 │
   ┌────▼────┐      ┌─────▼──────┐    ┌────▼────┐
   │  TUI    │      │    Web     │    │Gateway  │
   │         │      │            │    │(Tg,QQ)  │
   │ compose │      │ run_agent  │    │ compose │
   │ via     │      │ _loop()    │    │ via     │
   │ agent ✅│      │ ✅         │    │ agent ✅│
   │         │      │ SSE events │    │ build ✅│
   │ □ build │      │ HTTP abort │    │ execute │
   │ □ tools │      │            │    │ ✅      │
   │ □ persist│     │            │    │         │
   │         │      │            │    │ □ stream│
   │ ──── 工具执行 ─│ ──── 工具执行─│ ──── 工具执行─│
   │ 🔴 Phase 1:   │ 🔴 顺序同步  │ 🔴 顺序同步  │
   │   spawn_block │   阻塞async  │   阻塞async  │
   │ 🔴 Phase 2:   │   任务线程   │   任务线程   │
   │   内联阻塞UI  │ 🔴 无panic   │ 🔴 无panic   │
   │ 🔴 无panic    │   保护       │   保护       │
   │   保护        │              │              │
   │ ──── 目标 ────│ ──── 目标 ───│ ──── 目标 ───│
   │ 🟢 全部工具用 │ 🟢 全部工具  │ 🟢 全部工具  │
   │   spawn_block │   并行在     │   并行在     │
   │   + catch_   │   spawn_    │   spawn_    │
   │   unwind     │   blocking   │   blocking   │
   └──────────┘   └─────────────┘ └────────────┘
    ✅ = 已使用 AgentRuntime
    □ = 仍可直接替换（低风险）
    ❌ = 仍有重复实现
    🔴 = 当前风险
    🟢 = 改造后状态


## 代码行数统计 (粗略)

| 文件 | 新增/修改 |
|---|---|
| `src/agent/runtime.rs` | ~650 行新代码（含 CancellationToken、context compaction、10 个单元测试） |
| `src/tooling/registry.rs` | +5 行 |
| `src/web/state.rs` | +6 行 |
| `src/web/mod.rs` | +45 行 |
| `src/web/routes/messages.rs` | ~-140 行净减少 (删除旧重复代码) |
| `src/gateway/telegram/channel.rs` | +50 行（新增 agent 字段 + 替换 compose/build/execute） |
| `src/gateway/qq.rs` | +40 行（新增 agent 字段 + 替换 compose/build/execute） |
| `src/gateway/shared.rs` | -18 行（删除 compose_system_prompt） |
| `src/app/mod.rs` | -10 行（compose_system_prompt 简化为委托调用） |
| `src/app/runtime/run.rs` | +12 行（App::new_with_paths 中初始化 AgentRuntime） |

AgentRuntime 的实现比被替换的 web 代码更精简，因为不再需要手动处理流式事件循环中低层细节。
