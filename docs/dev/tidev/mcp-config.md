# tidev MCP 配置说明

## 1. MCP 功能概述

tidev 内置 MCP（Model Context Protocol）支持，用于连接外部 MCP 服务器并将远程工具作为本地工具注册。

核心能力包括：

- 管理多个 MCP 服务器连接
- 支持 `stdio`、streamable HTTP 和 legacy SSE
- 自动从 MCP 服务器拉取工具列表并将它们暴露给 tidev 的工具执行系统
- 通过现有工具权限机制，对 MCP 工具做 `read` / `search` / `execute` 的权限映射
- 在终端 UI 中展示 MCP 服务器状态，并支持连接、断开、刷新、添加、编辑、删除

## 2. MCP 在 tidev 中的实现位置

主要实现代码所在：

- `src/config/mcp.rs`：MCP 配置结构定义
- `src/mcp.rs`：MCP 服务器连接、工具获取、调用执行以及状态管理
- `src/tooling/registry.rs`：将 MCP 工具与本地工具合并到统一工具池
- `src/app/mcp_panel.rs`：MCP 面板交互与命令支持
- `src/app.rs`：应用启动时从配置加载 MCP 服务器，并初始化 MCP 管理器

## 3. 支持的 MCP 服务器类型

tidev 当前支持以下 MCP 服务器传输方式：

1. `stdio`
   - 启动一个子进程作为 MCP 客户端/服务器
   - 常用于本地 MCP 实现或基于 Node 的 MCP 服务器
2. `http`
   - 通过 HTTP/HTTPS 连接 streamable HTTP MCP 服务，响应可使用 JSON 或 SSE
3. `sse`
   - 通过初始 GET SSE 长连接获取 endpoint，再向 endpoint POST JSON-RPC 消息
   - 服务端通过初始 SSE 长连接推送 JSON-RPC 响应

## 4. 配置位置与格式

tidev 的 MCP 配置采用业界通用的标准 JSON 文件格式（与 Claude Desktop / Cursor / VS Code 一致）。

支持双层配置发现与合并：

- **全局配置**：`~/.config/tidev/mcp.json`
- **工作区配置（可选）**：`<workspace_root>/.tidev/mcp.json`

标准 JSON 配置示例：

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
      "cwd": ".",
      "env": {
        "RUST_LOG": "info"
      }
    },
    "remote": {
      "url": "https://example.com/mcp",
      "headers": {
        "Authorization": "Bearer your_token"
      }
    },
    "events": {
      "type": "sse",
      "url": "https://example.com/sse",
      "headers": {
        "X-Token": "secret"
      }
    }
  }
}
```

## 5. `stdio` 配置字段

```json
{
  "command": "./my-mcp-server",
  "args": ["--serve"],
  "cwd": ".",
  "env": {
    "RUST_LOG": "info"
  }
}
```

字段说明：

- `command`：要执行的命令，可写绝对路径或相对路径
- `args`：命令参数列表，可选
- `cwd`：可选，工作目录；相对路径会以当前工作区根目录为基准
- `env`：可选，附加环境变量键值表

## 6. `http` / `sse` 配置字段

```json
{
  "url": "https://example.com/mcp",
  "headers": {
    "Authorization": "Bearer token"
  }
}
```

或显式指定 SSE：

```json
{
  "type": "sse",
  "url": "https://example.com/sse"
}
```

字段说明：

- `url`：MCP 服务器地址（HTTP POST endpoint 或 SSE stream endpoint）
- `type` / `kind`：传输类型，可选（缺省时根据含有 `command` 或 `url` 自动推断为 `stdio` 或 `http`）
- `headers`：可选，HTTP 请求头键值对

## 7. 使用方式

tidev 启动时会自动读取全局及工作区 MCP 配置并初始化 MCP 管理器。

当前交互方式：

- 在 TUI 命令行中输入 `/mcp` 打开 MCP 面板（快捷键：`Ctrl+P` 选择 `MCP Servers`）
- 在 MCP 面板中：
  - `Enter`：连接 / 断开选中服务器
  - `r`：刷新选中服务器并重新加载工具列表
  - `n`：添加新服务器（自动持久化写入 `mcp.json`）
  - `e`：编辑选中服务器（自动持久化写入 `mcp.json`）
  - `d`：删除选中服务器（自动持久化写入 `mcp.json`）
  - `/` 或 `s`：过滤搜索服务器
  - `Esc` 或 `q`：关闭 MCP 面板

## 8. MCP 工具如何工作

tidev 会将已连接 MCP 服务器中的工具转换为内部工具定义：

- 工具名称会被映射成 `server-name / tool-name` 的形式
- 描述、参数 schema 会被保留
- tidev 会根据 MCP 工具的 `read_only_hint` 或内置名称，自动映射权限类型
- MCP 工具与本地工具统一进入 `ToolRegistry`，可在会话中一起执行

## 9. 调试与常见问题

- 如果 MCP 服务器无法连接，请检查命令是否存在、URL 是否可达、依赖是否安装
- `stdio` 服务器的 `cwd` 若是相对路径，会以工作区根目录解析
- 连接失败时，tidev 会显示 `failed` 状态并保留错误消息
- 通过 MCP 面板刷新服务器，可重新读取最新工具列表

## 10. 备注

- 全局与工作区配置自动合并，同名服务器工作区配置优先覆盖全局配置。
- 面板中的添加、编辑、删除操作会同步持久化至 `~/.config/tidev/mcp.json`。
- MCP 工具的执行仍由 tidev 的工具调用系统负责，MCP 服务器仅提供工具描述与实际执行能力。
