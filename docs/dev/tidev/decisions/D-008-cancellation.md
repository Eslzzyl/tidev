# D-008: 取消机制设计

**日期**: 2026-07-03  
**状态**: ✅ 已定案

## 需求

用户取消时，所有正在进行的操作必须立即终止，没有任何延迟：

| 操作 | 终止要求 |
|------|----------|
| LLM 流式请求 | 断开 HTTP 连接，停止接收 |
| Bash 命令 | 杀进程组（SIGKILL） |
| 子代理（可能嵌套 bash） | 级联终止 |
| 工具执行（read/write/search 等） | 正在运行的跑完，未启动的不执行 |

已产生的部分流式内容保留，追加"用户已取消"说明。

## 架构概览

```
TUI: runtime.cancel()
  │
  ▼
tidev-core::Runtime::cancel()
  ├── 1. cancel_token.cancel()          ← 合作式终止信号
  ├── 2. 短暂等待（100ms）              ← 给合作式退出机会
  ├── 3. run_loop_handle.abort()        ← 强制杀 loop task
  └── 4. kill_all_children()            ← 全局进程注册表清理
```

两个层面：**合作式**（检查点 + select!）和 **强制式**（abort + 进程杀）。

---

## 1. 合作式终止

### 1.1 CancellationToken 所有权

`CancellationToken`（来自 `tokio_util::sync`）由 tidev-core 的 `Runtime` 创建。

```
Runtime::build():
  cancel_token = CancellationToken::new()

Runtime::run_session():
  config = AgentLoopConfig { cancel: cancel_token.child_token(), ... }
  handle = tokio::spawn(run_agent_loop(&ctx, config))
  self.run_loop_handle = Some(handle)

Runtime::cancel():
  self.cancel_token.cancel()
```

### 1.2 AgentLoopConfig

AgentLoopConfig 只保留 session_id、system_prompt、thinking_level、AgentEvent
通道、CancellationToken 和排队消息。mode、审批和 BackendEvent 都属于
tidev-core 宿主层。

tidev-agent 新增 `tokio-util = { version = "0.7", features = ["sync"] }` 依赖。

### 1.3 run_agent_loop 检查点

每轮开始检查：

```rust
// loop_.rs
if config.cancel.is_cancelled() {
    return Ok(());
}
```

### 1.4 LLM 流式终止（stream_turn）

`AgentContext::stream_turn()` 实现中使用 `select!` 赛跑 `cancel.cancelled()` 和 LLM 事件通道，确保流式过程中能立即响应取消。

```rust
// tidev-core 的 AgentContext impl
async fn stream_turn(&self, ...) -> Result<AssistantTurn> {
    let (tx, mut rx) = unbounded_channel();
    let handle = tokio::spawn({
        let llm = self.llm.clone();
        async move { llm.stream_chat(..., tx, ...).await; }
    });

    let mut turn = AssistantTurn::default();
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Some(ev) => {
                        // forward to TUI, accumulate turn
                    }
                    None => break,  // LLM 完成
                }
            }
            _ = self.cancel_token.cancelled() => {
                handle.abort();     // 强制断 HTTP 连接
                break;
            }
        }
    }
    Ok(turn)
}
```

### 1.5 Bash 工具终止

Bash 使用进程组执行。tidev-tools 的 exec.rs 保留现有的 `ACTIVE_CHILDREN` 全局注册表和 `kill_process_group` 函数。

在 `select!` 中赛跑 bash 进程和取消信号：

```rust
// tidev-tools/exec.rs
tokio::select! {
    status = child.wait() => {
        // 正常退出
    }
    _ = cancel_token.cancelled() => {
        kill_process_group(pid);
        // 返回已产生的部分输出
    }
}
```

全局注册表仅用于强制式清理（见第 2 节）。bash 工具自身通过 `select!` 处理取消。

### 1.6 子代理

子 agent loop 共享同一个 `cancel_token`（`child_token()` 克隆）。同样在每轮开始检查 `is_cancelled()`，流式时用 `select!` 赛跑取消。

