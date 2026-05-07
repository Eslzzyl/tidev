# Dynamic Context Pruning (DCP) — 工作原理总结

> 基于对 `opencode-dynamic-context-pruning` 项目源代码的深入分析。
> 项目地址：<https://github.com/Tarquinen/opencode-dynamic-context-pruning>
> 
> **代码位置约定**：所有路径均相对于 `opencode-dynamic-context-pruning/` 目录。

---

## 一、概述

**Dynamic Context Pruning (DCP)** 是一个 OpenCode 插件，核心目标是在与 LLM 的长时间对话会话中**自动减少 Token 消耗**，通过智能地压缩、去重和清理过时的上下文，在不丢失关键信息的前提下保持上下文窗口的"高信噪比"。

> 代码入口：`index.ts`（插件注册与钩子绑定）

### 关键设计原则

| 原则 | 说明 | 相关代码 |
|------|------|----------|
| **不修改会话历史** | DCP 从不删除或修改 OpenCode 的原始会话记录。它只在发送给 LLM 的请求中注入占位符和摘要。 | `lib/messages/prune.ts` — `filterCompressedRanges()` 在运行时替换内容 |
| **模型驱动的压缩** | 压缩由 LLM 自主决定何时触发（或通过 `/dcp compress` 手动触发），非自动静默策略。 | `lib/compress/range.ts` / `lib/compress/message.ts` — compress 工具 |
| **保护关键内容** | 受保护的工具输出（如 `task`, `skill`）、用户消息、和文件路径模式在压缩时被保留在摘要中。 | `lib/compress/protected-content.ts` |
| **层级化压缩块** | 新压缩可以嵌套旧压缩块，信息层层保留而非稀释。 | `lib/compress/range-utils.ts` — `injectBlockPlaceholders()` / `appendMissingBlockSummaries()` |

---

## 二、核心架构

### 模块结构

