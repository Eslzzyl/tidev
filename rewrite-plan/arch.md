# 架构设计

这份文档定义 tidev 在 crate 合并完成后的理想架构。当前三个关键问题驱动了这个设计：TUI 直接写 DB、TUI 持有会话业务状态（ContextManager）、TUI 和 AgentLoop 各有一份 Conversation 副本且无单一权威来源。本设计通过重新划定职责边界和引入三通道通信协议解决这些问题。以下只描述目标状态，不涉及迁移路径。

## 组件分层

```
TUI（纯前端）
  依赖：tidev-agent

SessionManager（会话入口）
  属于：tidev-agent
  依赖：tidev-storage, tidev-llm, tidev-tools, tidev-snapshot, tidev-instructions

AgentLoop（执行引擎）
  属于：tidev-agent
  创建自：SessionManager
```

## 三个通信通道

### FrontendMessage（TUI → SessionManager）

用户意图的上行通道。每种意图对应一个变体：

- 提交提示词（含文本和附件）
- 命令（压缩、撤销、重做、中断）
- 工具审批响应
- 会话切换

### AgentEvent（AgentLoop → SessionManager）

AgentLoop 执行结果的报告通道。SessionManager 每收到一个事件就更新权威状态、写 DB、派生 DisplayEvent 发给 TUI：

- LLM 流式输出（Delta / ReasoningDelta）
- LLM 轮次完成（含消息体和用量数据）
- 工具执行完毕
- 工具审批请求（需要用户决策）
- 上下文压缩完成（含总结文本和压缩前状态）
- AgentLoop 退出（正常或失败）

### DisplayEvent（SessionManager → TUI）

TUI 只通过这个通道获取状态变更。TUI 不轮询、不直接读 DB、不持有任何会话业务状态：

- 会话初始快照（切换或加载时发送全量消息列表）
- 消息追加
- 流式增量更新
- 消息完成
- 上下文压缩完成（含 compaction 消息体）
- 撤销后可见范围变化
- 状态变更（空闲/思考中/规划中/错误）
- 工具审批对话框弹出

## 职责边界

### SessionManager

- 维护活跃会话列表（session_id → 会话状态）
- 接收并处理所有 FrontendMessage
- 启动 AgentLoop（传入权威状态的快照）
- 取消 AgentLoop（通过 CancellationToken）
- 设置和清除 pending_compact 原子标志
- 接收 AgentEvent，更新权威状态，写 DB，发送 DisplayEvent
- 无 AgentLoop 时自行执行 Compact、Undo、Redo 等命令
- 唯一有权写 SessionStore 的组件

### AgentLoop

- 接收快照（conversation 工作副本 + context_manager 工作副本）
- 持有 pending_compact 标志的共享引用
- 循环：build_request_messages → LLM 请求 → 流式响应 → 工具执行 → 报告
- 空闲检查点（工具执行后、下一轮 LLM 前）：
  - 读取 pending_compact → 执行手动压缩
  - 检查 context.needs_compaction → 执行自动压缩
- 所有输出通过 AgentEvent，不直接写 DB

### TUI

- 持有显示用消息列表（DisplayEvent 驱动的只读快照）
- 持有 UI 状态：光标、面板开关、滚动位置、对话框状态
- 不持有 ContextManager
- 不写 DB
- 不调用 LLM
- 不调度压缩
- 不管理会话业务状态

## 消息生命周期

### 提交到显示完成

1. TUI 发送 FrontendMessage::SubmitPrompt
2. SessionManager 追加用户消息到权威 conversation，写 messages 表，发 DisplayEvent::MessageAppended
3. SessionManager 启动 AgentLoop（快照 + pending_compact 标志引用）
4. AgentLoop 构建 request messages，调用 LLM
5. LLM 输出 → AgentEvent::Delta → SessionManager → DisplayEvent::MessageDelta → TUI 更新流式消息
6. LLM 完成 → AgentEvent::TurnFinished（含消息体和用量）
7. SessionManager 追加 assistant 消息到权威 conversation，写 DB
8. SessionManager 发 DisplayEvent::MessageFinalized → TUI 确认
9. AgentLoop 检查工具调用，有则重复 4-8，无则退出

