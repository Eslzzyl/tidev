# tidev 目标态路线图（tidev-types 拆分之后）

**状态**: 待实施
**日期**: 2026-08-03
**前置条件**: `tidev-types-split.md`（tidev-types 拆分）已完成并验收通过
**执行对象**: Coding Agent。本文档是路线图：给出阶段划分、依赖顺序、关键设计点与验收标准；每阶段的具体执行细节在开工时按本文档展开。

## 1. 目标态概述

目标态 = 2026-08-03 讨论的全部重构完成后的状态。核心诉求：

- **tidev-llm**：叶子 crate（只依赖外部库），通用 LLM 客户端，其他项目可单独依赖做纯 LLM 调用。
- **tidev-agent**：完整 agent 运行时内核，**只依赖 tidev-llm**。其他 agent 产品**只依赖 tidev-agent** 即可开发（压缩、工具注册、循环、默认运行时开箱即用）。
- **tidev-core**：tidev 产品层（CoreContext、SessionManager、MCP、快照、指令、BackendEvent、prompts），是内核的一个宿主实现。
- 依赖图无循环；`tidev-agent` 除 llm 外不依赖任何 tidev crate。

目标态依赖图：

```
tidev-llm（叶子）           协议 + 提供方 + LlmEvent
tidev-utils（叶子）         canonical_tool_name 等纯函数
tidev-agent ──→ tidev-llm   完整内核：循环 / AgentContext / AgentEvent / Tool trait
                            / ToolRegistry / ContextManager / MessageBuffer / AgentRuntime
tidev-tools ──→ llm, utils, config, instructions   工具类型 + ShellOutput 本地 + builtin
tidev-core ──→ agent, tools, llm, config, storage, snapshot, instructions, logging, search, utils
                            产品层：BackendEvent / CoreContext / Runtime / SessionManager / MCP / prompts
tidev-storage ──→ llm, tools
tidev-tui ──→ core, llm, tools, config, utils, search
tidev-acp ──→ core, llm, config, utils
```

## 2. 前置状态（拆分完成后的起点）

拆分完成后已落位（来自 tidev-types-split.md，本路线图不再涉及）：

- 协议类型（message/reasoning/SessionMode/精简 ToolDefinition/ApiType 合并）在 tidev-llm；**BackendEvent 临时驻留 tidev-llm**（带 `TODO(event-split)` 注释）。
- 全量工具类型在 tidev-tools；canonical_tool_name 在 tidev-utils；死代码已删除。
- agent_type（数据 + 工厂 + agent 系统提示词）在 tidev-core；task 工具已解耦；fixer Plan 检查在 core 的 execute_task_tool。
- `AgentLoopConfig.system_prompt: String` 已替换 definition；trait 的 `tools()` 已用 `tidev_llm::ToolDefinition`。
- tidev-agent 依赖仅 tidev-llm；prompts.rs 只剩 mode reminders。
- 遗留待办：BackendEvent 的 TODO 注释（本路线图 P1 处理）。

## 3. 阶段总览

| 阶段 | 内容 | 前置 | 估算 |
|---|---|---|---|
| P1 事件三层拆分 | LlmEvent（llm）/ AgentEvent（agent）/ BackendEvent（core）+ ShellOutput 本地化 | 拆分完成 | 3–4 天 |
| P2 循环与 trait 去 tidev 化 | trait 瘦身、注入迁移、TuiRequest 移出、SessionMode→Mode、mode reminders 归位 | P1 | 3–4 天 |
| P3 内核组件迁入 | MessageBuffer / ContextManager / Tool trait / ToolRegistry 进 tidev-agent | P1、P2 | 4–5 天 |
| P4 默认运行时与收口 | AgentRuntime + MessageStore/ApprovalHandler、CoreContext 收口、子代理机制统一、消费方示例 | P3 | 4–6 天 |
| P5 清理与验证 | TODO 清除、文档同步、最终依赖图验证 | 全部 | 0.5–1 天 |

**合计约 14–20 个工作日（3–4 周）**。阶段边界是自然的中途止损点：**每个阶段结束时工作区必须 `cargo check` + `cargo test --workspace` 全绿**（与拆分的一次性策略不同，本路线图按阶段增量交付）。