```
opencode-dynamic-context-pruning/
│
├── index.ts                          ← 插件入口，注册钩子和 compress 工具
│
├── dcp.schema.json                   ← 配置 JSON Schema
│
├── lib/
│   ├── config.ts                     ← 配置加载与合并（全局 / 项目级 / 默认值）
│   ├── auth.ts                       ← 安全模式下的客户端认证
│   ├── hooks.ts                      ← 核心钩子：系统提示、消息转换、命令执行、事件处理
│   ├── host-permissions.ts           ← 与 OpenCode 宿主权限交互
│   ├── compress-permission.ts        ← compress 工具权限状态解析
│   ├── message-ids.ts                ← 消息 ID（mNNNN / bN）分配与解析
│   ├── token-utils.ts                ← Token 计数（使用 Anthropic Tokenizer）
│   ├── protected-patterns.ts         ← 通配符匹配、文件路径和工具名称保护
│   ├── logger.ts                     ← 调试日志
│   │
│   ├── state/                        ← 会话状态管理
│   │   ├── types.ts                  ← SessionState、CompressionBlock 等类型定义
│   │   ├── state.ts                  ← 会话状态创建、重置、初始化
│   │   ├── persistence.ts            ← 状态磁盘持久化（JSON 文件）
│   │   ├── utils.ts                  ← 辅助函数（折叠检测、消息编号等）
│   │   └── tool-cache.ts             ← 工具调用参数的缓存与同步
│   │
│   ├── compress/                     ← 压缩工具核心逻辑
│   │   ├── index.ts                  ← 导出
│   │   ├── types.ts                  ← 压缩相关的类型定义
│   │   ├── range.ts                  ← "范围模式"（range mode）压缩工具
│   │   ├── message.ts                ← "消息模式"（message mode）压缩工具
│   │   ├── range-utils.ts            ← 范围模式辅助函数（边界解析、占位符、嵌套）
│   │   ├── message-utils.ts          ← 消息模式辅助函数（验证、跳过处理）
│   │   ├── pipeline.ts               ← 压缩前准备 + 压缩后收尾的公共管道
│   │   ├── state.ts                  ← 压缩块 ID 分配、状态应用、摘要包装
│   │   ├── search.ts                 ← 会话消息获取、搜索上下文构建、边界解析
│   │   ├── protected-content.ts      ← 保护内容（用户消息、工具输出）追加到摘要
│   │   └── timing.ts                 ← 压缩耗时追踪
│   │
│   ├── strategies/                   ← 辅助策略（在压缩工具执行时触发）
│   │   ├── index.ts
│   │   ├── deduplication.ts          ← 工具调用去重
│   │   └── purge-errors.ts           ← 错误工具调用的输入清理
│   │
│   ├── messages/                     ← 消息变换管线
│   │   ├── index.ts
│   │   ├── prune.ts                  ← 核心修剪：压缩块替换 + 工具输出/输入/错误修剪
│   │   ├── shape.ts                  ← 消息结构验证与过滤
│   │   ├── query.ts                  ← 消息查询（最后用户消息、保护状态等）
│   │   ├── sync.ts                   ← 压缩块状态同步（激活/停用）
│   │   ├── priority.ts               ← 消息优先级分类（low/medium/high）
│   │   ├── utils.ts                  ← 工具函数（文本操作、幻觉剥离）
│   │   ├── reasoning-strip.ts        ← 推理内容剥离
│   │   └── inject/
│   │       ├── inject.ts             ← 消息 ID 注入、压缩提醒注入
│   │       ├── subagent-results.ts   ← 子代理结果扩展注入
│   │       └── utils.ts              ← 辅助函数（锚点管理、上下文超限检测）
│   │
│   ├── prompts/                      ← 提示词管理（可自定义覆盖）
│   │   ├── index.ts
│   │   ├── store.ts                  ← 提示词存储、加载、渲染
│   │   ├── system.ts                 ← 系统提示词（压缩哲学 + 使用指南）
│   │   ├── compress-range.ts         ← 范围模式工具描述
│   │   ├── compress-message.ts       ← 消息模式工具描述
│   │   ├── context-limit-nudge.ts    ← 上下文超限紧急提醒
│   │   ├── turn-nudge.ts             ← 轮次提醒
│   │   ├── iteration-nudge.ts        ← 迭代提醒
│   │   └── extensions/
│   │       ├── system.ts             ← 系统提示扩展（手动模式、子代理、保护工具）
│   │       ├── tool.ts               ← 工具格式扩展（不可自定义的 JSON schema 描述）
│   │       └── nudge.ts              ← 提醒上下文扩展（压缩块列表、优先级信息）
│   │
│   ├── subagents/                    ← 子代理集成
│   │   └── subagent-results.ts       ← 子代理结果获取与合并
│   │
│   ├── commands/                     ← /dcp 命令实现
│   │   ├── index.ts
│   │   ├── context.ts, stats.ts, sweep.ts, decompress.ts,
│   │   │   recompress.ts, manual.ts, help.ts
│   │   └── compression-targets.ts    ← sweep 命令的压缩目标选择
│   │
│   └── ui/
│       ├── notification.ts           ← 压缩完成通知（chat / toast）
│       └── utils.ts                  ← UI 工具函数（系统提示 Token 缓存）
```

### 插件入口数据流

```
OpenCode 启动
    │
    ▼
index.ts: Plugin 初始化                              ← index.ts (L23-133)
    ├── 加载配置 getConfig(ctx)                      ← lib/config.ts
    ├── 创建 SessionState                            ← lib/state/state.ts createSessionState()
    ├── 创建 PromptStore                              ← lib/prompts/store.ts
    │
    ├── 注册系统提示钩子 (experimental.chat.system.transform)  ← lib/hooks.ts createSystemPromptHandler()
    ├── 注册消息变换钩子 (experimental.chat.messages.transform) ← lib/hooks.ts createChatMessageTransformHandler()
    ├── 注册命令执行钩子 (command.execute.before)             ← lib/hooks.ts createCommandExecuteHandler()
    ├── 注册事件处理钩子 (event)                              ← lib/hooks.ts createEventHandler()
    ├── 注册 compress 工具 (tool.compress)                    ← lib/compress/range.ts 或 message.ts
    │
    └── config() 回调：注入 DCP 配置到 OpenCode            ← index.ts (L88-132)
        （权限、主工具列表等）
```

