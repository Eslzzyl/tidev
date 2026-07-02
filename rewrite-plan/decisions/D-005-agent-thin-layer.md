# D-005: tidev-agent 薄层设计

**日期**: 2026-07-02  
**状态**: 待实现

## 背景

旧 `AgentRuntime` 持有所有资源（store、LLM client、tools、config、auth 等），子代理启动时 `.clone()` 整个结构体（见旧 `agent/runtime/mod.rs`）。子代理的 agent loop 完整复制了主 loop 的逻辑（`subagent.rs` ~500 行 vs `agent_loop.rs` ~500 行）。

## 决策

**tidev-agent 只定义 agent 循环的骨架和类型，不持有实现资源。**

```
tidev-agent（薄层）
├── AgentType                    — 7 种 agent 类型的枚举
├── AgentDefinition              — 完整的 agent 配置定义
├── AgentOverride                — 覆盖配置
├── prompts.rs                   — 各 agent 系统提示词
├── AgentContext trait           — 循环需要的外部能力接口
└── run_agent_loop()             — 循环骨架函数

tidev-core（编排层）
└── 实现 AgentContext
└── SessionManager（含 SubagentHost）
```

## AgentContext trait 定义

```rust
#[async_trait]
pub trait AgentContext: Send + Sync {
    /// 获取当前工具列表
    fn tools(&self) -> Vec<ToolDefinition>;

    /// 事件通道
    fn event_tx(&self) -> &UnboundedSender<BackendEvent>;

    /// 流式调用 LLM
    async fn stream_turn(&self, messages: &[Message],
        system_prompt: &str, thinking_level: &ThinkingLevelType) -> Result<AssistantTurn>;

    /// 请求工具权限审批
    async fn request_tool_approval(&self,
        tool_calls: &[ToolCall], mode: SessionMode) -> Result<Vec<ApprovedTool>>;

    /// 执行一批已审批的工具
    async fn execute_tools(&self,
        approved_tools: &[ApprovedTool], request_id: u64) -> Result<Vec<(ToolCall, ToolExecutionResult)>>;

    /// 持久化消息
    async fn save_messages(&self, messages: &[Message]) -> Result<()>;

    /// 加载消息历史
    async fn load_messages(&self) -> Result<Vec<Message>>;
}
```

## 依赖

```
tidev-agent ─── tidev-types
            ├── serde
            ├── async-trait
            └── tokio (sync)
```

不依赖 tidev-storage / tidev-config / tidev-llm / tidev-tools / tidev-mcp。

## 理由

1. **复用**：主 agent 和子 agent 共用同一个 `run_agent_loop()` 函数，只传入不同的 `AgentContext` 实现
2. **可测试**：`AgentContext` 可以 mock，纯循环逻辑可单元测试
3. **边界清晰**：循环"怎么转"在 tidev-agent，"用什么转"在 tidev-core
