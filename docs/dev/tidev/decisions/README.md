# 重写过程中的架构决策记录

每个决策独立文件，位于 `decisions/` 目录。

| 编号 | 标题 | 状态 |
|------|------|------|
| [D-001](decisions/D-001-session-merge.md) | 共享类型阶段性合并（历史） | ⏳ 已 supersede |
| [D-002](decisions/D-002-tool-layering.md) | 工具类型系统分层 | ✅ 已采纳 |
| [D-003](decisions/D-003-tools-self-contained.md) | tidev-tools 依赖原则 | ✅ 已定案 |
| [D-004](decisions/D-004-search-migration.md) | tidev-search 独立迁移 | ✅ 已完成 |
| [D-005](decisions/D-005-agent-thin-layer.md) | tidev-agent 薄层设计与 v1 子代理边界 | ✅ 已定案 |
| [D-006](decisions/D-006-skip-mcp.md) | MCP 客户端归入 tidev-agent | ✅ 已落地 |
| [D-007](decisions/D-007-skip-hooks.md) | HookEngine 不纳入重写 | ✅ 已定案 |
| [D-008](decisions/D-008-cancellation.md) | 取消机制设计 | ✅ 已定案 |
| [D-009](decisions/D-009-skip-file-read-tracker.md) | FileReadTracker 不纳入重写 | ✅ 已定案 |
| [D-010](decisions/D-010-git-panel.md) | Git 面板的数据边界 | ✅ 已采用 |
