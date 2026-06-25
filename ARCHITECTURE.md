# Tidev Architecture (Proposed)

## 现状问题

当前 agent 运行时最根本的架构缺陷是：**所有会话共享同一个事件通道**。

父会话和子会话（subagent）通过同一个 `UnboundedSender<BackendEvent>` 发送事件，
事件通过 `BackendEvent.session_id` 字段来区分目标会话。这导致：

- `BackendEvent` 的每个变体都要携带 `session_id: Uuid`
- TUI 收到事件后必须检查 `session_id`，不匹配则做全量状态切换
  （`with_temporary_session_context`）
- 子会话的流式内容无法直达前端，必须通过三个聚合事件中转
  （`SubagentStatus` / `SubagentToolResult` / `SubagentCompleted`）
- 事件处理管道的每一层都要判断当前会话上下文
- TUI 的 App struct 需要维护跨会话的运行时缓存、活跃请求 ID、取消令牌等状态

这些都不是功能需求带来的复杂度，而是「共享通道」这个设计选择的后果。

---

## 核心思路：Per-Session Event Bus

```
                    ┌──────────────────────┐
                    │    SessionManager     │
                    │  spawn / cancel /     │
                    │  subscribe / navigate │
                    └──┬────┬────┬──────────┘
                       │    │    │
              ┌────────┤    │    ├────────┐
              ▼             ▼             ▼
       ┌──────────┐  ┌──────────┐  ┌──────────┐
       │ Session A │  │ Session B │  │ Session C │
       │ (parent)  │  │ (child)   │  │ (child)   │
       │           │  │           │  │           │
       │ AgentLoop │  │ AgentLoop │  │ AgentLoop │
       │ event_tx_a│  │ event_tx_b│  │ event_tx_c│
       └─────┬─────┘  └─────┬─────┘  └─────┬─────┘
             │              │              │
             ▼              ▼              ▼
       ┌──────────────────────────────────────┐
       │           Frontend (TUI)              │
       │                                       │
       │  active: Session B                    │
       │  → reads event_tx_b                   │
       │  → no demux, no context switch        │
       │                                       │
       │  overlay: Session C (subagent card)   │
       │  → reads event_tx_c (compact render)  │
       └──────────────────────────────────────┘
```

**每个会话拥有独立的事件通道。前端订阅当前活跃会话的通道，
不需要 `session_id` 字段，不需要上下文切换，不需要聚合事件。**

---

## 组件设计

### 1. AgentLoop — 可复用的 Agent 循环

主 agent 和子 agent 使用同一份实现。差异只是参数：

```rust
pub struct AgentLoop {
    session_id: Uuid,
    model: ActiveModel,
    context: ContextManager,
    tools: Vec<ToolDefinition>,
    store: Arc<SessionStore>,
    llm: LlmClient,
    event_tx: UnboundedSender<BackendEvent>,
    cancel_token: CancellationToken,
}

impl AgentLoop {
    pub async fn run(mut self) -> Result<()> {
        loop {
            // 1. Check cancellation
            if self.cancel_token.is_cancelled() { break }

            // 2. Load messages from store
            let messages = self.store.load_messages(self.session_id)?;

            // 3. Build request via ContextManager
            let request = self.context.build_request_messages(&messages);

            // 4. Stream LLM turn
            let turn = self.stream_turn(request).await?;

            // 5. Execute tools
            if turn.has_tool_calls() {
                self.execute_tools(&turn.tool_calls).await?;
                // loop back to step 1 (next turn)
            } else {
                // Final response — done
                self.event_tx.send(BackendEvent::StreamEnd)?;
                break;
            }
        }
    }

    async fn stream_turn(&self, messages: Vec<Message>) -> Result<AssistantTurn> {
        let (tx, mut rx) = unbounded_channel();
        tokio::spawn(self.llm.stream_chat(/* ... */, tx));

        let mut turn = AssistantTurn::default();
        while let Some(event) = rx.recv().await {
            // Forward to frontend via self.event_tx
            self.event_tx.send(event.clone())?;
            // Accumulate into turn
            match event { /* Delta → turn.content, Finished → break, etc. */ }
        }
        Ok(turn)
    }

    async fn execute_tools(&self, calls: &[ToolCall]) -> Result<()> {
        for call in calls {
            if call.name == "task" {
                // task 工具：通过 SessionManager 创建子会话
                // 工具本身不执行子会话，它把执行委托给上层
                // 通过回调或事件通知 SessionManager
                self.event_tx.send(BackendEvent::SubtaskRequested {
                    tool_call: call.clone(),
                })?;
                // 工具返回一个占位结果，子会话完成后被替换
            } else {
                let result = ToolRegistry::execute(call).await;
                self.event_tx.send(BackendEvent::ToolCompleted {
                    tool_call: call, result,
                })?;
            }
        }
    }
}
```

