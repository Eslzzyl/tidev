# AgentRuntime 迁移计划

## 目标

将 TUI、Web、Gateway 三者共用的 LLM ↔ 工具执行循环逻辑提取到共享的 `AgentRuntime`（`src/agent/runtime.rs`），消除三处重复实现，确保行为一致。

---

## 已完成的工作

### 1. 创建共享 AgentRuntime

**`src/agent/runtime.rs`** — 新增文件，包含：

| 方法 | 功能 |
|---|---|
| `compose_system_prompt()` | 合并 base prompt + 指令文件 (AGENTS.md/CLAUDE.md/CONTEXT.md) + 模式提醒 (Plan/Build) + 环境信息 (OS/git/日期) + 工作区记忆 |
| `build_request_messages()` | 从 DB 消息构建预处理后的 LLM 请求消息（回退点过滤、孤立 tool call/result 处理、模式切换注入） |
| `tool_definitions()` | 返回 `ToolRegistry::all_definitions()`（15+ 内置工具 + MCP 工具 + 模型适配过滤） |
| `run_single_turn()` | 流式调用 LLM，实时转发 `BackendEvent`，返回最终的 `AssistantTurn` |
| `execute_tool_calls()` | 执行一组 tool call，持久化结果到 DB，发送 `ToolCompleted` 事件 |
| `persist_assistant_message()` | 持久化 assistant 消息到 DB |
| `run_agent_loop()` | 完整 agent 循环：`load → compose → stream → (execute + loop)` |

### 2. Web 后端接入 AgentRuntime

**不再重复的代码：**

| 之前的问题 | 现在的方案 |
|---|---|
| `tools = vec![]` — LLM 看不到工具 | `ToolRegistry::all_definitions()` — 全部内置 + MCP 工具 |
| 仅使用 `model_config.system_prompt` | `compose_system_prompt()` — 含指令文件、环境信息、记忆 |
| 手动在 `BackendEvent::Finished` 时持久化 assistant | AgentRuntime 内部自动持久化 |
| 无工具执行 (LLM 返回 tool call 后无反应) | `run_agent_loop` 自动执行工具并回环直到完成 |
| 不使用 `ContextManager::build_request_messages()` | AgentRuntime 调用 `build_request_messages()` 做消息预处理 |

**修改的文件：**

| 文件 | 变更 |
|---|---|
| `src/agent/mod.rs` | 添加 `pub mod runtime;` |
| `src/agent/runtime.rs` | 新文件 |
| `src/tooling/registry.rs` | 添加 `pub fn memory_store()` |
| `src/web/state.rs` | `AppState` 新增 `agent: AgentRuntime` |
| `src/web/mod.rs` | 初始化 AgentRuntime（ToolRegistry + MemoryStore + McpManager） |
| `src/web/routes/messages.rs` | `send_message` 改用 `agent.run_agent_loop()` |

---

## 还需要做的工作

### 1. TUI 迁移到 AgentRuntime ✅

**已完成：**
- **`src/app/mod.rs:compose_system_prompt()`** → 已替换为 `AgentRuntime::compose_system_prompt()`（第 497-501 行）

**仍然保留的 TUI 特有逻辑（不应放入 AgentRuntime）：**
- Permission dialog (`permission.rs`)
- File read tracking (`FileReadTracker`)
- Workspace snapshot
- Subagent delegation
- Message render cache
- Input event handling

**下一步可能：**
- 评估能否用 `AgentRuntime::run_single_turn()` 替代 `start_assistant_turn()` 中的手动 `llm.stream_chat()` 调用
- 评估能否用 `AgentRuntime::execute_tool_calls()` 替代 `permission.rs` 中的工具执行逻辑

### 2. Gateway (Telegram / QQ) 迁移到 AgentRuntime ✅

**Telegram gateway (`src/gateway/telegram/channel.rs`) 已完成：**
- 新增 `agent: AgentRuntime` 字段，在 `TelegramChannel::new()` 中初始化
- `run_single_streaming_turn()` → `compose_system_prompt()` 替换为 `agent.compose_system_prompt()`
- `run_single_streaming_turn()` → `build_request_messages()` 替换为 `agent.build_request_messages()`
- `tool_definitions()` 替换为 `agent.tool_definitions()`
- 移除对 `shared::compose_system_prompt` 的依赖

