# Magiccode Memory System

## Overview

Magiccode implements a file-based, hierarchical memory system that allows agents to retain persistent knowledge across sessions. Unlike opencode's two-phase pipeline (extract → consolidate), magiccode uses a **user-driven, manual-save** paradigm with a structured taxonomy and automated background extraction.

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    Magiccode Memory System                       │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────────┐   ┌─────────────────┐   ┌──────────────┐  │
│  │   memdir        │   │  SessionMemory  │   │ extractMemories│
│  │   (auto memory) │   │  (session scope)│   │ (background)  │
│  └────────┬────────┘   └────────┬────────┘   └──────┬───────┘  │
│           │                     │                    │          │
│           ▼                     │                    ▼          │
│  ┌─────────────────────────────┴──────────────────────────────┐ │
│  │              File System (memory directory)                 │ │
│  └────────────────────────────────────────────────────────────┘ │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Directory Structure

```
~/.claude/projects/<project>/memory/
├── MEMORY.md                    # 入口索引文件（每条一行，约150字符）
├── user_role.md                 # 记忆文件（独立主题）
├── feedback_testing.md
├── project_deadline.md
└── reference_linear.md

# 仅团队模式 (feature('TEAMMEM'))
~/.claude/projects/<project>/memory/team/
├── MEMORY.md                    # 团队记忆索引
└── ...

# 仅 KAIROS 模式 (feature('KAIROS'))
~/.claude/projects/<project>/memory/
├── MEMORY.md                    # 蒸馏索引（夜间生成）
└── logs/
    ├── YYYY/
    │   └── MM/
    │       └── YYYY-MM-DD.md    # 每日日志（追加模式）
```

## Memory Types

### Type Taxonomy (`src/memdir/memoryTypes.ts`)

Four closed types capturing context NOT derivable from project state:

```typescript
export const MEMORY_TYPES = ['user', 'feedback', 'project', 'reference'] as const
export type MemoryType = (typeof MEMORY_TYPES)[number]
```

| Type | Scope | Description |
|------|-------|-------------|
| `user` | always private | 用户角色、目标、知识水平 |
| `feedback` | private/team | 用户对工作方式的指导（纠正和确认） |
| `project` | private/team | 项目状态、目标、截止日期、决策 |
| `reference` | usually team | 外部系统指针（Linear, Slack, Grafana） |

### Memory Frontmatter Format

```yaml
---
name: Memory Title
description: One-line hook for MEMORY.md index
type: user|feedback|project|reference
---
```

## Core Modules

### memdir (Auto Memory)

**Source**: `src/memdir/memdir.ts`

Main memory directory management module.

#### Key Functions

```typescript
// 构建记忆行为指令（不含 MEMORY.md 内容）
export function buildMemoryLines(
  displayName: string,
  memoryDir: string,
  extraGuidelines?: string[],
  skipIndex = false,
): string[]

// 构建含 MEMORY.md 内容的完整提示
export function buildMemoryPrompt(params: {
  displayName: string
  memoryDir: string,
  extraGuidelines?: string[]
}): string

// 加载系统提示用的统一记忆提示
export async function loadMemoryPrompt(): Promise<string | null>

// 确保记忆目录存在
export async function ensureMemoryDirExists(memoryDir: string): Promise<void>

// 截断 MEMORY.md 内容（超过 200 行或 25,000 字节时）
export function truncateEntrypointContent(raw: string): EntrypointTruncation
```

#### Memory Entrypoint Constraints

```typescript
export const ENTRYPOINT_NAME = 'MEMORY.md'
export const MAX_ENTRYPOINT_LINES = 200
export const MAX_ENTRYPOINT_BYTES = 25_000
```

### paths (Memory Path Resolution)

**Source**: `src/memdir/paths.ts`

Memory directory path resolution with security validation.

#### Key Functions

```typescript
// 自动记忆目录路径（解析顺序：覆盖 > settings.json > 默认路径）
export const getAutoMemPath = memoize((): string => { ... })

// 自动记忆入口文件路径
export function getAutoMemEntrypoint(): string

// 检查路径是否在自动记忆目录内
export function isAutoMemPath(absolutePath: string): boolean

// 是否启用自动记忆功能
export function isAutoMemoryEnabled(): boolean
```

#### Path Resolution Order

1. `CLAUDE_COWORK_MEMORY_PATH_OVERRIDE` env var (full-path override)
2. `autoMemoryDirectory` in settings.json (trusted sources only)
3. `<memoryBase>/projects/<sanitized-git-root>/memory/`

