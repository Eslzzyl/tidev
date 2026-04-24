# OpenCode 模式与工具详解

本文档详细说明 OpenCode 的核心功能，包括 Plan/Build 模式设计、内置工具列表以及自定义模式的配置方法。

---

## 目录

- [模式概述](#模式概述)
- [Build 模式](#build-模式)
- [Plan 模式](#plan-模式)
- [模式切换](#模式切换)
- [内置工具列表](#内置工具列表)
- [自定义模式](#自定义模式)
- [配置示例](#配置示例)

---

## 模式概述

OpenCode 提供两种内置模式，用于适应不同的使用场景：

| 模式 | 说明 |
|------|------|
| **Build** | 默认模式，启用所有工具，适用于完整的开发工作 |
| **Plan** | 限制模式，仅用于分析和规划，不允许修改文件 |

模式通过 `agent` 配置项进行设置，原有的 `mode` 选项已弃用。

---

## Build 模式

Build 是 OpenCode 的**默认模式**，具有完整的工具访问权限。

### 特点

- 所有工具默认启用
- 支持完整的文件操作（读写、编辑）
- 支持执行 Shell 命令
- 适用于标准开发工作

### 配置

```json title="opencode.json"
{
  "$schema": "https://opencode.ai/config.json",
  "mode": {
    "build": {
      "model": "anthropic/claude-sonnet-4-20250514",
      "prompt": "{file:./prompts/build.txt}",
      "tools": {
        "write": true,
        "edit": true,
        "bash": true,
        "read": true,
        "grep": true,
        "glob": true,
        "list": true,
        "patch": true,
        "todowrite": true,
        "webfetch": true
      }
    }
  }
}
```

---

## Plan 模式

Plan 模式是一个**受限模式**，专为分析和规划设计，适用于仅需要查看代码、提出建议或制定计划而不实际修改代码的场景。

### 禁用工具

Plan 模式下默认禁用的工具：

| 工具 | 状态 | 说明 |
|------|------|------|
| `write` | ❌ 禁用 | 禁止创建新文件 |
| `edit` | ❌ 禁用 | 禁止修改文件（`.opencode/plans/*.md` 除外） |
| `patch` | ❌ 禁用 | 禁止应用补丁 |
| `bash` | ❌ 禁用 | 禁止执行 Shell 命令 |

### 允许工具

以下工具在 Plan 模式下仍然可用：

- `read` — 读取文件内容
- `grep` — 搜索文件内容
- `glob` — 按模式查找文件
- `list` — 列出目录内容

### 配置

```json title="opencode.json"
{
  "$schema": "https://opencode.ai/config.json",
  "mode": {
    "plan": {
      "model": "anthropic/claude-haiku-4-20250514",
      "tools": {
        "write": false,
        "edit": false,
        "bash": false,
        "patch": false,
        "read": true,
        "grep": true,
        "glob": true,
        "list": true
      }
    }
  }
}
```

### 适用场景

- 代码分析和评估
- 变更方案制定
- 代码审查
- 架构讨论

---

## 模式切换

在会话中切换模式：

1. 按 **Tab** 键
2. 或使用配置的 `switch_mode` 快捷键绑定

---

## 内置工具列表

OpenCode 提供以下内置工具：

### 文件操作工具

| 工具 | 功能 | 权限控制 |
|------|------|----------|
| **read** | 读取文件内容，支持指定行范围 | `read` |
| **write** | 创建新文件或覆盖现有文件 | `edit` |
| **edit** | 使用精确字符串替换修改文件 | `edit` |
| **list** | 列出目录内容，支持 glob 过滤 | `list` |
| **patch** | 应用补丁文件 | `edit` |

### 搜索工具

| 工具 | 功能 | 权限控制 |
|------|------|----------|
| **grep** | 使用正则表达式搜索文件内容 | `grep` |
| **glob** | 使用 glob 模式查找文件（如 `**/*.js`） | `glob` |

### 系统工具

| 工具 | 功能 | 权限控制 |
|------|------|----------|
| **bash** | 执行 Shell 命令 | `bash` |
| **webfetch** | 获取网页内容 | `webfetch` |
| **websearch** | 使用 Exa AI 执行网络搜索 | `websearch` |

### 其他工具

| 工具 | 功能 | 权限控制 |
|------|------|----------|
| **skill** | 加载 Skill 文件（SKILL.md）并返回内容 | `skill` |
| **todowrite** | 管理任务列表，跟踪复杂操作的进度 | `todowrite` |
| **question** | 在执行过程中向用户提问 | `question` |
| **lsp** | 与配置的 LSP 服务器交互（实验性） | `lsp` |

---

## 工具参数详解

### read

读取文件内容，支持行范围指定。

**源码：** `opencode/packages/opencode/src/tool/read.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 文件路径（绝对或相对路径） |
| `offset` | number | 否 | 起始行号（1-based），用于大文件分页 |
| `limit` | number | 否 | 最大读取行数，默认为 2000 |

**写入限制：**

- 文件不存在时不报错，返回空内容
- 大文件建议使用 `offset` 和 `limit` 进行分页读取
- 内部使用 `Filesystem.read()`，自动处理二进制文件检测

**提示词：**
```
Read a text file. Output format: LINE_NUM|CONTENT. Use offset and limit for large files.
```

---

### write

创建新文件或覆盖现有文件。

**源码：** `opencode/packages/opencode/src/tool/write.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 文件路径 |
| `content` | string | 是 | 文件内容 |

**写入限制：**

- 默认禁止覆盖现有文件（`overwrite` 默认为 `false`）
- 启用 `overwrite: true` 可强制覆盖
- 默认父目录必须存在（`create_parents` 默认为 `false`）
- 某些目录被黑名单保护，禁止写入

**提示词：**
```
Write content to a text file. Overwrites if the file already exists; creates parent directories as needed.
```

**黑名单目录（默认禁止写入）：**
- `.git/`
- `node_modules/`
- 项目的关键配置文件目录

---

### edit

使用精确字符串替换修改文件。

**源码：** `opencode/packages/opencode/src/tool/edit.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | 文件路径 |
| `old_text` | string | 是 | 要替换的原文本（必须精确匹配） |
| `new_text` | string | 是 | 替换后的文本 |
| `replace_all` | boolean | 否 | 是否替换所有匹配项（默认 false） |

**写入限制：**

- `old_text` 必须精确匹配文件中的内容
- 匹配多个位置时需要使用 `replace_all: true`
- Plan 模式下禁止使用（`.opencode/plans/*.md` 除外）
- 文件必须存在

**提示词：**
```
Edit a file by replacing old_text with new_text. Tolerates minor whitespace/indentation differences.
```

**使用示例：**
```json
{
  "path": "/path/to/file.ts",
  "old_text": "const oldValue = 1;",
  "new_text": "const newValue = 2;"
}
```

---

### bash

执行 Shell 命令。

**源码：** `opencode/packages/opencode/src/tool/bash.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `command` | string | 是 | 要执行的 Shell 命令 |
| `working_dir` | string | 否 | 工作目录，默认为项目根目录 |
| `timeout` | number | 否 | 超时时间（秒），默认 60 秒，最大 600 秒 |
| `interactive` | boolean | 否 | 是否启用交互模式（默认 false） |

**安全限制：**

- 默认禁用危险命令（`rm -rf`, `format`, `dd`, `shutdown` 等）
- 可配置 `restrictToWorkspace` 限制文件访问范围
- 输出默认截断为 10,000 字符
- 交互模式需要明确启用

**提示词：**
```
Execute a shell command and return its output. Prefer read_file/write_file over cat/echo/sed, and grep/glob over shell find/grep.
```

**危险命令黑名单：**
```typescript
const DANGEROUS_PATTERNS = [
  /rm\s+-rf/,
  /format/,
  /dd\s+/,
  /shutdown/,
  /mkfs/,
  /fdisk/,
  // ... 更多模式
]
```

---

### grep

使用正则表达式搜索文件内容。

**源码：** `opencode/packages/opencode/src/tool/grep.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | 正则表达式模式 |
| `path` | string | 否 | 搜索目录，默认项目根目录 |
| `include` | string | 否 | 文件模式过滤（如 `*.js`, `*.{ts,tsx}`） |

**输出限制：**

- 最多返回 100 个匹配结果
- 超过限制时提示使用更具体的路径或模式
- 单行文本最大长度限制为 2000 字符

**提示词：**
```
Search file contents with a regex pattern. Default output_mode is files_with_matches (file paths only); use content mode for matching lines with context.
```

**输出格式：**
```
Found N matches (showing first 100)
/path/to/file1.ts:
  Line 5: const lineText = ...
  Line 10: const lineText = ...

/path/to/file2.ts:
  Line 15: const lineText = ...
```

**底层实现：**
- 底层使用 ripgrep（rg）
- 默认遵守 `.gitignore` 规则
- 按文件修改时间排序（最新优先）

---

### glob

使用 glob 模式查找文件。

**源码：** `opencode/packages/opencode/src/tool/glob.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `pattern` | string | 是 | glob 模式（如 `**/*.js`） |
| `path` | string | 否 | 搜索目录，默认项目根目录 |

**输出限制：**

- 默认返回 100 个结果
- 超过限制时提示使用更具体的模式

**提示词：**
```
Find files matching a glob pattern (e.g., '*.py', 'tests/**/test_*.py'). Results are sorted by modification time (newest first).
```

**底层实现：**
- 使用 ripgrep 的文件列表功能
- 自动按修改时间排序

---

### list

列出目录内容（树形结构）。

**源码：** `opencode/packages/opencode/src/tool/ls.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 否 | 目录路径（绝对路径） |
| `ignore` | string[] | 否 | 额外的 glob 模式排除列表 |

**默认忽略目录：**
```
node_modules/, __pycache__/, .git/, dist/, build/, target/, vendor/, bin/, obj/, .idea/, .vscode/, .zig-cache/, zig-out, .coverage/, coverage/, tmp/, temp/, .cache/, cache/, logs/, .venv/, venv/, env/
```

**输出限制：**

- 默认返回 100 个文件

**提示词：**
```
List the contents of a directory. Set recursive=true to explore nested structure.
```

**输出格式：**
```
/path/to/dir/
  subdir/
    file1.ts
    file2.ts
  root.txt
```

---

### webfetch

获取网页内容。

**源码：** `opencode/packages/opencode/src/tool/webfetch.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `url` | string | 是 | 要获取的 URL（必须以 http:// 或 https:// 开头） |
| `format` | enum | 否 | 返回格式：`text`/`markdown`/`html`，默认 `markdown` |
| `timeout` | number | 否 | 超时时间（秒），最大 120 秒 |

**限制：**

- 最大响应大小：5MB
- 最大超时时间：120 秒
- 自动处理 Cloudflare 拦截（使用 `opencode` UA 重试）
- 图片返回 Base64 编码

**提示词：**
```
Fetch a URL and extract readable content (HTML → markdown/text). Output is capped at maxChars (default 50,000).
```

**format 选项说明：**
- `text`: 提取纯文本
- `markdown`: 转换为 Markdown 格式
- `html`: 返回原始 HTML

---

### websearch

使用 Exa AI 执行网络搜索。

**源码：** `opencode/packages/opencode/src/tool/websearch.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `query` | string | 是 | 搜索查询 |
| `numResults` | number | 否 | 返回结果数量，默认 8 |
| `livecrawl` | enum | 否 | 实时爬取模式：`fallback`/`preferred`，默认 `fallback` |
| `type` | enum | 否 | 搜索类型：`auto`/`fast`/`deep`，默认 `auto` |
| `contextMaxCharacters` | number | 否 | 上下文最大字符数，默认 10000 |

**限制：**

- 仅在使用 OpenCode Provider 或设置 `OPENCODE_ENABLE_EXA=1` 时可用
- 无需 API Key

**提示词：**
```
Search the web. Returns titles, URLs, and snippets. count defaults to 5 (max 10). Use webfetch to read a specific page in full.
```

---

### todowrite

管理任务列表。

**源码：** `opencode/packages/opencode/src/tool/todo.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `todos` | Todo[] | 是 | 任务列表 |

**Todo 对象结构：**
```typescript
interface Todo {
  id: string;
  content: string;
  status: "pending" | "in_progress" | "completed";
  priority?: "low" | "medium" | "high";
}
```

**限制：**

- 子代理默认禁用此工具
- 会话隔离，每个会话维护独立的 TODO 状态

**提示词：**
```
Manage a to-do list to track progress for complex, multi-step tasks (tasks like world changes, code refactoring, multiple files creation).
```

---

### lsp

与 LSP 服务器交互（实验性）。

**源码：** `opencode/packages/opencode/src/tool/lsp.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `operation` | enum | 是 | LSP 操作类型 |
| `filePath` | string | 是 | 文件路径（绝对或相对） |
| `line` | number | 是 | 行号（1-based） |
| `character` | number | 是 | 字符偏移（1-based） |

**operation 选项：**

| 操作 | 说明 |
|------|------|
| `goToDefinition` | 跳转到定义 |
| `findReferences` | 查找引用 |
| `hover` | 获取悬停信息 |
| `documentSymbol` | 获取文档符号 |
| `workspaceSymbol` | 获取工作区符号 |
| `goToImplementation` | 跳转到实现 |
| `prepareCallHierarchy` | 准备调用层次 |
| `incomingCalls` | 传入调用 |
| `outgoingCalls` | 传出调用 |

**限制：**

- 实验性功能，需要配置 LSP 服务器
- 文件必须存在

**提示词：**
```
Query Language Server Protocol (LSP) for code intelligence (definitions, references, symbols, etc.). Experimental feature.
```

---

### skill

加载 Skill 文件。

**源码：** `opencode/packages/opencode/src/tool/skill.ts`

**参数：**

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 是 | Skill 名称 |

**限制：**

- Skill 必须存在（SKILL.md 文件）
- 返回内容包含技能说明和关联文件列表

**提示词：**
```
Load a specialized skill that provides domain-specific instructions and workflows.
```

**返回内容格式：**
```
<skill_content name="xxx">
# Skill: xxx

[SKILL.md 内容]

Base directory for this skill: ...
<skill_files>
[文件列表，最多 10 个]
</skill_files>
</skill_content>
```

---

## 自定义模式

可以自定义内置模式或创建全新的模式。

### 配置方式

有两种配置方式：

#### JSON 配置

在 `opencode.json` 中定义模式：

```json title="opencode.json"
{
  "$schema": "https://opencode.ai/config.json",
  "mode": {
    "docs": {
      "prompt": "{file:./prompts/documentation.txt}",
      "tools": {
        "write": true,
        "edit": true,
        "bash": false,
        "read": true,
        "grep": true,
        "glob": true
      }
    }
  }
}
```

#### Markdown 配置

使用 Markdown 文件定义模式：

**文件位置：**
- 全局：`~/.config/opencode/modes/`
- 项目级：`.opencode/modes/`

```markdown title=".opencode/modes/debug.md"
---
temperature: 0.1
tools:
  bash: true
  read: true
  grep: true
  write: false
  edit: false
---

You are in debug mode. Your primary goal is to help investigate and diagnose issues.

Focus on:

- Understanding the problem through careful analysis
- Using bash commands to inspect system state
- Reading relevant files and logs
- Searching for patterns and anomalies
- Providing clear explanations of findings

Do not make any changes to files. Only investigate and report.
```

### 配置选项

#### model

覆盖模式的默认模型：

```json
{
  "mode": {
    "plan": {
      "model": "anthropic/claude-haiku-4-20250514"
    }
  }
}
```

#### temperature

控制响应的随机性和创造性：

| 范围 | 特点 | 适用场景 |
|------|------|----------|
| 0.0-0.2 | 高度聚焦和确定性 | 代码分析、规划 |
| 0.3-0.5 | 平衡响应 | 一般开发任务 |
| 0.6-1.0 | 更具创造性 | 头脑风暴、探索 |

#### prompt

指定自定义系统提示文件：

```json
{
  "mode": {
    "review": {
      "prompt": "{file:./prompts/code-review.txt}"
    }
  }
}
```

路径相对于配置文件位置。

#### tools

控制模式的可用工具：

```json
{
  "mode": {
    "readonly": {
      "tools": {
        "write": false,
        "edit": false,
        "bash": false,
        "read": true,
        "grep": true,
        "glob": true
      }
    }
  }
}
```

### 常见使用场景

| 模式 | 说明 | 工具配置 |
|------|------|----------|
| **Build** | 完整开发工作 | 所有工具启用 |
| **Plan** | 分析和规划 | 仅读取和搜索工具 |
| **Review** | 代码审查 | 只读 + 文档工具 |
| **Debug** | 调试和诊断 | bash 和读取工具 |
| **Docs** | 文档编写 | 文件操作，无 Shell |

---

## 配置示例

### 完整配置示例

```json title="opencode.json"
{
  "$schema": "https://opencode.ai/config.json",
  "mode": {
    "build": {
      "model": "anthropic/claude-sonnet-4-20250514",
      "temperature": 0.3,
      "tools": {
        "write": true,
        "edit": true,
        "bash": true,
        "read": true,
        "grep": true,
        "glob": true,
        "list": true,
        "patch": true,
        "todowrite": true,
        "webfetch": true
      }
    },
    "plan": {
      "model": "anthropic/claude-haiku-4-20250514",
      "temperature": 0.1,
      "tools": {
        "write": false,
        "edit": false,
        "bash": false,
        "patch": false,
        "read": true,
        "grep": true,
        "glob": true,
        "list": true
      }
    },
    "review": {
      "model": "anthropic/claude-sonnet-4-20250514",
      "temperature": 0.1,
      "tools": {
        "write": false,
        "edit": false,
        "bash": false,
        "read": true,
        "grep": true,
        "glob": true
      }
    }
  }
}
```

### 权限配置

通过 `permission` 字段控制工具行为：

```json title="opencode.json"
{
  "$schema": "https://opencode.ai/config.json",
  "permission": {
    "edit": "deny",
    "bash": "ask",
    "webfetch": "allow"
  }
}
```

权限选项：
- `allow` — 允许执行，无需确认
- `deny` — 禁止执行
- `ask` — 需要用户确认

支持通配符控制多个工具：

```json
{
  "permission": {
    "mymcp_*": "ask"
  }
}
```

---

## 相关文档

- [OpenCode 设计文档](./opencode-design.md)
- [UI 文档](./ui.md)
- [权限配置](/docs/permissions)
- [自定义工具](/docs/custom-tools)
- [MCP 服务器](/docs/mcp-servers)