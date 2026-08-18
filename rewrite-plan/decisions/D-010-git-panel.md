# D-010 Git 面板的数据边界

**状态**：已采用

## 背景

TUI 需要提供当前工作区的 Git 状态、提交历史和 diff 查看能力。现有
`tidev-snapshot` 使用独立的内部 Git 仓库保存 undo/redo 快照，不能直接作为
工作区面板的数据源。

## 决策

在 `tidev-core` 增加只读 `GitService`，使用 `Runtime` 的 workspace 路径执行
Git 查询并返回结构化数据。`tidev-tui` 使用独立的异步结果通道承接查询结果，
以请求 ID 过滤过期响应，Git 面板负责状态、历史和 diff 三个视图的渲染。

Git 面板第一版只提供查询能力，暂不提供 stage、unstage、commit、checkout、
reset 或其他仓库写操作。

## 原因

工作区 Git 状态属于全局工作区数据，与会话级 `BackendEvent` 生命周期无关；
独立通道可以保持会话事件和 ACP 事件语义稳定。将查询服务放在 core 可以让
TUI 只处理交互和渲染，同时保留后续 ACP 或其他前端复用查询能力的空间。

查询命令关闭 pager 和 optional locks。状态使用 porcelain v2，历史使用 NUL
字段和记录分隔符，diff 限制最大返回字节数，从而减少解析歧义并限制渲染开销。
