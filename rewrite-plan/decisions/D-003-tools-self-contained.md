# D-003: tidev-tools 依赖原则

**日期**: 2026-07-02  
**更新**: 2026-07-03  
**状态**: ✅ 已定案

## 决策

tidev-tools 依赖以下 tidev crate：
- `tidev-types`（工具返回类型、BackendEvent）
- `tidev-utils`（路径函数、decode_command_output）
- `tidev-instructions`（resolve_nearby_instructions，供 file.rs 读取附近指令用）
- `tidev-config`（WebSearchConfig、AuthStore，供 web 工具用）

对存储层（tidev-storage）的依赖通过 trait 切断。todowrite 工具定义 `TodoPersistence` trait，由 tidev-core 在实现 `AgentContext` 时桥接 `SessionStore`。

## 依赖图

```
tidev-tools
  ├── tidev-types
  ├── tidev-utils
  ├── tidev-instructions
  ├── tidev-config
  │
  ├── glob / grep / ignore / globset / rayon    （search 工具）
  ├── diffy / base64 / mime_guess               （file 工具）
  ├── async_trait / reqwest / pulldown-cmark / url  （web 工具）
  └── log / libc(unix) / tempfile(dev)
```

tidev-core 通过 `TodoPersistence` trait 桥接 tidev-storage：

```
tidev-tools::TodoPersistence (trait)
    ↕  impl by tidev-core
tidev-storage::SessionStore
```

tidev-tools 和 tidev-storage 之间没有直接依赖。

## 理由

1. **避免让工具 crate 依赖数据库 crate**——工具执行与数据持久化是不同层面的职责
2. **trait 足够轻量**——`TodoPersistence` 只有两个方法，不是重量级抽象
3. **tidev-core 做桥接**——它是唯一"知道两边"的 crate，由其连接工具和存储
