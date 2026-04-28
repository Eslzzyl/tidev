# Tidev 多代理协作系统调研与设计计划

本文档记录了对 oh-my-opencode-slim 项目的调研结果，以及在 Tidev 中实现原生多代理协作系统的设计计划。

---

## 第一部分：oh-my-opencode-slim 项目调研

### 项目概述

**oh-my-opencode-slim** 是 OpenCode 的多智能体编排插件，灵感来自希腊神话中的 Pantheon（万神殿）。它将任务分配给不同的专业代理，实现**质量、速度和成本的平衡**。

项目地址：https://github.com/alvinunreal/oh-my-opencode-slim

### 核心架构

#### 1. 多代理团队 (Pantheon)

插件定义了 6 个专业代理：

| 代理 | 角色 | 默认模型 |
|------|------|----------|
| **Orchestrator** | 主协调器 + 实现者 | `openai/gpt-5.5` |
| **Explorer** | 代码库探索、并行搜索 | `openai/gpt-5.4-mini` |
| **Librarian** | 文档查阅、API参考 | `openai/gpt-5.4-mini` |
| **Oracle** | 战略顾问、代码审查 | `openai/gpt-5.5` |
| **Designer** | UI/UX 设计工作 | `openai/gpt-5.4-mini` |
| **Fixer** | 代码修复、Bug 处理 | `openai/gpt-5.4-mini` |

#### 2. 自动委托机制

**Orchestrator** 根据任务类型自动委托给专业代理：

```
用户请求 → Orchestrator 分析 → @explorer (搜索发现)
                          → @librarian (文档查阅)
                          → @oracle (战略决策)
                          → @designer (UI工作)
                          → @fixer (代码修复)
```

#### 3. Council Manager（多 LLM 共识）

支持多 LLM 并行协商：
- 多个 `councillor` 并行运行
- 收集结果后由 `council` 代理综合
- 支持超时控制

#### 4. 会话管理 (Task Session Manager)

- 跟踪子代理调用和上下文文件
- 最大会话数限制 (`maxSessionsPerAgent`)
- 维护读取文件的历史记录

#### 5. 复用层 (Multiplexer)

- **Tmux** 或 **Zellij** 终端复用器
- 支持在独立 pane 中运行子代理
- 提供 `session-manager` 管理多个会话

### 技术实现分析

#### 插件入口点

```typescript
// oh-my-opencode-slim/src/index.ts
const OhMyOpenCodeLite: Plugin = async (ctx) => {
  const { client, directory, ... } = ctx;

  return {
    agents: {...},    // 注册多代理
    tools: {...},     // 注册工具
    mcps: {...},      // 注册 MCP 服务器
    hooks: {...},     // 注册生命周期钩子
    commands: {...},  // 注册命令
  };
};
```

#### 代理注册

```typescript
// oh-my-opencode-slim/src/agents/index.ts
export function createAgents(config?: PluginConfig): AgentDefinition[] {
  return [
    createOrchestratorAgent(...),  // 主代理
    createExplorerAgent(...),      // 探索代理
    createLibrarianAgent(...),     // 文档代理
    createOracleAgent(...),        // 战略代理
    createDesignerAgent(...),      // 设计代理
    createFixerAgent(...),         // 修复代理
  ];
}
```

每个代理的定义结构：
```typescript
interface AgentDefinition {
  name: string;              // "orchestrator", "explorer", ...
  displayName?: string;      // "@explorer" 用户友好名称
  description?: string;
  config: AgentConfig;       // OpenCode SDK 的代理配置
}
```

#### 生命周期钩子

插件通过 **钩子系统** 拦截 OpenCode 的各个阶段：

| 钩子名称 | 作用 | 实现的模块 |
|----------|------|------------|
| `config()` | 注册代理、工具、MCP | `src/index.ts` |
| `tool.execute.after` | 捕获 task 工具输出 | `delegate-task-retry`, `task-session-manager` |
| `event()` | 处理会话事件 | `task-session-manager`, `todo-continuation` |
| `command()` | 注册 `/interview`, `/preset` 命令 | `interview`, `preset-manager` |
| `chat.system.transform` | 修改系统提示词 | `task-session-manager` |
| `chat.headers` | 修改请求头 | `chat-headers` |
| `experimental.chat.messages.transform` | 修改用户消息 | `filter-available-skills` |

#### 任务委托机制

```typescript
// oh-my-opencode-slim/src/hooks/task-session-manager/index.ts
'tool.execute.after': async (input, output) => {
  if (input.tool === 'task') {
    // 解析 task_id
    const taskId = parseTaskIdFromTaskOutput(output.output);

    // 跟踪子代理会话
    sessionManager.trackPendingTask({
      callId,
      parentSessionId,
      agentType,
      label,
    });
  }
}
```

#### MCP 服务器集成

