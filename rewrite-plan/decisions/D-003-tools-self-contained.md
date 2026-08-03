# D-003: tidev-tools 依赖原则

**日期**：2026-07-02
**更新**：2026-08-04
**状态**：已定案并验证

## 决策

tidev-tools 只依赖：

- tidev-llm：协议消息和工具结果类型。
- tidev-utils：路径和纯函数工具。
- tidev-instructions：附近指令文件解析。
- tidev-config：web search 和 auth 配置。

tidev-tools 不依赖 tidev-agent、tidev-core 或 tidev-storage。todowrite 通过
TodoPersistence trait 访问宿主存储；core 提供该 trait 的实现。

ShellOutput 是 tidev-tools 的本地事件类型。core 的 adapter 负责将其桥接
成 AgentEvent，并在 ToolCompleted 前同步 drain，以保持事件顺序。

## 理由

工具执行和产品编排属于不同层次。保持 tools 自包含可以让 agent runtime
复用工具契约，也避免把数据库、审批和 UI 依赖带入工具实现。