---

## 三、核心工作原理

### 3.1 请求-响应管线 (每轮对话)

每次用户发送消息、即将请求 LLM 时，DCP 通过 `experimental.chat.messages.transform` 钩子拦截消息列表，执行以下管线：

> 实现位置：`lib/hooks.ts` — `createChatMessageTransformHandler()` (L98-156)

```
原始消息列表
    │
    ▼
 1. checkSession()                   ← lib/state/state.ts (L17-63)
    ← 检测是否切换了会话，加载持久化状态；检测 OpenCode compaction 并重置
    │
    ▼
 2. stripHallucinations()             ← lib/messages/utils.ts
    ← 剥离 LLM 幻觉产生的虚假工具调用（部分输出被错误地格式化为工具调用）
    │
    ▼
 3. assignMessageRefs()               ← lib/message-ids.ts (L119-153)
    ← 为每条消息分配 mNNNN 引用 ID
    │
    ▼
 4. syncCompressionBlocks()           ← lib/messages/sync.ts (L15-124)
    ← 同步压缩块状态：源消息丢失的块自动停用；新块激活；
      被消费的旧块标记为停用
    │
    ▼
 5. syncToolCache()                   ← lib/state/tool-cache.ts
    ← 同步工具调用参数缓存到 state.toolParameters
    │
    ▼
 6. buildToolIdList()                 ← lib/messages/utils.ts
    ← 构建所有工具调用 ID 的有序列表（state.toolIdList）
    │
    ▼
 7. prune()                           ← lib/messages/prune.ts (L14-25)
    ← ★ 核心修剪步骤（见下方 3.2）
    │
    ▼
 8. injectExtendedSubAgentResults()   ← lib/messages/inject/subagent-results.ts
    ← 注入子代理的完整会话结果
    │
    ▼
 9. injectCompressNudges()            ← lib/messages/inject/inject.ts (L33-143)
    ← 注入压缩提醒标签（上下文超限 / 轮次 / 迭代）
    │
    ▼
10. injectMessageIds()                ← lib/messages/inject/inject.ts (L145-215)
    ← 将 mNNNN 标签注入到消息文本中
    │
    ▼
11. applyPendingManualTrigger()       ← lib/commands/manual.ts
    ← 处理 /dcp compress 挂起的手动触发
    │
    ▼
12. stripStaleMetadata()              ← lib/messages/reasoning-strip.ts
    ← 清理过期的元数据标签
    │
    ▼
发送给 LLM
```

### 3.2 核心修剪步骤 (`prune()`)

> 实现位置：`lib/messages/prune.ts` — `prune()` (L14-25)

`prune()` 函数执行四种子修剪：

```
prune()
    │
    ├── filterCompressedRanges()      ← lib/messages/prune.ts (L149-233)
    │     遍历消息列表，将所有被压缩块覆盖的消息从最终列表移除；
    │     在最后一个被覆盖的用户消息后面插入压缩摘要作为合成用户消息；
    │     摘要使用 [Compressed conversation section] 头部标记。
    │
    ├── pruneToolOutputs()            ← lib/messages/prune.ts (L73-121)
    │     将已标记为需修剪的工具调用输出替换为占位符文本：
    │     "[Output removed to save context - information superseded or no longer needed]"
    │
    ├── pruneToolInputs()             ← lib/messages/prune.ts (L122-148)
    │     将错误工具调用的输入替换为：
    │     "[input removed due to failed tool call]"
    │
    └── pruneToolErrors()             ← lib/messages/prune.ts (L149-233)
         （处理 `ask`/`question` 工具的输入替换）
         替换为："[questions removed - see output for user's answers]"
```