## 4. 阶段细则

### P1 事件三层拆分

**目标**：事件按层归属——llm 只发 LlmEvent、内核用 AgentEvent、tidev UI 用 BackendEvent（回到 core）；删除 BackendEvent 的 TODO 注释。

**LlmEvent**（tidev-llm 定义，即当前 llm 实际产生的全部变体）：`Delta`、`ReasoningDelta`、`ToolCallUpdated`、`UsageStats`、`Finished(AssistantTurn)`、`Failed`、`Retrying`。无 session_id/request_id（宿主概念不进入协议层）。

**AgentEvent**（tidev-agent 定义，内核事件）：LlmEvent 的 7 个变体 + `TurnStarting`、`StreamEnd`、`ToolStarting`、`ToolCompleted`、`ContextCompacted`、`ShellOutput`。携带 `request_id`（循环的轮次号，供前端丢弃过期事件）；无 session_id。

**BackendEvent**（tidev-core）：保留全部现有变体与 session_id（tui/acp 的渲染与翻译逻辑不变）；仅 core 发射。

**转换层**（两处，各一个函数）：
- `LlmEvent → AgentEvent`：在 tidev-agent（补 request_id），供 AgentRuntime 与 CoreContext 复用。
- `AgentEvent → BackendEvent`：在 tidev-core（补 session_id）。落点即现有 `agent_ctx.rs:500` 的事件匹配循环（当前在 stream_turn 内匹配 BackendEvent 并转发 TUI——改为匹配 AgentEvent 后转 BackendEvent）。映射必须覆盖全部变体，**漏一个变体 TUI 渲染会静默出错**。

**改动点**：
- tidev-llm：41 处 `BackendEvent::` 构造点（anthropic 9 / gemini 10 / lib 3 / openai 9 / responses 10）改为 `LlmEvent::`；`stream_chat` 签名去掉 session_id/request_id 参数（事件通道类型改 `UnboundedSender<LlmEvent>`）。
- tidev-agent：`AgentEvent` 定义 + 转换函数；`AgentLoopConfig.event_tx` 与 `loop_.rs` 的事件发射改 `UnboundedSender<AgentEvent>`。
- tidev-tools：**ShellOutput 本地化**——`ToolContext.event_tx`（builtin/mod.rs:35）改 `Option<UnboundedSender<ShellOutput>>`，ShellOutput 为 tidev-tools 本地结构体（session_id/request_id/content/finished 等数据字段）；exec.rs 的 5 处事件通道改型。tidev-tools 不再引用 BackendEvent。
- tidev-core：BackendEvent 从 llm 迁入 core（新模块或 message 模块内）；ShellOutput 桥接（core 订阅 tools 的 ShellOutput 通道 → `BackendEvent::ShellOutput`）；stream_turn 的匹配循环改造。

**验收**：llm/agent/tools 零 `BackendEvent` 引用；TUI 渲染行为与拆分前一致（BackendEvent 变体全集未变）；`grep -rn "TODO(event-split)"` 无残留。

### P2 循环与 trait 去 tidev 化

**目标**：`AgentContext` trait 只剩通用契约（对齐 D-005 设计）；循环不知道任何 tidev 行为；tidev-agent 零 tidev 内容。

**trait 最终面**（8 个方法）：`tools()`、`event_tx()`、`workspace_root()`、`stream_turn()`、`request_tool_approval()`、`execute_tools()`、`save_messages()`、`load_messages()`。删除 `inject_instructions`、`append_instruction_sources`、`update_message_content`。

**注入迁移**（行为保持，字节级不变性铁律）：
- 指令注入 + mode reminder 注入从 `loop_.rs` 移入 `CoreContext::load_messages` 内部。顺序不变：加载后立即注入、先于 `TurnStarting` 事件；去重逻辑（content.starts_with、already_injected 集合）原样保留。
- 指令源持久化（现循环在工具执行后调 `append_instruction_sources`，loop_.rs:216）移入 `CoreContext::execute_tools` 内部（它已持有结果）。
- `update_message_content` 的调用（现用于注入持久化，loop_.rs:341）随注入消失；消息编辑走 core 的 SessionManager API（session.rs:135 已有）。