**QQ gateway (`src/gateway/qq.rs`) 已完成：**
- 相同的改动（新增 agent 字段、替换 compose/build/tool_definitions）

**`src/gateway/shared.rs`**：
- 移除了不再使用的 `compose_system_prompt()` 函数
- 保留 `compose_instruction_prompt()`（仍被 `gateway/mod.rs` 使用）

**仍然保留的 gateway 特有逻辑：**
- Draft editing（Telegram 的消息编辑）
- 工具结果发送给用户
- 取消支持（`check_cancellation`）/stop 命令
- 对话管理（`load_or_create_chat_conversation`）
- 模型选择交互

### 3. 改进 AgentRuntime 本身 ✅

- **取消支持** ✅：`run_agent_loop` 新增可选参数 `cancel_token: Option<CancellationToken>`。在每次 loop 迭代开始时和 tool execution 前检查，若已取消则提前返回 `Ok(())`。
- **Context compaction**：暂未内置到 `run_agent_loop`。TUI 和 Web 各自有独立的 compaction 调度，未来可考虑统一。
- **Subagent 任务**：AgentRuntime 的 `execute_tool_calls` 使用 `ToolRegistry::execute_call`，已间接支持 subagent。subagent 的事件流（`SubagentStatus`、`SubagentToolResult`、`SubagentCompleted`）通过 `ToolRegistry` 内部处理，AgentRuntime 不需要特殊干预。

### 4. 测试

- 为 `AgentRuntime` 添加单元测试覆盖 `build_request_messages`、`compose_system_prompt`
- 集成测试验证 `run_agent_loop` 在有/无 tool call 场景下的行为

---

## 架构关系图

```
┌──────────────────────────────────────────────┐
│              AgentRuntime                     │
│  (src/agent/runtime.rs)                      │
│                                               │
│  compose_system_prompt()   ← TUI/Web/Gateway │
│  build_request_messages()  ← TUI/Web/Gateway │
│  tool_definitions()        ← TUI/Web/Gateway │
│  run_single_turn()         ← Web             │
│  execute_tool_calls()     ← Web             │
│  run_agent_loop()          ← Web             │
│  + CancellationToken support                 │
│                                               │
│  依赖注入:                                    │
│  - LlmClient                                  │
│  - ToolRegistry (含 McpManager + SkillCatalog)│
│  - ContextManager                             │
│  - SessionStore                               │
│  - MemoryStore                                │
└──────────────────────┬───────────────────────┘
                       │
         ┌─────────────┼─────────────┐
         │             │             │
   ┌─────▼────┐ ┌──────▼──────┐ ┌───▼────┐
   │   TUI    │ │    Web      │ │Gateway │
   │          │ │             │ │(Tg,QQ) │
   │ compose  │ │ run_agent   │ │ compose│
   │ via agent│ │ _loop()     │ │ via    │
   │          │ │ SSE events  │ │ agent  │
   │ permission│ │ HTTP abort  │ │msg send│
   │ dialogs  │ │             │ │/edit   │
   │ snapshot │ │             │ │        │
   └──────────┘ └─────────────┘ └────────┘
    compose ✅   run_loop ✅    compose ✅
    build ✅                    build ✅


## 代码行数统计 (粗略)

| 文件 | 新增/修改 |
|---|---|
| `src/agent/runtime.rs` | ~470 行新代码（含 CancellationToken 支持） |
| `src/tooling/registry.rs` | +5 行 |
| `src/web/state.rs` | +6 行 |
| `src/web/mod.rs` | +45 行 |
| `src/web/routes/messages.rs` | ~-140 行净减少 (删除旧重复代码) |
| `src/gateway/telegram/channel.rs` | +40 行（新增 agent 字段 + 替换调用） |
| `src/gateway/qq.rs` | +40 行（新增 agent 字段 + 替换调用） |
| `src/gateway/shared.rs` | -18 行（删除 compose_system_prompt） |
| `src/app/mod.rs` | -10 行（compose_system_prompt 简化为委托调用） |
| `src/app/runtime/run.rs` | +12 行（App::new_with_paths 中初始化 AgentRuntime） |

AgentRuntime 的实现比被替换的 web 代码更精简，因为不再需要手动处理流式事件循环中低层细节。