关键点：
- `task` 工具不执行子会话，它发出 `SubtaskRequested` 事件
- `SessionManager` 监听此事件，创建子会话
- AgentLoop 不需要知道父/子关系

### 2. SessionManager — 中心协调器

```rust
pub struct SessionManager {
    sessions: HashMap<Uuid, SessionHandle>,
    store: Arc<SessionStore>,
    llm: LlmClient,
    tools: ToolRegistry,
}

struct SessionHandle {
    task: JoinHandle<()>,
    cancel: CancellationToken,
    events: UnboundedSender<BackendEvent>,
    parent: Option<Uuid>,
}

impl SessionManager {
    /// 创建新会话，返回 (session_id, event_receiver)
    pub fn create_session(
        &mut self,
        parent_id: Option<Uuid>,
        config: SessionConfig,
    ) -> (Uuid, UnboundedReceiver<BackendEvent>) { ... }

    /// 取消会话及其所有子会话
    pub fn cancel_session(&self, session_id: Uuid) { ... }

    /// 前端订阅某个会话的事件通道
    pub fn subscribe(&self, session_id: Uuid) -> UnboundedReceiver<BackendEvent> { ... }

    /// 处理 SubtaskRequested 事件：创建子会话
    fn handle_subtask(&mut self, parent_id: Uuid, call: ToolCall) { ... }

    /// 等待子会话完成并收集结果
    async fn collect_subtask_result(child_id: Uuid) -> ToolExecutionResult { ... }

    /// 获取会话的当前消息列表
    pub fn load_conversation(&self, session_id: Uuid) -> Conversation { ... }
}
```

关键点：
- 创建会话时自动为子会话创建 child `CancellationToken`
- 父令牌取消时自动级联取消所有子令牌
- `subscribe` 返回 Receiver，前端通过它接收事件

### 3. Frontend — 纯展示层

TUI 不再需要：
- `cached_sessions: HashMap<Uuid, CachedSessionRuntime>`
- `running_subagent_executions`
- `pending_assistant_turns`
- `active_request_id`
- `request_cancel_token`
- `processing_child_session`
- `pending_request`（大部分）
- `with_temporary_session_context`
- `is_active_request` / `prime_active_request`

TUI 只需要：

```rust
struct App {
    session_manager: SessionManager,
    active_session: (Uuid, UnboundedReceiver<BackendEvent>),
    conversation: Conversation,
    // 子 agent card overlay — 通过订阅子会话事件实现
    subagent_overlays: Vec<(Uuid, UnboundedReceiver<BackendEvent>)>,
}
```

切换会话：
```rust
fn switch_session(&mut self, session_id: Uuid) {
    let rx = self.session_manager.subscribe(session_id);
    self.active_session = (session_id, rx);
    self.conversation = self.session_manager.load_conversation(session_id);
    // 不需要缓存/恢复任何运行时状态
}
```

事件处理：
```rust
fn handle_event(&mut self, event: BackendEvent) {
    match event {
        Delta { content } => append_to_streaming(content),
        Finished { turn } => finalize_turn(turn),
        ToolCompleted { tool_call, result } => add_tool_result(tool_call, result),
        TurnStarting { request_id } => create_streaming_message(),
        StreamEnd => cleanup(),
        // 没有 SubagentStatus / SubagentToolResult / SubagentCompleted
        // 没有 session_id 检查
    }
}
```

子 agent 卡片渲染：订阅子会话的通道，原地渲染原始 `Delta` 事件。

---

## 事件定义（简化后）

```rust
enum BackendEvent {
    // 流式事件 — 不需要 session_id
    Delta { request_id: u64, content: String },
    ReasoningDelta { request_id: u64, content: String },
    ToolCallUpdated { request_id: u64, tool_call: ToolCall },
    UsageStats { /* ... */ },

    // 生命周期事件
    TurnStarting { request_id: u64 },
    Finished { request_id: u64, turn: AssistantTurn },
    Failed { request_id: u64, error: String },
    Retrying { /* ... */ },
    StreamEnd { request_id: u64 },

    // 工具执行 — 不需要 SubagentStatus / SubagentToolResult / SubagentCompleted
    ToolCompleted { request_id: u64, tool_call: ToolCall, result: ToolExecutionResult },
    ShellOutput { /* ... */ },

    // 子任务请求 — 由 SessionManager 消费
    SubtaskRequested { tool_call: ToolCall },
}
```