**其余内容**：
- `TuiRequest` / `TuiResponse` / `ToolCallWithViolations` → tidev-core（tidev 的审批 UI 媒介；tui 经 core re-export 取用，路径兼容）。`ApprovedTool` 留 tidev-agent（trait 返回类型）。
- `SessionMode` → `Mode` 改名（tidev_llm::mode；全仓机械替换，P2 一次改完）。
- mode reminders（mode_reminder/plan_mode_reminder/build_mode_reminder/plan_switch_reminder/build_switch_reminder）→ tidev-core 的 prompts 模块。tidev-agent 的 prompts.rs 删除，**tidev-agent 零 prompts、零 tidev 内容**。
- `AgentLoopConfig` 最终面：`session_id`、`system_prompt: String`、`mode: Mode`、`thinking_level`、`event_tx: UnboundedSender<AgentEvent>`、`cancel`、`queued_messages`。

**验收**：trait 方法数与 D-005 一致；loop_.rs 无 tidev 概念；同会话同消息列表下发给 LLM 的字节与拆分前一致（铁律回归测试）。

### P3 内核组件迁入

**目标**：压缩与工具注册机制归 tidev-agent。

- **MessageBuffer**（tidev-core/src/message_buf.rs，72 行）→ tidev-agent：纯移动，无逻辑改动。
- **ContextManager**（tidev-core/src/context.rs，718 行）→ tidev-agent：
  - 依赖改为 LlmClient + MessageBuffer + 协议类型（均可达：agent → llm）。
  - `BackendEvent::ContextCompacted` → `AgentEvent::ContextCompacted`（P1 前置）。
  - 全量 ToolDefinition → 精简版（`to_llm_tool_def` 转换随迁或保留在 core）。
  - **`build_request_messages` 与压缩逻辑逐字节不变**（铁律；现有测试兜底）。
- **Tool trait**（tidev-agent 定义）：`definition() -> ToolDefinition`、`read_only() -> bool`、`async execute(args, ctx: &dyn ToolContext) -> Result<ToolExecutionResult>`；`ToolContext`：`workspace_root()` + `event_tx() -> UnboundedSender<AgentEvent>`。
- **ToolRegistry**（tidev-agent 定义，通用）：注册/按名派发/输出大小限制/执行事件。**权限与审批留在宿主**（`request_tool_approval`，与 D-005 一致）——内核 registry 不做权限判定。
- **CoreContext 适配（部分）**：改用内核 MessageBuffer/ContextManager；tidev 工具适配器——每个 builtin 包装为内核 `Tool`（execute 内部构建 tidev 的 ToolContext 调 `tidev_tools::execute_tool_call`，MCP 工具同理），适配器持有 tidev 上下文（skills/auth/web search/todo），**tidev-tools 不依赖内核**（依赖方向保持 tools → llm/utils/config/instructions）。

**验收**：压缩输出字节不变（铁律）；tidev-core 不再定义 MessageBuffer/ContextManager；`cargo test --workspace` 全绿。

### P4 默认运行时与收口

**目标**：tidev-agent 开箱即用（AgentRuntime）；tidev 的 CoreContext 成为内核的一个宿主实现；子代理机制统一；消费方验证。

- **AgentRuntime**（tidev-agent，实现 AgentContext）：接线 LlmClient + LlmProviderConfig + ToolRegistry + ContextManager + MessageBuffer + event_tx + cancel；通用 `execute_tools` 实现（只读并行 / 写串行 + 取消）。
  - 对外抽象：`MessageStore` trait（load_messages/save_messages，宿主实现持久化）、`ApprovalHandler` trait（request_approval，宿主实现审批 UI）。
  - **子代理**：设计决策点——内核提供可插拔 `SubagentHost` trait（AgentRuntime 的 execute_tools 检测 task 类工具时经其派发），或 v1 不支持子代理（宿主自行在 execute_tools 处理）。tidev 需要子代理，倾向提供 SubagentHost；若采用，`AgentEvent` 增加 `SubagentStatus`/`SubagentCompleted` 变体。