### 3.3 压缩工具的执行流程

当 LLM 调用 `compress` 工具时：

```
LLM 调用 compress 工具
    │
    ▼
pipeline.prepareSession()             ← lib/compress/pipeline.ts (L37-77)
    │
    ├── 请求用户权限（ask/allow）               ← toolCtx.ask()
    ├── 从 OpenCode API 获取当前会话的原始消息  ← lib/compress/search.ts fetchSessionMessages()
    ├── 初始化/恢复会话状态
    ├── 分配消息引用 ID
    ├── 执行去重策略 (deduplicate)              ← lib/strategies/deduplication.ts
    └── 执行错误清理策略 (purgeErrors)           ← lib/strategies/purge-errors.ts
    │
    ▼
构建 SearchContext（消息索引映射 + 压缩块映射） ← lib/compress/search.ts buildSearchContext()
    │
    ▼
根据模式（range / message）处理压缩
    │
    ├── [RANGE 模式]                          ← lib/compress/range.ts createCompressRangeTool()
    │   ├── resolveRanges()                   ← lib/compress/range-utils.ts (L41-68)
    │   │   解析 startId/endId 边界（支持 mNNNN / bN）
    │   ├── validateNonOverlapping()          ← lib/compress/range-utils.ts (L70-99)
    │   │   验证同一批中的范围不重叠
    │   ├── parseBlockPlaceholders()          ← lib/compress/range-utils.ts (L100-129)
    │   │   解析摘要中的 (bN) 占位符
    │   ├── validateSummaryPlaceholders()     ← lib/compress/range-utils.ts (L131-172)
    │   │   验证占位符完备（无缺失、无多余、无重复）
    │   ├── injectBlockPlaceholders()         ← lib/compress/range-utils.ts (L174-234)
    │   │   将 (bN) 替换为实际压缩块内容（嵌套展开）
    │   ├── appendProtectedUserMessages()     ← lib/compress/protected-content.ts (L16-54)
    │   ├── appendProtectedTools()            ← lib/compress/protected-content.ts (L56-154)
    │   └── appendMissingBlockSummaries()     ← lib/compress/range-utils.ts (L236-308)
    │       补全被吞并但未在摘要中引用的块内容
    │
    ├── [MESSAGE 模式]                        ← lib/compress/message.ts createCompressMessageTool()
    │   ├── resolveMessages()                 ← lib/compress/message-utils.ts
    │   │   解析消息 ID，跳过已压缩/受保护的消息
    │   └── appendProtectedTools()            ← lib/compress/protected-content.ts (L56-154)
    │
    ▼
applyCompressionState()               ← lib/compress/state.ts (L62-268)
    ← 创建 CompressionBlock，更新会话状态：
      分配 blockId，记录消息/工具 ID 映射，更新 Token 统计
    │
    ▼
finalizeSession()                     ← lib/compress/pipeline.ts (L79-106)
    ├── 应用压缩耗时数据（applyPendingCompressionDurations）
    ├── 保存会话状态到磁盘（saveSessionState）
    ├── 计算当前 Token 使用情况
    └── 发送压缩完成通知（sendCompressNotification）
```

---

## 四、压缩模式详解

### 4.1 范围压缩模式 (Range Mode)

> `lib/compress/range.ts` — `createCompressRangeTool()` (L52-180)
> `lib/compress/range-utils.ts` — 全部辅助函数
> 提示词：`lib/prompts/compress-range.ts`

- LLM 指定 `startId`/`endId` 边界（格式 `mNNNN` 或 `bN`）
- 压缩连续的一段对话范围为一个或多个摘要
- 支持**块占位符嵌套**：在摘要中使用 `(bN)` 引用已存在的压缩块
  - 占位符正则：`/(b(\d+))|\{block_(\d+)\}/gi`（`range-utils.ts` L12）
