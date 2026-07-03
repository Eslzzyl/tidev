# tidev 架构改进方案

## 解决的问题

### 1. TUI 承载了后端初始化职责

旧 `tidev-tui/src/core/run.rs` 的 `App::new_with_paths()` 直接创建 ConfigPaths、AppConfig、AuthStore、ToolRegistry、SnapshotService、SessionManager 及多个后台任务，导致 TUI 直接依赖 8 个 tidev crate。

### 2. tidev-tools 混合了定义与执行

旧 `ToolRegistry` 同时负责工具定义、工具执行、MCP 路由、权限校验。定义与执行是不同层面的概念，不应在一个结构体中。

### 3. 缺少接口抽象

crate 之间通过具体 struct 直接耦合。AgentLoop 直接持有 `ToolRegistry` 而非 `Box<dyn ToolExecutor>`，SessionManager 直接持有 `SessionStore` 而非 `Box<dyn SessionRepository>`。

### 4. 子代理重复实现

子 agent loop 完整复制了主 loop 的逻辑（`subagent.rs` ~500 行 vs `agent_loop.rs` ~500 行），维护成本高。

### 5. 消息状态多权威

TUI、agent loop、ContextManager 各持有一份消息列表，同步困难，直接导致上下文压缩协调和取消后后台残留问题。

---

## 铁律

**字节级不变性**——同一 session 内，任何两次 `build_request_messages()` 的输出，如果消息列表相同，必须字节相同。这是前提，不是可选项。

发送给 LLM 的字节序列必须是确定性的、幂等的。每一轮构造的 `Vec<Message>` 必须是前一轮的严格前缀加上新消息。任何变动都会炸掉前缀缓存，让用户承担重新处理整个上下文的费用。

---

## 目标架构

```
tidev-tui
  依赖: tidev-core, tidev-types, tidev-config(UI配置), tidev-utils

tidev-core
  依赖: tidev-agent, tidev-tools, tidev-config,
        tidev-storage, tidev-llm, tidev-snapshot,
        tidev-instructions, tidev-search

  Runtime             运行时上下文，持有全部资源
  RuntimeBuilder      将 TUI 散落的初始化逻辑收拢至此
  SessionManager      会话生命周期
  AgentContext impl   实现 tidev-agent 定义的 trait
  ContextManager      上下文压缩（build_request_messages + compact）
  ToolRegistry        工具注册与执行（impl ToolExecutor）
  消息缓存            追加写的 Vec<Message>，唯一权威副本

tidev-tools
  依赖: tidev-types, tidev-utils, tidev-instructions, tidev-config
        + 外部 crate（glob, grep, ignore, diffy, reqwest 等）
  不依赖: tidev-storage（通过 TodoPersistence trait 切断）

  所有 builtin 工具实现（file, exec, search, web, apply_patch 等）
  分派函数 execute_tool_call() + ExecutionContext
  SkillCatalog
  TodoPersistence trait（2 个方法，供 tidev-core 桥接）

tidev-agent（薄层）
  依赖: tidev-types + serde, async-trait, tokio, tokio-util, anyhow, chrono, uuid, log
  不依赖: tidev-storage / tidev-config / tidev-llm / tidev-tools

  AgentType + AgentDefinition + AgentOverride
  AgentContext trait（7 个方法）
  run_agent_loop() 骨架（主 agent 和子 agent 共用）
  AgentLoopConfig（含 cancel_token）
  ApprovedTool / PendingToolApproval
  prompts（6 套系统提示词）

tidev-types（扩展）
  现有: tools.rs, message.rs, prompts.rs, reasoning.rs
  新增: approval.rs — PendingToolApproval, ApprovedTool
                    （从 tidev-agent 移入，属于跨 crate 协议类型）
```

---

## 关键决策

### 1. 消息缓存的权威持有者

**tidev-core 持有唯一权威的消息列表。**

- 初始化时从数据库加载，之后只追加
- `load_messages()` 从内存缓存读取，不读 DB
- `save_messages()` 追加到缓存 + 写入 DB
- `build_request_messages()` 在 tidev-core 里，是访问消息列表的唯一出口