- **CoreContext 收口**：`request_tool_approval` 用 Mode + TuiRequest（已随迁 core）；`execute_tools` 保持 tidev 行为（子代理/敏感文件/边界/取消守卫/undo），底层基于内核 ToolRegistry；`stream_turn` 复用 P1 的 LlmEvent→AgentEvent 转换。
- **task 工具机制统一**：tidev-tools 的 task.rs 校验桩与 tidev-core 的 execute_task_tool（子代理派发）经 SubagentHost 统一；agent_type 已在 core，本次是机制整合。
- **消费方示例**（验收核心）：新增示例（如 `crates/tidev-agent/examples/minimal_agent.rs` 或独立 example 目录）——只依赖 tidev-agent + tidev-llm，实现 MessageStore + ApprovalHandler + 注册两个工具，跑通 `run_agent_loop`。这是"另一个 agent 产品只依赖 tidev-agent"的证明。

**验收**：示例 crate 的 Cargo.toml 中 tidev 依赖仅有 tidev-agent（+tidev-llm 传递）；tidev 自身功能回归全绿。

### P5 清理与验证

- 删除全部遗留 TODO（event-split 等）；`grep` 确认无临时注释残留。
- 同步 rewrite-plan/architecture.md 与 D-005 等设计文档至目标态。
- 最终 `cargo tree` 验证 §1 依赖图（叶子、无循环、agent 仅依赖 llm）。
- 可选（用户决策）：tidev-llm / tidev-agent / tidev-protocol 相关类型的 crates.io 发布准备（版本、文档、license）。

## 5. 执行顺序依据

1. **P1 最先**：BackendEvent 的 TODO 在 llm 中悬置；AgentEvent 是 P3（ContextManager 发 ContextCompacted）的前置；事件类型是所有后续阶段的地基。
2. **P2 在 P3 之前**：trait 定稿后，内核组件（ToolRegistry/AgentRuntime）按最终契约编写，避免返工；注入迁移独立于组件迁移。
3. **P3 在 P4 之前**：AgentRuntime 需要 MessageBuffer/ContextManager/ToolRegistry 就位。
4. **P4 最后**：集成收口；CoreContext（1610 行）在 P3、P4 各触碰一次（组件替换、行为收口），分两次降低单次风险。

## 6. 总验收标准

- 依赖图与 §1 完全一致；`tidev-agent` 的 tidev 内部依赖仅 tidev-llm。
- 消费方示例只依赖 tidev-agent 跑通完整循环。
- `cargo check --workspace`、`cargo test --workspace` 全绿（现有 857 单测 + 集成测试 + 各阶段新增测试）。
- 压缩/请求构造的字节级不变性铁律保持（P2、P3 各做一次回归验证）。
- 除已确认的行为变化（fixer Plan 检查，拆分时已落地）外，tidev 产品行为不变。

## 7. 风险与注意事项

- **压缩字节不变性是铁律**：ContextManager 迁移（P3）与注入迁移（P2）是最容易破坏请求字节的两处；迁移前后用同一会话数据对比下发消息字节。
- **事件映射覆盖**：AgentEvent→BackendEvent（P1）漏变体不会编译报错，只会让 TUI 渲染静默出错——逐变体核对，并用现有 TUI 集成测试覆盖。
- **CoreContext 是最大改动面**：P3/P4 两次触碰；每次改动保持小步提交。
- **子代理抽象（P4）是最大设计点**：tidev 的子代理含 session 管理（SQLite）与审批，通用 SubagentHost trait 的边界需谨慎；若设计成本过高，v1 可允许宿主自实现（AgentRuntime 不内建子代理），在文档中明确记录该决策。
- **AgentRuntime 与 CoreContext 的行为分叉**：tidev 产品行为（指令注入、undo、敏感文件、MCP、快照）留在 CoreContext，不要试图全部塞进通用内核——内核提供机制，tidev 提供策略。
- **新增共享类型的归属纪律**：拆分后 llm 与 utils 为叶子；任何新共享类型必须先定归属，防止"types crate"死灰复燃。