不需要独立的 SubagentHost trait。子代理创建和取消是 core 的
`execute_tools()` 内部细节；子代理和主代理共享 `run_agent_loop()`。

---

## 2. 强制式终止

合作式终止无法覆盖的极端情况：

- LLM 流式的 `select!` 已响应，但 spawn 的 LLM task 正在等待网络 I/O
- Bash 的 `child.wait()` 先于 `cancel_token.cancelled()` 完成，下一个循环才检查
- `kill_process_group` 调用后子进程变成了孤儿进程

### 2.1 JoinHandle::abort()

tidev-core 持有 `run_agent_loop` 的 `JoinHandle`。取消→等待 100ms→abort：

```rust
impl Runtime {
    pub fn cancel(&self) {
        self.cancel_token.cancel();          // 1. 发信号

        // 2. 给合作式退出 100ms 窗口
        let handle = self.run_loop_handle.take();
        if let Some(h) = handle {
            tokio::time::sleep(Duration::from_millis(100)).await;
            h.abort();                       // 3. 强制杀
        }

        kill_all_children();                 // 4. 进程兜底
    }
}
```

`abort()` 的安全性：

- LLM 流式 task 不持有任何共享锁或关键资源
- 被 abort 后 task 内所有资源被 drop，包括 reqwest Response body → HTTP 连接断开
- tool 执行 task 同理：read/write/search 等是纯 I/O，abort 后文件句柄被 drop

### 2.2 进程注册表清理

`kill_all_children()` 遍历 `ACTIVE_CHILDREN` 做两阶段杀：

```rust
pub fn kill_all_children() {
    let pids: Vec<u32> = ACTIVE_CHILDREN.lock().unwrap().iter().copied().collect();
    for &pid in &pids {
        unsafe { libc::kill(pid as i32, libc::SIGTERM); }
    }
    std::thread::sleep(Duration::from_millis(200));
    for &pid in &pids {
        unsafe { libc::kill(pid as i32, libc::SIGKILL); }
    }
}
```

无论 bash 还是子代理的 bash，子进程都注册到同一个全局 `ACTIVE_CHILDREN`。

---

## 3. 已流式内容保留

取消前已通过 `BackendEvent::Delta`/`ReasoningDelta` 发送到 TUI 的内容**不动**。

取消后 tidev-core 通过 `event_tx` 发送：

```rust
BackendEvent::StreamEnd { session_id, request_id }
```

TUI 收到后：

- 如果当前有 streaming 消息（`MessageRole::Assistant` 且 `streaming = true`），将其标记为已完成
- 在其后追加一条系统消息："用户已取消"

已流式的 Delta/text 内容保留在用户的对话视图中。

---

## 4. 与旧实现的区别

| 项目 | 旧实现 | 新实现 |
|------|--------|--------|
| token 所有权 | TUI 创建，传给 agent loop | tidev-core 创建，TUI 调用 `runtime.cancel()` |
| 主 loop LLM 流式 | 无 cancel 检查，阻塞到 turn 结束 | select! 赛跑 cancel |
| 工具 task | 无 cancel 检查，跑完才结束 | select!（bash）+ JoinHandle abort（兜底） |
| 后台残留 | 多个场景可能残留 | 合作式 + 强制式双层保障 |

## 5. 涉及的文件

| 文件 | 变更 |
|------|------|
| `tidev-agent/Cargo.toml` | 新增 `tokio-util` 依赖 |
| `tidev-agent/src/context.rs` | `AgentLoopConfig` 加 `cancel: CancellationToken` |
| `tidev-agent/src/loop_.rs` | `run_agent_loop` 每轮检查 `is_cancelled()` |
| `tidev-core` AgentContext impl | `stream_turn()` 用 select! 赛跑 cancel |
| `tidev-tools/src/exec.rs` | `execute_tool_call` 接受 `CancellationToken`，bash 用 select! |
| `tidev-tools/src/lib.rs` | 导出 `kill_all_children`、`ACTIVE_CHILDREN` |
