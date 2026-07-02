# Subagent 设计问题与改进方向

**状态**: 待设计（阶段 2 实现）
**涉及文件**: `tidev-types/src/message.rs`（BackendEvent subagent 变体）、tidev-agent（SubagentHost trait）、tidev-core（SessionManager 实现）

## 旧实现的问题

### 1. AgentRuntime 整体克隆

每次子代理启动都 `.clone()` 整个 AgentRuntime（含 Store、LLM、Tools、Config、Auth），重量级且不必要。

### 2. 嵌套 agent loop 重复实现

`run_subagent_inner`（~500 行）手写了一套完整的 LLM 流式调用 + 工具执行循环，与主 `run_agent_loop` 大量重复。新设计应复用同一个 agent loop 函数，只是传入不同的 config。

### 3. 事件转发混乱

子代理事件通过父的 `event_tx` 发送，TUI 靠 `session_id` 区分子 session 和父 session 的事件。应拆分为两个通道：
- `child_tx`：子代理完整流事件 → TUI 渲染子对话
- `parent_tx`：仅状态/完成通知 → TUI 更新父 session 的子代理卡片

### 4. 串行/并行调度逻辑侵入 agent_loop

读写权限判断和调度策略散落在 agent_loop 中，应在调用方处理。

## architecture.md 中的方案

```rust
pub trait SubagentHost: Send + Sync {
    fn spawn_subagent(
        &self,
        parent_id: Uuid,
        model: &ActiveModel,
        tool_call: &ToolCall,
    ) -> impl Future<Output = ToolExecutionResult>;
}
```

"子代理的 BackendEvent 直接发给 TUI，父 AgentLoop 只等结果。"

## 待实现时确定的细节

### SubagentRequest 结构

trait 签名需要补充的参数（当前签名不够）：
- 子代理类型（Explorer/Fixer/等）→ 决定系统提示词和工具集
- 子通道和父通道的事件发送器
- 取消令牌

### BackendEvent subagent 变体

当前三个变体（`SubagentStatus`、`SubagentToolResult`、`SubagentCompleted`）从旧代码搬来，字段是否合适、是否需要增减，等实现 SubagentHost 时再调整。现在不动。

### CancellationToken

旧代码依赖 `tokio_util::sync::CancellationToken`。是否保留还是用更简单的方式实现取消，等实现时再决定。

## 决策时机

阶段 2：定义 SubagentHost trait，SessionManager 实现它，AgentLoop 改用 trait。
