# D-007: HookEngine 不纳入重写

**日期**: 2026-07-03  
**状态**: ✅ 已定案

## 决策

`HookEngine`（后处理钩子系统）不纳入重写。不在 tidev-core 或任何新 crate 中实现。

## 理由

1. **未实际使用**：团队未使用过 hooks 功能，旧代码的正确性无法验证
2. **非核心路径**：后处理钩子不影响 LLM 循环的核心逻辑，属于可选增强
3. **避免未经验证的抽象**：在不了解实际使用场景的情况下设计接口，容易过度设计

## 未来添加时的位置

如果以后需要，HookEngine 应加在 tidev-core 的 `execute_tools()` 内部：

```
tidev_tools::execute() → HookEngine::on_post_tool_use() → 追加到 result → 返回
```

配置类型（`HooksConfig`）在 tidev-config 中，HookEngine 实现在 tidev-core 中。不需要修改 tidev-tools 或 tidev-agent。

## 参考

旧代码位于 `_archive/v0.6.x/crates/tidev-engine/src/hooks/`，约 207 行（engine.rs）+ 配置 + 匹配 + 执行器。
