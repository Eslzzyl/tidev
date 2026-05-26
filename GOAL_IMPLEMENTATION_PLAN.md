# tidev `/goal` 命令实现方案

> 本文档描述 tidev 的 `/goal` 功能实现计划。
> Codex 的 `/goal` 实现分析见 [CODEX_GOAL_COMMAND.md](./CODEX_GOAL_COMMAND.md)。

---

## 1. 概述

`/goal` 命令允许用户为当前会话设定一个持久目标，使 agent 跨轮次自主推进，直到目标达成或被用户干预。

核心设计原则：

- **最小架构侵入**：不引入额外 agent 层（无监工/审查 agent），利用现有 subagent 机制分解大任务
- **以提示词驱动**：借鉴 codex 的 completion audit + fidelity 约束，让提示词本身的约束力对抗模型的偷懒倾向
- **模型可操作**：通过工具让模型能查询和标记目标，形成闭环
- **每轮可见**：目标在每一轮 LLM 调用时都注入上下文中，不会被滚动冲掉

---

## 2. 数据结构

### 2.1 Goal

```rust
pub struct Goal {
    pub session_id: Uuid,
    pub objective: String,
    pub status: GoalStatus,
    pub tokens_used: i64,          // 已消耗 token 数
    pub time_used_seconds: i64,    // 已消耗时间（秒）
    pub created_at: String,        // RFC3339
    pub updated_at: String,        // RFC3339
}
```

不实现 token budget 功能。codex 的 token budget 设计（BudgetLimited 状态 + 预算耗尽提示）在第一版中暂不纳入，保持简单。

### 2.2 GoalStatus

```rust
pub enum GoalStatus {
    Active,
    Paused,
    Complete,
}
```

状态转换：

```
                    ┌─────────┐
                    │  Active  │
                    └────┬─────┘
                         │
               ┌─────────┼─────────┐
               │         │         │
          /goal pause    │    update_goal
               │         │    (complete)
               │    /goal resume    │
               │         │         │
          ┌────▼─────┐   │    ┌────▼──────┐
          │  Paused  │───┘    │  Complete  │
          └──────────┘        └───────────┘

          ┌──────────┐
          │  (无目标) │←── /goal clear (从任何状态)
          └──────────┘
```

---

## 3. 持久化存储

### 3.1 数据库表

在 `tidev-storage` 的 `session_goals` 表中存储，与 session 一一对应：

```sql
CREATE TABLE IF NOT EXISTS session_goals (
    session_id TEXT PRIMARY KEY REFERENCES sessions(id) ON DELETE CASCADE,
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    tokens_used INTEGER NOT NULL DEFAULT 0,
    time_used_seconds INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### 3.2 SessionStore 新增方法

```rust
impl SessionStore {
    /// 获取当前 session 的目标（如无则返回 None）
    fn get_goal(&self, session_id: Uuid) -> Result<Option<Goal>>;

    /// 设置/覆盖目标（INSERT OR REPLACE）
    fn set_goal(&self, session_id: Uuid, objective: &str) -> Result<Goal>;

    /// 更新目标状态
    fn update_goal_status(&self, session_id: Uuid, status: GoalStatus) -> Result<()>;

    /// 清除目标
    fn clear_goal(&self, session_id: Uuid) -> Result<()>;

    /// 累加 token 和耗时
    fn account_goal_usage(&self, session_id: Uuid, tokens: i64, elapsed_secs: i64) -> Result<()>;
}
```

### 3.3 Schema 版本

递增 `SCHEMA_VERSION`，添加迁移。

---

## 4. 引擎集成

### 4.1 注入点

在 `agent_loop.rs` 的 `run_agent_loop_with_tools_inner` 中，现有流程：

```
1. 加载消息 ← 这里注入 goal
2. 拾取排队用户消息
3. 注入指令文件
4. build_request_messages → request_messages
5. stream LLM turn
6. 处理 tool calls
7. ← 循环
```

在步骤 1 和 2 之间（或步骤 4 之后、5 之前），注入一条 system 角色的 goal 提示消息。

**注入条件**：session 有 goal 且 status 为 `Active`。

### 4.2 提示模板

参考 codex 的 continuation prompt，用纯文本渲染：

```
Continue working toward the active thread goal.

<objective>
{{ objective }}
</objective>

Continuation behavior:
- This goal persists across turns. Ending this turn does not require finishing everything now.
- Keep the full objective intact. If it cannot be finished now, make concrete progress.
- Do not redefine success around a smaller or easier task than what is requested.

Completion audit:
Before deciding the goal is achieved, treat completion as unproven and verify against the actual current state:
- Derive concrete requirements from the objective.
- Preserve the original scope; do not redefine success around work that already exists.
- For every requirement, identify authoritative evidence that would prove it, then inspect the relevant current-state sources.
- If any requirement lacks proof, the goal is not complete — continue working.

