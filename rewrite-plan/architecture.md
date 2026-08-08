# tidev 当前目标架构

**状态**：P1-P5 已落地，最终验收完成
**更新**：2026-08-04

本文档描述当前重写代码的实际边界。本文档不再保留旧 tidev-types 架构的
兼容描述。

## 设计约束

### LLM 请求字节不变

同一会话内，已经发送给 LLM API 的内容在后续请求中必须保持字节级不变。
请求消息由协议层的 Message 列表、系统提示词、工具定义和 provider 配置
共同决定。任何消息注入、压缩、持久化重载、工具结果合成和事件顺序的修改，
都必须检查它是否改变下一轮请求的消息字节。

本轮不实现 P0 请求字节捕获 harness。工程上采用小步提交、确定性构造、针对
消息顺序的单测和代码审查控制风险；这不是对铁律的放宽。

### 单一消息事实源

tidev-core::CoreMessageBuffer 持有协议消息和 tidev 应用数据的配对：

    CoreMessageBuffer
    ├── tidev-agent::MessageBuffer<Message>
    └── HashMap<MessageId, tidev-storage::MessageAppData>

协议消息进入 tidev-agent::ContextManager 前会经过 core 的压缩、指令注入和
mode reminder 处理。应用数据不会进入 LLM 协议消息。

### 工具结果顺序

工具可以按策略并行执行，但事件顺序和持久化顺序是两件事：

- ToolStarting、ShellOutput、ToolCompleted 在工具实际执行时发出。
- AgentContext::execute_tools 返回值必须按 assistant 原始 tool_calls 顺序排列。
- run_agent_loop 按返回顺序构造并保存 tool result 消息。

这样只读工具的完成竞态不会改变下一轮发送给 LLM 的协议消息顺序。

## Crate 边界

    tidev-llm
      协议类型、provider 实现、LlmEvent
      只依赖外部库

    tidev-agent -> tidev-llm
      AgentContext、run_agent_loop、AgentEvent
      MessageBuffer、ContextManager
      Tool、ToolContext、ToolRegistry
      AgentRuntime、MessageStore
      MCP client、McpRegistry

    tidev-tools -> tidev-llm, tidev-utils, tidev-config, tidev-instructions
      builtin 工具、工具定义、权限声明、ShellOutput
      不依赖 tidev-agent、tidev-core、tidev-storage

    tidev-core -> tidev-agent, tidev-tools, tidev-llm, tidev-config,
                   tidev-storage, tidev-snapshot, tidev-instructions,
                   tidev-logging, tidev-search, tidev-utils
      CoreContext、Runtime、SessionManager、BackendEvent
      审批媒介、Mode、SessionMessage、快照、指令注入、undo
      MCP 产品集成和 tidev 工具适配器

    tidev-tui -> tidev-core, tidev-llm, tidev-tools, tidev-config, tidev-utils
    tidev-acp -> tidev-core, tidev-llm, tidev-config, tidev-utils

tidev-agent 的 tidev 内部依赖只有 tidev-llm。rmcp 是 agent 的外部依赖，
不把 tidev 配置、session、审批或存储类型带入 agent。

## tidev-agent

### AgentContext

AgentContext 只有七个方法：

    tools() -> Vec<tidev_llm::ToolDefinition>
    event_tx() -> UnboundedSender<AgentEvent>
    workspace_root() -> &Path
    stream_turn(...)
    execute_tools(...)
    save_messages(...)
    load_messages(...)

循环不识别审批、mode、快照、应用数据或子代理 session。宿主通过
execute_tools 自行实现这些产品策略。

### AgentRuntime

AgentRuntime 是不需要审批和 tidev 产品状态时的默认实现：

- 使用 MessageBuffer 管理协议消息。
- 通过 MessageStore 读写消息。
- 使用 ContextManager 构造请求消息和压缩上下文。
- 只读工具并行执行，写工具串行执行，取消时生成确定性的取消结果。
- 保持 tool result 的原始调用顺序。

examples/minimal_agent.rs 展示了仅依赖 tidev-agent 的消费方、两个内置
工具和可选 stdio MCP server。

### 事件

    tidev-llm::LlmEvent
            │ llm_event_to_agent_event()
            ▼
    tidev-agent::AgentEvent  (request_id，无 session_id)
            │ agent_event_to_backend_event()
            ▼
    tidev-core::BackendEvent (补 session_id)

