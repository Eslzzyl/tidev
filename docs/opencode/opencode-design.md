# OpenCode 设计文档

本文档详细说明 OpenCode 的核心架构设计，涵盖终端用户界面（TUI）布局、斜杠命令、提示词构建逻辑以及核心 Agent 循环。

## 目录

- [代码结构概览](#代码结构概览)
- [TUI 架构设计](#tui-架构设计)
- [UI 组件布局](#ui-组件布局)
- [斜杠命令系统](#斜杠命令系统)
- [提示词构建逻辑](#提示词构建逻辑)
- [核心 Agent 循环](#核心-agent-循环)
- [Agent 类型定义](#agent-类型定义)

---

## 代码结构概览

OpenCode 项目位于 `opencode/` 目录下，主要源代码在 `packages/opencode/src/`：

```
packages/opencode/src/
├── agent/          # Agent 定义与配置
├── command/        # 斜杠命令实现
├── session/        # 会话管理、消息处理、提示词、LLM 交互
├── cli/            # CLI/TUI 实现
│   └── cmd/tui/    # TUI 主实现
├── tool/           # 工具实现（read、write、edit、grep 等）
├── provider/       # LLM 提供商集成
├── config/         # 配置管理
└── project/        # 项目和实例管理
```

---

## TUI 架构设计

### 技术栈

TUI 基于以下技术栈构建：
- **@opentui/core** / **@opentui/solid**: 核心渲染引擎
- **SolidJS**: UI 框架
- **@tui/util**: 工具库（剪贴板、选择等）

### 主入口 (`app.tsx`)

TUI 入口在 `packages/opencode/src/cli/cmd/tui/app.tsx`，主要功能：

1. **渲染配置**: 设置终端渲染参数（FPS、鼠标支持等）
2. **Providers 层级**: 嵌套多层 Context Provider 管理状态
3. **路由系统**: 支持 `home`、`session`、`plugin` 三种路由
4. **命令注册**: 注册所有斜杠命令和快捷键
5. **事件监听**: 处理各种 TUI 事件

```typescript
// app.tsx 核心渲染结构
return (
  <box width={dimensions().width} height={dimensions().height}>
    <Switch>
      <Match when={route.data.type === "home"}>
        <Home />
      </Match>
      <Match when={route.data.type === "session"}>
        <Session />
      </Match>
    </Switch>
  </box>
)
```

### Provider 层级

状态通过嵌套的 Provider 管理（`app.tsx:198-250`）：

```
ArgsProvider
  └─ ExitProvider
        └─ KVProvider
              └─ ToastProvider
                    └─ RouteProvider
                          └─ TuiConfigProvider
                                └─ SDKProvider
                                      └─ ProjectProvider
                                            └─ SyncProvider
                                                  └─ ThemeProvider
                                                        └─ LocalProvider
                                                              └─ KeybindProvider
                                                                    └─ ...
```

---

## UI 组件布局

### 路由结构

TUI 支持三种主要路由（`app.tsx:909-916`）：

1. **Home 路由** (`routes/home.tsx`): 初始会话页面
2. **Session 路由** (`routes/session/index.tsx`): 核心对话页面
3. **Plugin 路由**: 插件扩展页面

### Session 页面布局

Session 页面是主要工作区域，包含以下组件（`routes/session/index.tsx`）：

| 组件 | 文件 | 描述 |
|------|------|------|
| Footer | `footer.tsx` | 底部状态栏，显示目录、权限、LSP、MCP 状态 |
| Sidebar | `sidebar.tsx` | 侧边栏，包含文件列表、MCP、Todo 等 |
| Prompt | `component/prompt/index.tsx` | 输入框，支持历史记录、自动补全 |
| Timeline | `dialog-timeline.tsx` | 时间线视图 |
| Subagent | `subagent-footer.tsx` | 子 Agent 状态显示 |

### 底部状态栏 (Footer)

`routes/session/footer.tsx` 显示：
- 当前工作目录
- 连接状态提示
- 权限请求数量
- LSP 服务状态
- MCP 服务器连接数

### 组件通信

组件间通过 Context 进行通信：
- `useRoute()`: 路由导航
- `useSync()`: 同步数据（会话、消息、权限等）
- `useEvent()`: 事件系统
- `useTheme()`: 主题管理
- `useKV()`: 键值存储

---

## 斜杠命令系统

### 命令定义

斜杠命令在 `packages/opencode/src/command/index.ts` 中定义：

```typescript
export const Info = z.object({
  name: z.string(),
  description: z.string().optional(),
  agent: z.string().optional(),
  model: z.string().optional(),
  source: z.enum(["command", "mcp", "skill"]).optional(),
  template: z.promise(z.string()).or(z.string()),
  subtask: z.boolean().optional(),
  hints: z.array(z.string()),
})
```

### 内置命令

#### 1. `/init` - 初始化 AGENTS.md

模板位于 `packages/opencode/src/command/template/initialize.txt`，功能：
- 为仓库创建或更新 `AGENTS.md`
- 提取关键开发命令、测试命令、构建配置
- 识别 monorepo 结构、框架特性、代码风格

#### 2. `/review` - 代码审查

模板位于 `packages/opencode/src/command/template/review.txt`，功能：
- 支持多种输入：commit hash、branch、PR URL
- 审查维度：Bug、代码结构、性能、行为变更
- 输出直接、可操作的反馈

### 命令注册 (app.tsx)

主要命令在 `app.tsx:452-791` 中注册：

| 斜杠命令 | 功能 | 对应操作 |
|----------|------|----------|
| `/sessions` | 切换会话 | `DialogSessionList` |
| `/new` | 新建会话 | 导航到 home |
| `/models` | 切换模型 | `DialogModel` |
| `/agents` | 切换 Agent | `DialogAgent` |
| `/mcps` | 管理 MCP | `DialogMcp` |
| `/variants` | 切换变体 | `DialogVariant` |
| `/connect` | 连接提供商 | `DialogProviderList` |
| `/org` | 切换组织 | `DialogConsoleOrg` |
| `/status` | 查看状态 | `DialogStatus` |
| `/themes` | 切换主题 | `DialogThemeList` |
| `/help` | 帮助 | `DialogHelp` |
| `/exit` | 退出 | 应用退出 |

### 动态命令

命令可来自多个来源（`command/index.ts:83-167`）：
1. **内置命令**: `init`、`review`
2. **配置文件**: `config.command`
3. **MCP Prompts**: 来自 MCP 服务器的提示
4. **Skills**: 自定义技能

---

## 提示词构建逻辑

### System Prompt 选择

`packages/opencode/src/session/system.ts` 根据模型选择对应的 System Prompt：

```typescript
export function provider(model: Provider.Model) {
  if (model.api.id.includes("gpt-4") || model.api.id.includes("o1"))
    return [PROMPT_BEAST]
  if (model.api.id.includes("gpt"))
    return [model.api.id.includes("codex") ? PROMPT_CODEX : PROMPT_GPT]
  if (model.api.id.includes("gemini-")) return [PROMPT_GEMINI]
  if (model.api.id.includes("claude")) return [PROMPT_ANTHROPIC]
  // ...
}
```

### System Prompt 文件

| 文件 | 适用模型 |
|------|----------|
| `prompt/anthropic.txt` | Claude 系列 |
| `prompt/gpt.txt` | GPT-4 系列 |
| `prompt/beast.txt` | GPT-4o, o1, o3 |
| `prompt/gemini.txt` | Gemini 系列 |
| `prompt/codex.txt` | OpenAI Codex |
| `prompt/kimi.txt` | Kimi 系列 |
| `prompt/trinity.txt` | Trinity 模型 |
| `prompt/default.txt` | 默认 fallback |

### 提示词构建流程

核心构建在 `packages/opencode/src/session/prompt.ts`：

1. **输入处理**: 解析用户输入，提取文件引用
2. **Parts 解析**: `resolvePromptParts()` 解析模板中的文件引用
3. **消息转换**: `MessageV2.toModelMessagesEffect()` 转换为模型消息
4. **历史压缩**: 处理上下文长度限制

### 环境信息注入

System Prompt 包含环境上下文（`system.ts:49-63`）：

```
<env>
  Working directory: /path/to/dir
  Workspace root folder: /path/to/worktree
  Is directory a git repo: yes
  Platform: darwin
  Today's date: Sun Apr 12 2026
</env>
```

### 技能系统

Skills 在 `system.ts:66-77` 中注入：

```typescript
skills: Effect.fn("SystemPrompt.skills")(function* (agent: Agent.Info) {
  const list = yield* skill.available(agent)
  return Skill.fmt(list, { verbose: true })
})
```

---

## 核心 Agent 循环

### 循环入口

核心循环实现位于 `packages/opencode/src/session/processor.ts`:

```typescript
export class Service extends Context.Service<Service, Interface>()
  ("@opencode/SessionProcessor") {}
```

### 处理流程

`SessionProcessor.create()` 启动处理流程（`processor.ts:106-123`）：

1. **快照捕获**: 在 LLM 流开始前捕获初始快照
2. **上下文初始化**: 设置 toolcalls、状态标记
3. **事件处理**: 监听 LLM 流事件并处理

### 事件类型

处理多种 LLM 事件（`processor.ts:214-300`）：

| 事件类型 | 描述 |
|----------|------|
| `start` | 开始处理，设置状态为 busy |
| `reasoning-start/delta/end` | 推理过程 |
| `tool-input-start` | 工具输入开始 |
| `tool-call` | 工具调用触发 |
| `text-delta` | 文本增量输出 |
| `finish` | 完成处理 |

### 工具调用处理

核心工具调用处理（`processor.ts:132-212`）：

```typescript
// 工具调用状态更新
const updateToolCall = Effect.fn("SessionProcessor.updateToolCall")(
  function* (toolCallID, update) { /* 更新工具调用状态 */ }
)

// 工具调用完成
const completeToolCall = Effect.fn("SessionProcessor.completeToolCall")(
  function* (toolCallID, output) { /* 标记为完成，记录输出 */ }
)

// 工具调用失败
const failToolCall = Effect.fn("SessionProcessor.failToolCall")(
  function* (toolCallID, error) { /* 标记为错误，处理权限拒绝 */ }
)
```

### 循环控制

处理结果类型（`processor.ts:28`）：

```typescript
export type Result = "compact" | "stop" | "continue"
```

- **compact**: 需要上下文压缩
- **stop**: 停止处理
- **continue**: 继续处理

### Doom Loop 防护

`processor.ts:25` 定义了循环阈值：

```typescript
const DOOM_LOOP_THRESHOLD = 3
```

防止 Agent 陷入无限循环。

---

## Agent 类型定义

Agent 定义在 `packages/opencode/src/agent/agent.ts`：

```typescript
export const Info = z.object({
  name: z.string(),
  description: z.string().optional(),
  mode: z.enum(["subagent", "primary", "all"]),
  native: z.boolean().optional(),
  hidden: z.boolean().optional(),
  permission: Permission.Ruleset,
  model: z.object({ modelID, providerID }).optional(),
  variant: z.string().optional(),
  prompt: z.string().optional(),
  steps: z.number().int().positive().optional(),
})
```

### 内置 Agent

| Agent | 描述 | 权限 |
|-------|------|------|
| `build` | 默认 Agent，执行工具操作 | 允许编辑、提问、计划 |
| `plan` | 计划模式，禁止编辑 | 允许提问、退出计划 |
| `general` | 通用研究 Agent | 允许大部分操作 |
| `explore` | 代码探索 Agent | 只读权限 |
| `compaction` | 上下文压缩 Agent | 特殊压缩任务 |
| `title` | 会话标题生成 | 只读权限 |
| `summary` | 会话摘要生成 | 只读权限 |

### Agent 权限系统

权限定义在 `agent.ts:86-103`，包含规则：
- `*`: 默认权限
- `doom_loop`: 循环检测
- `external_directory`: 外部目录访问
- `question`: 提问权限
- `plan_enter/exit`: 计划模式切换
- `read`: 读取权限（含文件模式匹配）

---

## 上下文管理

### 上下文溢出检测

当会话 token 数量接近模型上下文限制时，需要触发压缩。溢出检测在 `packages/opencode/src/session/overflow.ts` 中实现：

```typescript
export function isOverflow(input: {
  cfg: Config.Info
  tokens: MessageV2.Assistant["tokens"]
  model: Provider.Model
}) {
  const context = input.model.limit.context
  const reserved = input.cfg.compaction?.reserved ?? COMPACTION_BUFFER
  const usable = context - ProviderTransform.maxOutputTokens(input.model)
  return count >= usable
}
```

关键参数：
- `COMPACTION_BUFFER`: 20,000 tokens 保留缓冲
- `PRUNE_MINIMUM`: 最小修剪阈值（20,000 tokens）
- `PRUNE_PROTECT`: 保护阈值（40,000 tokens）

### 上下文修剪 (Prune)

`packages/opencode/src/session/compaction.ts` 中的 `prune()` 方法从后向前扫描，保留最近的工具调用输出：

1. 从最新的消息向前遍历
2. 跳过用户消息和摘要消息
3. 计算已完成的工具调用输出 token
4. 超过 `PRUNE_PROTECT` 时标记为已压缩
5. 保护 `skill` 工具的输出不被修剪

```typescript
// 关键修剪逻辑 (compaction.ts:108-127)
for (let msgIndex = msgs.length - 1; msgIndex >= 0; msgIndex--) {
  // 跳过最近的两个对话轮次
  if (turns < 2) continue
  // 遇到摘要则停止
  if (msg.info.role === "assistant" && msg.info.summary) break
  // 累加工具输出直到超过保护阈值
  if (total > PRUNE_PROTECT) toPrune.push(part)
}
```

### 上下文压缩 (Compaction)

压缩流程在 `processCompaction()` 中实现：

1. 创建新的用户消息作为压缩父节点
2. 调用专门的 `compaction` Agent 分析历史对话
3. 生成压缩摘要替换原始消息
4. 释放旧消息的上下文空间

### 会话摘要

`packages/opencode/src/session/summary.ts` 提供会话摘要功能：

```typescript
export interface Interface {
  readonly summarize: (input: { sessionID: SessionID; messageID: MessageID }) => Effect.Effect<void>
  readonly diff: (input: { sessionID: SessionID; messageID?: MessageID }) => Effect.Effect<Snapshot.FileDiff[]>
  readonly computeDiff: (input: { messages: MessageV2.WithParts[] }) => Effect.Effect<Snapshot.FileDiff[]>
}
```

- `summarize()`: 为指定消息生成摘要
- `diff()`: 计算会话期间的代码变更
- `computeDiff()`: 通过 `step-start` 和 `step-finish` 快照对比计算 diff

### 消息结构

会话消息在 `packages/opencode/src/session/message-v2.ts` 中定义，支持多种 Part 类型：

| Part 类型 | 描述 |
|-----------|------|
| `text` | 文本输出 |
| `reasoning` | 推理过程 |
| `tool` | 工具调用及结果 |
| `file` | 文件引用 |
| `subtask` | 子任务 |
| `step-start/step-finish` | 步骤边界标记 |

---

## 工具系统

### 工具注册

工具注册在 `packages/opencode/src/tool/registry.ts` 中实现：

```typescript
export interface Interface {
  readonly ids: () => Effect.Effect<string[]>
  readonly all: () => Effect.Effect<Tool.Def[]>
  readonly tools: (model: { providerID, modelID, agent }) => Effect.Effect<Tool.Def[]>
}
```

### 工具定义

工具定义在 `packages/opencode/src/tool/tool.ts` 中：

```typescript
export interface Def<Parameters extends z.ZodType = z.ZodType, M extends Metadata = Metadata> {
  id: string
  description: string
  parameters: Parameters
  execute(args: z.infer<Parameters>, ctx: Context): Effect.Effect<ExecuteResult<M>>
  formatValidationError?(error: z.ZodError): string
}
```

工具执行上下文包含：

```typescript
export type Context<M extends Metadata = Metadata> = {
  sessionID: SessionID
  messageID: MessageID
  agent: string
  abort: AbortSignal
  callID?: string
  extra?: { [key: string]: any }
  messages: MessageV2.WithParts[]
  metadata(input: { title?: string; metadata?: M }): Effect.Effect<void>
  ask(input: Omit<Permission.Request, "id" | "sessionID" | "tool">): Effect.Effect<void>
}
```

### 内置工具列表

| 工具 ID | 实现文件 | 功能 |
|---------|----------|------|
| `read` | `tool/read.ts` | 读取文件内容 |
| `write` | `tool/write.ts` | 写入文件 |
| `edit` | `tool/edit.ts` | 编辑文件 |
| `bash` | `tool/bash.ts` | 执行 Shell 命令 |
| `glob` | `tool/glob.ts` | 文件模式匹配 |
| `grep` | `tool/grep.ts` | 内容搜索 |
| `task` | `tool/task.ts` | 启动子 Agent |
| `todo` | `tool/todo.ts` | 任务管理 |
| `webfetch` | `tool/webfetch.ts` | 网页抓取 |
| `websearch` | `tool/websearch.ts` | 网络搜索 |
| `codesearch` | `tool/codesearch.ts` | 代码搜索 |
| `apply_patch` | `tool/apply_patch.ts` | 应用补丁 |
| `question` | `tool/question.ts` | 向用户提问 |
| `skill` | `tool/skill.ts` | 加载技能 |
| `lsp` | `tool/lsp.ts` | LSP 功能 |

### 工具权限系统

工具执行前需要通过权限检查。权限定义在 `packages/opencode/src/permission/index.ts`：

```typescript
export const Action = z.enum(["allow", "deny", "ask"])
export type Action = "allow" | "deny" | "ask"

export const Ruleset = Rule.array()
export interface Rule {
  permission: string
  pattern: string
  action: Action
}
```

权限请求流程：

1. Agent 调用工具时，`ctx.ask()` 触发权限检查
2. 根据 Agent 的 `permission` 规则集评估是否允许
3. 如需询问用户，弹出权限请求对话框
4. 用户可选择 "一次"、"始终" 或 "拒绝"

### 工具输出截断

工具输出可能很大，系统使用 `packages/opencode/src/tool/truncate.ts` 进行截断处理：

- 每个工具执行后自动截断
- 截断阈值根据模型输出限制动态计算
- 保留截断标记和原始输出路径

### 插件扩展

工具系统支持插件扩展：

```typescript
// 从插件加载自定义工具 (registry.ts:130-150)
function fromPlugin(id: string, def: ToolDefinition): Tool.Def {
  return {
    id,
    parameters: z.object(def.args),
    description: def.description,
    execute: (args, toolCtx) => /* 执行插件逻辑 */,
  }
}
```

---

## 相关文件索引

### 核心源文件

| 文件路径 | 说明 |
|----------|------|
| `packages/opencode/src/cli/cmd/tui/app.tsx` | TUI 主入口 |
| `packages/opencode/src/cli/cmd/tui/routes/session/index.tsx` | Session 页面 |
| `packages/opencode/src/command/index.ts` | 斜杠命令定义 |
| `packages/opencode/src/session/prompt.ts` | 提示词构建 |
| `packages/opencode/src/session/processor.ts` | Agent 循环处理 |
| `packages/opencode/src/session/llm.ts` | LLM 流处理 |
| `packages/opencode/src/session/system.ts` | System Prompt 选择 |
| `packages/opencode/src/agent/agent.ts` | Agent 定义 |
| `packages/opencode/src/tool/` | 工具实现 |

### 模板文件

| 文件路径 | 说明 |
|----------|------|
| `packages/opencode/src/command/template/initialize.txt` | init 命令模板 |
| `packages/opencode/src/command/template/review.txt` | review 命令模板 |
| `packages/opencode/src/session/prompt/anthropic.txt` | Claude System Prompt |
| `packages/opencode/src/session/prompt/gpt.txt` | GPT System Prompt |
| `packages/opencode/src/session/prompt/default.txt` | 默认 System Prompt |