```typescript
// oh-my-opencode-slim/src/mcp/index.ts
export function createBuiltinMcps(disabledMcps): Record<string, McpConfig> {
  return {
    websearch: createWebsearchConfig(),   // Exa 搜索
    context7: context7,                    // Context7 文档
    grep_app: grep_app,                    // 代码库搜索
  };
}
```

### 关键代码位置

| 功能 | 文件路径 |
|------|----------|
| 插件入口 | `oh-my-opencode-slim/src/index.ts` |
| 代理工厂 | `oh-my-opencode-slim/src/agents/index.ts` |
| Orchestrator 定义 | `oh-my-opencode-slim/src/agents/orchestrator.ts` |
| Explorer 定义 | `oh-my-opencode-slim/src/agents/explorer.ts` |
| Librarian 定义 | `oh-my-opencode-slim/src/agents/librarian.ts` |
| Oracle 定义 | `oh-my-opencode-slim/src/agents/oracle.ts` |
| Council 管理器 | `oh-my-opencode-slim/src/council/council-manager.ts` |
| 任务会话管理 | `oh-my-opencode-slim/src/hooks/task-session-manager/index.ts` |
| 委托重试钩子 | `oh-my-opencode-slim/src/hooks/delegate-task-retry/hook.ts` |
| MCP 服务 | `oh-my-opencode-slim/src/mcp/index.ts` |
| 工具定义 | `oh-my-opencode-slim/src/tools/council.ts` |
| Interview 服务 | `oh-my-opencode-slim/src/interview/service.ts` |
| 复用层 | `oh-my-opencode-slim/src/multiplexer/` |
| 配置文件 | `oh-my-opencode-slim/src/config/` |

### 用户可见性

**用户如何看到多代理协作**：

1. **通过主代理的"总结"输出** - 子代理的完整输出不会直接显示给用户，由 Orchestrator 总结后展示

2. **通过 Task Tool 输出** - OpenCode 的 `task` 工具返回 `task_id` 和简短结果摘要

3. **恢复会话** - 使用 `task_id` 可以恢复子代理会话继续对话

4. **Interview Dashboard** - 独立的 HTML 页面查看 interview 记录

5. **Tmux/Zellij 分离会话** - 如果启用了 Multiplexer，可以在独立 pane 中运行子代理

### UI 修改

**oh-my-opencode-slim 不修改 OpenCode 的核心 UI**。它：

- 不改变聊天界面、输入框等
- 不添加/修改任何侧边栏元素
- 通过钩子间接影响请求级别的展示（如 HTTP 请求头）

唯一的"UI"是 Interview 模块提供的独立服务页面。

---

## 第二部分：Tidev 多代理系统设计计划

### 设计目标

在 Tidev 中实现原生多代理协作系统，提供比插件更紧密的集成：

1. **简化代理注册** - 直接在代码中定义代理，无需 JSON 配置
2. **更好的错误处理** - 原生错误传播，而非插件隔离
3. **更高效的会话管理** - 直接共享存储和上下文
4. **更灵活的 UI 集成** - 新增代理状态面板

### 核心模块计划

#### 1. 代理核心 (`src/agent/`)

新建以下模块：

| 文件 | 功能 |
|------|------|
| `mod.rs` | 代理类型枚举、代理定义结构体 |
| `factories.rs` | 各代理的创建工厂函数 |
| `orchestrator.rs` | Orchestrator 特殊处理逻辑 |
| `prompts.rs` | 各代理的系统提示词模板 |

**代理类型**：
- `Orchestrator` - 主协调器
- `Explorer` - 代码库探索代理
- `Librarian` - 文档查阅代理
- `Oracle` - 战略顾问代理
- `Designer` - UI 设计代理
- `Fixer` - 代码修复代理
- `Council` - Council 代理（多 LLM 共识）
- `Councillor` - Council 议员

#### 2. 委托管理 (`src/delegate/`)

新建以下模块：

| 文件 | 功能 |
|------|------|
| `mod.rs` | 委托管理器核心 |
| `session_tracker.rs` | 父子会话跟踪 |
| `context_manager.rs` | 上下文传递管理 |

**功能**：
- 跟踪待处理的委托任务
- 解析 task 工具输出获取 `task_id`
- 管理子代理的上下文文件
- LRU 清理超出限制的会话

#### 3. Council 系统 (`src/council/`)

新建以下模块：

| 文件 | 功能 |
|------|------|
| `mod.rs` | Council 核心定义 |
| `manager.rs` | Council 管理器 |
| `synthesis.rs` | 多 LLM 结果综合 |

**功能**：
- 并行启动多个议员
- 收集并格式化结果
- 综合生成最终答案
- 支持超时和错误处理

#### 4. 配置扩展 (`src/config/`)

扩展 `src/config/mod.rs`：

| 配置类型 | 功能 |
|----------|------|
| `AgentConfig` | 代理系统总配置 |
| `PresetAgents` | 预设代理配置 |
| `AgentOverride` | 代理参数覆盖 |
| `SessionManagerConfig` | 会话管理配置 |
| `CouncilConfig` | Council 配置 |
| `CouncillorConfig` | 议员配置 |