ShellOutput 在 tidev-tools 独立产生。core 的适配器在发出 ToolCompleted
前同步 drain ShellOutput，保持原有 TUI 可见顺序。

## tidev-core

### CoreContext

CoreContext 是 AgentContext 的 tidev 宿主实现，而不是 AgentRuntime 的
包装器。原因是 tidev 的审批、快照、指令注入、应用数据、敏感文件和子代理
session 都属于产品策略，无法安全地塞进通用 runtime。

CoreContext 的职责：

1. 通过 ContextManager 做压缩，并保存 compaction marker 和状态。
2. 在 load_messages 中按既定顺序注入 instruction 和 mode reminder。
3. 在 execute_tools 中执行权限检查、审批、取消、只读并行、写操作串行和
   子代理派发。
4. 在 save_messages 中维护协议消息与 MessageAppData，处理快照、diff、
   child session 关联和工具发现的 instruction sources。
5. 把 agent 事件补充 session_id 后发往 TUI/ACP。

工具实现通过 core 的 adapter 接入 agent 的 Tool 契约；工具本身不依赖
agent。MCP 的 client 和通用 registry 在 agent，core 只负责配置映射、工作区
路径、权限和 UI 状态。

### 审批

审批完全属于 core：

    CoreContext::execute_tools
      ├── ToolPermission / mode 检查
      ├── 工作区边界和敏感文件检查
      ├── TuiRequest -> TuiResponse
      ├── 拒绝调用合成 ToolExecutionResult
      └── 已批准调用进入工具执行调度

AgentContext 和 AgentRuntime 没有 ApprovalHandler 或审批方法。
拒绝结果与执行结果最终都由 loop 按原始 tool call 顺序保存。

### 子代理

v1 不提供通用 SubagentHost trait，也不让 AgentRuntime 特判 task。
子代理的 session 创建、模型解析、工具过滤、审批继承、事件关联、取消和
结果合成全部由 CoreContext::execute_tools 及其 core 内部 helper 负责；
子代理与主代理共享 tidev-agent::run_agent_loop。

tidev-tools::builtin::task 只负责参数和 agent type 的基础校验及工具定义，
不创建 session，也不执行子代理。

## 持久化和请求构造

    SQLite SessionMessage
            │ load
            ▼
    CoreMessageBuffer (protocol Message + MessageAppData)
            │ load_messages
            ├── context compaction / marker
            ├── instruction injection
            └── mode reminder injection
            ▼
    AgentContext::stream_turn
            ▼
    tidev-llm provider request

助手消息先保存，工具执行结果由 loop 在 execute_tools 返回后统一保存。
instruction source 的 replay system message 在工具结果保存之后追加，保持历史
顺序。

应用字段 mode、snapshot、diff、instruction source 和 child session id 只在
storage/core 侧维护；它们不能出现在发送给 LLM 的协议消息中。

## 子代理和取消

主代理和子代理使用同一个 run_agent_loop。子代理使用受限工具定义、独立
session 和自己的消息 buffer，并通过 core 的 BackendEvent 通道向前端报告。

取消分两层处理：

- 工具和 LLM 调用接收 CancellationToken，在可合作的检查点退出。
- core 使用 JoinSet::abort_all 和 RAII guard，确保强制取消后仍产生工具
  完成事件及确定性的取消结果。

已经发送到前端的内容不回滚；取消结果仍按原始 tool call 顺序持久化。

## 实施状态和验收

已完成：

- P1 事件三层拆分和 ShellOutput 顺序桥接。
- P1.5 协议字段与应用数据拆分、storage v40、mode 迁移。
- P2 七方法 AgentContext、宿主审批、注入迁移和循环净化。
- P3 MessageBuffer、ContextManager、Tool、ToolRegistry、MCP 迁移。
- P4 AgentRuntime、MessageStore、消费方示例和 core 工具适配。
- CoreContext 工具结果按原始调用顺序返回的回归测试。

最终验收：

- 清理旧设计文档和遗留临时说明。
- cargo tree、全仓 grep、cargo check --workspace 和
  cargo test --workspace --all-targets 已完成。

本轮明确不做：

  - P0 请求字节捕获 harness。
  - 将 tidev 的审批、SQLite session 或子代理策略抽象进 tidev-agent。