- DCP 自动将占位符展开为完整块内容，实现层级化压缩
- 非重叠验证：同一批中的多个范围不得重叠

### 4.2 消息压缩模式 (Message Mode) — 实验性

> `lib/compress/message.ts` — `createCompressMessageTool()` (L41-137)
> `lib/compress/message-utils.ts` — 辅助函数
> 提示词：`lib/prompts/compress-message.ts`

- LLM 指定独立的 `messageId`，每条消息独立压缩
- 支持"通用清理"（general cleanup）批处理
- 优先级系统：根据 Token 大小和是否已包含 compress 调用，标记为 low/medium/high
  - 实现位置：`lib/messages/priority.ts` — `buildPriorityMap()` (L20-62)
- 不涉及块嵌套（因为每次只压缩一条消息）

---

## 五、辅助策略

### 5.1 工具调用去重 (Deduplication)

> `lib/strategies/deduplication.ts` — `deduplicate()` (L16-94)

- 检测相同工具名 + 相同参数的重复调用
- 参数归一化：`createToolSignature()` (L96-103) — 对参数排序标准化后生成特征字符串
- 只保留最近一次的调用输出（每组中最后一个 ID 保留，其余修剪）
- 受保护的工具和文件路径模式不会被去重
- 在每次 `compress` 工具执行时通过 `pipeline.ts` 调用

### 5.2 错误输入清理 (Purge Errors)

> `lib/strategies/purge-errors.ts` — `purgeErrors()` (L19-88)

- 对于状态为 `error` 的工具调用，经过可配置的轮次数（默认 4 轮）后，移除其输入内容
- 但保留错误消息本身
- 轮次比较：`turnAge = state.currentTurn - metadata.turn >= turnThreshold` (L72-73)
- 同样在每次 `compress` 工具执行时触发

---

## 六、消息 ID 与标签系统

> `lib/message-ids.ts` — 全部

DCP 为每条消息分配一个 `mNNNN` 引用 ID，并在消息文本中注入 XML 标签：

```xml
<dcp-message-id priority="high">m0007</dcp-message-id>
```

- **ID 分配**：`assignMessageRefs()` (L119-153) — 在每轮消息变换时运行
- **格式生成**：`formatMessageIdTag()` (L101-117) — 生成 XML 标签
- **ID 解析**：`parseBoundaryId()` (L70-91) — 解析 `mNNNN` 或 `bN`
- 标签注入到用户消息的文本部分，或助手消息的所有工具输出部分
  - 注入位置：`lib/messages/inject/inject.ts` — `injectMessageIds()` (L145-215)
- 压缩块引用：`bN` 格式，通过 `formatBlockRef()` (L37-42) 生成
- 标签中 `priority` 属性仅在消息模式下使用

---

## 七、提醒系统 (Nudges)

> `lib/messages/inject/inject.ts` — `injectCompressNudges()` (L33-143)
> 提示词定义（可自定义）：
> - `lib/prompts/context-limit-nudge.ts` — 上下文超限提醒
> - `lib/prompts/turn-nudge.ts` — 轮次提醒
> - `lib/prompts/iteration-nudge.ts` — 迭代提醒

DCP 通过注入 `<dcp-system-reminder>` 标签在上下文中催促 LLM 执行压缩：

| 提醒类型 | 触发条件 | 实现逻辑 |
|---------|---------|----------|
| **上下文超限提醒** | 当前 Token ≥ `maxContextLimit`（默认 85%） | `isContextOverLimits()` (inject/utils.ts)，锚点由 `addAnchor()` 管理 |
| **轮次提醒** | Token ≥ `minContextLimit`（默认 65%）且出现新用户-助手轮次 | 每轮在 `turnNudgeAnchors` 中添加用户+助手消息 ID |
| **迭代提醒** | Token ≥ `minContextLimit` 且同一用户消息后迭代次数超阈值 | `getIterationNudgeThreshold()`，默认 3 次 |

