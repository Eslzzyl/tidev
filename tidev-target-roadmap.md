# tidev 目标态路线图（tidev-types 拆分之后）

**状态**: 待实施
**日期**: 2026-08-03
**修订**: 2026-08-03（第二轮讨论后）——澄清铁律权威定义（§1）；新增 P1.5「llm 协议净化」；审批完全移出内核（trait 7 方法，P2）；MCP 客户端入 tidev-agent（P3）；child_session_id 迁移 v40（P1.5）
**前置条件**: `tidev-types-split.md`（tidev-types 拆分）已完成并验收通过
**执行对象**: Coding Agent。本文档是路线图：给出阶段划分、依赖顺序、关键设计点与验收标准；每阶段的具体执行细节在开工时按本文档展开。

## 1. 目标态概述

目标态 = 2026-08-03 讨论的全部重构完成后的状态。核心诉求：

- **tidev-llm**：叶子 crate（只依赖外部库），**绝对干净的 LLM API 聚合层**——只含协议类型（Message/ToolCall/ToolDefinition/AssistantTurn/LlmEvent/ThinkingLevelType 等）与提供方实现，**不含任何 tidev 产品概念**（BackendEvent/Mode/快照/指令/subagent/审批一律不出现）。其他项目可单独依赖做纯 LLM 调用。
- **tidev-agent**：完整 agent 运行时内核，tidev crate 依赖仅 tidev-llm（+ 外部依赖 rmcp 提供 MCP 客户端）。其他 agent 产品**只依赖 tidev-agent** 即可开发（压缩、工具注册、循环、默认运行时、MCP 开箱即用）。
- **tidev-core**：tidev 产品层（CoreContext、SessionManager、Mode、审批媒介、MCP 集成、快照、指令、BackendEvent、prompts），是内核的一个宿主实现。
- **审批不在内核**：AgentContext 无任何审批钩子——审批是宿主 `execute_tools` 的内部策略。权限声明在 tidev-tools（ToolPermission），审批 UI 媒介在 tidev-core（TuiRequest/ApprovedTool）。
- 依赖图无循环；`tidev-agent` 的 tidev 内部依赖仅 tidev-llm。

> **铁律（权威定义；本路线图所有"铁律"均指此条）**：任何已经发送给 LLM API 的内容，在后续请求中必须保持字节级不变——同一会话内，第 N 轮发给模型的字节，第 N+1 轮必须一字不差地再次发送。
>
> 四个 provider 构造请求时逐字段挑选（anthropic.rs:433 / openai.rs:412），从不整体序列化 Message——因此 llm 类型中与请求无关的字段（快照/指令/subagent 元数据）移出类型不触碰铁律。真正的风险点：注入决策链（P1.5+P2）、build_request_messages/压缩（P3）、存储重载循环（P1.5）、审批迁移的拒绝/结果消息顺序（P2）。

目标态依赖图：