这解决了：
- **每次循环读数据库** — 热路径走缓存
- **ContextManager 协调困难** — 压缩是 tidev-core 的内部操作：读自己的缓存 → 调 LLM → 更新自己的 retained_from → 发 ContextCompacted 事件通知 TUI 刷新渲染
- **TUI 膨胀** — TUI 只维护一份渲染用的消息副本，通过 BackendEvent 增量更新，不做任何修改

### 2. ContextManager

在 tidev-core。因为 `build_request_messages()` 需要访问消息缓存，而缓存在 tidev-core。ContextManager 也需要 LLM 来生成摘要，tidev-core 持有 LLM 客户端。

### 3. system_prompt 组装

不是 `AgentContext` 的 trait 方法。system prompt 在 session 创建时预组装好，存入 `AgentLoopConfig.system_prompt` 字段，session 生命周期内不变。

```rust
pub struct AgentLoopConfig {
    pub session_id: Uuid,
    pub definition: AgentDefinition,
    pub mode: SessionMode,
    pub thinking_level: ThinkingLevelType,
    pub event_tx: UnboundedSender<BackendEvent>,
    pub system_prompt: String,
    pub cancel: CancellationToken,
}
```

组装逻辑是 tidev-core 里 session 创建时的自由函数：

```rust
fn compose_system_prompt(
    agent_type: AgentType,
    instructions: &[String],
    tool_descriptions: &str,
    mode: SessionMode,
) -> String
```

mode 切换不修改 system prompt。两种 mode 的定义在 session 开始时一次性注入。后续切换 mode 通过 `<system-reminder>` 附加在用户消息开头，不影响前缀缓存。

模型切换会重新组装 system prompt（tool descriptions 随模型变化），用户接受前缀缓存失效的成本。

### 4. ToolExecutor trait

```rust
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(
        &self,
        tool_call: &ToolCall,
        ctx: &ExecutionContext,
    ) -> Result<ToolExecutionResult>;
}

pub struct ExecutionContext<'a> {
    pub session_id: Uuid,
    pub mode: SessionMode,
    pub allow_outside: bool,
    pub sensitive_file_approved: bool,
}
```

ToolRegistry 在 tidev-core 实现 ToolExecutor。执行前做：权限检查、文件读取追踪检查、路径边界检查。执行后做：文件读取记录。

AgentLoop 通过 `&dyn ToolExecutor` 调用工具，不持有具体 ToolRegistry。

### 5. tidev-tools 执行接口

tidev-tools 提供纯函数分派，不做权限检查：

```rust
pub fn execute_tool_call(
    tool_name: &str,
    arguments: &Value,
    ctx: &ExecutionContext,
) -> Result<ToolExecutionResult>;
```

依赖：tidev-types, tidev-utils, tidev-instructions, tidev-config + 外部 crate。

todowrite 工具的存储通过 `TodoPersistence` trait 切断对 tidev-storage 的直接依赖：

```rust
pub trait TodoPersistence: Send + Sync {
    fn load_todos(&self, session_id: Uuid) -> Result<Vec<TodoItem>>;
    fn replace_todos(&self, session_id: Uuid, todos: &[TodoItem]) -> Result<()>;
}
```

tidev-core 在实现 AgentContext 时桥接 SessionStore。

### 6. 工具权限审批

独立于 BackendEvent 的双向通道：

```
tidev-core 创建 (perm_tx, perm_rx)
perm_tx → tidev-core 的 AgentContext impl（发送 PendingToolApproval）
perm_rx → TUI（接收审批请求，弹对话框）
oneshot → TUI 回复 Vec<ApprovedTool>
```

协议类型（`PendingToolApproval`、`ApprovedTool`）在 tidev-types 中，属于跨 crate 协议。

### 7. 子代理

不设独立的 SubagentHost trait。子代理创建和调度是 `AgentContext::execute_tools()` 的内部细节。

```
execute_tools() 解析 task 工具
  → 解析 AgentType，分类只读/写
  → 构造子 AgentContext（受限工具集 + 子 session 存储 + 同一 event_tx）
  → 串行/并行调用 run_agent_loop()
  → 收集结果返回
```

子代理与父代理共享同一个 `run_agent_loop()` 函数和同一个 `BackendEvent` 通道。TUI 按 session_id 分派渲染。

### 8. 取消

参见 [D-008 取消机制设计](decisions/D-008-cancellation.md)。

