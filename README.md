# TiDev

一个终端 AI 编码助手，使用 Rust 和 ratatui 构建。

## 特性

- **TUI 界面** - 基于 ratatui 的全终端用户界面，支持深色/浅色主题切换
- **多提供商支持** - 支持 OpenAI、Anthropic 等多种 LLM API，通过 TOML 配置管理
- **会话管理** - SQLite 持久化会话，支持会话历史、回溯和恢复
- **MCP 集成** - 内置 Model Context Protocol 支持，可连接外部 MCP 服务器
- **工具系统** - 统一的工具注册和执行机制，支持 read/search/execute 权限控制
- **上下文管理** - 自动管理对话上下文，支持指令文件处理和工作区快照
- **命令行面板** - 内置命令面板（`/` 触发），支持快速操作

## 安装

```bash
cargo install --path .
```

## 配置

配置文件位于 `~/.config/tidev/config.toml`：

```toml
[providers.openai]
display_name = "OpenAI"
base_url = "https://api.openai.com/v1"

[providers.openai.models.gpt-4]
display_name = "GPT-4"
context_window = 128000
max_output_tokens = 4096
temperature = 0.7
```

API 密钥存储在 `~/.local/share/tidev/auth.json`：

```json
{
  "providers": {
    "openai": {
      "api_key": "your-api-key"
    }
  }
}
```

### MCP 服务器配置

```toml
[mcp.servers.my_server]
kind = "stdio"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem", "."]
```

## 快捷键

| 快捷键 | 功能 |
|--------|------|
| `/` | 打开命令面板 |
| `Ctrl+C` | 中止当前请求 |
| `Ctrl+L` | 切换主题 |
| `Ctrl+S` | 打开会话面板 |
| `Ctrl+M` | 打开模型面板 |
| `Esc` | 关闭面板/取消操作 |

## 构建

```bash
# 开发构建
cargo build

# 发布构建
cargo build --release

# 运行测试
cargo test
```
