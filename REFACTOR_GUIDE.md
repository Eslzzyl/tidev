# Tidev 项目重构指南

## 当前状态

- 总计：27,354 行 Rust 代码
- 文件结构：单 `src/` 目录 + 多个子模块

## 问题分析

### 1. app.rs (2588 行) — 职责过重

**职责**：事件处理、会话管理、UI 状态、渲染缓存、后端通信、主题/模型面板、权限对话框、问题对话框、MCP 面板、命令面板、@提及、撤销、鼠标选择等。

**问题**：
- 单一结构体持有所有状态
- 80+ 个方法混在一起
- UI 逻辑与业务逻辑未分离

### 2. storage.rs (1288 行) — 数据库操作混杂

**职责**：
- SQLite 连接管理
- 会话 CRUD 操作
- 消息存储
- 上下文压缩
- 工具调用记录

**问题**：
- 单一文件包含所有数据库操作
- 未分离 schema 管理、数据访问、业务逻辑

### 3. tooling/tools.rs (1302 行) — 工具实现混杂

**职责**：
- 内置工具实现（grep, glob, exec, edit_file, web_fetch 等 20+ 个工具）
- 参数解析与验证
- 工具宏定义

**问题**：
- 所有工具实现堆在一个文件
- 难以维护和扩展

### 4. 其他大文件

| 文件 | 行数 | 职责 |
|------|------|------|
| render_dialog.rs | 1583 | 对话框渲染 |
| render_chat.rs | 1540 | 聊天消息渲染 |
| app.rs | 2588 | 主应用逻辑 |
| storage.rs | 1288 | 数据持久化 |
| tools.rs | 1302 | 内置工具实现 |

---

## 重构方案

### 方案一：按功能域拆分（推荐）

```
src/
├── main.rs                    # 入口点
├── lib.rs                     # 库入口
│
├── app/                       # 应用核心
│   ├── mod.rs                 # App 结构体定义
│   ├── run.rs                 # 运行循环
│   ├── state.rs               # 应用状态
│   ├── session.rs             # 会话管理
│   ├── event.rs               # 事件处理
│   ├── context.rs             # 上下文管理
│   ├── command.rs             # 命令系统
│   ├── clipboard.rs           # 剪贴板操作
│   └── dialog.rs              # 对话框管理
│
├── ui/                        # UI 组件
│   ├── mod.rs                 # UI 模块入口
│   ├── panels/
│   │   ├── mod.rs
│   │   ├── session_panel.rs
│   │   ├── theme_panel.rs
│   │   ├── model_panel.rs
│   │   ├── mcp_panel.rs
│   │   └── command_palette.rs
│   ├── render/
│   │   ├── mod.rs
│   │   ├── chat.rs
│   │   ├── dialog.rs
│   │   ├── diff.rs
│   │   ├── scroll.rs
│   │   └── cursor.rs          # 光标选择
│   └── widgets/
│       ├── mod.rs
│       ├── at_mention.rs
│       ├── permission.rs
│       ├── question.rs
│       ├── undo.rs
│       └── mouse_selection.rs
│
├── storage/                   # 数据持久化
│   ├── mod.rs
│   ├── connection.rs          # 数据库连接
│   ├── schema.rs              # Schema 定义
│   ├── session.rs             # 会话存储
│   ├── message.rs             # 消息存储
│   ├── compaction.rs          # 上下文压缩
│   └── tool_call.rs           # 工具调用记录
│
├── tooling/                   # 工具系统
│   ├── mod.rs
│   ├── registry.rs            # 工具注册
│   ├── schema.rs              # 工具 schema
│   ├── builtin/
│   │   ├── mod.rs
│   │   ├── file.rs            # 文件工具 (read/write/edit)
│   │   ├── search.rs          # 搜索工具 (grep/glob)
│   │   ├── exec.rs            # 执行工具 (exec/spawn)
│   │   ├── web.rs             # 网络工具 (fetch/search)
│   │   ├── info.rs            # 信息工具 (list/lsp/skill)
│   │   └── misc.rs            # 其他工具 (todo/memory)
│   └── permission.rs          # 权限管理
│
├── llm/                       # LLM 客户端
│   ├── mod.rs
│   ├── client.rs              # 通用客户端
│   ├── openai.rs
│   ├── anthropic.rs
│   ├── attachments.rs
│   ├── error.rs
│   └── think_parser.rs
│
├── mcp/                       # MCP 支持
│   ├── mod.rs
│   └── client.rs
│
├── config/                    # 配置管理
│   ├── mod.rs
│   ├── app.rs
│   ├── auth.rs
│   ├── logging.rs
│   ├── mcp.rs
│   ├── paths.rs
│   ├── provider.rs
│   └── ui.rs
│
├── provider_setup/            # Provider 配置
│   ├── mod.rs
│   ├── edit.rs
│   └── new.rs
│
├── snapshot/                  # 快照功能
│   ├── mod.rs
│   └── git.rs
│
├── theme/                     # 主题管理
│   └── mod.rs
│
├── markdown_render/           # Markdown 渲染
│   ├── mod.rs
│   ├── highlight.rs
│   ├── line.rs
│   ├── links.rs
│   ├── styles.rs
│   ├── table.rs
│   └── wrap.rs
│
├── commands.rs                # 命令定义
├── context.rs                 # 上下文管理
├── input.rs                   # 输入处理
├── instructions.rs            # 指令加载
├── prompts.rs                 # Prompt 管理
├── session.rs                 # 会话数据结构
├── skills.rs                  # 技能目录
├── webtools.rs                # Web 工具
└── logging.rs                 # 日志初始化
```

