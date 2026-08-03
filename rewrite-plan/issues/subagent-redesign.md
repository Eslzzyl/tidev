# 子代理边界决策

**状态**：已关闭，v1 不实现通用 SubagentHost

## 当前设计

AgentRuntime 不识别 task，也不依赖 session、审批或产品事件。tidev-core 的
CoreContext::execute_tools 负责：

- 解析和校验 task 参数。
- 解析 AgentType、选择模型和过滤工具。
- 创建或恢复子 session，保存应用数据关联。
- 继承宿主审批策略并处理取消。
- 通过同一个 tidev-agent::run_agent_loop 执行子代理。
- 将子代理状态发送到 BackendEvent，并合成父调用结果。

tidev-tools::builtin::task 只提供工具定义和基础参数校验，不创建子 session。

## 关闭原因

通用 SubagentHost 的抽象边界需要表达 SQLite session、审批继承和
tidev-specific BackendEvent；把这些字段放入 tidev-agent 会破坏 agent
crate 的独立性。后续产品若需要不同的子代理策略，应在自己的 AgentContext
实现中处理，而不是修改通用 runtime。
