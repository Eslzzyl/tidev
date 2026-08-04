# 功能实现差距跟踪

**更新时间**：2026-08-04

本文件记录重构范围内的已知差距。路线图 P1-P5 已完成；以下原先记录的
差距已经在当前代码中闭合：

- per-agent 模型配置：`tidev-config` 提供解析和持久化，`tidev-core`
  在子代理创建时解析，`tidev-tui` 的 `ModelPanel` 提供多 agent tab、模型
  选择和思考等级配置。
- TUI Runtime 接入：`tidev-tui` 由 `tidev-core::Runtime` 统一持有资源，
  通过 `Runtime::submit_prompt`、`event_rx` 和 `request_rx` 完成请求、事件
  和审批交互。
- 图片 base64 编码：`MessageAttachment::Image` 保留原始字节，各 provider
  在请求构造时完成编码，详见 `rewrite-plan/issues/image-base64-encoding.md`。

本轮明确不纳入：

- P0 请求字节捕获 harness。铁律仍然有效，当前采用确定性请求构造、消息
  顺序回归测试、小步提交和代码审查控制风险。
- HookEngine。该功能未纳入本轮重写，决策见
  `rewrite-plan/decisions/D-007-skip-hooks.md`。
