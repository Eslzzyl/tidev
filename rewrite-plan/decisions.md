# 重写过程中的架构决策记录

## D-001: 合并 tidev-session 进 tidev-types

**日期**: 2026-07-02  
**状态**: 已采纳

### 背景

旧项目有两个共享类型 crate：
- `tidev-types`：`ThinkingLevelType`、`SessionMode` 等配置类枚举
- `tidev-session`：`Message`、`BackendEvent`、`ToolCall` 等运行时数据结构

### 决策

**合并为一个 `tidev-types` crate，删掉 `tidev-session`。**

### 理由

1. **两者本质相同**：都是零业务逻辑的纯数据类型定义，无实际区分标准
2. **类型互相引用**：`Message` 的字段直接使用 `ThinkingLevelType` 和 `SessionMode`，拆开只是制造了一条无意义的依赖边
3. **共享程度一致**：tidev-llm、tidev-agent、tidev-tools、tidev-storage、tidev-tui 都需要同时使用两者的类型
4. **"session" 命名模糊**：容易被误解为"会话管理"，实际内容是消息数据结构

### 模块组织

```
tidev-types/
  src/
    lib.rs         — pub mod reasoning; pub mod prompts; pub mod message;
    reasoning.rs    — ThinkingLevelType 及子级别
    prompts.rs      — SessionMode
    message.rs      — Message, MessageRole, MessageAttachment, ToolCall,
                      ToolExecutionResult, AssistantTurn, BackendEvent 等
```

`message` 比 `session` 更准确地表达了内容——这些是跨 crate 流转的消息协议类型。
