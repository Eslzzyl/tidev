# Codex `/goal` 命令工作原理

> 本文档基于 codex 子模块的源码分析，详细描述了 `/goal` 命令从用户输入到代理行为影响的全链路设计。

---

## 目录

1. [概述](#1-概述)
2. [数据结构](#2-数据结构)
3. [命令解析与分发](#3-命令解析与分发)
4. [事件系统与应用层处理](#4-事件系统与应用层处理)
5. [服务端 API 实现](#5-服务端-api-实现)
6. [持久化存储](#6-持久化存储)
7. [Token 与时间记账](#7-token-与时间记账)
8. [代理侧工具集成](#8-代理侧工具集成)
9. [目标提示注入机制](#9-目标提示注入机制)
10. [客户端 UI 渲染](#10-客户端-ui-渲染)
11. [完整数据流](#11-完整数据流)
12. [关键文件索引](#12-关键文件索引)

---

## 1. 概述

`/goal` 是 Codex TUI 中用于**长期运行任务的目标管理**的斜杠命令。它允许用户为当前会话设置一个持久化的目标，该目标会：

- 存储在服务端数据库中
- 持续追踪 token 消耗和耗时
- 在每次对话回合间以隐藏 developer 消息注入到模型上下文中
- 允许模型通过工具（`create_goal` / `get_goal` / `update_goal`）自主查询和标记完成
- 在 TUI 底部状态栏显示紧凑的状态指示器

### 功能开关

Goals 是一个 **experimental 功能**，需要在配置中启用 `Feature::Goals`。如果未启用，`/goal` 命令将无响应。

---

## 2. 数据结构

### 2.1 ThreadGoal（协议层）

**文件:** `codex/codex-rs/protocol/src/protocol.rs:3633`

```rust
pub struct ThreadGoal {
    pub thread_id: ThreadId,
    pub objective: String,            // 目标描述文本
    pub status: ThreadGoalStatus,     // 当前状态
    pub token_budget: Option<i64>,    // 可选的 token 预算上限
    pub tokens_used: i64,             // 已消耗的 token 数
    pub time_used_seconds: i64,       // 已消耗的时间（秒）
    pub created_at: i64,              // 创建时间戳
    pub updated_at: i64,              // 最后更新时间戳
}
```

### 2.2 ThreadGoalStatus（状态枚举）

**文件:** `codex/codex-rs/protocol/src/protocol.rs:3609`

```rust
pub enum ThreadGoalStatus {
    Active,          // 活跃：代理正在主动追求该目标
    Paused,          // 暂停：目标被用户暂停
    BudgetLimited,   // 预算耗尽：token 预算已用尽，自动触发
    Complete,        // 完成：目标已达成（由模型通过工具标记）
}
```

状态转换图：

```
                    ┌─────────┐
         ┌─────────→│  Active  │←──────────┐
         │          └────┬─────┘           │
         │               │                 │
    /goal pause     token耗尽         /goal resume
         │               │                 │
         │          ┌────▼──────┐          │
         │          │BudgetLimited│         │
         │          └───────────┘          │
         │                                 │
    ┌────▼─────┐              ┌───────────┐│
    │  Paused  │──────────────→│ Complete  ││
    └──────────┘  模型调用      └───────────┘│
                   update_goal              │
                                            │
    ┌──────────┐                            │
    │  (无目标) │←───────────────────────────┘
    └──────────┘       /goal clear
```

### 2.3 ThreadGoalSetMode（设置模式）

**文件:** `codex/codex-rs/tui/src/app_event.rs:62`

```rust
pub enum ThreadGoalSetMode {
    ConfirmIfExists,   // 如果已存在目标，则询问用户确认是否替换
    ReplaceExisting,   // 直接替换已有目标
}
```

### 2.4 各层的对应类型

| 概念 | Protocol 层 | State 层 | App-Server-Protocol 层 | TUI 层 |
|------|------------|----------|----------------------|--------|
| Goal 结构体 | `ThreadGoal` | `ThreadGoal` (含 `goal_id`) | `v2::ThreadGoal` | `AppThreadGoal` (别名) |
| 状态枚举 | `ThreadGoalStatus` | `ThreadGoalStatus` (含 `is_active()`/`is_terminal()`) | `v2::ThreadGoalStatus` | `AppThreadGoalStatus` (别名) |
| 通知 | `ThreadGoalUpdatedEvent` | — | `v2::ThreadGoalUpdatedNotification` | — |

> **注意:** State 层（`codex-rs/state`）的 `ThreadGoal` 包含额外的 `goal_id: String` 字段，用于乐观并发控制（OCC）。

---

## 3. 命令解析与分发

### 3.1 斜杠命令注册

**文件:** `codex/codex-rs/tui/src/slash_command.rs:39`

```rust
pub enum SlashCommand {
    // ...
    Goal,
    // ...
}
```

使用 `#[strum(serialize_all = "kebab-case")]`，序列化为 `"goal"`，对应输入 `/goal`。

### 3.2 分发逻辑

**文件:** `codex/codex-rs/tui/src/chatwidget/slash_dispatch.rs`

#### `/goal`（无参数，裸命令）

```rust
// 第 205-218 行
SlashCommand::Goal => {
    // 检查 Goals feature 是否启用
    if let Some(thread_id) = self.thread_id {
        // 发送 OpenThreadGoalMenu 事件，弹窗显示目标摘要
        self.app_event_tx.send(AppEvent::OpenThreadGoalMenu { thread_id });
    } else {
        // 会话未启动时显示使用帮助
        self.add_info_message("Usage: /goal <objective>", Some("Example: /goal improve benchmark coverage"));
    }
}
```

#### `/goal clear|pause|resume`（控制子命令）

```rust
// 第 624-661 行
let control_command = match trimmed.to_ascii_lowercase().as_str() {
    "clear"  => Some(GoalControlCommand::Clear),
    "pause"  => Some(GoalControlCommand::SetStatus(AppThreadGoalStatus::Paused)),
    "resume" => Some(GoalControlCommand::SetStatus(AppThreadGoalStatus::Active)),
    _        => None,
};
```

控制命令会立即发送对应的 `AppEvent`，不会产生对话轮次。

#### `/goal <任意文本>`（设置目标）

```rust
// 第 663-703 行
let objective = args.trim();
// 发送 SetThreadGoalObjective 事件，mode = ConfirmIfExists
self.app_event_tx.send(AppEvent::SetThreadGoalObjective {
    thread_id,
    objective: objective.to_string(),
    mode: ThreadGoalSetMode::ConfirmIfExists,
});
```

**注意:** `--tokens` 等选项**不会被客户端解析**，而是作为目标文本的一部分透传给服务端。token_budget 通过 API 参数单独传递（由上层或模型工具设置）。

---

## 4. 事件系统与应用层处理

### 4.1 AppEvent 定义

**文件:** `codex/codex-rs/tui/src/app_event.rs:221-241`

```rust
pub enum AppEvent {
    OpenThreadGoalMenu { thread_id: ThreadId },
    SetThreadGoalObjective { thread_id: ThreadId, objective: String, mode: ThreadGoalSetMode },
    SetThreadGoalStatus { thread_id: ThreadId, status: ThreadGoalStatus },
    ClearThreadGoal { thread_id: ThreadId },
}
```

### 4.2 事件分发

**文件:** `codex/codex-rs/tui/src/app/event_dispatch.rs:654-671`

| 事件 | 处理器 | 说明 |
|------|--------|------|
| `OpenThreadGoalMenu` | `open_thread_goal_menu()` | 从服务端拉取目标信息并显示摘要弹窗 |
| `SetThreadGoalObjective` | `set_thread_goal_objective()` | 调用服务端 API 设置/替换目标 |
| `SetThreadGoalStatus` | `set_thread_goal_status()` | 调用服务端 API 更新状态 |
| `ClearThreadGoal` | `clear_thread_goal()` | 调用服务端 API 清除目标 |

### 4.3 处理器实现详情

**文件:** `codex/codex-rs/tui/src/app/thread_goal_actions.rs`

#### `open_thread_goal_menu()`（第 15-43 行）

1. 调用 `app_server.thread_goal_get(thread_id)` 获取当前目标
2. 如果存在目标 → 调用 `chat.show_goal_summary(goal)` 显示摘要
3. 如果不存在目标 → 显示使用帮助信息

#### `set_thread_goal_objective()`（第 72-120 行）

1. 如果 `mode == ConfirmIfExists` 且目标已存在 → 显示替换确认对话框
2. 否则直接调用 `app_server.thread_goal_set(thread_id, objective, ActiveStatus)`

#### `set_thread_goal_status()`（第 122-149 行）

1. 调用 `app_server.thread_goal_set(thread_id, status_only)` 仅更新状态

#### `clear_thread_goal()`（第 151-177 行）

1. 调用 `app_server.thread_goal_clear(thread_id)` 清除目标

---

## 5. 服务端 API 实现

### 5.1 协议定义

**文件:** `codex/codex-rs/app-server-protocol/src/protocol/common.rs:492-509`

| 端点 | 方法 | 说明 |
|------|------|------|
| `thread/goal/set` | `ThreadGoalSet` | 创建或更新目标 |
| `thread/goal/get` | `ThreadGoalGet` | 获取当前目标 |
| `thread/goal/clear` | `ThreadGoalClear` | 清除目标 |

所有端点均标记为 `#[experimental]`。

### 5.2 请求/响应类型

**文件:** `codex/codex-rs/app-server-protocol/src/protocol/v2.rs`

**ThreadGoalSetParams（第 4252 行）:**
```rust
pub struct ThreadGoalSetParams {
    pub thread_id: String,
    pub objective: Option<String>,   // 提供时更新/设置目标文本
    pub status: Option<ThreadGoalStatus>,  // 提供时更新状态
    pub token_budget: Option<Option<i64>>, // 提供时更新 token 预算
}
```

**ThreadGoalGetResponse（第 4285 行）:**
```rust
pub struct ThreadGoalGetResponse {
    pub goal: Option<ThreadGoal>,
}
```

**ThreadGoalClearResponse（第 4299 行）:**
```rust
pub struct ThreadGoalClearResponse {
    pub cleared: bool,
}
```

### 5.3 服务端处理器

**文件:** `codex/codex-rs/app-server/src/request_processors/thread_goal_processor.rs`

#### `thread_goal_set()`（第 30 行）

核心逻辑 `thread_goal_set_inner()`（第 92-218 行）:

1. 解析参数中的 `objective`（如果提供，需通过 `validate_thread_goal_objective()` 校验）
2. 解析 `token_budget`（需通过 `validate_goal_budget()` 校验）
3. 如果存在正在运行中的线程，先通知它准备外部目标变更（`prepare_external_goal_mutation()`）
4. 如果提供了 `objective`：
   - 如果目标已存在且 objective 相同且状态不是 Complete → `update_thread_goal()`
   - 否则 → `replace_thread_goal()`（新 ID，新目标）
5. 如果未提供 `objective` 但提供了 `status`/`token_budget` → 调用 `update_thread_goal()`
6. 发送通知到相应线程的监听器
7. 发出目标状态快照（`emit_thread_goal_snapshot()`）

#### `thread_goal_get()`（第 40 行）

1. 调用 `state_db.get_thread_goal(thread_id)` 读取
2. 转换为 `ThreadGoalGetResponse` 返回

#### `thread_goal_clear()`（第 49 行）

1. 通知运行中的线程准备外部变更
2. 调用 `state_db.clear_thread_goal(thread_id)`
3. 发送 `ThreadGoalClearedNotification`
4. 发出目标状态快照

---

## 6. 持久化存储

### 6.1 数据库 Schema

**文件:** `codex/codex-rs/state/migrations/0029_thread_goals.sql`

```sql
CREATE TABLE thread_goals (
    thread_id TEXT PRIMARY KEY NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    goal_id TEXT NOT NULL,
    objective TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('active', 'paused', 'budget_limited', 'complete')),
    token_budget INTEGER,
    tokens_used INTEGER NOT NULL DEFAULT 0,
    time_used_seconds INTEGER NOT NULL DEFAULT 0,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
```

- `thread_id` 是主键且外键引用 `threads(id)`，级联删除
- `status` 使用 CHECK 约束限制合法值
- 每个线程同时只能有一个目标

### 6.2 State 层映射

**文件:** `codex/codex-rs/state/src/model/thread_goal.rs`

- `ThreadGoal`（第 52-63 行）: ORM 模型，与数据库字段对应
- `ThreadGoalRow`（第 65-75 行）: 内部行结构，用于 SQL 查询映射
- 提供了 `TryFrom<&str>` 用于状态字符串与枚举之间的转换

---

## 7. Token 与时间记账

### 7.1 核心记账函数

**文件:** `codex/codex-rs/state/src/runtime/goals.rs`

#### `account_thread_goal_usage()`（第 316-405 行）

```rust
pub async fn account_thread_goal_usage(
    &self,
    thread_id: ThreadId,
    time_delta_seconds: i64,     // 本轮的耗时增量
    token_delta: i64,            // 本轮的 token 消耗增量
    mode: ThreadGoalAccountingMode,  // 记账模式
    expected_goal_id: Option<&str>,  // 乐观锁：期望的 goal_id
) -> anyhow::Result<ThreadGoalAccountingOutcome>
```

#### 记账模式（第 15-21 行）

```rust
pub enum ThreadGoalAccountingMode {
    ActiveStatusOnly,   // 仅活跃状态
    ActiveOnly,         // 活跃 + budget_limited
    ActiveOrComplete,   // 活跃 + budget_limited + complete
    ActiveOrStopped,    // 活跃 + paused + budget_limited
}
```

#### 自动预算耗尽检测

当 `tokens_used >= token_budget` 时，SQL UPDATE 语句自动将状态切换为 `BudgetLimited`（第 362-366 行）：

```sql
status = CASE
    WHEN {budget_limit_status_filter}
         AND token_budget IS NOT NULL
         AND tokens_used + ? >= token_budget
    THEN 'budget_limited'
    ELSE status
END
```

### 7.2 触发时机

记账在以下时刻被触发（`codex/codex-rs/core/src/goals.rs`）:

| 时机 | 函数 | 作用 |
|------|------|------|
| 模型响应后 | `account_thread_goal_progress()` | 记录本轮 token 消耗 |
| 轮次完成时 | `account_thread_goal_wall_clock_usage()` | 记录耗时 |
| 外部变更前 | `ExternalMutationAboutToBeDone` | 同步最新状态 |
| 工具执行时 | `ToolCompletedGoal` | 工具完成目标后的记账 |

---

## 8. 代理侧工具集成

### 8.1 工具定义

**文件:** `codex/codex-rs/tools/src/goal_tool.rs`

模型可以通过三个工具与目标系统交互：

| 工具名 | 函数 | 说明 |
|--------|------|------|
| `get_goal` | `create_get_goal_tool()` | 获取当前线程的目标和状态（只读） |
| `create_goal` | `create_create_goal_tool()` | 创建新目标（仅当无目标时有效） |
| `update_goal` | `create_update_goal_tool()` | 更新目标状态（**只能设为 complete**） |

### 8.2 工具约束

- **`create_goal`**: 必须提供 `objective`，可选 `token_budget`。如果已存在目标则调用失败
- **`update_goal`**: 只能将状态设为 `"complete"`。暂停/恢复等状态变更由用户通过 UI 控制
- **`get_goal`**: 无参数，返回当前目标完整状态

### 8.3 工具注册

**文件:** `codex/codex-rs/tools/src/tool_registry_plan.rs:223-241`

```rust
if config.goal_tools {
    plan.register_tool(create_get_goal_tool());
    plan.register_handler("get_goal", ToolHandlerKind::Goal);
    plan.register_tool(create_create_goal_tool());
    plan.register_handler("create_goal", ToolHandlerKind::Goal);
    plan.register_tool(create_update_goal_tool());
    plan.register_handler("update_goal", ToolHandlerKind::Goal);
}
```

受 `Feature::Goals` 和 `with_goal_tools_allowed()` 开关控制。

### 8.4 工具处理器

**文件:** `codex/codex-rs/core/src/tools/handlers/goal.rs`

- `handle_get_goal()`: 从 state_db 读取目标并返回格式化 JSON
- `handle_create_goal()`: 验证参数 → 创建新目标 → 触发 GoalRuntimeEvent
- `handle_update_goal()`: 验证 status="complete" → 更新状态 → 触发 ToolCompletedGoal

---

## 9. 目标提示注入机制

这是目标系统的核心：目标信息如何影响模型的行为。

### 9.1 续作提示（Continuation Prompt）

**文件:** `codex/codex-rs/core/templates/goals/continuation.md`

当代理空闲且有活跃目标时，系统会生成一条隐藏的 `developer` 角色消息注入到模型上下文中：

```markdown
Continue working toward the active thread goal.

<untrusted_objective>
{{ objective }}
</untrusted_objective>

Budget:
- Time spent: {{ time_used_seconds }} seconds
- Tokens used: {{ tokens_used }}
- Token budget: {{ token_budget }}
- Tokens remaining: {{ remaining_tokens }}
```

提示中包含详细的完成审计指令，要求模型在执行更新目标前进行严格验证。

### 9.2 预算耗尽提示（Budget Limit Prompt）

**文件:** `codex/codex-rs/core/templates/goals/budget_limit.md`

当 token 预算耗尽时注入：

```markdown
The active thread goal has reached its token budget.

<untrusted_objective>
{{ objective }}
</untrusted_objective>

Budget: ...

The system has marked the goal as budget_limited, so do not start new substantive work
for this goal. Wrap up this turn soon...
```

### 9.3 注入机制

**文件:** `codex/codex-rs/core/src/goals.rs`

#### `maybe_start_goal_continuation_turn()`（第 1067-1110 行）

```
用户发送消息
       │
       ▼
   模型响应
       │
       ▼
   轮次完成
       │
       ▼
   代理进入空闲
       │
       ▼
   maybe_continue_goal_if_idle_runtime()
       │
       ├── maybe_start_turn_for_pending_work()  ← 先检查是否有待处理工作
       └── maybe_start_goal_continuation_turn() ← 然后检查是否需要目标续作
              │
              ├── 获取 continuation_lock（信号量）
              ├── 检查目标是否活跃且 state_db 中仍有效
              ├── 检查是否没有正在进行的活跃轮次
              ├── 生成 GoalContinuationCandidate
              │     └── developer 角色消息包含渲染后的续作提示
              ├── 注入到 pending_input
              └── 启动新轮次
```

#### `goal_continuation_candidate_if_active()`（第 1146-1215 行）

使用 `CONTINUATION_PROMPT_TEMPLATE.render()` 渲染模板，其中 objective 经过 `escape_xml_text()` 转义以防止注入。

#### `maybe_start_goal_continuation_turn()` 中的守卫条件

1. **continuation_lock**: 信号量防止并发续作
2. **活跃轮次检查**: 如果有正在进行的轮次则跳过
3. **目标有效性检查**: 重新从数据库读取目标，确认 goal_id 和 status 仍然匹配
4. **线程持久性检查**: 临时线程（ephemeral thread）不支持目标

### 9.4 预算耗尽转向（Budget Limit Steering）

当 token 预算在轮次中间耗尽时，系统会通过 `inject_response_items()` 动态注入预算限制提示，告诉模型停止新工作并总结当前进展。

---

## 10. 客户端 UI 渲染

### 10.1 底部状态栏指示器

**文件:** `codex/codex-rs/tui/src/bottom_pane/footer.rs:537-594`

状态栏右侧显示紧凑的目标状态指示器（品红色）：

| 状态 | 显示文本 |
|------|---------|
| Active | `Pursuing goal (12.5K / 50K)` 或 `Pursuing goal (2m)` |
| Paused | `Goal paused (/goal resume)` |
| BudgetLimited | `Goal unmet (63.9K / 50K tokens)` 或 `Goal abandoned` |
| Complete | `Goal achieved (40K tokens)` 或 `Goal achieved (10h 12m)` |

### 10.2 目标摘要弹窗

**文件:** `codex/codex-rs/tui/src/chatwidget/goal_menu.rs`

当用户输入裸 `/goal` 命令时，显示：

```
Goal
Status: active
Objective: improve benchmark coverage
Time used: 2h 30m
Tokens used: 12.5K
Token budget: 50K

Commands: /goal pause, /goal clear
```

### 10.3 目标状态管理器

**文件:** `codex/codex-rs/tui/src/chatwidget/goal_status.rs`

`GoalStatusState` 负责：

1. 存储当前目标数据及观察时间戳
2. 实时计算活跃时间（考虑当前轮次的运行时间）
3. 生成 `GoalStatusIndicator` 供底部状态栏渲染

### 10.4 时间格式化

**文件:** `codex/codex-rs/tui/src/goal_display.rs`

- `format_goal_elapsed_seconds()`: 将秒数格式化为人类可读形式（`2d 23h 42m`、`1h 30m`、`30m`、`59s`）
- `format_tokens_compact()`: 将 token 数格式化为紧凑形式（`98.5K`、`12.5M`、`1.2B`）

### 10.5 实时刷新

**文件:** `codex/codex-rs/tui/src/chatwidget.rs:9657-9674`

`refresh_goal_status_indicator_for_time_tick()` 每秒被定时器触发，重新计算并更新底部状态栏中的目标指示器。

### 10.6 暂停目标的恢复提示

当对话恢复时，如果检测到目标处于 Paused 状态，会弹出选择对话框：

```
Resume paused goal?
Goal: Keep improving the bare goal command until it feels calm and useful.

› 1. Resume goal   Mark it active and continue when idle
  2. Leave paused  Keep it paused; use /goal resume later
```

---

## 11. 完整数据流

### 11.1 设置目标 (`/goal improve benchmark coverage`)

```
用户输入 /goal improve benchmark coverage
       │
       ▼
  SlashDispatch (slash_dispatch.rs)
  ─────────────────────────────────
  • 解析 "goal" 命令
  • trimmed = "improve benchmark coverage"
  • 非控制命令（clear/pause/resume）
  • 发送 SetThreadGoalObjective { objective, mode: ConfirmIfExists }
       │
       ▼
  AppEventHandler (event_dispatch.rs)
  ────────────────────────────────────
  • 匹配 SetThreadGoalObjective
  • 调用 set_thread_goal_objective()
       │
       ▼
  ThreadGoalActions (thread_goal_actions.rs)
  ───────────────────────────────────────────
  • mode == ConfirmIfExists → 检查是否有已有目标
  • 无已有目标 → 直接调用 app_server.thread_goal_set()
       │
       ▼
  JSON-RPC 调用 thread/goal/set
       │
       ▼
  ThreadGoalProcessor (thread_goal_processor.rs)
  ────────────────────────────────────────────────
  • 校验 objective（validate_thread_goal_objective）
  • 校验 token_budget（validate_goal_budget）
  • 通知运行中线程准备变更
  • 调用 state_db.replace_thread_goal()
       │
       ▼
  StateDb (state/src/runtime/goals.rs)
  ───────────────────────────────────────
  • INSERT OR REPLACE INTO thread_goals
  • 目标创建成功（Active 状态）
       │
       ▼
  • 发出 thread/goal/updated 通知
  • 发出目标状态快照
       │
       ▼
  TUI 收到通知
  ────────────
  • on_thread_goal_updated()
  • 创建 GoalStatusState
  • 更新底部状态栏 → "Pursuing goal (2m)"
```

### 11.2 目标续作（模型空闲时自动触发）

```
代理完成一轮对话，进入空闲状态
       │
       ▼
  Session::maybe_continue_goal_if_idle_runtime()
       │
       ▼
  Session::maybe_start_goal_continuation_turn()
       │
       ├── 获取 continuation_lock
       ├── 检查是否有活跃的 Go
       ├── 检查无进行中的轮次
       ├── 从 state_db 重新读取目标确认有效性
       ├── 生成续作提示（continuation.md 模板）
       │     └── developer 角色消息，包含目标文本和预算信息
       ├── 注入到 pending_input
       └── 启动轮次
            │
            ▼
      模型看到续作提示，继续工作
```

### 11.3 标记目标完成（模型调用 `update_goal`）

```
模型决定目标已完成
       │
       ▼
  调用 update_goal { status: "complete" }
       │
       ▼
  GoalToolHandler (core/src/tools/handlers/goal.rs)
  ──────────────────────────────────────────────────
  • 验证 status == "complete"
  • 调用 session.update_thread_goal(goal_id, Complete)
       │
       ▼
  state_db.update_thread_goal()
  ────────────────────────────
  • UPDATE thread_goals SET status = 'complete'
       │
       ▼
  发送 thread/goal/updated 通知到客户端
       │
       ▼
  TUI 更新状态栏 → "Goal achieved (40K tokens)"
```

### 11.4 暂停/恢复/清除目标

```
/goal pause
       │
       ▼
  SlashDispatch → SetThreadGoalStatus { status: Paused }
       │
       ▼
  AppEventHandler → set_thread_goal_status()
       │
       ▼
  app_server.thread_goal_set(status: paused)
       │
       ▼
  ThreadGoalProcessor → state_db.update_thread_goal(status: paused)
       │
       ▼
  通知 → TUI 更新状态栏 → "Goal paused (/goal resume)"
```

---

## 12. 关键文件索引

### 客户端（TUI）

| 文件 | 用途 |
|------|------|
| `codex/codex-rs/tui/src/slash_command.rs` | `/goal` 命令的枚举定义 |
| `codex/codex-rs/tui/src/chatwidget/slash_dispatch.rs` | 斜杠命令分发逻辑 |
| `codex/codex-rs/tui/src/app_event.rs` | 目标相关 AppEvent 定义 |
| `codex/codex-rs/tui/src/app/event_dispatch.rs` | AppEvent → 处理器的路由 |
| `codex/codex-rs/tui/src/app/thread_goal_actions.rs` | 目标操作的应用层处理器 |
| `codex/codex-rs/tui/src/chatwidget/goal_menu.rs` | 目标摘要弹窗 |
| `codex/codex-rs/tui/src/chatwidget/goal_status.rs` | 目标状态指示器渲染 |
| `codex/codex-rs/tui/src/goal_display.rs` | 目标信息格式化工具 |
| `codex/codex-rs/tui/src/bottom_pane/footer.rs` | 底部状态栏中的目标指示器行 |
| `codex/codex-rs/tui/src/chatwidget.rs` | 目标状态管理（`current_goal_status` 等） |
| `codex/codex-rs/tui/src/app_server_session.rs` | TUI → App-Server 的 JSON-RPC 调用 |

### 服务端

| 文件 | 用途 |
|------|------|
| `codex/codex-rs/app-server/src/request_processors/thread_goal_processor.rs` | 目标 API 的服务端实现 |
| `codex/codex-rs/app-server-protocol/src/protocol/v2.rs` | 目标 API 的请求/响应类型 |
| `codex/codex-rs/app-server-protocol/src/protocol/common.rs` | 目标 API 的端点注册 |
| `codex/codex-rs/state/src/model/thread_goal.rs` | 目标的状态模型 |
| `codex/codex-rs/state/src/runtime/goals.rs` | 目标数据库操作与记账 |
| `codex/codex-rs/state/migrations/0029_thread_goals.sql` | 数据库 schema |

### 核心引擎

| 文件 | 用途 |
|------|------|
| `codex/codex-rs/core/src/goals.rs` | 目标续作、记账调用、代理循环集成 |
| `codex/codex-rs/core/src/tools/handlers/goal.rs` | 模型侧目标工具处理器 |
| `codex/codex-rs/core/templates/goals/continuation.md` | 目标续作提示模板 |
| `codex/codex-rs/core/templates/goals/budget_limit.md` | 预算耗尽提示模板 |
| `codex/codex-rs/tools/src/goal_tool.rs` | 模型侧目标工具定义 |
| `codex/codex-rs/tools/src/tool_registry_plan.rs` | 工具注册与条件包含 |
| `codex/codex-rs/protocol/src/protocol.rs` | 协议层 ThreadGoal 定义 |

---

> **文档生成日期:** 2026-05-12
> **基于 codex 子模块 commit:** 当前工作目录版本
