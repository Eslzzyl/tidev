# Tidev 重写实施计划

基于 `ARCHITECTURE.md`（Per-Session Event Bus 设计）和 `REWRITE-PLAN.md`（15-crate 工作区结构）两份设计文档，
结合对现有 66,329 行 Rust 代码（180 个 `.rs` 文件，6 个 crate）的综合分析。


> **当前状态**：Phase 0-5 已完成，Phase 6（TUI 移植）进行中，Phase 7 待开始。
> 详细进度见文末[检查清单](#14-检查清单)。
> 实施过程中发现的简化详情见 [`SIMPLIFICATIONS.md`](./SIMPLIFICATIONS.md)。

---

## 目录

1. [核心目标](#1-核心目标)
2. [总体策略](#2-总体策略)
3. [Phase 0：归档现有代码](#3-phase-0归档现有代码)
4. [Phase 1：Layer 0-1 类型基础与核心数据](#4-phase-1layer-0-1-类型基础与核心数据)
5. [Phase 2：Layer 2 存储配置与LLM](#5-phase-2layer-2-存储配置与llm)
6. [Phase 3：Layer 3-4 基础设施](#6-phase-3layer-3-4-基础设施)
7. [Phase 4：Layer 4-5 工具系统MCP与上下文](#7-phase-4layer-4-5-工具系统mcp与上下文)
8. [Phase 5：Layer 6 Agent运行时核心架构变更](#8-phase-5layer-6-agent运行时核心架构变更)
9. [Phase 6：Layer 7-8 应用层](#9-phase-6layer-7-8-应用层)
10. [Phase 7：清理收尾](#10-phase-7清理收尾)
11. [依赖关系总览](#11-依赖关系总览)
12. [风险与缓解](#12-风险与缓解)

---

## 1. 核心目标

| 问题 | 现状 | 目标 |
|------|------|------|
| God Crate | `tidev-engine` = 19,564 LOC, 68 文件 | 拆分为 9 个独立 crate |
| God Module | `tidev-tui::App` = 2,257 行, ~90 字段 | 按 panel 拆分状态, ~30 字段 |
| 共享通道 | 所有 session 共享一个 BackendEvent 通道 | Per-Session Event Bus |
| 子 agent 事件 | 3 个聚合事件中转（SubagentStatus / SubagentToolResult / SubagentCompleted） | 前端直接订阅子 session 通道 |
| 循环依赖 | `tooling ↔ mcp`, `tooling ↔ instructions` | 严格单向依赖 |
| Leaky Abstractions | TUI 直接 import engine 内部类型 | 通过公共 API 访问 |
| 重复类型 | ToolDefinition 在 tidev-llm 和 engine 中重复定义 | 统一为 tidev-types::ToolSchema |

---

## 2. 总体策略

```
现有代码 (master)
  │
  ├── Phase 0: git mv → _archive/v0.6.x/
  │
  └── 从零构建新 workspace（增量式，逐层推进）
       │
       ├── Phase 1: Layer 0-1 ───── tidev-types, tidev-session
       ├── Phase 2: Layer 2 ─────── tidev-config, tidev-storage, tidev-llm
       ├── Phase 3: Layer 3-4 ───── tidev-hooks, tidev-instructions, tidev-snapshot,
       │                               tidev-sync, tidev-search
       ├── Phase 4: Layer 4-5 ───── tidev-tools, tidev-mcp, tidev-context
       ├── Phase 5: Layer 6 ──────── tidev-agent (Per-Session Event Bus) ⭐
       ├── Phase 6: Layer 7-8 ───── tidev-tui, tidev (root)
       └── Phase 7 ──────────────── 删除 _archive, 更新文档, 最终测试
```

### 核心原则

- **增量替换**：每个 Phase 独立编译、独立测试。没有「大爆炸」式重写。
- **先底层后上层**：严格按依赖层次构建，上层依赖下层就绪后才能编译。
- **存档即参考**：`_archive/` 中的旧代码保持完整可编译，随时查阅参考。
- **每 Phase 一个 commit**：清晰的 git 历史，方便回退。

---

## 3. Phase 0：归档现有代码

### 目标

将当前完整的工作区代码移动到 `_archive/v0.6.x/` 目录，作为重写期间的参考。

### 操作步骤

```bash
# 1. 创建归档目录
mkdir -p _archive/v0.6.x

# 2. 移动根 crate 文件（保留 git 历史）
git mv src/ _archive/v0.6.x/
git mv Cargo.toml _archive/v0.6.x/
git mv Cargo.lock _archive/v0.6.x/

# 3. 移动所有子 crate
git mv crates/ _archive/v0.6.x/

# 4. 提交
git commit -m "归档 v0.6.x 代码至 _archive/，准备重写"
```

### 归档后目录结构

```
_archive/v0.6.x/
├── src/                       # 根 CLI dispatch (~421 LOC)
├── crates/
│   ├── tidev-types/           # 959 LOC
│   ├── tidev-session/         # 2,062 LOC
│   ├── tidev-storage/         # 4,438 LOC
│   ├── tidev-llm/             # 5,758 LOC
│   ├── tidev-engine/          # 19,564 LOC（God Crate）
│   └── tidev-tui/             # 33,547 LOC（God Module）
├── Cargo.toml
├── Cargo.lock
└── ...
```

### 不移入归档的文件

以下与 crate 代码无关的文件**保留原位**，重写过程中直接复用：

| 文件 / 目录 | 说明 |
|-------------|------|
| `docs/` | 文档 |
| `scripts/` | 辅助脚本 |
| `npm/` | npm 包集成 |
| `presets.toml` | Provider 预设配置 |
| `AGENTS.md` | Agent 行为说明 |
| `BROWSER_ENHANCEMENT.md` | 浏览器增强说明 |
| `DCP-WORKING-PRINCIPLE.md` | DCP 工作原理 |
| `WINDOWS_TUI_GHOST_CHARS.md` | Windows TUI 问题记录 |
| `README.md`, `LICENSE` | 项目元数据 |
| `.github/` | CI 配置 |
| `.gitignore` | git 忽略规则 |
| `rewrite-plan/` | 重写计划文档（本文件所在目录） |

### 验证标准

```bash
cd _archive/v0.6.x && cargo check  # 旧代码仍可编译
cd ../.. && cargo check             # 新 workspace 为空，OK
```

---

## 4. Phase 1：Layer 0-1 类型基础与核心数据

### 4.1 `tidev-types`（目标 ~1,500 LOC）

#### 已有内容（保留，微调即可）

| 类型 | 文件 | 说明 |
|------|------|------|
| `ModelId`, `ProviderId` 等核心类型 | `types.rs` | 已有 |
| `ToolPermission`, `PermissionMode` | `types.rs` | **从 `engine::tooling` 移入** |
| 提示词、系统提示 | `prompts.rs` | 已有 |
| `ReasoningLevel`, `ThinkingLevelType` | `reasoning.rs` | 已有 |
| `ThemePalette` | `theme.rs` | 已有 |
| `SessionMode` | `prompts.rs` | 已有 |

#### 新增类型

##### `ApiType` 枚举（从 `tidev-llm::types` 移入）

```rust
/// LLM API 类型。放在 tidev-types 中以避免 config → llm 依赖。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApiType {
    Chat,
    Responses,
    Anthropic,
    Gemini,
}
```

##### `ToolSchema`（新建）

```rust
/// The LLM-facing tool interface. Minimal — only what providers need.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
```

**目的**：取代 `tidev-llm::types::ToolDefinition`（目前两个 crate 各有自己的 ToolDefinition，需要 llm_bridge.rs 做转换）。统一后：
- 所有 provider 直接接受 `ToolSchema`
- 不再需要 `ToolDefinition → ToolSchema` 转换
- 消除 `llm_bridge.rs` 中的转换逻辑

#### 外部依赖

`serde`, `serde_json`, `uuid`, `chrono`, `log`

#### 验证标准

```bash
cargo test -p tidev-types
cargo clippy -p tidev-types
```

---

### 4.2 `tidev-session`（目标 ~2,000 LOC）

#### 核心变更：删除 BackendEvent 中的 session_id 字段和 Subagent 变体

在 Per-Session Event Bus 架构下，每个 session 拥有独立的事件通道，`BackendEvent` 不再需要 `session_id` 字段来区分目标。子 agent 的流式事件通过前端直接订阅子 session 的通道传递，不再需要中转。

```rust
// 新 BackendEvent — 无 session_id，无 Subagent 聚合事件
#[derive(Clone, Debug)]
pub enum BackendEvent {
    /// 流式文本增量
    Delta { request_id: Uuid, content: String },
    /// 推理过程增量
    ReasoningDelta { request_id: Uuid, content: String },
    /// 工具调用状态更新
    ToolCallUpdated { request_id: Uuid, tool_call: ToolCall },
    /// LLM 回合完成（含最终回复或工具调用）
    Finished { request_id: Uuid, turn: AssistantTurn },
    /// LLM 请求失败
    Failed { request_id: Uuid, error: String },
    /// LLM 重试
    Retrying { request_id: Uuid, attempt: u32, max_attempts: u32, reason: String, retry_after_secs: u64 },
    /// 指令文件加载完成
    InstructionsLoaded { sources: Vec<String> },
    /// 工具执行完成
    ToolCompleted { request_id: Uuid, tool_call: ToolCall, result: ToolExecutionResult },
    // ❌ 已删除: SubagentStatus, SubagentToolResult, SubagentCompleted
    //    这些事件不再需要，前端直接订阅子 session 的事件通道。
    /// 用量统计
    UsageStats { request_id: Uuid, input_tokens: u32, output_tokens: u32, total_tokens: u32, cache_read_tokens: u32, cache_write_tokens: u32, model_id: String, duration_ms: u64 },
    /// 上下文压缩
    ContextCompacted { request_id: Uuid, compacted: bool, manual: bool, summary: Option<String>, retained_from: Option<usize>, error: Option<String> },
    /// 侧边栏快照就绪
    SidebarSnapshotReady { request_id: Uuid, message_id: Uuid, file_diffs_json: String },
    /// Shell 输出
    ShellOutput { content: String, finished: bool, exit_code: Option<i32> },
    /// 回合开始
    TurnStarting { request_id: Uuid },
    /// 流结束
    StreamEnd { request_id: Uuid },
}
```

#### 保持不变的模块

| 模块 | 说明 |
|------|------|
| `session.rs` (除 BackendEvent 外) | Conversation, Message, MessageRole, ToolCall, ToolExecutionResult, AssistantTurn |
| `balance/` | TokenUsage, Balance |
| `stats/` | UsageSummary, ModelUsageEntry |
| `system_info.rs` | SystemInfo |
| `utils.rs` | 工具函数 |

#### 依赖

`tidev-types`, `serde`, `serde_json`, `uuid`, `chrono`

#### 验证标准

```bash
cargo test -p tidev-session
# 确认 BackendEvent 的每个变体都不再包含 session_id 字段
grep -n "session_id" crates/tidev-session/src/session.rs  # 应该只出现在 Conversation 等数据模型中，不出现在 BackendEvent 中
```

---

## 5. Phase 2：Layer 2 存储配置与LLM

### 5.1 `tidev-config`（~2,000 LOC）— **新 crate**

从 `engine::config/`（9 个文件，~3,000 LOC）提取配置管理。

#### 模块结构

```
tidev-config/src/
├── lib.rs          — 重新导出所有公共类型
├── mod.rs          — AppConfig（分解为子结构体）
├── auth.rs         — AuthStore, ActiveModel, ProviderAuth
├── provider.rs     — ProviderConfig, ModelConfig, ProviderSource
├── mcp.rs          — McpServerConfig
├── ui.rs           — UiConfig
├── logging.rs      — LogConfig
├── reasoning.rs    — ReasoningConfig
├── snapshot.rs     — SnapshotConfig
├── paths.rs        — ConfigPaths
└── presets.rs      — Provider preset 合并（从根目录 presets.toml 加载）
```

#### AppConfig 分解方案

当前 `engine/config/mod.rs` 中的 `AppConfig` 是单一结构体（~40 个字段）。拆分为：

```rust
pub struct AppConfig {
    pub provider: ProviderConfig,
    pub model: Option<ModelConfig>,
    pub auth: AuthConfig,
    pub ui: UiConfig,
    pub logging: LogConfig,
    pub reasoning: ReasoningConfig,
    pub snapshot: SnapshotConfig,
    pub tmp: TmpConfig,
    pub mcp: Vec<McpServerConfig>,
}
```

#### 外部依赖

`tidev-types`, `toml`, `dirs`, `serde`, `serde_json`

#### 验证标准

```bash
cargo test -p tidev-config
cargo clippy -p tidev-config
# 集成测试：加载实际配置文件
```

---

### 5.2 `tidev-storage`（~4,400 LOC）

#### 变更

| 文件 | 变更内容 |
|------|---------|
| `database.rs` | 使用新的 `tidev-types::ToolSchema` 类型；SessionStore 接口更新 |
| `schema.rs` | 基本不变 |
| `migration.rs` | 基本不变 |
| `compression.rs` | 不变 |

#### 关键接口

`SessionStore` 不再需要直接感知 `BackendEvent` 的 session_id 字段（因为事件流操作被 `SessionManager` 接管）。存储层专注**持久化**：save/load Conversation, Message, ToolCall。

#### 依赖

`tidev-types`, `tidev-session`, `rusqlite` (bundled), `zstd`, `uuid`

#### 验证标准

```bash
cargo test -p tidev-storage  # 所有数据库测试通过
```

---

### 5.3 `tidev-llm`（~5,758 LOC）

#### 变更

| 模块 | 变更内容 |
|------|---------|
| `types.rs` | 删除 `ToolDefinition`，删除 `ApiType`（移到 `tidev-types`）；统一使用 `tidev-types::ToolSchema` |
| `anthropic.rs` | 更新 `ToolSchema` 的序列化 |
| `openai.rs` | 同上 |
| `gemini.rs` | 同上 |
| `responses.rs` | 同上 |
| `llm_bridge.rs` | **删除** —— 不再需要转换逻辑，所有 provider 直接使用 `ToolSchema` |
| `turn.rs` | 不变 |
| `think_parser.rs` | 不变 |
| `tool_call_format.rs` | 使用 `ToolSchema` |
| `attachments.rs` | 不变 |
| `error.rs` | 不变 |
| `debug.rs` | 不变 |

#### LlmClient trait 更新

```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn stream_turn(
        &self,
        request: LlmRequest,              // 使用 ToolSchema
        event_tx: UnboundedSender<BackendEvent>,
    ) -> Result<AssistantTurn>;
}
```

其中 `LlmRequest.tools` 类型从 `Vec<ToolDefinition>` 变为 `Vec<ToolSchema>`。

#### 依赖

`tidev-types`, `tidev-session`, `reqwest`, `tokio`, `serde`, `serde_json`

#### 验证标准

```bash
cargo test -p tidev-llm
# 确认所有 provider 编译通过
```

---

## 6. Phase 3：Layer 3-4 基础设施

本阶段的 5 个 crate 都是从 `tidev-engine` 中直接提取，**主要是代码移动 + 少量适配**，不需要大幅修改逻辑。

### 6.1 `tidev-hooks`（~300 LOC）

从 `engine/hooks/` 提取。

| 模块 | 来源 | 说明 |
|------|------|------|
| `config.rs` | `engine/hooks/config.rs` | 钩子配置 |
| `engine.rs` | `engine/hooks/engine.rs` | `HookEngine` 实现 |
| `matcher.rs` | `engine/hooks/matcher.rs` | 事件匹配 |
| `runner.rs` | `engine/hooks/runner.rs` | 钩子执行器 |

**依赖**：`tidev-types`, `tidev-session`, `tokio`

### 6.2 `tidev-instructions`（~500 LOC）

从 `engine/instructions.rs` 提取。

- `resolve_nearby_instructions()` — 向上目录遍历
- `canonicalize_display()` — 路径显示规范化
- `read_instruction_files()` — 指令文件读取
- `globset` 匹配逻辑

**依赖**：`tidev-types`, `globset`, `ignore`

### 6.3 `tidev-snapshot`（~800 LOC）

从 `engine/snapshot/` 提取。

- `SnapshotService` — git 快照管理
- `FileDiff`, `StepPatch` — diff/patch 类型
- `blake3` 哈希
- `diffy` diff 计算

**依赖**：`tidev-types`, `blake3`, `diffy`, `serde`, `serde_json`

### 6.4 `tidev-sync`（~600 LOC）

从 `engine/sync/` 提取。

- `SyncService` — SSH session 同步
- `transport/` — 传输层

**依赖**：`tidev-types`, `tidev-session`, `tidev-storage`, `tokio`

### 6.5 `tidev-search`（~400 LOC）— **新 crate**

从 `engine/shared/file_search.rs` 提取。

- `FileIndex` — 文件索引
- `FileSearch` — 文件搜索
- `current_at_fragment()` — `@` 片段补全

**依赖**：`tidev-types`, `ignore`, `notify`, `grep`, `fuzzy-matcher`

### Phase 3 验证

```bash
# 逐一验证
for p in tidev-hooks tidev-instructions tidev-snapshot tidev-sync tidev-search; do
    cargo test -p $p
    cargo clippy -p $p
done
```

---

## 7. Phase 4：Layer 4-5 工具系统MCP与上下文

### 7.1 `tidev-mcp`（~1,500 LOC）

从 `engine/mcp.rs` 提取。

- `McpManager` — MCP Server 生命周期管理
- Client 连接（child-process 和 streamable-http）
- 工具注册与发现

**依赖**：`tidev-types`, `tidev-session`, `rmcp`, `tokio`, `serde_json`

### 7.2 `tidev-tools`（~5,000 LOC）— **最大的基础设施 crate**

从 `engine/tooling/`（包括 `builtin/` 和 `bundled_skills/`）提取。

#### 模块结构

```
tidev-tools/src/
├── lib.rs
├── registry.rs       — ToolRegistry（工具注册、查找、执行）
├── tools.rs          — ToolDefinition（包含权限、来源等元数据）
├── skills.rs         — SkillCatalog
├── file_read_tracker.rs
├── builtin/          — 所有内置工具
│   ├── mod.rs
│   ├── file.rs       — read/write/edit 等文件操作
│   ├── exec.rs       — 命令执行（含 shell detection、encoding）
│   ├── apply_patch.rs
│   ├── search.rs     — glob/grep
│   ├── task.rs       — 子 agent 任务（通过 tidev-agent 公开 API 调用）
│   ├── todo.rs
│   ├── sensitive.rs
│   ├── utils.rs      — resolve_workspace_path, canonicalize_display 等
│   └── ...
└── bundled_skills/
```

#### 关键解耦

| 当前耦合 | 解耦方式 |
|----------|---------|
| `ToolRegistry` 持有 `McpManager` | `tidev-mcp` 通过 trait 注册工具到 `ToolRegistry` |
| `file.rs` 调用 `resolve_nearby_instructions()` | 通过 `tidev-instructions` 公共 API |
| `exec.rs` 依赖 shell detection / encoding | 将 `ResolvedShell` 和 `decode_command_output` 移入 `tidev-tools` |
| `task.rs` 工具需要调用子 agent | 通过 `tidev-agent::SessionManager` trait/回调 |

**依赖**：`tidev-types`, `tidev-session`, `tidev-config`, `tidev-storage`, `tidev-instructions`, `tidev-snapshot`, `tidev-search`, `reqwest`, `serde`, `serde_json`, `shlex`, `pulldown-cmark`, `html2md`

### 7.3 `tidev-context`（~600 LOC）— **新 crate**

从 `engine/context.rs` 提取。

- `ContextManager` — 系统提示词组装
- System prompt composition（base instruction + session mode + instructions）
- Token budget management

**依赖**：`tidev-types`, `tidev-session`, `tidev-config`, `tidev-instructions`

### Phase 4 验证

```bash
cargo test -p tidev-tools    # 核心工具 crate，测试最多
cargo test -p tidev-mcp
cargo test -p tidev-context
```

---

## 8. Phase 5：Layer 6 Agent运行时（核心架构变更）⭐

### 8.1 `tidev-agent`（~2,000 LOC）

这是整个重写中**改动最大**的部分。基于 ARCHITECTURE.md 的 Per-Session Event Bus 设计。

#### 模块结构

```
tidev-agent/src/
├── lib.rs
├── agent_loop.rs      — AgentLoop（可复用的 agent 循环）
├── session_manager.rs — SessionManager（session 生命周期管理）
├── types.rs           — SessionConfig, SessionHandle, SessionInfo,
│                          ApprovedTool, PendingToolApproval
├── agent_types.rs     — AgentDefinition, 6种 agent type（General, Explorer, 等）
├── prompts.rs         — Agent 类型对应的系统提示词
└── tests.rs           — 单元测试（使用 mock store + mock LLM）
```

#### 8.1.1 AgentLoop 设计

```rust
pub struct AgentLoop {
    session_id: Uuid,
    model: ActiveModel,
    context: ContextManager,
    tools: Vec<ToolDefinition>,
    store: Arc<SessionStore>,
    llm: Arc<dyn LlmClient>,
    event_tx: UnboundedSender<BackendEvent>,
    cancel_token: CancellationToken,
}

impl AgentLoop {
    pub fn new(config: AgentLoopConfig) -> Self { ... }

    pub async fn run(mut self) -> Result<()> {
        loop {
            // 1. 检查取消
            if self.cancel_token.is_cancelled() { break; }

            // 2. 从 store 加载消息
            let messages = self.store.load_messages(self.session_id)?;

            // 3. 组装请求（system prompt + conversation）
            let request = self.build_request(&messages)?;

            // 4. 流式调用 LLM（通过 event_tx 发送事件）
            let turn = self.llm.stream_turn(request, self.event_tx.clone()).await?;

            // 5. 持久化
            self.store.save_turn(self.session_id, &turn)?;

            // 6. 如果是最终回复（无工具调用），结束循环
            if turn.tool_calls.is_empty() {
                break;
            }

            // 7. 执行工具调用
            for result in self.execute_tools(&turn.tool_calls).await {
                self.event_tx.send(BackendEvent::ToolCompleted { ... })?;
            }
        }
        Ok(())
    }
}
```

#### 8.1.2 SessionManager 设计

```rust
pub struct SessionManager {
    store: Arc<SessionStore>,
    llm: Arc<dyn LlmClient>,
    active_sessions: Arc<Mutex<HashMap<Uuid, SessionHandleInner>>>,
}

impl SessionManager {
    /// 创建新 session 并 spawn AgentLoop
    pub fn spawn(&self, config: SessionConfig) -> SessionHandle {
        let (event_tx, event_rx) = unbounded_channel();
        let cancel_token = CancellationToken::new();
        let child_token = if let Some(parent_id) = config.parent_id {
            // 级联取消：父 session 取消时自动取消子 session
            let parent_handle = self.active_sessions.lock().get(&parent_id).cloned();
            parent_handle.map(|h| h.cancel_token.child_token())
                .unwrap_or_else(|| CancellationToken::new())
        } else {
            CancellationToken::new()
        };

        let loop_ = AgentLoop::new(AgentLoopConfig {
            session_id: config.session_id,
            model: config.model,
            context: ContextManager::new(...),
            tools: config.tools,
            store: self.store.clone(),
            llm: self.llm.clone(),
            event_tx: event_tx.clone(),
            cancel_token: child_token.clone(),
        });

        // Spawn agent loop task
        let session_id = config.session_id;
        tokio::spawn(async move {
            loop_.run().await.ok();
        });

        // 注册活跃 session
        self.active_sessions.lock().insert(session_id, SessionHandleInner {
            cancel_token,
            event_tx,
        });

        SessionHandle {
            session_id,
            event_rx,
            cancel_token: child_token,
        }
    }

    /// 订阅 session 的事件流
    pub fn subscribe(&self, session_id: Uuid) -> Option<UnboundedReceiver<BackendEvent>> {
        // TUI 或其他消费者可以随时订阅/取消订阅某个 session 的事件
        // 用于切换活跃会话或在 overlay 中显示子会话
        ...
    }

    /// 取消 session（级联取消所有子 session）
    pub fn cancel(&self, session_id: Uuid) { ... }

    /// 获取活跃 session 列表
    pub fn list_active(&self) -> Vec<SessionInfo> { ... }
}
```

#### 8.1.3 子 agent 调度方式变更

| 旧方案 | 新方案 |
|--------|--------|
| `run_subagent()` 在当前进程中创建 AgentRuntime 递归调用 | `SessionManager::spawn()` 创建独立任务 |
| 通过 `SubagentStatus` / `SubagentToolResult` / `SubagentCompleted` 中转事件 | 前端通过 `subscribe(child_session_id)` 直接读取事件流 |
| 事件需要经过 3 层中转才到达 TUI | 事件从子 AgentLoop 直达 TUI，零中转 |
| 子 agent 的生命周期与父 agent 的 event loop 线程耦合 | 子 agent 是独立的 tokio task，通过 child_token 级联取消 |

#### 8.1.4 验证标准

```rust
#[tokio::test]
async fn test_agent_loop_basic() {
    let (tx, mut rx) = unbounded_channel();
    let store = MockSessionStore::new();
    let llm = MockLlmClient::new()
        .with_response("Hello, world!");

    let loop_ = AgentLoop::new(AgentLoopConfig {
        session_id: Uuid::new_v4(),
        model: default_model(),
        context: ContextManager::dummy(),
        tools: vec![],
        store: Arc::new(store),
        llm: Arc::new(llm),
        event_tx: tx,
        cancel_token: CancellationToken::new(),
    });

    loop_.run().await.unwrap();

    // 验证接收到 TurnStarting → Delta → StreamEnd
    assert!(received_finished_event(&mut rx));
}
```

```bash
cargo test -p tidev-agent        # 使用 mock store + mock LLM
cargo clippy -p tidev-agent
```

---

## 9. Phase 6：Layer 7-8 应用层

### 9.1 `tidev-tui`（目标 ~10,000 LOC，从 33,547 LOC 精简）

#### 新模块结构

```
tidev-tui/src/
├── lib.rs                    — App struct 精简 (~200 行)
├── app/
│   ├── mod.rs                — App 定义、字段声明
│   ├── init.rs               — 初始化、engine wiring
│   ├── event.rs              — 顶层事件分发
│   └── state.rs              — AppState（最小）
├── core/
│   ├── run.rs                — 事件循环
│   ├── permissions.rs        — 工具审批流程（从 ui/permission.rs）
│   ├── questions.rs          — 问题流程（从 ui/question.rs）
│   ├── workspace.rs          — 工作区边界检查（从 ui/workspace_boundary.rs）
│   └── undo.rs               — 撤销管理
├── panels/                   — 每个 panel 独立状态 + 逻辑
│   ├── session.rs
│   ├── model.rs
│   ├── settings.rs
│   ├── mcp.rs
│   ├── sync.rs
│   ├── skills.rs
│   ├── search.rs
│   └── theme.rs
├── render/                   — 渲染
│   ├── chat.rs               — 消息渲染
│   ├── tool_cards.rs         — 工具调用卡片渲染
│   ├── diff.rs               — 统一 diff 渲染（从 render/diff_render.rs）
│   ├── panels.rs             — Panel widget 渲染
│   └── dialogs.rs            — 对话框 widget 渲染
├── input/                    — 输入处理
│   ├── keyboard.rs           — 键盘事件
│   ├── mouse.rs              — 鼠标事件
│   └── composer.rs           — 文本输入
├── markdown/                 — Markdown → ratatui 渲染（基本不变）
├── theme/                    — 颜色主题（基本不变）
└── widgets/                  — 可复用 UI 组件
```

#### App 结构体精简方案

```rust
// 新 App struct — 约 30 个字段（从 ~90 精简）
struct App {
    // === 核心生命周期 ===
    should_quit: bool,
    screen: Screen,
    workspace_root: PathBuf,
    config: SharedConfig,
    store: SessionStore,
    theme: ThemeManager,
    mode: SessionMode,
    pending_mode: Option<SessionMode>,

    // === Session 管理（新架构）===
    session_manager: SessionManager,
    active_session_id: Uuid,
    /// 当前活跃 session 的事件接收器
    event_rx: UnboundedReceiver<BackendEvent>,

    // === Panels (通过枚举分派，每个面板有独立的 state struct) ===
    active_panel: Option<PanelKind>,  // enum { Session, Model, Settings, MCP, Sync, Skills, Search, Theme }
    panel_states: PanelStates,        // 每个 panel 的独立状态

    // === 输入 ===
    composer: Composer,
    at_mention: AtMentionState,
    snippet: Option<SnippetState>,
    shell_completion: Option<ShellCompletionState>,
    mouse_selection: Option<MouseSelectionState>,

    // === 对话框（enum，一次只能开一个）===
    dialog: Option<Dialog>,

    // === 执行状态 ===
    pending_approvals: Vec<PendingToolApproval>,
    running_tool_executions: HashMap<Uuid, RunningToolExecution>,
    active_request_id: Option<Uuid>,
    request_cancel_token: CancellationToken,

    // === 缓存 ===
    message_render_cache: LruCache<...>,

    // === 布局 ===
    layout: LayoutCache,
}
```

#### 消除 Leaky Abstractions

| 当前泄漏 | 修复方式 |
|----------|---------|
| TUI import `tooling::builtin::utils::resolve_workspace_path` | 通过 `tidev-tools::path_utils` 公共 API |
| TUI import `tooling::builtin::sensitive::is_path_sensitive` | 通过 `ToolRegistry::check_sensitive_path()` |
| TUI import `agent::runtime::types::ApprovedTool` | `tidev-agent::ApprovedTool` 公共类型 |
| TUI import `shared::undo::StepPatch` | `tidev-snapshot::StepPatch` 公共类型 |
| TUI import `shared::file_search::current_at_fragment` | `tidev-search::current_at_fragment` |
| TUI 直接解析 `TaskArgs`, `TodoItem` JSON | 通过 `tidev-tools` 提供的序列化辅助函数 |

#### 事件管道变更

```
旧管道：
  AgentLoop → BackendEvent { session_id, ... }
    → TUI event_rx
    → 检查 session_id → 匹配？正常渲染 : 缓存到 cached_sessions
    → with_temporary_session_context() 切换上下文

新管道：
  SessionManager.subscribe(active_session_id)
    → TUI event_rx（只收到当前 session 的事件）
    → 直接渲染，无需检查 session_id
    → 切换 session = 重新 subscribe
```

#### 依赖

`tidev-types`, `tidev-session`, `tidev-storage`, `tidev-config`, `tidev-llm`,
`tidev-agent`, `tidev-tools`, `tidev-mcp`, `tidev-snapshot`, `tidev-search`,
`tidev-instructions`, `tidev-context`,
`ratatui`, `crossterm`, `syntect`, `two-face`, `pulldown-cmark`

### 9.2 `tidev`（root crate）— CLI dispatch

基本保持不变，仅更新 dependencies 指向新的子 crate 列表。

```toml
[package]
name = "tidev"
# ... 不变

[dependencies]
# 旧的依赖（保留）
anyhow, clap, log, rusqlite, tokio, uuid

# 新的 workspace crate 依赖（更新）
tidev-types = { path = "crates/tidev-types" }
tidev-session = { path = "crates/tidev-session" }
tidev-storage = { path = "crates/tidev-storage" }
tidev-config = { path = "crates/tidev-config" }
tidev-llm = { path = "crates/tidev-llm" }
tidev-agent = { path = "crates/tidev-agent" }
tidev-tools = { path = "crates/tidev-tools" }
tidev-tui = { path = "crates/tidev-tui" }
```

#### 验证

```bash
cargo build --workspace        # 15 个 crate 全部编译通过
cargo test --workspace         # 所有测试通过
cargo clippy --workspace       # 无新警告
```

---

## 10. Phase 7：清理收尾

### 10.1 删除归档代码（可选）

当确认新架构稳定后，可以删除 `_archive/` 以保持工作区整洁。建议在重写完成后 1-2 周再做此操作。

### 10.2 更新文档

| 文档 | 更新内容 |
|------|---------|
| `AGENTS.md` | 更新 workspace 结构、构建命令、crate 描述 |
| `README.md` | 更新架构说明 |
| `rewrite-plan/` | 标记实施计划完成，移入 `docs/archive/` |

### 10.3 最终验证

```bash
# 完整构建
cargo build --release --locked

# 完整测试
cargo test --workspace

# lint
cargo clippy --workspace -- -D warnings

# 检查是否有未使用的依赖
cargo +nightly udeps  # nightly only, optional

# 二进制大小检查
ls -lh target/release/tidev
```

### 10.4 生成 CHANGELOG

使用 git 历史生成从 v0.6.x 到新版本的变更日志。

---

## 11. 依赖关系总览

### 最终 15 个 crate

```
Layer 8: tidev (root) ──────────────────────────────────────────┐
           │                                                      │
Layer 7: tidev-tui ──────────────────────────────────────────┐   │
           │                                                    │   │
Layer 6: tidev-agent ─────────────────────────────────────┐   │   │
           │                                                  │   │   │
Layer 5: tidev-context ───────────────────────────────┐      │   │   │
           │                                                │      │   │   │
Layer 4: tidev-tools ─── tidev-mcp ───────────────────┤      │      │   │   │
           │         │         │                              │      │      │   │   │
Layer 3: tidev-hooks  tidev-instructions  tidev-snapshot  tidev-sync  tidev-search
           │         │         │         │         │
Layer 2: tidev-config ── tidev-storage ── tidev-llm
           │         │         │
Layer 1: tidev-session
           │
Layer 0: tidev-types
```

### 每 crate 依赖关系

| 层级 | Crate | 内部依赖 | 需被谁依赖 |
|------|-------|----------|-----------|
| 0 | tidev-types | 无 | 所有其他 crate |
| 1 | tidev-session | tidev-types | storage, llm, config, tools, agent, tui |
| 2 | tidev-config | tidev-types | tools, context, agent, tui, root |
| 2 | tidev-storage | tidev-types, tidev-session | sync, tools, agent, tui, root |
| 2 | tidev-llm | tidev-types, tidev-session | context, agent, tui |
| 3 | tidev-hooks | tidev-types, tidev-session | agent |
| 3 | tidev-instructions | tidev-types | tools, agent |
| 3 | tidev-snapshot | tidev-types | tools, agent, tui |
| 3 | tidev-sync | tidev-types, tidev-session, tidev-storage | tui |
| 3 | tidev-search | tidev-types | tools, tui |
| 4 | tidev-mcp | tidev-types, tidev-session | agent, tui |
| 4 | tidev-tools | tidev-types, tidev-session, tidev-config, tidev-storage, tidev-instructions, tidev-snapshot, tidev-search | context, agent, tui |
| 5 | tidev-context | tidev-types, tidev-session, tidev-config, tidev-instructions, tidev-llm, tidev-tools | agent, tui |
| 6 | tidev-agent | tidev-types, tidev-session, tidev-storage, tidev-config, tidev-llm, tidev-hooks, tidev-instructions, tidev-snapshot, tidev-tools, tidev-mcp, tidev-context | tui, root |
| 7 | tidev-tui | 除 tidev-hooks 外的所有下层 crate | root |
| 8 | tidev (root) | tidev-types, tidev-storage, tidev-config, tidev-agent, tidev-tui | — |

**零循环依赖**。每条边都是单向的。

---

## 12. 预估工作量

| Phase | Crates | 性质 | 预估 LOC | 关键难度 | 可并行？ |
|-------|--------|------|----------|---------|---------|
| 0 | 归档 | 文件移动 | — | 确保 git history 保留 | — |
| 1 | tidev-types, tidev-session | 修改 + 新增 | ~1,500 + 适配 | BackendEvent 重设计（关键决策） | 可并行 |
| 2 | tidev-config, tidev-storage, tidev-llm | 提取 + 适配 | ~12,000 | AppConfig 分解为子结构体 | 可并行 |
| 3 | 5 个基础设施 crate | 纯提取 | ~2,600 | 识别 crate 边界 | 可并行 |
| 4 | tidev-tools, tidev-mcp, tidev-context | 提取 + 适配 | ~7,100 | ToolRegistry 依赖关系解耦 | tidev-tools 最大 |
| 5 | tidev-agent | **全新编写** | ~2,000 | Per-Session Event Bus 实现 | **核心串行** |
| 6 | tidev-tui | 大重构 | 33,547→~10,000 | App 状态拆分，新事件管道 | 最大工作量 |
| 7 | 清理 | 文档 + 验证 | — | — | — |

### 关键路径

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 → Phase 6 → Phase 7
                                                          ↑
                                                  Phase 5 是 Phase 6 的前置依赖，
                                                  因为 TUI 事件管道需要 Per-Session Bus API
```

---

## 13. 风险与缓解

| 风险 | 严重程度 | 缓解措施 |
|------|---------|---------|
| 新 BackendEvent 遗漏某会话必须的字段 | 高 | 严格类型检查 + `#[non_exhaustive]` + 新旧代码对比审计 |
| SessionManager 与现有存储耦合过深 | 中 | `SessionStore` 作为 trait 定义，AgentLoop 通过 trait object 使用 |
| TUI 重构期间功能丢失 | 高 | 逐个 Panel 迁移，每个 Panel 迁移后 `cargo test` + 手动验收 |
| 子 agent 流式传输性能下降 | 低 | Per-Session 通道是 `UnboundedSender`，零锁，性能优于现有方案（减少了中转） |
| 现有数据库不兼容 | 中 | 新的 BackendEvent schema 使用 `#[serde(deny_unknown_fields)]` 严格解析；_archive 中的旧代码可读旧库作为 fallback |
| 15 个 crate 编译时间增加 | 低 | 增量编译下只重新编译变更的 crate，总编译时间接近现有关键路径 |
| 代码拆分后类型不匹配 | 中 | 每个 Phase 完成后运行 `cargo check --workspace` 即时发现 |

---

## 14. 检查清单

### Phase 0 [完成]
- [x] 创建 `_archive/v0.6.x/`
- [x] `git mv src/ Cargo.toml Cargo.lock crates/`
- [x] `git commit -m "归档 v0.6.x 代码至 _archive/，准备重写"`
- [x] 验证 `_archive/v0.6.x/cargo check` 通过

### Phase 1 [完成]
- [x] `tidev-types` 新增 `ApiType`, `ToolSchema`, `ToolPermission`
- [x] `tidev-session` 更新 `BackendEvent`（删除 session_id, 删除 3 个 Subagent 变体）
- [x] `cargo test -p tidev-types -p tidev-session`

### Phase 2 [完成]
- [x] `tidev-config` 从 `engine::config` 提取，分解 AppConfig
- [x] `tidev-storage` 更新接口
- [x] `tidev-llm` 使用 ToolSchema，删除 llm_bridge.rs
- [x] `cargo test -p tidev-config -p tidev-storage -p tidev-llm`

### Phase 3 [完成]
- [x] `tidev-hooks` 提取
- [x] `tidev-instructions` 提取
- [x] `tidev-snapshot` 提取
- [x] `tidev-sync` 提取
- [x] `tidev-search` 提取
- [x] `cargo test -p tidev-hooks -p tidev-instructions -p tidev-snapshot -p tidev-sync -p tidev-search`

### Phase 4 [完成]
- [x] `tidev-tools` 提取（最大的基础设施 crate）
- [x] `tidev-mcp` 提取
- [x] `tidev-context` 提取
- [x] `cargo test -p tidev-tools -p tidev-mcp -p tidev-context`

### Phase 5 ⭐ [完成]
- [x] `AgentLoop` 实现（可复用的 agent 循环）
- [x] `SessionManager` 实现（session 生命周期管理）
- [x] Per-Session Event Bus 架构评审
- [~] 子 agent 通过 SessionManager::spawn() 实现（类型系统已就绪，未接入 task 工具）
- [x] Mock store + Mock LLM 测试（未编写）
- [x] `cargo check -p tidev-agent` 通过

### Phase 6 [进行中]
- [x] App struct 精简（~90 字段 → ~30 字段）
- [x] Panel 状态拆分（panels/ 目录）
- [x] 事件管道更新（subscribe 替代 demux）
- [x] 消除所有 Leaky Abstractions
- [x] `cargo test -p tidev-tui`

### Phase 7 [待开始]
- [x] `cargo build --workspace` 全量编译
- [x] `cargo test --workspace` 全量测试
- [x] `cargo clippy --workspace -- -D warnings` 无警告
- [x] 更新文档（AGENTS.md, README.md）
- [x] 可选：删除 `_archive/`
- [x] 标记 `rewrite-plan/` 为完成

---

> **文档版本**：v1.0
> **更新日期**：2026-06-25
> **参考文档**：`ARCHITECTURE.md`, `REWRITE-PLAN.md`