两个层面：
- **合作式**：CancellationToken 检查点 + select! 赛跑
- **强制式**：JoinHandle::abort() + kill_all_children()

### 9. HookEngine / Persistence 辅助函数

HookEngine：跳过，见 [D-007](decisions/D-007-skip-hooks.md)。

Persistence 辅助函数：不需要。`build_assistant_message()` 已在 tidev-agent 的 loop_.rs 中，`Message::tool_result()` 是 tidev-types 的构造器，`save_messages()` 是 trait 方法。

### 10. 排队消息

不在 `run_agent_loop` 里处理。TUI 通过 `runtime.submit_prompt()` 提交消息：

```rust
impl Runtime {
    pub async fn submit_prompt(&self, session_id: Uuid, content: String) {
        // 1. 追加到缓存 + DB
        // 2. 如无活跃 loop，启动一个
        // 3. 如有活跃 loop，下一轮迭代自动加载到新消息
    }
}
```

`run_agent_loop` 每次 `load_messages()` 读取最新消息列表，新消息自然在其中。

---

## 各 crate 精确边界

### tidev-types

```
src/
  lib.rs
  tools.rs          ToolDefinition, ToolOrigin, ToolPermission, PermissionConfig,
                    ToolArgs trait + macros, 所有 *Args struct,
                    canonical_tool_name, FileReadStamp, TodoItem
  message.rs        Message, MessageRole, MessageAttachment, ToolCall,
                    ToolExecutionResult, ToolMetadata, FileChangeInfo,
                    AssistantTurn, BackendEvent
  prompts.rs        SessionMode
  reasoning.rs      ThinkingLevelType 及子级别
  permission.rs（新增）
                    PendingToolApproval, ApprovedTool
```

### tidev-agent

```
src/
  lib.rs            导出
  agent_type.rs     AgentType, AgentDefinition, AgentOverride,
                    create_agent, create_all_agents, create_sub_agents
  context.rs        AgentContext trait（7 方法）
                    AgentLoopConfig, ApprovedTool, PendingToolApproval
  loop_.rs          run_agent_loop() 骨架
  prompts.rs        6 套 agent 系统提示词
```

`AgentContext` trait：

```rust
#[async_trait]
pub trait AgentContext: Send + Sync {
    fn tools(&self) -> Vec<ToolDefinition>;
    fn event_tx(&self) -> UnboundedSender<BackendEvent>;
    async fn stream_turn(&self, messages: &[Message],
        system_prompt: &str, thinking_level: &ThinkingLevelType) -> Result<AssistantTurn>;
    async fn request_tool_approval(&self,
        tool_calls: &[ToolCall], mode: SessionMode) -> Result<Vec<ApprovedTool>>;
    async fn execute_tools(&self,
        approved_tools: &[ApprovedTool], session_id: Uuid,
        request_id: u64) -> Result<Vec<(ToolCall, ToolExecutionResult)>>;
    async fn save_messages(&self, session_id: Uuid, messages: &[Message]) -> Result<()>;
    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>>;
}
```

### tidev-tools

```
src/
  lib.rs            导出 execute_tool_call, ExecutionContext, ToolExecutor 等
  builtin/
    mod.rs          分派路由，definitions()，execute_tool_call()
    file.rs         read / write / edit / apply_patch
    exec.rs         bash（含 ACTIVE_CHILDREN, kill_all_children, kill_process_group）
    search.rs       glob / grep
    task.rs         子代理参数验证
    todo.rs         todowrite + TodoPersistence trait
    sudo.rs         sudo 检测和包装
    sensitive.rs    敏感文件检测
    utils.rs        truncate_in_place 等工具函数
    web/
      mod.rs        websearch / webfetch 分派
      fetch.rs      webfetch 实现
      brave.rs      Brave Search
      exa.rs        Exa Search
      google.rs     Google Custom Search
      tavily.rs     Tavily Search
    apply_patch/
      mod.rs        导出
      parser.rs     patch 解析器
      seek_sequence.rs  模糊行匹配
      apply.rs      patch 应用
  skills.rs         SkillCatalog

pub fn execute_tool_call(
    tool_name: &str,
    arguments: &Value,
    ctx: &ExecutionContext,
) -> Result<ToolExecutionResult>;

pub trait TodoPersistence: Send + Sync {
    fn load_todos(&self, session_id: Uuid) -> Result<Vec<TodoItem>>;
    fn replace_todos(&self, session_id: Uuid, todos: &[TodoItem]) -> Result<()>;
}
```