#### 5. UI 集成 (`src/app/`)

扩展 `src/app/ui/`：

| 文件 | 功能 |
|------|------|
| `agent_panel.rs` | 代理状态面板渲染 |
| `delegate_panel.rs` | 活跃委托任务面板 |

**显示内容**：
- 各代理状态（在线/离线）
- 当前活跃的委托任务
- 进度条
- 模型信息

#### 6. 命令扩展 (`src/app/commands.rs`)

新增命令：

| 命令 | 功能 |
|------|------|
| `list agents` | 列出所有代理状态 |
| `switch <agent>` | 切换当前代理 |
| `resume <session>` | 恢复子代理会话 |
| `council <prompt>` | 发起 Council 会议 |

#### 7. 工具扩展 (`src/tooling/`)

扩展 `src/tooling/mod.rs`：

- `TaskArgs` 增加 `subagent_type` 字段
- 支持指定代理类型进行委托
- 支持使用 `task_id` 恢复会话

### 配置文件结构

```toml
# config.toml 示例

[agent]
enabled = true
preset = "default"

[agent.presets.default]
orchestrator.model = "openai/gpt-4o"
explorer.model = "openai/gpt-4o-mini"
librarian.model = "openai/gpt-4o-mini"
oracle.model = "openai/gpt-4o"
designer.model = "openai/gpt-4o-mini"
fixer.model = "openai/gpt-4o-mini"

[agent.session_manager]
max_sessions_per_agent = 2
read_context_min_lines = 10
read_context_max_files = 8

[agent.council]
enabled = true
timeout_secs = 120
stagger_ms = 500

[[agent.council.councillors]]
name = "alpha"
model = "openai/gpt-4o-mini"
prompt = "You are a coding expert."

[[agent.council.councillors]]
name = "beta"
model = "anthropic/claude-3-5-sonnet"
prompt = "You are a code reviewer."
```

### 工作流程

```
用户输入
    │
    ▼
┌─────────────────────────────────────────────────────┐
│  Orchestrator (主代理)                               │
│  - 分析任务                                          │
│  - 决定是否委托                                      │
│  - 综合结果                                          │
└────────┬────────────────────────────────────────────┘
         │ @explorer 委托
         ▼
┌─────────────────────────────────────────────────────┐
│  Explorer (独立会话)                                 │
│  - 探索代码库                                        │
│  - 返回结果                                          │
└────────┬────────────────────────────────────────────┘
         │
         │ @librarian 委托
         ▼
┌─────────────────────────────────────────────────────┐
│  Librarian (独立会话)                                │
│  - 查阅文档                                          │
│  - 返回 API 参考                                     │
└─────────────────────────────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────────────────────┐
│  Orchestrator 综合 → 返回结果给用户                  │
└─────────────────────────────────────────────────────┘
```

### 实现阶段

**阶段一：基础架构**

1. 创建 `src/agent/` 模块，定义代理类型和结构
2. 创建 `src/delegate/` 模块，实现委托管理
3. 扩展 `src/config/` 添加代理配置
4. 扩展 `src/tooling/` 支持代理类型参数

**阶段二：代理实现**

5. 实现各代理的工厂函数和提示词
6. 实现 Orchestrator 的委托逻辑
7. 实现会话跟踪和上下文传递
8. 扩展 `src/app/commands.rs` 添加代理命令

**阶段三：UI 集成**

9. 创建代理状态面板 `src/app/ui/agent_panel.rs`
10. 创建委托任务面板 `src/app/ui/delegate_panel.rs`
11. 在主界面中集成代理面板

**阶段四：高级功能**

12. 创建 `src/council/` 模块，实现 Council 系统
13. 实现多 LLM 并行协商
14. 实现结果综合功能

**阶段五：测试与优化**

15. 添加单元测试
16. 性能优化
17. 文档完善

### 关键技术决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 代理定义方式 | Rust 结构体 | 类型安全，易于扩展 |
| 并发委托 | tokio async | 与现有架构一致 |
| 会话存储 | 复用现有 SessionStore | 减少重复代码 |
| UI 渲染 | 复用 ratatui | 保持 TUI 风格 |
| 配置格式 | toml | 与现有配置一致 |

### 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 上下文膨胀 | 子代理过多导致 token 溢出 | 实现最大会话限制和 LRU 清理 |
| 循环委托 | Orchestrator 互相委托死循环 | 设置最大委托深度 |
| 模型兼容性 | 不同模型 API 不同 | 统一 LlmClient 接口 |
| 配置复杂性 | 用户配置困难 | 提供合理的默认值 |

---

## 附录：参考资源

- oh-my-opencode-slim 仓库：https://github.com/alvinunreal/oh-my-opencode-slim
- OpenCode 插件 API：https://opencode.ai
- OpenCode SDK：@opencode-ai/plugin, @opencode-ai/sdk