**删除的变体：**
- `SubagentStatus` — 替代方案：前端订阅子会话通道
- `SubagentToolResult` — 同上
- `SubagentCompleted` — 同上
- 所有变体中的 `session_id: Uuid` 字段 — 按通道路由，不再需要

---

## 父子会话生命周期

```
SessionManager              Parent AgentLoop          Child AgentLoop        Frontend
    │                             │                        │                   │
    │  create_session(parent)     │                        │                   │
    │  → spawn ParentLoop         │                        │                   │
    │────────────────────────────>│                        │                   │
    │                             │                        │                   │
    │                    [parent loop running]              │                   │
    │                             │                        │                   │
    │                             │  LLM generates         │                   │
    │                             │  task("explorer", ...)  │                   │
    │                             │                        │                   │
    │  receive SubtaskRequested   │                        │                   │
    │<────────────────────────────│                        │                   │
    │                             │                        │                   │
    │  create_session(child)      │                        │                   │
    │  → spawn ChildLoop          │                        │                   │
    │─────────────────────────────────────────────────────>│                   │
    │                             │                        │                   │
    │  subscribe(child)           │                        │                   │
    │─────────────────────────────────────────────────────────────────────────>│
    │                             │                        │                   │
    │                             │              [child loop running]          │
    │                             │                        │                   │
    │                             │                  Delta / ReasoningDelta     │
    │                             │                        │───────────────────>│
    │                             │                        │  (subagent card)  │
    │                             │                        │                   │
    │                             │                  ToolCompleted             │
    │                             │                        │───────────────────>│
    │                             │                        │                   │
    │                             │                  StreamEnd                 │
    │                             │                        │───────────────────>│
    │                             │                        │                   │
    │  collect_result(child)      │                        │                   │
    │  inject into parent loop    │                        │                   │
    │────────────────────────────>│  ToolCompleted(task)   │                   │
    │                             │───────────────────────>│                   │
    │                             │                        │                   │
```

关键点：
- 父 AgentLoop 发出 `SubtaskRequested` 后继续执行其他工作
- SessionManager 异步创建并运行子会话
- 子会话的结果通过 `collect_result` 异步注入回父会话
- 前端独立订阅每个会话的事件通道

---

## 级联取消

```rust
// SessionManager 创建会话时
fn create_session(&mut self, parent_id: Option<Uuid>, config: SessionConfig) {
    let cancel = CancellationToken::new();

    // 如果是子会话，创建 child_token
    let child_cancel = parent_id
        .and_then(|pid| self.sessions.get(&pid))
        .map(|parent| parent.cancel.child_token())
        .unwrap_or_else(|| cancel.clone());

    let handle = SessionHandle {
        task: tokio::spawn(AgentLoop { cancel_token: child_cancel, ... }.run()),
        cancel,
        events: tx,
        parent: parent_id,
    };
}
```

`child_token()` 是 `CancellationToken` 内置方法。父令牌取消 → 所有子令牌自动取消。
不需要手动传播取消状态。

---

## 受益

| 方面 | 当前 | 新架构 |
|------|------|--------|
| 事件路由 | 按 session_id 分发 → 上下文切换 | 按通道分发 → 零开销 |
| 子会话流式传输 | 三个聚合事件中转 | 直接读取子会话通道 |
| 子 agent 调度 | 串行/并行 inline 在 agent_loop.rs | SessionManager 统一管理 |
| 取消传播 | 共享 token，但 spawned task 不受控 | child_token 级联取消 |
| TUI 会话管理 | cached_sessions + with_temporary_session_context | subscribe + load_conversation |
| 事件处理器 | 每个 handler 检查 session_id / request_id | 无上下文判断 |
| 代码行数 | subagent.rs 550行 + agent_loop.rs 880行 + TUI 事件管道数百行 | AgentLoop ~400行 + SessionManager ~200行 |

---

## 迁移策略

1. 在 `tidev-engine` 中实现 `AgentLoop` 独立组件（与现有代码并行）
2. 在 `tidev-engine` 中实现 `SessionManager`（与现有调度逻辑并行）
3. 将 `task` 工具从 inline dispatch 改为通过 `SessionManager` 创建子会话
4. 逐个删除 `SubagentStatus` / `SubagentToolResult` / `SubagentCompleted`
5. 重写 TUI 事件处理管道，删除 `with_temporary_session_context`
6. 删除 `BackendEvent.session_id` 字段和三个废弃变体
7. 删除旧代码

每一步都是独立的，可以增量替换。没有「大爆炸」式重写。