Task decomposition:
If this objective is large, consider using the task tool to spawn sub-agents for independent sub-tasks. Each sub-agent works in its own context, keeping the main context focused on orchestration.
```

### 4.3 注入方式

Goal 提示以 `MessageRole::System` 角色注入，位于 request_messages 的首位（或次位，如果已有 system prompt）。

注入逻辑封装为 `AgentRuntime::build_goal_prompt(goal: &Goal) -> Option<String>`，返回渲染后的提示文本。

### 4.4 无工具调用时的行为

当 goal 为 `Active` 时，即使模型本轮未产生 tool call，也**继续循环**而不是退出。下一轮再次注入 continuation prompt，迫使模型继续。

当 goal 为 `Complete` 或 `Paused` 时，走原有逻辑（无 tool call 则退出循环）。

### 4.5 Token 记账

每轮结束后，在 `persist_assistant_message` 或之后调用 `account_goal_usage`，累加该轮的 token 消耗和耗时。

---

## 5. 模型工具

注册两个工具，仅在 goal 为 `Active` 时暴露给模型：

### 5.1 get_goal

```json
{
    "name": "get_goal",
    "description": "Get the current active goal for this session, including status and resource usage.",
    "parameters": { "type": "object", "properties": {} }
}
```

返回当前 goal 的完整信息（objective, status, tokens_used, time_used_seconds）。

### 5.2 update_goal

```json
{
    "name": "update_goal",
    "description": "Mark the current goal as complete. Call this only when you have verified every requirement against the current state.",
    "parameters": {
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["complete"],
                "description": "Only 'complete' is supported."
            },
            "reasoning": {
                "type": "string",
                "description": "Brief explanation of verification evidence for each requirement."
            }
        },
        "required": ["status", "reasoning"]
    }
}
```

工具 handler 中做简单的日志记录后更新状态。`reasoning` 参数强迫模型在标记完成时提供理由，有利于审计。

---

## 6. TUI 命令

### 6.1 命令注册

在 `commands.rs` 的 `CommandAction` 中添加 `Goal` 变体，注册 `/goal` 的 `CommandSpec`。

### 6.2 命令解析

| 用户输入 | 行为 |
|----------|------|
| `/goal` | 显示当前 goal 状态（如无则提示用法） |
| `/goal <objective>` | 设置新目标，覆盖已有目标 |
| `/goal clear` | 清除当前目标 |
| `/goal pause` | 暂停目标（状态 → Paused） |
| `/goal resume` | 恢复目标（状态 → Active） |

### 6.3 与 Runtime 的通信

命令 handler 通过 `AgentRuntime` 暴露的 goal 方法操作目标。TUI 持有 runtime 引用，可以直接调用：

- `runtime.set_goal(session_id, objective)`
- `runtime.clear_goal(session_id)`
- `runtime.update_goal_status(session_id, status)`

### 6.4 状态指示器

在底部状态栏添加一行（类似 codex 的 `goal_status_indicator_line`），显示：

- 无目标：不显示
- Active：`⚡ 目标: <截断的目标文本> [Active]  <tokens> tokens`
- Paused：`⏸ 目标: <截断的目标文本> [Paused]`
- Complete：`✅ 目标: <截断的目标文本> [Complete]  <tokens> tokens`

状态信息通过 `BackendEvent::GoalStatusChanged` 事件从引擎推送至 UI。

---

## 7. Gateway 支持

在 `tidev-gateway/src/commands.rs` 中添加 `/goal` 命令解析。

Gateway 平台的 goal 交互简化：

- `/goal <objective>` → 设置目标，回复「目标已设置」
- `/goal` → 回复当前目标状态
- `/goal clear` → 清除目标

Gateway 不需要 TUI 那样实时渲染状态指示器，但可以在每次 agent 回复后附加一行目标状态信息。

---

## 8. Subagent 鼓励

在 goal continuation prompt 中显式鼓励模型使用 subagent 分解大任务：

```
Task decomposition:
If this objective is large, consider using the task tool
to spawn sub-agents for independent sub-tasks. Each sub-agent
works in its own context, keeping the main context focused
on orchestration and verification.
```

这在 tidev 中有天然优势：tidev 已经实现了 `task` 工具（调用 fixer/oracle/explorer 等 sub-agent），goal 模式下只是更积极地引导模型使用它。

---

## 9. 实施阶段

### Phase 1 — 数据模型与存储

- 定义 `Goal`、`GoalStatus` 类型（`tidev-types` 或直接放在 `tidev-storage` 中）
- 添加 `session_goals` 表，递增 SCHEMA_VERSION
- 在 `SessionStore` 中实现 CRUD 方法
- 添加数据迁移

### Phase 2 — 引擎集成

- 实现 continuation prompt 模板（纯文本，编译期嵌入）
- 在 agent loop 中添加 goal 注入逻辑
- 修改无 tool call 退出条件，支持 goal 模式下的自动继续
- 添加 token 记账
- 注册 `get_goal` / `update_goal` 工具

### Phase 3 — TUI 命令

- 添加 `CommandAction::Goal` 和 `/goal` CommandSpec
- 实现命令解析和参数处理
- 添加 goal 状态指示器到底部状态栏
- 添加 `BackendEvent::GoalStatusChanged` 事件

### Phase 4 — Gateway 支持

- 在 gateway 命令解析器中添加 `/goal`
- 实现简化的 goal 交互流程

---

## 10. 未纳入第一版的内容

- **Token budget**：codex 的 `--tokens` 参数和 `BudgetLimited` 状态暂不实现。如后续需要，可基于已有的 `account_goal_usage` 机制扩展。
- **Goal 编辑器**：codex TUI 有 `/goal edit` 打开内联编辑器的功能。第一版中用户直接通过 `/goal <new objective>` 覆盖即可。
- **监工/审查 agent**：不引入额外的 agent 层。对抗模型偷懒依赖提示词约束 + completion audit 工具 reasoning 参数 + subagent 任务分解。
