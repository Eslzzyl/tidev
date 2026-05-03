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

### 1. TUI 迁移到 AgentRuntime

TUI 目前内联实现了所有 `AgentRuntime` 提供的功能。建议逐步替换：

- **`src/app/mod.rs:compose_system_prompt()`** → 替换为 `AgentRuntime::compose_system_prompt()`
- **`src/app/mod.rs:start_assistant_turn()`** 中的 `context_manager.build_request_messages()` + `self.tools.all_definitions()` → 替换为 `AgentRuntime::build_request_messages()` + `tool_definitions()`
- **`src/app/mod.rs:finish_assistant_turn()` → `begin_tool_execution()` → `process_pending_tool_execution()`** 链 → 评估能否用 `AgentRuntime::run_agent_loop()` 替代，或至少复用 `execute_tool_calls()`

**注意**：TUI 有 UI 特有逻辑（permission dialog、file read tracking、snapshot、subagent delegation），这些不应放入 AgentRuntime。迁移时应保留 UI 专属部分，只替换纯逻辑层。

### 2. Gateway (Telegram / QQ) 迁移到 AgentRuntime

Telegram gateway (`src/gateway/telegram/channel.rs`) 已经自己实现了类似的循环模式：

- `run_agent_with_tools()` — 类似 `run_agent_loop()`
- `run_single_streaming_turn()` — 类似 `run_single_turn()`
- `execute_tool_calls()` — 类似 `execute_tool_calls()`

建议替换为 AgentRuntime 以消除重复。

### 3. 改进 AgentRuntime 本身

- **取消支持**：`run_agent_loop` 目前没有内置的取消/中止机制。可考虑通过 `CancellationToken` 或 `watch` channel 支持中断。
- **事件去重**：`run_single_turn` 已经将 `BackendEvent::Finished` 转发给 consumer，`run_agent_loop` 不再重复发送。确保各 consumer 正确处理。
- **Context compaction**：AgentRuntime 目前不做上下文压缩（`ContextManager::compact_if_needed`）。TUI 在 turn 完成后会调度 compaction，web 也有独立的 compaction 接口。未来可以内置到 `run_agent_loop`。
- **Subagent 任务**：`task` 工具创建子会话执行子任务。目前 AgentRuntime 的 `execute_tool_calls` 使用 `ToolRegistry::execute_call` 来执行，已经支持 subagent 通过 `builtin::task::execute_tool_call`。但 subagent 的事件流（`SubagentStatus`、`SubagentToolResult`、`SubagentCompleted`）尚未在 AgentRuntime 中特殊处理。

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
│  compose_system_prompt()   ← 共享             │
│  build_request_messages()  ← 共享             │
│  tool_definitions()        ← 共享             │
│  run_single_turn()         ← 共享             │
│  execute_tool_calls()      ← 共享             │
│  run_agent_loop()          ← 共享             │
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
   │ permission│ │ SSE events  │ │msg send│
   │ dialogs  │ │ HTTP abort  │ │/edit   │
   │ snapshot │ │ no blocking │ │no block│
   │ keyboard │ │             │ │        │
   └──────────┘ └─────────────┘ └────────┘
    需要迁移      已完成          待迁移
```

## 代码行数统计 (粗略)

| 文件 | 新增/修改 |
|---|---|
| `src/agent/runtime.rs` | ~440 行新代码 |
| `src/tooling/registry.rs` | +5 行 |
| `src/web/state.rs` | +6 行 |
| `src/web/mod.rs` | +45 行 |
| `src/web/routes/messages.rs` | ~-140 行净减少 (删除旧重复代码) |

AgentRuntime 的实现比被替换的 web 代码更精简，因为不再需要手动处理流式事件循环中低层细节。