### SessionMemory (Session Scope)

**Source**: `src/services/SessionMemory/sessionMemory.ts`

Session-scoped memory management.

#### Key Functions

```typescript
// 加载会话记忆
export async function loadSessionMemory(): Promise<SessionMemory>

// 解析会话记忆文件
export function parseSessionMemory(content: string): SessionMemory

// 获取会话记忆文件路径
export function getSessionMemoryPath(): string

// 获取会话记忆目录
export function getSessionMemoryDir(): string

// 写入会话记忆
export async function writeSessionMemory(
  cwd: string,
  sessionId: string,
  content: string
): Promise<void>
```

### extractMemories (Background Extraction)

**Source**: `src/services/extractMemories/extractMemories.ts`

Background agent for memory extraction from conversation transcripts.

#### Key Functions

```typescript
// 运行记忆提取 turn-end fork
export async function runExtractMemoriesTurnEnd(params: {
  sessionId: string
  transcriptPath: string
  memoryDir: string
}): Promise<ExtractMemoriesResult>

// 解析提取结果
export function parseExtractMemoriesResult(
  raw: string
): ExtractMemoriesResult

// 是否有自上次提取以来的记忆写入
export function hasMemoryWritesSince(lastExtractedAt: number): boolean
```

#### Extraction Prompts

**Source**: `src/services/extractMemories/prompts.ts`

- `MEMORY_EXTRACT_SYSTEM_PROMPT`: System prompt for extraction agent
- `MEMORY_EXTRACT_USER_PROMPT`: User prompt with transcript content

### compact (Auto Compact)

**Source**: `src/services/compact/compact.ts`

Automatic session memory compaction triggered by memory threshold.

#### Key Functions

```typescript
// 运行自动压缩
export async function runAutoCompact(params: {
  sessionId: string
  transcriptPath: string
  cwd: string
}): Promise<CompactResult>

// 检查是否需要压缩
export function shouldCompact(params: {
  sessionMemory: SessionMemory
  compactThreshold: number
}): boolean
```

### sessionMemoryCompact (Session Memory Compaction)

**Source**: `src/services/compact/sessionMemoryCompact.ts`

Compacts session memory by summarizing older turns.

#### Key Functions

```typescript
// 压缩会话记忆
export async function compactSessionMemory(params: {
  sessionId: string
  sessionMemory: SessionMemory
  cwd: string
}): Promise<SessionMemoryCompactResult>

// 获取需要压缩的 turn 范围
export function getTurnsToCompact(
  sessionMemory: SessionMemory,
  compactThreshold: number
): TurnRange[]

// 构建压缩提示
export function buildCompactPrompt(params: {
  recentHistory: string
  turnsToCompact: TurnRange[]
}): string
```

### agentMemory (Agent Tool)

**Source**: `src/tools/AgentTool/agentMemory.ts`

Tool for agent to read/write memories.

#### Key Functions

```typescript
// 获取代理记忆提示（用于 agent 模式）
export function getAgentMemoryPrompt(): string

// 获取代理记忆工具描述
export function getAgentMemoryTool(): Tool
```

## Memory Saving Workflow

### Two-Step Process

```
Step 1: 写入独立的记忆文件
  └── user_role.md
        ---
        name: User Role
        description: Senior engineer, prefers concise responses
        type: user
        ---
        Content...

Step 2: 在 MEMORY.md 中添加指针
  └── MEMORY.md
        - [User Role](user_role.md) — Senior engineer, prefers concise responses
```

### buildMemoryLines Behavior

```typescript
// skipIndex = false 时，两步提示
const howToSave = [
  'Step 1 — write the memory to its own file...',
  'Step 2 — add a pointer to that file in `MEMORY.md`...',
]

// skipIndex = true 时，仅一步提示（用于特定场景）
const howToSave = [
  'Write each memory to its own file...',
]
```

## Feature Gates

| Feature | Flag | Description |
|---------|------|-------------|
| Auto Memory | — | 默认启用，可通过 `CLAUDE_CODE_DISABLE_AUTO_MEMORY` 禁用 |
| extractMemories | `tengu_passport_quail` | 背景提取 agent |
| Past Context Search | `tengu_coral_fern` | 过去上下文搜索（grep transcript） |
| Team Memory | `TEAMMEM` | 团队共享记忆 |
| KAIROS | `KAIROS` | 助手模式每日日志 |

