# tidev MCP 配置说明

## 1. MCP 功能概述

tidev 内置 MCP（Model Context Protocol）支持，用于连接外部 MCP 服务器并将远程工具作为本地工具注册。

核心能力包括：

- 管理多个 MCP 服务器连接
- 支持 `stdio`、`http` 和 `sse` 三类传输方式
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
   - 通过 HTTP/HTTPS 直接连接远端 MCP 服务
3. `sse`
   - 通过 Server-Sent Events 连接 MCP 服务

## 4. 配置位置

tidev 的 MCP 配置写在主配置文件中，默认路径为：

- `~/.config/tidev/config.toml`

目前配置格式如下：

```toml
[mcp]

[mcp.servers.my_server]
kind = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]

[mcp.servers.remote]
kind = "http"
url = "https://example.com/mcp"

[mcp.servers.events]
kind = "sse"
url = "https://example.com/sse"
```

## 5. `stdio` 配置字段

```toml
[mcp.servers.local]
kind = "stdio"
command = "./my-mcp-server"
args = ["--serve"]
cwd = "."
env = { RUST_LOG = "info" }
```

字段说明：

- `command`：要执行的命令，可写绝对路径或相对路径
- `args`：命令参数列表，可选
- `cwd`：可选，工作目录；相对路径会以当前工作区根目录为基准
- `env`：可选，附加环境变量

## 6. `http` / `sse` 配置字段

```toml
[mcp.servers.remote]
kind = "http"
url = "https://example.com/mcp"

[mcp.servers.events]
kind = "sse"
url = "https://example.com/sse"
```

字段说明：

- `url`：MCP 服务器地址

## 7. 使用方式

tidev 启动后会读取配置并初始化 MCP 管理器，但 MCP 服务器通常需要手动连接或刷新。

当前交互方式：

- 在命令行中输入 `/mcp` 打开 MCP 面板
- 在 MCP 面板中：
  - `Enter`：连接 / 断开选中服务器
  - `r`：刷新选中服务器并重新加载工具列表
  - `a`：添加新服务器
  - `e`：编辑选中服务器
  - `d`：删除选中服务器
  - `Esc`：关闭 MCP 面板

## 8. MCP 工具如何工作

tidev 会将已连接 MCP 服务器中的工具转换为内部工具定义：

- 工具名称会被映射成 `server-name / tool-name` 的形式
- 描述、参数 schema 会被保留
- tidev 会根据 MCP 工具的 `read_only_hint` 或内置名称，自动映射权限类型
- MCP 工具与本地工具统一进入 `ToolRegistry`，可在会话中一起执行

## 9. 调试与常见问题

- 如果 MCP 服务器无法连接，请检查 `kind` 是否正确、URL 是否可达、命令是否存在
- `stdio` 服务器的 `cwd` 若是相对路径，会以工作区根目录解析
- 连接失败时，tidev 会显示 `failed` 状态并保留错误消息
- 通过 MCP 面板刷新服务器，可重新读取最新工具列表

## 10. 备注

- 当前 MCP 配置只会在 tidev 启动时从 `config.toml` 加载一次
- 面板中的添加、编辑、删除操作会修改运行时的 MCP 管理状态，但不会自动写回到配置文件
- MCP 工具的执行仍由 tidev 的工具调用系统负责，MCP 服务器仅提供工具描述与实际执行能力