提醒频率由 `nudgeFrequency`（默认 3）控制。
一旦 LLM 成功调用了 `compress`（通过 `messageHasCompress()` 检测），所有提醒锚点被清除（inject.ts L53-57）。

锚点类型定义在 `lib/state/types.ts` (L87-91)：

```typescript
interface Nudges {
    contextLimitAnchors: Set<string>   // 上下文超限锚点
    turnNudgeAnchors: Set<string>      // 轮次提醒锚点
    iterationNudgeAnchors: Set<string> // 迭代提醒锚点
}
```

---

## 八、保护内容系统

> `lib/compress/protected-content.ts` — 全部
> `lib/protected-patterns.ts` — 通配符匹配逻辑

DCP 确保以下内容在压缩时不被丢弃：

### 保护的工具输出

- 默认保护工具：`task`, `skill`, `todowrite`, `todoread`（`lib/config.ts` L89）
- 在 `protectedTools` 中可扩展（支持 `*`/`?` 通配符，`lib/protected-patterns.ts` L109-128）
- `appendProtectedTools()`（protected-content.ts L56-154）将保护的工具输出附加到摘要末尾

### 保护的文件路径模式

- 通过 `protectedFilePatterns` 配置
- 支持 glob 通配符（`*`, `**`, `?`）
- 实现在 `lib/protected-patterns.ts` — `matchesGlob()` (L9-58)

### 保护的用户消息

- `protectUserMessages: true` 时，用户消息原文被追加到压缩摘要末尾
- 实现：`appendProtectedUserMessages()`（protected-content.ts L16-54）

### 子代理保护

- 当 `experimental.allowSubAgents: true` 时，保护工具列表中的 `task` 工具输出会包含子代理的完整会话结果
- 实现：`lib/subagents/subagent-results.ts` — `buildSubagentResultText()` / `mergeSubagentResult()`
- 结果缓存在 `state.subAgentResultCache` 中

---

## 九、压缩块生命周期

> `lib/state/types.ts` — `CompressionBlock` 接口 (L33-60)
> `lib/compress/state.ts` — 块分配与状态应用 (L1-268)
> `lib/messages/sync.ts` — 块状态同步 (L15-124)

```
创建 (compress 调用成功)                ← applyCompressionState() 分配 blockId
  │
  ▼
激活 (active = true)                   ← syncCompressionBlocks() 标记为活跃
  │  ├── 原始消息在 prune() 中被摘要替换
  │  └── 新压缩可以嵌套它（作为 consumedBlock）
  │
  ▼
嵌套/停用 (被更新的压缩块消费)           ← consumedBlockIds 列表记录
  │  旧块 deactivatedByBlockId = 新块ID
  │
  ▼
源消息丢失 (compressMessageId 所在消息被删除)
  │                                     ← syncCompressionBlocks() 检测源消息缺失
  │
  ▼
停用 (active = false)                  ← deactivatedAt 记录时间戳
```

`CompressionBlock` 关键字段：

| 字段 | 说明 |
|------|------|
| `blockId` | 块唯一标识 |
| `runId` | 压缩执行批次 ID |
| `active` | 是否活跃（在 prune 中生效） |
| `consumedBlockIds` | 此块消费（嵌套）的旧块 ID 列表 |
| `parentBlockIds` | 消费此块的父块 ID 列表 |
| `directMessageIds` | 此块直接压缩的消息 ID |
| `effectiveMessageIds` | 此块及其所有嵌套块的递归消息 ID 集合 |
| `summary` | 存储的完整摘要内容 |

---

## 十、配置层次

> `lib/config.ts` — 配置加载逻辑 (L930-987)
> `dcp.schema.json` — 完整配置 JSON Schema

配置按优先级从低到高为：

1. **内置默认值** — `lib/config.ts` 中定义
2. **默认配置文件** — `~/.config/opencode/dcp.json`
3. **项目级配置** — 项目目录下的 `dcp.json`
4. **运行时配置** — `presets.toml` 中的 `[plugins.dcp]` 段

