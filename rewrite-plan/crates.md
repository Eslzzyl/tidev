# Crate 合并决策

## 当前状态

Workspace 共有 19 个 crates（根 crate + 18 个成员）。

```
crates/
├── tidev-types
├── tidev-session
├── tidev-storage
├── tidev-llm
├── tidev-config
├── tidev-hooks
├── tidev-instructions
├── tidev-snapshot
├── tidev-sync
├── tidev-search
├── tidev-mcp
├── tidev-tools
├── tidev-context
├── tidev-agent
├── tidev-notification
├── tidev-logging
├── tidev-tui
└── tidev-system-info
```

## 合并内容

### 并入 tidev-agent

| Crate | 代码量 | 新增外部依赖（对宿主） | 原因 |
|-------|--------|----------------------|------|
| tidev-context | ~769 行 | 无（所有 deps 已在 tidev-agent） | ContextManager + compact 逻辑，唯一消费者是 AgentLoop |
| tidev-hooks | ~? 行 | 无 | PostToolUse hook 引擎，只在 AgentLoop 工具执行阶段调用 |
| tidev-system-info | ~218 行 | 无（chrono 已有） | 系统环境探测，只在 system prompt 组合时使用 |

### 并入 tidev-tui

| Crate | 代码量 | 新增外部依赖（对宿主） | 原因 |
|-------|--------|----------------------|------|
| tidev-notification | ~? 行 | 无（crossterm + tidev-config 已有） | 桌面通知 API，纯 TUI 能力 |
| tidev-logging | ~162 行 | 无（log + tidev-config 已有） | 日志子系统初始化，只在 TUI 入口调用 |

## 保持独立的 crates

| Crate | 原因 |
|-------|------|
| tidev-types | 基础类型层，所有 crate 依赖 |
| tidev-session | 数据模型，所有 crate 依赖 |
| tidev-storage | SQLite 持久化，独立业务 |
| tidev-llm | 4 个 provider 实现，真实复杂度 |
| tidev-config | 配置 + auth 管理，独立业务 |
| tidev-tools | 20+ 工具实现 + registry，体量大 |
| tidev-instructions | 共享基础库，tidev-agent + tidev-tools 均依赖，防止循环依赖 |
| tidev-search | 文件索引引擎，依赖重（rayon + ignore + notify），消费者多个 |
| tidev-snapshot | Git 操作，独立数据目录 |
| tidev-sync | SSH 同步，独立业务 |
| tidev-mcp | 实验性功能 |

## 合并后结构（14 crates）

```
crates/
├── tidev-types       (保留)
├── tidev-session     (保留)
├── tidev-storage     (保留)
├── tidev-llm         (保留)
├── tidev-config      (保留)
├── tidev-instructions(保留)
├── tidev-snapshot    (保留)
├── tidev-sync        (保留)
├── tidev-search      (保留)
├── tidev-mcp         (保留)
├── tidev-tools       (保留)
├── tidev-agent       (吸收 tidev-context + tidev-hooks + tidev-system-info)
├── tidev-tui         (吸收 tidev-notification + tidev-logging)
└── .                 (根 crate)
```

## 执行顺序

合并和架构重构必须交替进行：先清理 TUI 的越权行为，再用 crate 合并锁定边界。不能先把 crate 合并完再重构。

```
Phase 1 — 无害合并（纯机械，不影响编译）
  1. tidev-system-info → tidev-agent
  2. tidev-hooks → tidev-agent
  3. tidev-notification → tidev-tui
  4. tidev-logging → tidev-tui
  5. 更新 workspace members + cargo test

Phase 2 — TUI 清理（逐层剥离越权）
  6. 删除 TUI 的所有 store.* 写调用，替换为 SessionManager 方法
  7. 删除 TUI 的 context_manager 字段和所有引用
  8. 删除 schedule_context_compaction_for_session 和 apply_context_compaction
  9. 删除 compacting_sessions 和相关逻辑

Phase 3 — 锁定边界（合并 + 编译验证）
  10. tidev-context → tidev-agent（设为 pub(crate)，编译器确保 TUI 不可访问）
  11. 实现三通道通信协议（FrontendMessage / AgentEvent / DisplayEvent）
  12. 重构 SessionManager 为 DB 唯一写者
  13. cargo test 全量验证
```
