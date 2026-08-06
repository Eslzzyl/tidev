# tidev 目标态路线图审查报告

**审查日期**：2026-08-07<br>
**Git 基线**：`3fa992e2`（`补齐目标态验收并更新审查报告`）<br>
**审查对象**：基线之上的当前提交工作树<br>
**参考文档**：[tidev-target-roadmap.md](tidev-target-roadmap.md)<br>
**审查范围**：验证路线图目标状态与当前代码的一致性，记录已修复问题、仍存在的问题和潜在风险。

## 结论

当前代码已经达到路线图描述的目标状态。

当前工作树已经补齐并验证了 P1-01 至 P1-05、P2-01 和 P2-02 的实现与验收缺口：事件统一排队、工具调用顺序、消息稳定排序、MCP headers、事件字段保真、完整 agent loop、stdio MCP 调用和 legacy SSE 调用均已有代码与针对性测试。

仍需关注的主要问题如下：

- **无已确认的路线图阻塞项**：本次审查范围内的目标状态和消费方验收链路均已具备实现与测试证据。

因此，当前判断是：路线图要求的结构和本地验收链路已经达到，**目标状态已达成**。

## 问题清单

### P1-01：BackendEvent 存在异步转发竞态 ✅ 当前已修复

路线图要求 `AgentEvent -> BackendEvent` 的转换保持事件顺序，尤其是 `ShellOutput -> ToolCompleted` 必须有确定顺序。

当前代码已引入 session 级 FIFO `CoreEventBus`，AgentEvent 和 BackendEvent 统一进入同一底层队列。Runtime、CoreContext、工具完成 guard、压缩路径和子代理均通过该总线发送；子代理共享底层队列，但使用自身 session ID 进行事件转换。

验证证据：

- [backend_event.rs:342](crates/tidev-core/src/backend_event.rs:342) 定义有序事件总线。
- [backend_event.rs:493](crates/tidev-core/src/backend_event.rs:493) 的 `core_event_bus_preserves_mixed_event_order` 覆盖 `TurnStarting -> StreamEnd -> Delta` 混合顺序。
- [agent_ctx.rs:1613](crates/tidev-core/src/agent_ctx.rs:1613) 的测试验证 `ShellOutput -> ToolCompleted`。

### P1-02：orphan tool call 合成结果顺序不确定 ✅ 当前已修复

当前 pending tool call 已从无序 map 改为有序 `Vec<(id, name)>`，匹配结果时按 ID 删除，合成 orphan 结果时按首次出现顺序输出。

验证证据：

- [context_manager.rs:241](crates/tidev-agent/src/context_manager.rs:241) 使用有序 pending 列表。
- [context_manager.rs:617](crates/tidev-agent/src/context_manager.rs:617) 的 `build_request_messages_preserves_multiple_orphan_order` 覆盖三个 orphan 的 ID 和 tool name 顺序。

### P1-03：持久化重载缺少稳定排序键 ✅ 当前已修复

消息重载使用 `ORDER BY created_at ASC, rowid ASC`，最新消息查询使用反向双键；SQLite 导出和导入的消息扫描也使用同一排序。这样可以避免多个消息时间戳相同时，在重载或迁移过程中改变消息顺序。

验证证据：

- [lib.rs:1167](crates/tidev-storage/src/lib.rs:1167) 的消息重载使用双排序键。
- [lib.rs:1409](crates/tidev-storage/src/lib.rs:1409) 的最新消息查询使用反向双排序键。
- [lib.rs:2470](crates/tidev-storage/src/lib.rs:2470) 的 `message_reload_preserves_insert_order_for_equal_timestamps` 覆盖相同时间戳场景。

### P1-04：MCP HTTP headers 缺少贯通实现 ✅ 当前已修复

配置、core 映射、agent spec、ACP v1/v2 和 TUI 编辑器现在均保留 `BTreeMap<String, String>` headers。HTTP transport 使用 rmcp 的 `custom_headers` 转换为 `HeaderName/HeaderValue`，非法 header 会在连接前被拒绝。