```
tidev-llm（叶子）           协议类型 + 提供方 + LlmEvent（零 tidev 概念）
tidev-utils（叶子）         canonical_tool_name 等纯函数
tidev-agent ──→ tidev-llm + rmcp（外部）
                            完整内核：循环 / AgentContext(7 方法) / AgentEvent / Tool trait
                            / ToolRegistry / ContextManager / MessageBuffer / AgentRuntime
                            / MessageStore / MCP 客户端（McpClient / McpRegistry）
tidev-tools ──→ llm, utils, config, instructions   工具类型 + ToolPermission（权限声明）+ builtin
tidev-core ──→ agent, tools, llm, config, storage, snapshot, instructions, logging, search, utils
                            产品层：BackendEvent / Mode / CoreContext / Runtime / SessionManager
                            / 审批媒介（TuiRequest/ApprovedTool）/ MCP 集成（配置/权限/状态）
                            / prompts / 指令 / 快照 / undo
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
| P1.5 llm 协议净化 | Message/ToolMetadata/ToolExecutionResult 净化、SessionMode→Mode 移 core、tools 只读信号化、storage app-data 通道 + 迁移 v40 | P1 | 6–9 天 |
| P2 循环与 trait 去 tidev 化 | trait 7 方法（审批删除）、注入迁移、审批媒介归 core、AgentLoopConfig 无 mode | P1、P1.5 | 3–4 天 |
| P3 内核组件迁入 | MessageBuffer / ContextManager / Tool trait / ToolRegistry / MCP 客户端进 tidev-agent | P1、P2 | 5–6 天 |
| P4 默认运行时与收口 | AgentRuntime（无 ApprovalHandler）、CoreContext 收口、子代理机制统一、消费方示例（含 MCP） | P3 | 4–6 天 |
| P5 清理与验证 | TODO 清除、文档同步、最终依赖图验证 | 全部 | 0.5–1 天 |

**合计约 22–30 个工作日（5–6 周）**。阶段边界是自然的中途止损点：**每个阶段结束时工作区必须 `cargo check` + `cargo test --workspace` 全绿**（与拆分的一次性策略不同，本路线图按阶段增量交付；P1.5 例外——它与 P1 同属 llm 范围内的连锁改动，可合并为一次性迁移）。

## 4. 阶段细则

### P1 事件三层拆分

**目标**：事件按层归属——llm 只发 LlmEvent、内核用 AgentEvent、tidev UI 用 BackendEvent（回到 core）；删除 BackendEvent 的 TODO 注释。

**LlmEvent**（tidev-llm 定义，即当前 llm 实际产生的全部变体）：`Delta`、`ReasoningDelta`、`ToolCallUpdated`、`UsageStats`、`Finished(AssistantTurn)`、`Failed`、`Retrying`。无 session_id/request_id（宿主概念不进入协议层）。

**AgentEvent**（tidev-agent 定义，内核事件）：LlmEvent 的 7 个变体 + `TurnStarting`、`StreamEnd`、`ToolStarting`、`ToolCompleted`、`ContextCompacted`、`ShellOutput`。携带 `request_id`（循环的轮次号，供前端丢弃过期事件）；无 session_id。

**BackendEvent**（tidev-core）：保留全部现有变体与 session_id（tui/acp 的渲染与翻译逻辑不变）；仅 core 发射。

**转换层**（两处，各一个函数）：
- `LlmEvent → AgentEvent`：在 tidev-agent（补 request_id），供 AgentRuntime 与 CoreContext 复用。
- `AgentEvent → BackendEvent`：在 tidev-core（补 session_id）。落点即现有 `agent_ctx.rs:500` 附近的事件匹配循环（当前在 stream_turn 内匹配 BackendEvent 并转发 TUI——改为匹配 AgentEvent 后转 BackendEvent）。映射必须覆盖全部变体，**漏一个变体 TUI 渲染会静默出错**。

**改动点**：
- tidev-llm：41 处 `BackendEvent::` 构造点（anthropic 9 / gemini 10 / lib 3 / openai 9 / responses 10）改为 `LlmEvent::`；`stream_chat` 签名去掉 session_id/request_id 参数（事件通道类型改 `UnboundedSender<LlmEvent>`）。**`complete_with_messages`（lib.rs:112，非流式，ContextManager.compact 在用）同样去掉 session_id/request_id 并改发 LlmEvent——只改 stream_chat 会在 llm 残留 BackendEvent。**
- tidev-agent：`AgentEvent` 定义 + 转换函数；`AgentLoopConfig.event_tx` 与 `loop_.rs` 的事件发射改 `UnboundedSender<AgentEvent>`。
- tidev-tools：**ShellOutput 本地化**——`ToolContext.event_tx`（builtin/mod.rs:32）改 `Option<UnboundedSender<ShellOutput>>`，ShellOutput 为 tidev-tools 本地结构体（session_id/request_id/content/finished 等数据字段）；exec.rs 的 5 处事件通道改型。tidev-tools 不再引用 BackendEvent。**顺序约束**：ShellOutput 改独立通道后，宿主必须在发 ToolCompleted 前同步 drain 该通道——现状 ShellOutput 与 ToolCompleted 同通道天然有序，改独立通道后用异步转发任务会产生竞态、破坏 TUI 渲染顺序，行为必须保持。
- tidev-core：BackendEvent 从 llm 迁入 core（新模块或 message 模块内）；ShellOutput 桥接（core 订阅 tools 的 ShellOutput 通道 → `BackendEvent::ShellOutput`，同步 drain，见上）；stream_turn 的匹配循环改造。

**验收**：llm/agent/tools 零 `BackendEvent` 引用；TUI 渲染行为与拆分前一致（BackendEvent 变体全集未变）；`grep -rn "TODO(event-split)"` 无残留；**两个转换函数各加穷举全部变体的单元测试 + 变体数量守恒断言**（现有 TUI/ACP 的 tests/ 目录为空、无集成测试可依赖，只能靠单测兜底）。

### P1.5 llm 协议净化

**目标**：tidev-llm 绝对干净——只含协议类型与提供方实现；所有 tidev 产品概念移出。一次性迁移（同拆分策略）。

**划分规则**：llm 可以携带通用协议/agent 概念，不能携带 tidev 产品概念。判定依据：该类型/字段是否进入 LLM 请求序列化（provider 逐字段挑选，已核实）、是否为 tidev 特有语义（Plan/Build、快照、undo、指令、subagent）。

**移出清单**：

| 移出项 | 去向 | 说明 |
|---|---|---|
| `BackendEvent` | tidev-core | P1 已完成 |
| `SessionMode`（→`Mode`） | tidev-core 新 mode.rs | Plan/Build 是产品语义；provider 零引用（已核实）；serde 表示与旧列格式完全一致（mode 列原样保留） |
| `Message.mode` / `snapshot_hash` / `patch_files` / `file_diffs` | 列留在 DB（列式存储，零迁移）；storage 行结构保留列，经 app-data 通道暴露；core 合成 `SessionMessage`（llm Message + app 字段）供 TUI/undo/注入逻辑消费 | mode 列解析（现 storage lib.rs:139）上移 core；写路径拆分（lib.rs:1055-1061） |
| `ToolExecutionResult.snapshot_hash` / `patch_files` | **直接删除** | 全仓没有任何 tool 设置过它们（只有 message.rs 的拷贝与测试），是死载体 |
| `ToolExecutionResult.instruction_sources` | `ToolContext` 侧通道（core 所有） | DB 表 `session_instruction_sources` 是唯一事实源，字段只是工具→宿主的临时载体 |
| `ToolMetadata.child_session_id` | 新列 + 迁移 v40 | 旧值在 zstd 压缩的 metadata blob 里，纯 SQL 无法回填；migration.rs 需支持 Rust 回填步骤 |
| `QueuedUserMessage.mode` | 删字段；**类型留 llm** | 依赖图约束：tui 构造 → core 中转 → agent 消费，tui 不依赖 agent、agent 不依赖 core，共同祖先只有 llm；去 mode 后（content/attachments/thinking_level）是纯协议输入类型 |

**保留**（协议面，不逐字段解释）：ApiType / LlmProviderConfig / ToolDefinition、Message（净化后）、MessageRole（含 Shell）、MessageAttachment、ToolCall、AssistantTurn、ToolExecutionResult（净化后）、ToolMetadata（净化后）、FileChangeInfo、tool_output_preview、COMPACTION_MESSAGE_LABEL、LlmEvent 7 变体、ThinkingLevelType（塑造 API 请求，llm 自己的类型）。

**tools 只读信号化**（Mode 移出 llm 的依赖前提；tools 不能依赖 core，故用通用布尔信号而非 Mode 类型）：
- `ToolContext.mode`（builtin/mod.rs:27）→ `read_only: bool`；exec.rs 两处 Plan 守卫（:222/:481，本质是只读约束）改判 `read_only`。
- `ToolPermission::allowed_in_mode`（types.rs:45）→ `allowed_in_read_only`；调用点 core registry.rs:178、mcp.rs:312/345 同步改。
- task.rs 的 `_mode` 参数删除（已核实无人使用）。

**storage 改动**：
- `RawMessageRow` 保留 snapshot_hash/patch_files/file_diffs/mode 列；`decompress_and_parse`（lib.rs:112）不再填 llm Message，改经 app-data 访问器暴露（如 `load_message_app_data(session_id) -> HashMap<Uuid, MessageAppData>`）。
- 写路径拆分：`insert_message(&Message, app: Option<MessageAppData>)` 或两步写（先协议后 app 数据）。
- mode 列以 `Option<String>` 透传，core 侧解析为 Mode（列内 JSON 字符串格式不变）。
- 迁移 v40：messages 表加 `child_session_id` 列；Rust 回填步骤从旧 metadata blob（zstd 解压后解析 ToolMetadata JSON）提取。

**core 改动**：
- 新 `mode.rs`：Mode 定义（serde 表示与旧 SessionMode 完全一致，全仓 `SessionMode` 引用改 `Mode`）。
- `SessionMessage` 合成层：TUI/undo/注入逻辑改经此取数（diff 卡片、mode 徽标、undo 的 patch/snapshot、注入决策的 prev_mode）。
- `QueuedUserMessage` 构造处（runtime.rs:541）不再带 mode；TUI 用自己的 pending_modes 表（已有）显示。

**tui/acp**：导入路径从 `tidev_llm::mode` 改为 `tidev_core`；消息恢复/渲染改经 core 的 SessionMessage。

**验收**：
- `grep -rn "SessionMode\|snapshot_hash\|patch_files\|file_diffs\|instruction_sources\|child_session_id" crates/tidev-llm/src` 零命中。
- **铁律回归**：同一会话数据，净化前后下发给 LLM 的消息序列字节完全一致（P1.5 结束做一次；harness 复用 P2/P3 计划）。
- `cargo test --workspace` 全绿（含迁移 v40 的旧库升级测试）。

### P2 循环与 trait 去 tidev 化

**目标**：`AgentContext` trait 只剩通用契约；循环不知道任何 tidev 行为；tidev-agent 零 tidev 内容。

**trait 最终面**（7 个方法）：`tools()`、`event_tx()`、`workspace_root()`、`stream_turn()`、`execute_tools()`、`save_messages()`、`load_messages()`。删除 `inject_instructions`、`append_instruction_sources`、`update_message_content`、**`request_tool_approval`（审批完全移出内核，本轮修订核心）**。

**审批归位**：
- `request_tool_approval` 从 trait 删除；`execute_tools` 签名改为接收 `&[ToolCall]`、返回 `Vec<(ToolCall, ToolExecutionResult)>`——被拒绝的调用由宿主返回合成结果（如 `"User rejected: ..."`）。
- `TuiRequest` / `TuiResponse` / `ToolCallWithViolations` / `ApprovedTool` → tidev-core（审批 UI 媒介；acp 经 core re-export 取用，路径兼容；ApprovedTool 成为 core 内部类型，其 tidev 字段 allow_outside/sensitive_file_approved/user_reason/child_session_id 自然留在 core）。
- 循环简化：删除 `"task"` 工具名判断（loop_.rs:177）与批准分类/拒绝持久化逻辑；拒绝的 `ToolCompleted` 发射与拒绝持久化归宿主 execute_tools。
- **消息顺序约束（铁律）**：当前 buffer 顺序为"拒绝消息在前、执行结果在后"（两步 save_messages）。新设计由宿主控制返回顺序（拒绝在前、执行在后），循环统一持久化，最终 buffer 状态必须与拆分前逐字节一致。

**注入迁移**（行为保持，字节级不变性铁律）：
- 指令注入 + mode reminder 注入从 `loop_.rs` 移入 `CoreContext::load_messages` 内部。顺序不变：加载后立即注入、先于 `TurnStarting` 事件；去重逻辑（content.starts_with、already_injected 集合）原样保留。**mode 决策数据源**：P1.5 后 mode 经 app-data 获取（不再在 Message 上），决策保真必须一致。
- 指令源持久化（现循环在工具执行后调 `append_instruction_sources`，loop_.rs:216）移入 `CoreContext::execute_tools` 内部（P1.5 后 instruction_sources 经 ToolContext 侧通道收集，宿主持有结果）。
- `update_message_content` 的调用（现用于注入持久化，loop_.rs:341）随注入消失；消息编辑走 core 的 SessionManager API（session.rs:135 已有）。

**其余内容**：
- mode reminders（mode_reminder/plan_mode_reminder/build_mode_reminder/plan_switch_reminder/build_switch_reminder）→ tidev-core 的 prompts 模块（Mode 类型已在 core，P1.5 完成）。tidev-agent 的 prompts.rs 删除，**tidev-agent 零 prompts、零 tidev 内容**。
- `AgentLoopConfig` 最终面：`session_id`、`system_prompt: String`、`thinking_level`、`event_tx: UnboundedSender<AgentEvent>`、`cancel`、`queued_messages`（**无 mode**——审批与注入都不再需要，宿主自己知道 mode）。

**验收**：trait 7 方法且零审批概念；loop_.rs 无 tidev 概念（无 mode / 无 task 名 / 无审批）；同会话同消息列表下发给 LLM 的字节与拆分前一致（铁律回归测试，含拒绝/结果消息顺序）。

### P3 内核组件迁入

**目标**：压缩、工具注册、MCP 客户端归 tidev-agent。

- **MessageBuffer**（tidev-core/src/message_buf.rs，72 行）→ tidev-agent：纯移动，无逻辑改动。
- **ContextManager**（tidev-core/src/context.rs，718 行）→ tidev-agent：
  - 依赖改为 LlmClient + MessageBuffer + 协议类型（均可达：agent → llm）。
  - `BackendEvent::ContextCompacted` → `AgentEvent::ContextCompacted`（P1 前置）。
  - 全量 ToolDefinition → 精简版（`to_llm_tool_def` 转换随迁或保留在 core）。
  - **`build_request_messages` 与压缩逻辑逐字节不变**（铁律；现有测试兜底）。
- **Tool trait**（tidev-agent 定义）：`definition() -> ToolDefinition`、`read_only() -> bool`、`async execute(args, ctx: &dyn ToolContext) -> Result<ToolExecutionResult>`；`ToolContext`：`workspace_root()` + `event_tx() -> UnboundedSender<AgentEvent>`。
- **ToolRegistry**（tidev-agent 定义，通用）：注册/按名派发/输出大小限制/执行事件。**权限与审批留在宿主**（execute_tools 内部策略，P2 后内核无审批钩子）——内核 registry 不做权限判定。
- **MCP 客户端**（tidev-agent 新模块 `mcp`，外部依赖 rmcp，本轮修订新增）：
  - `McpServerSpec`：`Stdio { command, args, cwd, env }` / `Http { url, headers }`——宿主解析好路径后传入，spec 不含任何 tidev 类型。
  - `McpClient`：rmcp 连接（stdio / streamable HTTP）、列工具、`call_tool`、结果格式化（文本/资源链接/图片→attachments，现 mcp.rs:554-618 原样迁）。
  - `McpRegistry`：多服务器管理 + 连接状态（Disconnected/Connecting/Connected/Failed，本就是通用枚举）。
  - MCP 工具实现内核 `Tool` trait：`definition()`（llm 精简版 ToolDefinition）、`read_only()`（服务器 `read_only_hint` 注解）、`execute()`（转发调用）。工具命名约定 `server__tool`（通用）。
  - 事件：MCP 调用走既有 ToolStarting/ToolCompleted（AgentEvent），无需新变体；连接状态由宿主轮询（现状如此）。
- **CoreContext 适配（部分）**：改用内核 MessageBuffer/ContextManager；tidev 工具适配器——每个 builtin 包装为内核 `Tool`（execute 内部构建 tidev 的 ToolContext 调 `tidev_tools::execute_tool_call`），MCP 工具经内核 McpRegistry 注册，适配器持有 tidev 上下文（skills/auth/web search/todo），**tidev-tools 不依赖内核**（依赖方向保持 tools → llm/utils/config/instructions）。
- **tidev-core 的 mcp.rs 缩减为集成层**：McpServerConfig（tidev-config）→ McpServerSpec 映射、权限映射（websearch/webfetch 名字特判 + read_only → ToolPermission，现 mcp.rs:506-511）、TUI 状态（summaries/toggle/upsert/remove）、workspace cwd 解析。**tidev-core 删除 rmcp 依赖**。

**验收**：压缩输出字节不变（铁律）；tidev-core 不再定义 MessageBuffer/ContextManager、不再直接使用 rmcp 类型；MCP 工具的 definition/read_only/execute 行为与拆分前一致；`cargo test --workspace` 全绿。

### P4 默认运行时与收口

**目标**：tidev-agent 开箱即用（AgentRuntime）；tidev 的 CoreContext 成为内核的一个宿主实现；子代理机制统一；消费方验证。

- **AgentRuntime**（tidev-agent，实现 AgentContext）：接线 LlmClient + LlmProviderConfig + ToolRegistry + ContextManager + MessageBuffer + event_tx + cancel；通用 `execute_tools` 实现（只读并行 / 写串行 + 取消，**无审批**——需要审批的宿主自行实现 execute_tools）。
  - 对外抽象：`MessageStore` trait（load_messages/save_messages，宿主实现持久化）。（**ApprovalHandler 已删除**——审批是宿主 execute_tools 的内部策略，内核不提供审批钩子。）
  - **子代理**：设计决策点——内核提供可插拔 `SubagentHost` trait（AgentRuntime 的 execute_tools 检测 task 类工具时经其派发），或 v1 不支持子代理（宿主自行在 execute_tools 处理）。tidev 需要子代理，倾向提供 SubagentHost；若采用，`AgentEvent` 增加 `SubagentStatus`/`SubagentCompleted` 变体（注意：SubagentCompleted 目前是死变体，生产代码从不发射，采用时一并决定去留）。
- **CoreContext 收口**：`execute_tools` 内部自含审批流程（问 TuiRequest → 批准/拒绝 → 执行，拒绝结果合成）；保持 tidev 行为（子代理/敏感文件/边界/取消守卫/undo），底层基于内核 ToolRegistry；`stream_turn` 复用 P1 的 LlmEvent→AgentEvent 转换。
- **task 工具机制统一**：tidev-tools 的 task.rs 校验桩与 tidev-core 的 execute_task_tool（子代理派发）经 SubagentHost 统一；agent_type 已在 core，本次是机制整合。
- **消费方示例**（验收核心）：新增示例（如 `crates/tidev-agent/examples/minimal_agent.rs` 或独立 example 目录）——只依赖 tidev-agent + tidev-llm，实现 MessageStore + 注册两个内置工具 + 连接一个 MCP 服务器（execute_tools 用 AgentRuntime 通用实现或自定义），跑通 `run_agent_loop`。这是"另一个 agent 产品只依赖 tidev-agent"的证明。

**验收**：示例 crate 的 Cargo.toml 中 tidev 依赖仅有 tidev-agent（+tidev-llm 传递）；示例含 MCP 工具调用；tidev 自身功能回归全绿。

### P5 清理与验证

- 删除全部遗留 TODO（event-split 等）；`grep` 确认无临时注释残留。
- 同步 rewrite-plan/architecture.md 与 D-005 等设计文档至目标态（含本轮修订：铁律定义、7 方法 trait、审批归位、MCP 入 agent）。
- 最终 `cargo tree` 验证 §1 依赖图（叶子、无循环、agent 的 tidev 依赖仅 llm）。
- 可选（用户决策）：tidev-llm / tidev-agent / tidev-protocol 相关类型的 crates.io 发布准备（版本、文档、license；届时评估 `tidev-` 前缀的品牌问题）。

## 5. 执行顺序依据

1. **P1 最先**：BackendEvent 的 TODO 在 llm 中悬置；AgentEvent 是 P3（ContextManager 发 ContextCompacted）的前置；事件类型是所有后续阶段的地基。
2. **P1.5 在 P2 之前**：trait 冻结必须在净化后的类型上进行——Mode 已移 core、Message 已净化、注入移植读 app-data；P1.5 与 P1 同属 llm 范围内的连锁改动，做完再冻结 trait 避免返工。
3. **P2 在 P3 之前**：trait 定稿后，内核组件（ToolRegistry/AgentRuntime）按最终契约编写，避免返工；注入迁移独立于组件迁移。
4. **P3 在 P4 之前**：AgentRuntime 需要 MessageBuffer/ContextManager/ToolRegistry 就位。
5. **P4 最后**：集成收口；CoreContext（1611 行）在 P3、P4 各触碰一次（组件替换、行为收口），分两次降低单次风险。

## 6. 总验收标准

- 依赖图与 §1 完全一致；`tidev-agent` 的 tidev 内部依赖仅 tidev-llm（rmcp 为外部依赖）。
- 铁律（§1 权威定义）：P1.5、P2、P3 各做一次"同一会话数据下 LLM 请求字节一致"回归验证；P2 额外验证拒绝/结果消息顺序。
- 消费方示例只依赖 tidev-agent 跑通完整循环（含 MCP）。
- `grep` 确认 llm 零 tidev 概念（SessionMode / snapshot_hash / patch_files / file_diffs / instruction_sources / child_session_id 零命中）。
- `cargo check --workspace`、`cargo test --workspace` 全绿（现有 858 单测 + 集成测试 + 各阶段新增测试）。
- 除已确认的行为变化（fixer Plan 检查，拆分时已落地）外，tidev 产品行为不变。

## 7. 风险与注意事项

- **铁律的落实**：字节级不变性针对"下发 LLM 的请求字节"。最易破坏的两处是注入决策链（P1.5+P2 叠加：mode 数据源从消息字段变 app-data，决策保真与注入文本必须字节不变）与 ContextManager 迁移（P3，代码逐字节移动）；另加审批迁移的消息顺序（P2）与存储重载循环（P1.5）。每处迁移前后用同一会话数据对比下发消息字节。
- **事件映射覆盖**：AgentEvent→BackendEvent（P1）漏变体不会编译报错，只会让 TUI 渲染静默出错——逐变体核对，并为两个转换函数各加穷举全部变体的单元测试 + 变体数量守恒断言（现有 TUI/ACP 的 tests/ 目录为空，无集成测试可依赖）。
- **ShellOutput 顺序竞态（P1）**：tools 的 ShellOutput 改独立通道后，若用异步转发任务转发，ShellOutput 可能晚于 ToolCompleted 到达 TUI——必须在发 ToolCompleted 前同步 drain，保持与现状一致的事件顺序。
- **child_session_id 迁移（P1.5）**：旧值在 zstd 压缩的 metadata blob 里，纯 SQL 无法回填；migration.rs 需要支持 Rust 回填步骤（v40），验收含旧库升级测试。
- **CoreContext 是最大改动面**：P3/P4 两次触碰；每次改动保持小步提交。
- **子代理抽象（P4）是最大设计点**：tidev 的子代理含 session 管理（SQLite）与审批（宿主侧），通用 SubagentHost trait 的边界需谨慎；若设计成本过高，v1 可允许宿主自实现（AgentRuntime 不内建子代理），在文档中明确记录该决策。
- **AgentRuntime 与 CoreContext 的行为分叉**：tidev 产品行为（指令注入、undo、敏感文件、审批、快照）留在 CoreContext，不要试图全部塞进通用内核——内核提供机制，tidev 提供策略。
- **新增共享类型的归属纪律**：llm 与 utils 为叶子；任何新共享类型必须先定归属，防止"types crate"死灰复燃。QueuedUserMessage 因依赖图约束（tui 构造 → core 中转 → agent 消费，共同祖先仅 llm）留在 llm（去 mode 后为纯协议输入类型），是已记录的理由而非例外。