重要配置项：

| 配置 | 默认值 | 类型 | 说明 |
|------|-------|------|------|
| `enabled` | `true` | `boolean` | 是否启用 |
| `compress.mode` | `"range"` | `"range"\|"message"` | 压缩模式 |
| `compress.permission` | `"ask"` | `"ask"\|"allow"\|"deny"` | 权限模式 |
| `compress.maxContextLimit` | `"85%"` | `number\|string` | 最大上下文阈值（超限即触发紧急提醒） |
| `compress.minContextLimit` | `"65%"` | `number\|string` | 最小上下文阈值（用于温和提醒） |
| `compress.nudgeFrequency` | `3` | `number` | 提醒间隔（每 N 次触发一次） |
| `compress.protectedTools` | `[task, skill, todowrite, todoread]` | `string[]` | 保护工具列表 |
| `compress.protectUserMessages` | `false` | `boolean` | 压缩时保留用户消息原文 |
| `strategies.deduplication.enabled` | `true` | `boolean` | 启用工具去重 |
| `strategies.purgeErrors.enabled` | `true` | `boolean` | 启用错误清理 |
| `strategies.purgeErrors.turns` | `4` | `number` | 错误清理等待轮次 |
| `manualMode.enabled` | `false` | `boolean` | 手动模式（仅通过 `/dcp` 触发） |
| `protectedFilePatterns` | `[]` | `string[]` | 受保护的文件路径 glob 模式 |

---

## 十一、提示缓存影响

DCP 在每次请求前修改消息列表（注入标签、替换摘要、移除修剪消息），这会**改变缓存前缀**，导致从修改点之后的缓存失效。但同时大幅减少上下文大小，综合效果在长会话中通常是正向的。

> 实测：无 DCP 时缓存命中率约 90%，启用后约 85%。

> 文档参考：`README.md` L213-226

**无影响场景：**
- **基于请求数的计费** — 如 GitHub Copilot（按请求计费，非 Token）
- **统一 Token 定价** — 如 Cerebras（缓存与未缓存 Token 同价）

---

## 十二、/dcp 命令

> `lib/commands/` — 全部命令实现

| 命令 | 实现文件 | 功能 |
|------|---------|------|
| `/dcp context` | `lib/commands/context.ts` | 显示上下文使用状态（当前 Token、压缩块数量、配置阈值等） |
| `/dcp stats` | `lib/commands/stats.ts` | 显示压缩统计数据（总节省 Token、块数、运行次数） |
| `/dcp sweep [n]` | `lib/commands/sweep.ts` | 批量压缩最旧的 N 条可压缩消息（自动选择压缩目标） |
| `/dcp compress <focus>` | `lib/commands/manual.ts` | 手动触发压缩，可指定 focus 提示 |
| `/dcp decompress <blockId>` | `lib/commands/decompress.ts` | 解压缩恢复某个块（标记 deactivatedByUser） |
| `/dcp recompress <blockId>` | `lib/commands/recompress.ts` | 重新压缩某个块 |
| `/dcp manual [on/off]` | `lib/commands/manual.ts` | 切换手动模式 |
| `/dcp help` | `lib/commands/help.ts` | 显示帮助信息 |

命令入口：`lib/hooks.ts` — `createCommandExecuteHandler()` (L158-274)

---

## 十三、核心数据结构

### SessionState

> `lib/state/types.ts` (L93-111)

```typescript
interface SessionState {
    sessionId: string | null
    isSubAgent: boolean
    manualMode: false | "active" | "compress-pending"
    compressPermission: "ask" | "allow" | "deny" | undefined
    pendingManualTrigger: PendingManualTrigger | null
    prune: Prune                              // 修剪状态（工具 + 消息）
    nudges: Nudges                            // 提醒锚点
    stats: SessionStats                       // Token 统计
    compressionTiming: CompressionTimingState  // 耗时追踪
    toolParameters: Map<string, ToolParameterEntry>
    subAgentResultCache: Map<string, string>
    toolIdList: string[]
    messageIds: MessageIdState
    lastCompaction: number
    currentTurn: number
    modelContextLimit: number | undefined
    systemPromptTokens: number | undefined
}
```