验证证据：

- `tidev-config` 覆盖 HTTP 配置序列化和反序列化。
- `tidev-core` 覆盖配置到 agent spec 的 headers 映射。
- `tidev-agent` 覆盖有效 header 转换和非法 header 拒绝。
- `tidev-tui` 覆盖编辑器输入到配置的映射。

### P1-05：`tidev-llm` 仍包含 subagent 产品语义 ✅ 已修复

原实现中，LLM 层根据 `message.tool_name == "task"` 决定是否绕过通用工具输出预览截断。现在已改为通用协议元数据 `ToolMetadata::preserve_full_output`：

```rust
if message.metadata.preserve_full_output {
    message.content.clone()
}
```

provider 构造请求时仍调用该通用函数，例如 [openai.rs:415](crates/tidev-llm/src/openai.rs:415)，但 `tidev-llm` 不再识别 `task` 或 subagent。Core 层在 [agent_ctx.rs:118](crates/tidev-core/src/agent_ctx.rs:118) 对历史消息恢复标记，并在工具结果产生时设置标记；正常请求、自动压缩和手动压缩路径均覆盖。

这样既移除了 LLM 层的产品特判，也保留了 subagent 结果不截断的产品行为。历史消息没有新增数据库迁移；Core 在构造请求副本时补齐标记，保证已有会话下发给 LLM 的内容不变。

验证证据：`tidev-llm` 覆盖通用标记和默认预览行为，`tidev-core` 覆盖历史 task 消息恢复和仅 task 结果标记；`rg` 检查 `tidev-llm/src/attachments.rs` 已无 task 特判。

### P2-01：事件转换测试没有验证字段保真 ✅ 已修复

`LlmEvent -> AgentEvent` 的测试位于 [event.rs:212](crates/tidev-agent/src/event.rs:212)，`AgentEvent -> BackendEvent` 的测试位于 [backend_event.rs:411](crates/tidev-core/src/backend_event.rs:411)。两处现在都保留原有变体守恒测试，并增加了逐字段载荷测试。

新增测试逐字段断言以下内容原样传递：

- delta 和 reasoning 内容；
- tool call 的 ID、名称、参数和 thought signature；
- `AssistantTurn` 和 `ToolExecutionResult` 的完整字段；
- usage、model、重试参数和 duration；
- compaction summary、完成时间和错误信息；
- ShellOutput 的内容、结束标记和 exit code。

测试覆盖了 7 个 `LlmEvent -> AgentEvent` 变体和 13 个 `AgentEvent -> BackendEvent` 变体；`AssistantTurn`、`ToolExecutionResult`、tool call、usage、时间戳和错误字段均使用非默认载荷断言。路线图要求的事件字段保真现在有直接回归证据。

### P2-02：消费方示例没有证明完整验收链路 ✅ 已修复

新增 [consumer_contract.rs](crates/tidev-agent/tests/consumer_contract.rs)，用本地 scripted OpenAI-compatible provider 驱动两轮完整 `AgentRuntime::run`，并使用 [mcp_fixture.rs](crates/tidev-agent/src/bin/mcp_fixture.rs) 作为真实 stdio MCP 子进程。

测试验证了：

- 第一轮 provider 响应包含 tool call，runtime 产生并持久化 assistant/tool 消息；
- 第二轮请求包含原始 assistant tool call 和对应 tool result，最终文本消息被持久化；
- provider event、`ToolStarting`、`ToolCompleted` 和两轮 `Finished` 事件均被观察到；
- stdio MCP server 可以完成初始化、工具发现和 `tools/call`，结构化结果按既有格式返回；
- `minimal_agent` 通过 `tidev_agent::tidev_llm` 重导出访问协议类型，消费方不需要声明第二个 tidev crate。

该测试完全使用 loopback 和本地子进程，不依赖外部网络；它补齐了路线图要求的消费方完整链路证据。

