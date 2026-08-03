# D-001: 共享类型阶段性合并

**日期**：2026-07-02
**状态**：历史决策，已被 tidev-types-split 和目标路线图 supersede

## 背景

重写早期需要先把旧 tidev-session 的消息类型与旧 tidev-types 的配置类型
放到同一共享类型层，以便完成一次性拆分。该阶段性决策解决了旧 crate
之间的循环引用和迁移顺序问题。

## 当前状态

后续的 tidev-types-split 已完成，旧 tidev-types crate 已移除。当前类型归属
如下：

- LLM 协议类型在 tidev-llm。
- 工具定义和权限声明在 tidev-tools。
- tidev 产品事件、Mode、审批媒介和应用数据在 tidev-core。
- 通用 agent 机制在 tidev-agent。

本文件保留作为迁移历史，不再作为当前依赖或模块边界的依据。