### CompressionBlock

> `lib/state/types.ts` (L33-60)

```typescript
interface CompressionBlock {
    blockId: number
    runId: number
    active: boolean
    deactivatedByUser: boolean
    compressedTokens: number
    summaryTokens: number
    durationMs: number
    mode?: CompressionMode
    topic: string
    startId: string                        // 范围起始引用
    endId: string                          // 范围结束引用
    anchorMessageId: string                // 摘要插入位置的用户消息 ID
    compressMessageId: string              // 触发此压缩的助手消息 ID
    includedBlockIds: number[]             // 直接包含的块
    consumedBlockIds: number[]             // 消费（嵌套）的块
    parentBlockIds: number[]               // 消费此块的父块
    directMessageIds: string[]             // 直接压缩的消息
    effectiveMessageIds: string[]          // 递归所有消息
    effectiveToolIds: string[]             // 递归所有工具调用
    summary: string                        // 存储的摘要文本
}
```

### SearchContext

> `lib/compress/types.ts` (L44-49)

```typescript
interface SearchContext {
    rawMessages: WithParts[]
    rawMessagesById: Map<string, WithParts>
    rawIndexById: Map<string, number>
    summaryByBlockId: Map<number, CompressionBlock>
}
```

---

## 十四、状态持久化

> `lib/state/persistence.ts` — 全部

- 存储路径：`~/.local/share/opencode/storage/plugin/dcp/{sessionId}.json`
- 每次 `compress` 工具完成后保存
- 持久化内容：修剪的工具 ID、压缩块列表、提醒锚点、Token 统计
- 会话切换时自动恢复：`ensureSessionInitialized()` (L137-188) 从磁盘加载
- 文件格式：`PersistedSessionState` (L36-42)

---

## 十五、测试

测试文件位于 `tests/` 目录：

| 测试文件 | 测试内容 |
|---------|---------|
| `tests/compress-message.test.ts` | 消息模式压缩逻辑 |
| `tests/compress-range.test.ts` | 范围模式压缩逻辑 |
| `tests/compress-range-placeholders.test.ts` | 范围模式占位符解析与替换 |
| `tests/compression-groups.test.ts` | 压缩分组 |
| `tests/compression-targets.test.ts` | sweep 命令的压缩目标选择 |
| `tests/hooks-permission.test.ts` | 权限状态同步 |
| `tests/host-permissions.test.ts` | 宿主权限解析 |
| `tests/message-ids.test.ts` | 消息 ID 分配与解析 |
| `tests/message-priority.test.ts` | 消息优先级分类 |
| `tests/message-utils.test.ts` | 消息工具函数 |
| `tests/prompts.test.ts` | 提示词渲染 |
| `tests/token-counting.test.ts` | Token 计数 |
| `tests/token-usage.test.ts` | Token 使用统计 |

---

## 十六、总结：一句话工作原理

> DCP 作为一个 OpenCode 插件，在每次请求前通过**消息变换钩子**（`lib/hooks.ts` `createChatMessageTransformHandler`）对消息列表进行**修剪**（将已压缩范围替换为摘要、移除去重的工具输出、清理错误输入），同时暴露一个 **`compress` 工具**（`lib/compress/range.ts` 或 `lib/compress/message.ts`）供 LLM 自主调用以生成高质量的层级化摘要，并通过**提醒系统**（`lib/messages/inject/inject.ts` `injectCompressNudges`）在上下文接近限制时催促 LLM 执行压缩操作，从而在保持关键信息的前提下显著减小发送给 LLM 的上下文体积。