### R-01：`Sse` 配置变体的 legacy SSE 兼容性限制 ✅ 已修复

配置层仍提供 `McpServerConfig::Sse`，core 会将其映射为 `McpServerSpec::Sse`。agent 现在为该变体使用独立的 `LegacySseTransport`，而 `McpServerSpec::Http` 继续使用 rmcp 的 `StreamableHttpClientTransport`。

`LegacySseTransport` 完成旧版“先 GET `/sse` 获取 endpoint，再 POST `/messages`，由初始 SSE 长连接推送响应”的握手和消息收发，支持相对或绝对 endpoint URL，并复用配置 headers。`consumer_contract` 中的本地 HTTP fixture 已验证 initialize、`tools/list`、`tools/call`、GET/POST headers 和 registry 工具执行结果。

## 阶段目标判断

| 阶段 | 判断 | 依据 |
|---|---|---|
| P0 | 按决策不纳入 | 路线图明确不实现请求字节捕获 harness。 |
| P1 | 已达到 | 三层事件类型、转换函数、ShellOutput 本地化、顺序总线和逐字段转换测试均已存在。 |
| P1.5 | 基本达到 | app-data 通道、v40 `child_session_id` 回填、orphan 顺序、稳定持久化排序和通用完整输出标记均已实现。 |
| P2 | 已达到 | `AgentContext` 已为 7 个方法，审批策略在 core；事件字段保真、完整 loop、tool call 顺序和持久化均有回归测试。 |
| P3 | 已达到 | MessageBuffer、ContextManager、ToolRegistry、MCP registry 已进入 agent，headers 已贯通；stdio 和 legacy SSE MCP 初始化、发现和调用均已有真实本地 fixture 测试。 |
| P4 | 已达到 | AgentRuntime、CoreContext、core 子代理策略和只依赖 agent 的消费方路径均已落位并验证。 |
| P5 | 已达到 | fmt、workspace check/test/clippy 和消费方示例均已通过，文档与依赖边界已同步。 |

## 已确认的结构性目标

- `LlmEvent -> AgentEvent -> BackendEvent` 转换链已存在。
- `tidev-agent` 的 tidev 内部依赖仅为 `tidev-llm`；`rmcp` 是外部依赖。
- `tidev-core` 不直接依赖 `rmcp`。
- `AgentRuntime`、`ContextManager`、`ToolRegistry`、MCP registry 已进入 `tidev-agent`。
- `AgentContext` 已收敛为 7 个方法，通用 runtime 没有 `ApprovalHandler`。
- 工具结果持久化路径已有按 assistant 原始 tool call 顺序恢复的逻辑。
- v40 `child_session_id` schema migration、Rust 回填和 round-trip 测试已存在。
- ShellOutput 在 core 的工具完成 guard 中有同步 drain 测试。

## 当前验证结果

以下命令已在当前提交工作树执行并通过：

```text
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p tidev-agent --test consumer_contract --no-fail-fast
cargo run -p tidev-agent --example minimal_agent
```

`cargo test --workspace --all-targets` 共通过 **874 个测试**，无失败；其中新增消费方集成测试通过 **3 个测试**，覆盖 scripted provider 的两轮 loop、真实 stdio MCP fixture 和 legacy SSE MCP fixture。消费方 smoke test 输出为：

```text
echo result: hello from tidev-agent
```

该命令验证无 provider 配置时的内置工具路径；完整 loop 和 MCP 证据由 `consumer_contract` 提供。

## 建议的后续优先级

1. 后续可继续扩展 legacy SSE 的断线重连、Last-Event-ID 和服务端主动通知场景；这些不属于当前路线图目标的阻塞项。

## 审查边界

本报告记录的是当前代码状态和潜在问题，不把“常规测试全绿”解释为自动具备协议兼容性；legacy SSE 的关键握手和工具调用已有独立本地测试证据。P1-01 至 P2-02 的修复、本次 R-01 修复和本报告均纳入本次变更。