### 方案二：渐进式拆分（低风险）

**阶段一**：拆分现有模块到新目录（不改代码逻辑）
```
src/
├── app/                       # 现有 app.rs 内容
├── ui/                        # 现有 app/ 内容
├── storage/                   # 现有 storage.rs
├── tooling/                   # 现有 tooling/ 内容
├── llm/                       # 现有 llm/
├── mcp/                       # 现有 mcp.rs
├── config/                    # 现有 config/
├── provider_setup/            # 现有 provider_setup/
├── snapshot/                  # 现有 snapshot/
├── theme/                     # 现有 theme.rs
├── markdown_render/           # 现有 markdown_render/
├── commands.rs
├── context.rs
├── input.rs
├── instructions.rs
├── prompts.rs
├── session.rs
├── skills.rs
├── webtools.rs
└── logging.rs
```

**阶段二**：拆分大文件
1. `app.rs` → `app/` (state.rs, run.rs, event.rs, ...)
2. `storage.rs` → `storage/` (session.rs, message.rs, compaction.rs, ...)
3. `tools.rs` → `tooling/builtin/` (file.rs, search.rs, exec.rs, ...)
4. `render_dialog.rs` → `ui/render/dialog.rs`
5. `render_chat.rs` → `ui/render/chat.rs`

**阶段三**：提取共享组件
- 提取 `MessageRenderCache` 到独立模块
- 提取 `CachedSessionRuntime` 到独立模块
- 提取 `UiStateSnapshot` 到独立模块

---

## 拆分优先级

| 优先级 | 文件 | 建议 |
|--------|------|------|
| P0 | app.rs | 先拆分为 app/ 目录 |
| P0 | storage.rs | 拆分为 storage/ 目录 |
| P1 | tooling/tools.rs | 拆分为 tooling/builtin/ |
| P1 | render_dialog.rs | 移入 ui/render/ |
| P1 | render_chat.rs | 移入 ui/render/ |
| P2 | markdown_render/ | 整体移入 ui/render/ |
| P2 | config/ | 整体移入 config/ |

---

## 注意事项

1. **保持 API 兼容**：拆分后模块的公共接口应保持不变
2. **使用 `pub(crate)` 控制可见性**：避免不必要的公开
3. **提取共享类型**：将重复定义的结构体提取到独立文件
4. **更新 Cargo.toml**：添加 `mod` 声明
5. **更新所有 import 路径**：拆分后需要更新所有引用

---

## 验证步骤

```bash
# 1. 确保编译通过
cargo build

# 2. 确保测试通过
cargo test

# 3. 确保 clippy 无警告
cargo clippy -- -D warnings

# 4. 运行集成测试
cargo test --test integration
```