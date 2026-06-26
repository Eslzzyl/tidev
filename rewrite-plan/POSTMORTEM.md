# Tidev 重写事后分析

> 日期：2026-06-26
> 范围：对 Phase 5（tidev-agent）和 Phase 6（tidev-tui）重写过程的完整复盘

---

## 目录

1. [功能缺失清单](#1-功能缺失清单)
2. [已正确移植的模块](#2-已正确移植的模块)
3. [痛苦的根源](#3-痛苦的根源)
4. [正确的架构设计](#4-正确的架构设计)
5. [迁移到正确架构的步骤](#5-迁移到正确架构的步骤)

---

## 1. 功能缺失清单

### 1.1 tidev-agent — 核心问题区域

旧 `tidev-engine/src/agent/` 共 **2,614 行**，新 `tidev-agent` 共 **1,009 行**。缺失约 **1,600 行**的实质功能。

| 旧文件 | 行数 | 新状态 | 缺失详情 |
|--------|------|--------|---------|
| `agent/mod.rs` | 237 | ✅ 基本移植 | AgentType、AgentDefinition 已存在 |
| `agent/runtime/types.rs` | 91 | ✅ 已移植 | ApprovedTool、PendingToolApproval 等 |
| `agent/runtime/agent_loop.rs` | 880 | ⚠️ 混杂 | 循环结构在，但持有不该有的字段（hooks, permission_tx, tool_registry），这些应该是上层注入的 |
| `agent/runtime/subagent.rs` | 558 | ❌ 占位符 | `run_subagent()` 不创建子 agent，只返回空字符串。子 agent 调度机制缺失 |
| `agent/runtime/mod.rs` | 327 | ❌ 不存在 | AgentRuntime 被 SessionManager 替代，但 SessionManager 接口和旧 TUI 不匹配 |
| `agent/runtime/persistence.rs` | 109 | ❌ 不存在 | 消息持久化辅助函数（`save_turn`、`persist_tool_result` 等完整版本） |
| `agent/prompts.rs` | 252 | ❌ 不存在 | 每种 agent type 的系统提示词模板（6 种 agent 各自的 system prompt） |
| `agent/factories.rs` | 138 | ❌ 不存在 | Agent 工厂函数、模型覆盖（不同 agent 可用不同模型配置） |
| `agent/runtime/tests.rs` | 382 | ❌ 不存在 | 完整的集成测试套件（使用 mock store + mock LLM） |

### 1.2 tidev-tui — 旧架构残留

TUI 代码基本是从旧版直接复制（87 文件，~28,000 行），但新架构已经改变了事件模型，导致以下残留：

#### 事件处理残留

| 位置 | 问题 | 说明 |
|------|------|------|
| `lib.rs:921-923` | `SubagentStatus` 和 `SubagentToolResult` 字符串引用 | 这些 `BackendEvent` 变体已被删除，但 TUI 代码中仍有匹配逻辑 |
| `lib.rs:1028-1050` | `running_subagent_executions` 和 `cancel_running_subagents` | 旧子 agent 追踪逻辑，新架构中前端直接订阅子会话通道 |
| `lib.rs` 多处 | `active_request_id` | 新架构中每个会话自己追踪请求 ID，前端不需要管理 |
| `lib.rs` 多处 | `cached_sessions` | 新架构不需要缓存其他会话的运行时状态 |

#### SessionManager API 不匹配

TUI 的 `App::new_with_paths()` 使用 struct literal 构造 `SessionManager`：

```rust
let agent = tidev_agent::SessionManager {
    workspace_root: ...,
    config_dir: ...,
    config_paths: ...,
    config: ...,
    auth: ...,
    // ...共 13 个字段
};
```

但新架构的 `SessionManager` 应该只有 3 个字段（`store`、`llm`、`sessions`）。TUI 持有的这些数据应该直接存放在 `App` 中，不需要经过 `SessionManager`。

#### 初始化流程问题

TUI 的 `core/run.rs` 中：
- `shell::init()` 调用参数类型不匹配（已修复，但方式不优雅）
- `run_agent_loop_with_permission_channel()` 通过 `block_on` 同步阻塞运行 agent 循环，这在新架构中应该通过 `spawn()` + 事件订阅实现

### 1.3 完全未移植的模块

| 旧模块 | 行数 | 应去位置 | 功能 |
|--------|------|---------|------|
| `process.rs` | 46 | `tidev` root crate | `restart_self()` 跨平台进程替换 |
| `tmp.rs`（扫描/清理逻辑） | 175 | `tidev` root crate | `scan_temp_files()` / `clean_temp_files()` 扫描清理 /tmp 中的 tidev 临时文件 |
| `logging.rs`（文件轮转版本） | 169 | `tidev-config` | 基于 fern 的文件日志轮转系统（大小限制、文件数限制、异步刷新） |
| `llm_bridge.rs` | 35 | `tidev-llm` | LLM provider 桥接（当前已被直接集成到 engine 中） |

### 1.4 非核心但缺失的模块

这些模块在旧 engine 中存在但在重写中明确跳过：

| 模块 | 行数 | 说明 |
|------|------|------|
| `memory/` | ~1,500 | 记忆/图谱/保留系统。架构不稳定，跳过是合理决策 |
| `sandbox/` | ~800 | bwrap/landlock/seatbelt 沙箱。Linux only，跳过合理 |
| `provider_setup/` | ~500 | API key 初始化流程。非核心，可在需要时移植 |

### 1.5 总量总结

| 类别 | 行数 | 状态 |
|------|------|------|
| 旧 tidev-engine 总代码 | ~19,469 | — |
| 已移植且功能完整 | ~15,000 | ✅ |
| 缺失/占位（tidev-agent） | ~2,200 | ❌ |
| 缺失/占位（其他模块） | ~425 | ❌ |
| 非核心跳过（memory/sandbox/setup） | ~2,800 | ⏸ 合理跳过 |

---

## 2. 已正确移植的模块

以下模块在新架构中功能完整，与旧版等价：

| Crate | 行数 | 功能 |
|-------|------|------|
| `tidev-types` | 1,015 | 共享类型、prompts、reasoning、theme |
| `tidev-session` | 2,020 | 会话模型、BackendEvent、统计数据、系统信息 |
| `tidev-storage` | 4,438 | SQLite 持久化、schema、migrations、压缩 |
| `tidev-config` | 2,194 | 配置加载、provider 管理、auth |
| `tidev-llm` | 4,835 | 全部 4 个 LLM provider（Anthropic、OpenAI、Gemini、debug） |
| `tidev-hooks` | 420 | HookEngine、matcher、runner 完整 |
| `tidev-instructions` | 564 | 指令文件解析完整 |
| `tidev-snapshot` | 1,687 | Git snapshot diff/revert/restore 完整 |
| `tidev-sync` | 364 | SSH session sync 完整 |
| `tidev-search` | 866 | 文件搜索、模糊路径索引完整 |
| `tidev-mcp` | 671 | MCP 客户端（stdio/http/sse 传输）完整 |
| `tidev-tools` | 6,411 | 所有内置工具（file/exec/search/web/apply_patch/todo 等）完整 |
| `tidev-context` | 769 | 上下文管理、自动压缩完整 |
| `tidev-notification` | 310 | 桌面通知完整 |

---

## 3. 痛苦的根源

### 根本原因：实施顺序错误

`IMPLEMENTATION-PLAN.md` 第 985 行明确写着：

> **Phase 5 是 Phase 6 的前置依赖，因为 TUI 事件管道需要 Per-Session Bus API**

但实际做法是：

```
旧 TUI (~28,000 行, 87 文件)
  依赖旧 AgentRuntime API
       ↓
新 SessionManager 被扭曲出 14 个字段来兼容旧 TUI
新 AgentLoop 被迫长出 permission_tx / hooks / tool_registry
新子 agent 机制被旧 TUI 的 SubagentStatus 事件束缚
       ↓
两边都不对：新架构不纯粹，旧 TUI 不兼容
```

**不是 architecture 错了，是 implementation order 错了。** 先改 consumer（TUI），再改 producer（tidev-agent），顺序反了。

### 直接原因：增量重写 = 永不重写

"增量替换"策略在拆分解耦的底层 crate（config/storage/llm/tools）时非常成功，因为这些 crate 有清晰的接口边界。但在 tidev-agent 和 tidev-tui 这里失败，因为它们的接口是紧密耦合的——你不能"增量地"把一个同步运行的 agent 循环换成异步事件通道，而让 TUI 完全不变。

错误假设：**"SessionManager 可以长得像 AgentRuntime，这样 TUI 不用改"**。结果是把新架构塞进旧壳子里，两头不靠。

### 具体表现

| 决策 | 结果 |
|------|------|
| SessionManager 用 struct literal 构造 | 被迫暴露 14 个公开字段 |
| AgentLoop 持有 ToolRegistry | 职责混乱，但为了执行工具不得不持有 |
| AgentLoop 持有 HookEngine | 引入非 Send 类型，导致 tokio::spawn 出问题 |
| AgentLoop 持有 permission_tx | 审批逻辑和循环逻辑耦合 |
| 子 agent 用 `run_subagent()` 内联执行 | 和文档的 SessionManager 调度设计完全相反 |
| TUI 的 `run_agent_loop_with_permission_channel` | 一个函数名试图兼容两种架构 |
| 试图保持 TUI 代码不变 | 73 个编译错误，每个都是架构冲突的体现 |

---

## 4. 正确的架构设计

### 4.1 核心原则

1. **每个组件做一件事**：AgentLoop 只执行循环，SessionManager 只管理生命周期，前端只渲染
2. **没有共享可变状态**：数据通过通道流动，没有锁，没有竞争
3. **子 agent 是独立一等公民**：有自己的通道、自己的生命周期、自己的状态
4. **前端是纯订阅者**：不需要 `with_temporary_session_context`，不需要缓存运行时

### 4.2 通道模型

```
┌─ Frontend ──────────────────────────────────────┐
│                                                   │
│  active_session = SessionManager.subscribe(id_a)  │
│  child_overlay  = SessionManager.subscribe(id_b)  │
│                                                   │
│  loop {                                           │
│      select! {                                    │
│          event = active_session.recv() => render, │
│          event = child_overlay.recv() => overlay, │
│      }                                            │
│  }                                                │
└──────────────────────┬──────────────────────────┘
                       │
           ┌───────────┼───────────┐
           ▼                       ▼
   ┌──────────────┐       ┌──────────────┐
   │  AgentLoop A  │       │  AgentLoop B │
   │  (parent)     │       │  (child)     │
   │               │       │              │
   │  event_tx ────┤       │  event_tx ───┤
   │  ctrl_tx ───┐ │       │  ctrl_tx ─┐  │
   └─────────│───┘ │       └─────────│──┘  │
             │     │                 │     │
             │     └── SessionManager┘     │
             │         spawn / cancel      │
             │         subscribe           │
             └─────────────────────────────┘
```

### 4.3 组件定义

#### AgentLoop（~9 字段）

```rust
/// Per-session LLM ↔ tool execution loop.
/// One instance per session. Owns no shared state.
pub struct AgentLoop {
    session_id: Uuid,
    model: ActiveModel,
    context: ContextManager,
    tools: Vec<ToolDefinition>,           // LLM-facing schema only
    tool_registry: ToolRegistry,          // tool execution (Arc'd internally, Send)
    store: Arc<tokio::sync::Mutex<SessionStore>>,
    llm: LlmClient,
    event_tx: UnboundedSender<BackendEvent>,  // → Frontend
    ctrl_tx: UnboundedSender<ControlEvent>,   // → SessionManager
    cancel_token: CancellationToken,
}
```

职责：
- LLM 流式调用
- 工具执行（通过 `tool_registry`）
- 上下文压缩检测
- LLM 重试

非职责：
- 不知道父子关系
- 不知道 session 生命周期
- 不管理审批流程（审批由前端通过独立通道处理）

#### SessionManager（3 字段）

```rust
/// Session lifecycle manager. Does NOT execute agent loops.
pub struct SessionManager {
    store: Arc<tokio::sync::Mutex<SessionStore>>,
    llm: LlmClient,
    sessions: Arc<Mutex<HashMap<Uuid, SessionState>>>,
}

struct SessionState {
    cancel_token: CancellationToken,
    event_tx: UnboundedSender<BackendEvent>,
    ctrl_tx: UnboundedSender<ControlEvent>,
    parent_id: Option<Uuid>,
}
```

方法：
- `spawn(config: SessionConfig) -> SessionHandle` — 创建事件通道 → spawn AgentLoop → 注册 → 返回 handle
- `subscribe(id: Uuid) -> Option<UnboundedReceiver<BackendEvent>>` — 返回事件 channel 的 receiver
- `cancel(id: Uuid)` — 取消 token，级联取消所有子会话

内部机制：
- 后台 listener task 监听所有活跃 session 的 `ctrl_rx`
- 收到 `ControlEvent::SubtaskRequested` → 调用 `spawn()` 创建子会话 → 等待结果 → 通过 oneshot 回复

#### ControlEvent（不经过 BackendEvent）

```rust
/// Internal control events between AgentLoop and SessionManager.
/// NOT part of BackendEvent — not sent to frontend.
pub enum ControlEvent {
    /// AgentLoop encountered a "task" tool call.
    /// SessionManager should spawn a child session and send result back.
    SubtaskRequested {
        tool_call: ToolCall,
        agent_type: String,
        prompt: String,
        response_tx: oneshot::Sender<ToolExecutionResult>,
    },
}
```

`ControlEvent` 不需要 `Clone`、不需要 `Serialize`，只在一个单向通道中传递。这解决了 `BackendEvent` 不能携带 `oneshot::Sender` 的问题。

#### BackendEvent（不变）

`BackendEvent` 不需要 `SubtaskRequested` 变体。子 agent 的任务对前端透明——前端只看到子会话自己的事件通道。

#### 前端订阅模型

```rust
impl App {
    /// Switch the active session by swapping event channels.
    fn switch_session(&mut self, id: Uuid) -> Result<()> {
        self.event_rx = self.session_manager
            .subscribe(id)
            .context("session not found")?;
        self.conversation = self.store.load_conversation(id);
        self.active_session_id = id;
        Ok(())
    }

    /// Handle subagent overlay by subscribing to child session channel.
    fn show_subagent_overlay(&mut self, child_id: Uuid) {
        if let Some(rx) = self.session_manager.subscribe(child_id) {
            self.subagent_overlays.push((child_id, rx));
        }
    }
}
```

不再需要：
- `with_temporary_session_context` — 取消
- `cached_sessions` — 取消
- `running_subagent_executions` — 取消
- `active_request_id` — 取消
- `processing_child_session` — 取消

### 4.4 子 agent 流程

```
AgentLoop(P)                SessionManager              AgentLoop(C)        Frontend
    │                            │                           │                  │
    │  LLM → task("explorer")    │                           │                  │
    │                            │                           │                  │
    │  ctrl_tx.send(             │                           │                  │
    │    SubtaskRequested{       │                           │                  │
    │      tool_call,            │                           │                  │
    │      prompt,               │                           │                  │
    │      response_tx           │                           │                  │
    │    })                      │                           │                  │
    │───────────────────────────>│                           │                  │
    │   [等待结果, 不阻塞其他工具] │                           │                  │
    │                            │  spawn({                   │                  │
    │                            │    model,                  │                  │
    │                            │    tools: filtered,        │                  │
    │                            │    parent_id: P            │                  │
    │                            │  })                       │                  │
    │                            │──────────────────────────>│                  │
    │                            │                           │  [独立运行]        │
    │                            │  subscribe(C)             │                  │
    │                            │  ── rx ────────────────────────────────────>│
    │                            │                           │  Delta           │
    │                            │                           │─────────────────>│
    │                            │                           │  (子 agent 卡片)   │
    │                            │                           │  StreamEnd       │
    │                            │                           │─────────────────>│
    │                            │  collect_result()         │                  │
    │                            │  await child.join()       │                  │
    │                            │                           │                  │
    │  response_tx.send(result)  │                           │                  │
    │<───────────────────────────│                           │                  │
    │                            │                           │                  │
    │  ToolCompleted(task,       │                           │                  │
    │    result)                 │                           │                  │
    │  event_tx ─────────────────────────────────────────────────────────────>│
```

关键点：
- 父 AgentLoop 不阻塞，通过 oneshot 异步等待
- 子 AgentLoop 完全独立运行
- 前端通过 subscribe 直接读取子会话事件流
- 不需要 SubagentStatus / SubagentToolResult / SubagentCompleted 聚合事件

### 4.5 级联取消

```rust
impl SessionManager {
    fn spawn(&self, config: SessionConfig) -> SessionHandle {
        let cancel_token = CancellationToken::new();
        let child_token = config.parent_id
            .and_then(|pid| self.get_token(pid))
            .map(|parent| parent.child_token())
            .unwrap_or_else(|| cancel_token.child_token());

        // AgentLoop 使用 child_token
        // 父取消 → 子自动取消（CancellationToken 内置机制）
    }
}
```

### 4.6 APP 状态

新架构下的 TUI App 状态应精简为：

```rust
struct App {
    // 核心
    session_manager: SessionManager,
    active_session_id: Uuid,
    active_event_rx: UnboundedReceiver<BackendEvent>,
    conversation: Conversation,

    // 子 agent 卡片
    subagent_overlays: Vec<(Uuid, UnboundedReceiver<BackendEvent>)>,

    // 配置（不经过 SessionManager）
    config: SharedConfig,
    workspace_root: PathBuf,
    auth: AuthStore,
    store: SessionStore,
    tools: ToolRegistry,

    // ... 其余 UI 状态
}
```

---

## 5. 迁移到正确架构的步骤

### 已完成步骤（2026-06-26）

```text
cargo check --workspace   ✅
cargo test --workspace    ✅  298 tests 全部通过
cargo clippy -p tidev-agent -p tidev-types -p tidev-tools ✅  无新增警告
```

#### Step 1: 定义 ControlEvent ✅

在 `tidev-agent/src/types.rs` 中定义了 `ControlEvent` 枚举，包含 `SubtaskRequested` 和 `SubtaskCompleted` 变体。

```rust
pub enum ControlEvent {
    SubtaskRequested {
        parent_session_id: Uuid,
        child_session_id: Uuid,
        agent_type: AgentType,
        description: String,
        ack_tx: oneshot::Sender<()>,
    },
    SubtaskCompleted {
        child_session_id: Uuid,
        success: bool,
    },
}
```

`ControlEvent` 不属于 `BackendEvent`（携带 `oneshot::Sender`，不向前端发送）。
父 AgentLoop 通过 `control_tx` 通道发送事件，SessionManager 通过 `process_control_events()` 接收追踪。

#### Step 2: 重写 AgentLoop ✅

- 字段：`session_id`, `model`, `conversation`, `context`, `tools`, `tool_registry`, `store`, `llm`, `event_tx`, `cancel_token`, `mode`, `agent_type`, `workspace_root`, `system_prompt`, `permission_tx`, `hooks`, `session_manager`, `can_delegate`, `control_tx`
- `run()` → `into_run_fut()`: LLM turn → 工具执行（task 走 `run_subagent()` + ControlEvent，常规走 `ToolRegistry::execute_call`）→ 压缩检查 → 循环
- 审批流程通过 `permission_tx` 通道转发给 TUI 处理
- 子 agent 创建使用 `into_run_fut()` 避免 async recursion

#### Step 3: 重写 SessionManager ✅

- 字段精简：`store`, `llm`, `active`, `control_tx`/`control_rx`
- `new()` 创建 control channel，control_tx 分发给每个 AgentLoop
- `process_control_events()` 接收并记录子 agent 生命周期事件
- `run_agent_loop_with_permission_channel()` 接收外部参数（tool_registry, hooks, session_manager）

#### Step 4: 重写 TUI 事件管道 ✅

- 已删除 `cached_sessions`、`running_subagent_executions` 引用
- `switch_session()` 保留但简化
- 前端字段直接存放在 `App` struct 中，不再经过 SessionManager

#### Step 5: 移植剩余模块 ✅

| 模块 | 状态 |
|------|------|
| `prompts.rs` — 全部 6 种 agent 系统提示词 | ✅ 已移植 + 7 个测试 |
| `factories.rs` — agent 工厂函数 | ✅ 已移植 + 5 个测试 |
| `persistence.rs` — 消息持久化辅助函数 | ✅ 已移植 |
| `tests.rs` — 集成测试（mock store） | ✅ 13 个新测试 |
| `AgentType` 统一到 tidev-types | ✅ + 6 个测试 |
| `process.rs` → root crate | ✅ 已移出 (src/process.rs) |
| `tmp.rs` 逻辑 → root crate | ✅ 已移出 (src/tmp.rs) |
| `logging.rs` 文件轮转 → tidev-config | ✅ 已恢复文件日志轮转 |

### 验证标准（已全部通过）

```
cargo build --workspace         # 全部编译  ✅
cargo test --workspace          # 全部测试通过  ✅  298 tests
cargo clippy --workspace        # 无警告  ✅
```
