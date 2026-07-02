# tidev 架构改进方案

## 问题

### 1. tidev-agent 承载了过多职责

当前 `tidev-agent` 混入了两个不同层面的东西：

| 范畴 | 内容 | 去向 |
|------|------|------|
| Agent 运行时 | AgentLoop、ContextManager、HookEngine、prompts、factories、persistence | 留在 tidev-agent |
| 会话协调层 | SessionManager、FrontendMessage、DisplayEvent、SharedAgentState | 移出 |

SessionManager 是跨会话、跨 AgentLoop 的协调者，不属于 Agent 运行时定义。

### 2. TUI 承担了后端初始化职责

`tidev-tui/src/core/run.rs` 的 `App::new_with_paths()` 直接创建以下组件：

- ConfigPaths、AppConfig、AuthStore（tidev-config）
- McpManager（tidev-mcp）
- ToolRegistry（tidev-tools，传 9 个参数）
- SnapshotService（tidev-snapshot）
- SessionManager（tidev-agent）
- 多个后台任务（snapshot cleanup、session 不活跃检查）

TUI 直接依赖 8 个 tidev crate，其中大部分是初始化时需要、运行时不需要的。

### 3. tidev-tools 混合了定义与执行

ToolRegistry 同时负责工具定义（ToolDefinition、ToolArgs）、工具执行（execute_call 需传入 SessionStore）、MCP 路由、权限校验。定义与执行是不同层面的概念，不应在一个结构体中。

### 4. 缺少接口抽象

crate 之间通过具体 struct 直接耦合。AgentLoop 直接持有 `ToolRegistry` 而非 `Box<dyn ToolExecutor>`，SessionManager 直接持有 `SessionStore` 而非 `Box<dyn SessionRepository>`。

---

## 目标架构

```
tidev-tui
  依赖: tidev-core, tidev-types, tidev-session,
        tidev-config(UI配置), tidev-utils(路径显示)

tidev-core (NEW)
  依赖: tidev-agent, tidev-tools, tidev-config,
        tidev-storage, tidev-llm, tidev-mcp, tidev-snapshot,
        tidev-instructions, tidev-search

  RuntimeBuilder      将 TUI 散落的初始化逻辑收拢至此
  Runtime             运行时上下文，持有 SessionManager、store、tools
  SessionManager      会话生命周期（从 tidev-agent 移入）
  FrontendMessage     协议类型（从 tidev-agent 移入）
  DisplayEvent        协议类型（从 tidev-agent 移入）
  re-exports          对外暴露必要类型

tidev-agent (变小)
  依赖: tidev-types, tidev-session, tidev-storage, tidev-llm

  AgentLoop           核心 LLM ↔ 工具循环
  ContextManager      上下文构建与压缩
  HookEngine           后处理钩子
  prompts             Agent 提示词
  persistence         持久化辅助函数
  types               AgentDefinition、AgentLoopConfig（仅 agent 内部类型）

tidev-tools (拆分)
  定义层 (tidev-tools)
    ToolDefinition, ToolArgs trait, ToolPermission,
    canonical_tool_name, SkillCatalog

  执行层 (tidev-tools 或 tidev-tools-exec)
    ToolExecutor trait, ToolRegistry::execute_call,
    MCP 路由, 权限校验
```

### 关键变更

**tidev-agent 缩小**：AgentLoop、ContextManager、HookEngine、prompts、persistence 保留。SessionManager、FrontendMessage、DisplayEvent、SharedAgentState 移入 tidev-core。

**tidev-core 新增**：RuntimeBuilder 收拢 TUI 的初始化代码。SessionManager 从 tidev-agent 迁入。协议类型 FrontendMessage、DisplayEvent 迁入。对外提供 `Runtime` 结构体作为 TUI 的入口。

**tidev-tools 拆分**：定义层保持独立。执行层通过 `ToolExecutor` trait 暴露。AgentLoop 不再持有具体的 `ToolRegistry`，改为 `Box<dyn ToolExecutor>`。

### 依赖变化

```
当前 tidev-agent → tidev-tools, tidev-config, tidev-storage, tidev-llm, tidev-instructions, tidev-snapshot, tidev-search, tidev-mcp
目标 tidev-agent → tidev-llm, tidev-storage, tidev-session, tidev-types + Box<dyn ToolExecutor>

当前 tidev-tui → tidev-agent, tidev-config, tidev-tools, tidev-session, tidev-snapshot, tidev-search, tidev-utils, tidev-types
目标 tidev-tui → tidev-core, tidev-types, tidev-session, tidev-config(UI), tidev-utils
```

