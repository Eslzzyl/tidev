# 重写过程中的架构决策记录

## D-001: 合并 tidev-session 进 tidev-types

**日期**: 2026-07-02  
**状态**: 已采纳

### 背景

旧项目有两个共享类型 crate：
- `tidev-types`：`ThinkingLevelType`、`SessionMode` 等配置类枚举
- `tidev-session`：`Message`、`BackendEvent`、`ToolCall` 等运行时数据结构

### 决策

**合并为一个 `tidev-types` crate，删掉 `tidev-session`。**

### 理由

1. **两者本质相同**：都是零业务逻辑的纯数据类型定义，无实际区分标准
2. **类型互相引用**：`Message` 的字段直接使用 `ThinkingLevelType` 和 `SessionMode`，拆开只是制造了一条无意义的依赖边
3. **共享程度一致**：tidev-llm、tidev-agent、tidev-tools、tidev-storage、tidev-tui 都需要同时使用两者的类型
4. **"session" 命名模糊**：容易被误解为"会话管理"，实际内容是消息数据结构

### 模块组织

```
tidev-types/
  src/
    lib.rs         — pub mod reasoning; pub mod prompts; pub mod message;
    reasoning.rs    — ThinkingLevelType 及子级别
    prompts.rs      — SessionMode
    message.rs      — Message, MessageRole, MessageAttachment, ToolCall,
                      ToolExecutionResult, AssistantTurn, BackendEvent 等
```

`message` 比 `session` 更准确地表达了内容——这些是跨 crate 流转的消息协议类型。

---

## D-002: 工具类型系统分层

**日期**: 2026-07-02  
**状态**: 已采纳

### 背景

旧实现中工具相关的类型（`ToolDefinition`、`ToolPermission`、`ToolArgs` trait）和工具实现（file read/write/edit、bash、glob/grep、web 等）都在 `tidev-engine/src/tooling/` 下，没有 crate 边界。

### 决策

**拆分为三层：**

```
tidev-types/src/tools.rs   — 纯类型定义（ToolDefinition, ToolOrigin, ToolPermission,
                              PermissionConfig, ToolArgs trait + macros, Args structs,
                              canonical_tool_name, FileReadStamp）

tidev-tools/               — 工具实现（builtin/ 下的 read/write/edit/bash/glob/grep 等，
                              execute_tool_call() 路由，ToolContext，SkillCatalog）

tidev-core                 — 编排层（ToolRegistry：统一注册 builtin + MCP 工具，
                              权限检查，文件读取追踪）
```

### 理由

1. **依赖关系清晰**：tidev-mcp、tidev-llm 只需 tidev-types 获取 `ToolDefinition`，不必引入整个工具实现树
2. **编译分离**：工具实现依赖大量外部 crate（reqwest、diffy、base64 等），不影响类型层的编译
3. **职责单一**：types 定义"工具长什么样"，tools 实现"工具怎么执行"，core 协调"什么时候用什么工具"

---

## D-003: tidev-tools 自包含原则

**日期**: 2026-07-02  
**状态**: 暂缓（先做 tidev-agent）

### 决策

tidev-tools 应自包含，不依赖其他 tidev crate（除 tidev-types 外）。对于需要的外部能力（存储、配置、指令解析），通过 traits 或简单内部实现解决。

### 理由

tidev-storage、tidev-config、tidev-instructions 等 crate 尚未成熟，tidev-tools 不应被其阻塞。

---

## D-004: tidev-search 独立迁移

**日期**: 2026-07-02  
**状态**: 已完成

### 背景

`FileSearchIndex`（后台文件索引 + notify 文件系统监听）在旧代码中位于 `tidev-engine/src/shared/file_search.rs`（866 行），是独立的叶子模块。

### 决策

**整体迁移至 `tidev-search` crate，不做架构修改。**

### 模块组织

```
tidev-search/src/lib.rs
  └── FileSearchIndex        — 后台索引 + notify 监听
  └── FileEntryKind          — File / Directory / Image
  └── FileSuggestion         — 搜索建议结果
  └── current_at_fragment()  — @ 片段提取（TUI 补全用）
```

### 理由

1. 零内部 tidev 依赖，纯外部 crate（ignore、notify、rayon、serde、log）
2. 逻辑独立、稳定，不需要改动即可使用

---

## D-005: tidev-agent 薄层设计

**日期**: 2026-07-02  
**状态**: 待实现

### 背景

旧 `AgentRuntime` 持有所有资源（store、LLM client、tools、config、auth 等），子代理启动时 `.clone()` 整个结构体（见旧 `agent/runtime/mod.rs`）。子代理的 agent loop 完整复制了主 loop 的逻辑（`subagent.rs` ~500 行 vs `agent_loop.rs` ~500 行）。

### 决策

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

### AgentContext trait 定义

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

### 依赖

```
tidev-agent ─── tidev-types
            ├── serde
            ├── async-trait
            └── tokio (sync)
```

不依赖 tidev-storage / tidev-config / tidev-llm / tidev-tools / tidev-mcp。

### 理由

1. **复用**：主 agent 和子 agent 共用同一个 `run_agent_loop()` 函数，只传入不同的 `AgentContext` 实现
2. **可测试**：`AgentContext` 可以 mock，纯循环逻辑可单元测试
3. **边界清晰**：循环"怎么转"在 tidev-agent，"用什么转"在 tidev-core