### tidev-core

```
src/
  lib.rs            导出 Runtime, RuntimeBuilder
  runtime.rs        Runtime / RuntimeBuilder
  context.rs        ContextManager
  agent_ctx.rs      AgentContext impl（CoreContext）
  registry.rs       ToolRegistry（impl ToolExecutor）
  session.rs        SessionManager
  cache.rs          消息缓存（Vec<Message>, append-only）
```

`Runtime`：

```rust
pub struct Runtime {
    pub session_manager: SessionManager,
    store: SessionStore,
    tool_executor: Arc<dyn ToolExecutor>,
    snapshot_service: SnapshotService,
    file_read_tracker: Arc<FileReadTracker>,
    event_tx: UnboundedSender<BackendEvent>,
    event_rx: UnboundedReceiver<BackendEvent>,  // → TUI
    perm_tx: UnboundedSender<PendingToolApproval>,
    perm_rx: UnboundedReceiver<PendingToolApproval>,  // → TUI
    cancel_token: CancellationToken,
    run_loop_handle: Option<JoinHandle<()>>,
    message_cache: MessageCache,
    _background_tasks: Vec<JoinHandle<()>>,
}

impl Runtime {
    pub fn cancel(&self);
    pub async fn submit_prompt(&self, session_id: Uuid, content: String);
    pub fn event_rx(&self) -> UnboundedReceiver<BackendEvent>;
    pub fn perm_rx(&self) -> UnboundedReceiver<PendingToolApproval>;
    // ...
}
```

`RuntimeBuilder` 初始化顺序：
1. ConfigPaths / AppConfig / AuthStore（tidev-config）
2. SessionStore / Database（tidev-storage）
3. LlmClient（tidev-llm）
4. ToolRegistry（实现 ToolExecutor）
5. ContextManager
6. SnapshotService（tidev-snapshot）
7. SessionManager
8. 消息��存（从 DB 加载当前 session）
9. 事件通道 / 审批通道 / CancellationToken
10. 后台任务

### tidev-tui

```
只持有：
  runtime: Runtime
  backend_rx（BackendEvent 接收端）
  perm_rx（PendingToolApproval 接收端）
  渲染用消息副本（通过 BackendEvent 增量更新，只读）
  纯 UI 状态（theme, composer, panels, screen）
```

TUI → Core：通过 `Runtime` 方法
Core → TUI：通过 `BackendEvent` + `perm_rx`

---

## 依赖图

```
tidev-tui ──→ tidev-core ──→ tidev-agent ──→ tidev-types
                     │             │
                     │             └── tokio-util, async-trait, ...
                     │
                     ├── tidev-tools ──→ tidev-types
                     │       │          tidev-utils
                     │       │          tidev-instructions
                     │       │          tidev-config
                     │       │
                     │       ├── glob / grep / ignore / globset / rayon
                     │       ├── diffy / base64 / mime_guess
                     │       ├── async_trait / reqwest / pulldown-cmark / url
                     │       └── log / libc(unix) / tempfile(dev)
                     │
                     ├── tidev-config ──→ tidev-types
                     ├── tidev-storage ──→ tidev-types
                     ├── tidev-llm ──→ tidev-types
                     ├── tidev-snapshot ──→ tidev-utils, tidev-config
                     └── tidev-instructions ──→ tidev-utils
```

无循环依赖。tidev-agent 是唯一的"不知道具体实现"的 crate——它只面向 `AgentContext` trait 编程。

---

## 实现顺序

| 阶段 | Crate | 内容 | 依赖 |
|------|-------|------|------|
| 1 | tidev-tools | 迁移所有 builtin 工具实现 + SkillCatalog | tidev-types, tidev-utils, tidev-instructions, tidev-config |
| 2 | tidev-core | 消息缓存, ContextManager, ToolRegistry, AgentContext impl | 全部 tidev crate |
| 3 | tidev-core | Runtime / RuntimeBuilder | 同上 |
| 4 | tidev-tui | 接入 Runtime，删除直接持有的资源 | tidev-core |
| 5 | tidev-agent | 完善 loop_.rs（取消检查点） | tidev-types |