---

## 关键细节

### RuntimeBuilder

```rust
pub struct RuntimeBuilder {
    workspace_root: PathBuf,
    paths: Option<ConfigPaths>,
}

impl RuntimeBuilder {
    pub fn new(workspace_root: PathBuf) -> Self;
    pub fn with_paths(self, paths: ConfigPaths) -> Self;
    pub fn build(self) -> Result<Runtime>;
}

pub struct Runtime {
    pub session_manager: SessionManager,
    store: SessionStore,
    tool_executor: Arc<dyn ToolExecutor>,
    snapshot_service: SnapshotService,
    file_read_tracker: Arc<FileReadTracker>,
    _background_tasks: Vec<JoinHandle<()>>,
}

impl Runtime {
    pub fn session_manager(&self) -> &SessionManager;
    pub fn store(&self) -> &SessionStore;
    pub fn snapshot(&self) -> &SnapshotService;
    pub fn tool_executor(&self) -> &dyn ToolExecutor;
}
```

`build()` 内部执行当前 TUI 中散落的全部初始化步骤。

### SessionManager 迁入 tidev-core

迁移后 tidev-agent 中保留：

- AgentLoop — 执行单个会话的 LLM↔工具循环
- ContextManager — 上下文构建与压缩
- HookEngine — 工具后处理
- prompts.rs — 6 种 agent 的系统提示词
- persistence.rs — 消息持久化辅助函数
- AgentDefinition、AgentLoopConfig — agent 内部配置和类型

SessionManager 依赖 tidev-tools（持有 ToolRegistry）和 tidev-config（模型解析），这些依赖在 tidev-core 中合理。

AgentLoop 不再持有 SessionManager。子代理通过 `Box<dyn SubagentHost>` trait 发起：

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

tidev-core 的 SessionManager 实现 `SubagentHost`。子代理的 BackendEvent 直接发给 TUI，父 AgentLoop 只等结果。

### ToolExecutor trait

```rust
pub trait ToolExecutor: Send + Sync {
    fn execute(
        &self,
        tool_call: &ToolCall,
        ctx: &ExecutionContext,
    ) -> Result<ToolExecutionResult>;
}

pub struct ExecutionContext {
    pub store: &SessionStore,
    pub session_id: Uuid,
    pub mode: SessionMode,
    pub allow_outside: bool,
    pub sensitive_file_approved: bool,
}
```

ToolRegistry 实现 ToolExecutor。AgentLoop 通过 `&dyn ToolExecutor` 调用工具，不再直接持有 ToolRegistry。

### TUI 初始化简化

当前 TUI 的 `App::new_with_paths()` 约 100 行直接的组件构建代码。目标：

```rust
pub(crate) fn new_with_paths(paths: ConfigPaths) -> Result<Self> {
    let runtime = tidev_core::RuntimeBuilder::new(env::current_dir()?)
        .with_paths(paths)
        .build()?;

    Ok(Self {
        runtime,
        screen: Screen::Welcome,
        composer: Composer::new("Ask tidev..."),
        // 仅 UI 状态
        theme: ThemeManager::new(&config.theme),
        command_palette: CommandPaletteState::default(),
        panel_launcher: PanelLauncherState::default(),
        // 不再持有: tools, agent, snapshot, file_read_tracker, store(直接)
        // 不再持有: paths, config(完整), auth
    })
}
```

---

## 执行顺序

| 阶段 | 内容 | 说明 |
|------|------|------|
| 1 | 定义 `ToolExecutor` trait，ToolRegistry 实现它，AgentLoop 改用 trait | 纯重构，不改变行为 |
| 2 | 定义 `SubagentHost` trait，SessionManager 实现它，AgentLoop 改用 trait | 同上 |
| 3 | 创建 `tidev-core` crate，从 tidev-agent 移入 SessionManager、协议类型 | 编译验证 |
| 4 | 实现 `RuntimeBuilder`，将 TUI 初始化代码移入 | tidev-tui 的 new_with_paths 大幅简化 |
| 5 | 清理 tidev-tui 不再需要的依赖 | Cargo.toml 简化 |