## Safety Features

### Path Validation (`src/memdir/paths.ts`)

```typescript
function validateMemoryPath(raw: string | undefined, expandTilde: boolean): string | undefined {
  // 拒绝相对路径
  if (!isAbsolute(normalized)) return undefined
  // 拒绝根路径 (/)
  if (normalized.length < 3) return undefined
  // 拒绝 Windows 盘符根路径 (C:)
  if (/^[A-Za-z]:$/.test(normalized)) return undefined
  // 拒绝 UNC 路径
  if (normalized.startsWith('\\\\')) return undefined
  // 拒绝 null byte
  if (normalized.includes('\0')) return undefined
}
```

### Settings.json Security

Project-level `settings.json` (`autoMemoryDirectory`) 被故意排除：
- 恶意仓库可能设置 `~/.ssh` 路径获得敏感目录写权限
- 仅 policy/local/user settings 被信任

## Entry Point Management

### Truncation Logic

```typescript
export function truncateEntrypointContent(raw: string): EntrypointTruncation {
  const lineCount = contentLines.length
  const byteCount = trimmed.length

  const wasLineTruncated = lineCount > MAX_ENTRYPOINT_LINES
  const wasByteTruncated = byteCount > MAX_ENTRYPOINT_BYTES

  // 先按行截断，再按字节截断
  let truncated = wasLineTruncated
    ? contentLines.slice(0, MAX_ENTRYPOINT_LINES).join('\n')
    : trimmed

  if (truncated.length > MAX_ENTRYPOINT_BYTES) {
    const cutAt = truncated.lastIndexOf('\n', MAX_ENTRYPOINT_BYTES)
    truncated = truncated.slice(0, cutAt > 0 ? cutAt : MAX_ENTRYPOINT_BYTES)
  }

  // 追加截断警告
  truncated += `\n\n> WARNING: ${ENTRYPOINT_NAME} is ${reason}.`
}
```

## Comparison with Opencode

| Aspect | Opencode | Magiccode |
|--------|----------|-----------|
| Paradigm | Pipeline (extract → consolidate) | User-driven (manual save) |
| Storage | SQLite + file system | File system only |
| Extraction | Automatic per session | Background agent (turn-end fork) |
| Taxonomy | None (free-form) | Four closed types |
| Scope | Global (across sessions) | Session + persistent |
| Citation | `<oai-mem-citation>` block | Pointer in MEMORY.md |
| Index | `memory_summary.md` | `MEMORY.md` (one-line per entry) |

## File Locations

### Core Modules

| File | Description |
|------|-------------|
| `src/memdir/memdir.ts` | Main memory directory management |
| `src/memdir/paths.ts` | Memory path resolution |
| `src/memdir/memoryTypes.ts` | Memory type taxonomy |
| `src/services/SessionMemory/sessionMemory.ts` | Session-scoped memory |
| `src/services/SessionMemory/sessionMemoryUtils.ts` | Session memory utilities |
| `src/services/SessionMemory/prompts.ts` | Session memory prompts |
| `src/services/extractMemories/extractMemories.ts` | Background extraction |
| `src/services/extractMemories/prompts.ts` | Extraction prompts |
| `src/services/compact/compact.ts` | Auto compaction |
| `src/services/compact/autoCompact.ts` | Auto compaction logic |
| `src/services/compact/sessionMemoryCompact.ts` | Session memory compaction |
| `src/tools/AgentTool/agentMemory.ts` | Agent memory tool |

## Usage Example

```typescript
// 1. 检查记忆功能是否启用
if (isAutoMemoryEnabled()) {
  const memoryDir = getAutoMemPath()
  await ensureMemoryDirExists(memoryDir)

  // 2. 构建记忆提示
  const prompt = buildMemoryLines('auto memory', memoryDir)

  // 3. 截断入口文件内容
  const entrypoint = getAutoMemEntrypoint()
  const content = fs.readFileSync(entrypoint, { encoding: 'utf-8' })
  const truncated = truncateEntrypointContent(content)

  // 4. 加载到系统提示
  const memoryPrompt = await loadMemoryPrompt()
}

// 5. 背景提取（turn-end fork）
if (feature('EXTRACT_MEMORIES') && isExtractModeActive()) {
  const result = await runExtractMemoriesTurnEnd({
    sessionId,
    transcriptPath,
    memoryDir,
  })
}
```