### 上下文压缩

AgentLoop 在空闲检查点执行：

1. 检查 pending_compact 标志和 needs_compaction
2. 需要压缩时：更新工作副本 context_manager，生成总结
3. AgentEvent::ContextCompacted → SessionManager
4. SessionManager 更新权威 context_manager，写 sessions 表的 context_summary/context_retained_from
5. 创建 compaction 消息追加到权威 conversation，写 messages 表
6. DisplayEvent::ContextCompacted（含 compaction 消息体）→ TUI 插入显示

手动 /compact 的特殊处理：

- 有活跃 AgentLoop：设置 pending_compact，AgentLoop 在下个空闲点执行
- 无活跃 AgentLoop：SessionManager 自行从 DB 加载权威状态，执行 compact（委托给独立任务，用 AgentEvent 通道汇报），写 DB，发 DisplayEvent
- AgentLoop 在 LLM 请求或工具执行中时：pending_compact 被设置但不处理，AgentLoop 退出后标志随 Arc 释放而丢失
- 不产生额外的用户消息，不插入 /compact 文本到 conversation

### 撤销

1. TUI 发送 FrontendMessage::Command(Undo)
2. SessionManager 检查是否有活跃 AgentLoop，有则拒绝
3. SessionManager 操作权威 conversation：找到目标用户消息，隐藏其后的所有消息
4. 恢复 context_manager：检查隐藏范围内是否有 compaction 消息，有则恢复到该消息记录的 prior 状态，无则保留当前状态
5. 写 session_reverts 表记录撤销状态
6. DisplayEvent::MessagesHidden（含新的可见消息数）→ TUI 重新渲染

## 会话切换

TUI 发送 FrontendMessage::SwitchSession：

1. SessionManager 取消当前会话的 AgentLoop（CancellationToken）
2. 从 DB 加载目标会话的消息、上下文状态等
3. 构造权威 conversation 和 context_manager
4. DisplayEvent::SessionLoaded（含全量消息列表快照）→ TUI 替换显示列表
5. 旧会话的 AgentLoop 如果正在运行，在取消点退出，已发出的事件被 SessionManager 正常处理

## crate 合并后各 crate 职责

### tidev-agent（含 context、hooks、system-info）

```
session_manager.rs    会话管理、命令路由、DB写入
agent_loop.rs         执行引擎
context.rs            ContextManager + compaction
hooks.rs              生命周期钩子
system_info.rs        系统环境探测
types.rs              配置类型
prompts.rs            系统提示组合
factories.rs          agent 工厂
persistence.rs        消息持久化辅助
```

### tidev-tui（含 notification、logging）

```
core/                 入口、事件循环、撤销逻辑
input/                键盘鼠标输入、@ 搜索
render/               消息渲染、主题
ui/                   面板、对话框
notification.rs       桌面通知
logging.rs            日志初始化
markdown/             markdown 渲染
theme/                颜色主题
```

### 其余独立 crate（职责不变）

- tidev-types：共享类型
- tidev-session：数据模型
- tidev-storage：SQLite 持久化
- tidev-llm：LLM provider 抽象
- tidev-config：配置 + auth
- tidev-tools：工具注册 + 20+ 工具
- tidev-instructions：指令文件查找
- tidev-snapshot：Git 快照
- tidev-sync：SSH 同步
- tidev-search：文件索引引擎
- tidev-mcp：实验性

## DB 写入口

| 写操作 | 执行者 |
|--------|--------|
| append_message | SessionManager |
| create_session | SessionManager |
| delete_sessions | SessionManager |
| update_session_context_state | SessionManager |
| set_revert_message_id | SessionManager |
| update_message_patch | SessionManager |
| update_message_snapshot | SessionManager |
| update_message_file_diffs | SessionManager |
| delete_messages | SessionManager |
| remember_tool_permission | SessionManager |
| save_tool_output | SessionManager |
| save_model_thinking_level | SessionManager |
| update_session_model | SessionManager |
| record_usage | SessionManager |

所有 DB 写入集中在 SessionManager 中处理。AgentLoop 和 TUI 不直接写 DB。
