# TiDev 上下文管理说明

本文说明 TiDev 当前的上下文管理行为，并与 opencode、magic-code 做对比。实现入口主要在 [src/context.rs](../../src/context.rs)、[src/session.rs](../../src/session.rs)、[src/storage/mod.rs](../../src/storage/mod.rs)、[src/llm/attachments.rs](../../src/llm/attachments.rs)、[src/app/runtime/run.rs](../../src/app/runtime/run.rs)、[src/app/runtime/undo.rs](../../src/app/runtime/undo.rs)、[src/app/input/event.rs](../../src/app/input/event.rs)。

## 1. 当前行为

### 1.1 请求消息如何组装

TiDev 每次发起模型请求时，都会从 `Conversation.visible_messages()` 取出可见消息，再交给上下文管理器整理成请求消息序列。

当前规则是：

- 如果存在已经压缩过的上下文摘要，会先插入一条 system message，内容格式为 `Context summary for continuation:`。
- streaming message 会被跳过，不会进入请求。
- system message 本身不会直接作为普通历史消息发送，只有压缩摘要会作为 system message 注入。
- user message 会原样进入请求。
- assistant message 会原样进入请求，同时保留 `tool_calls`。
- tool message 只有在能和最近的 assistant `tool_call_id` 对上时才会保留，避免出现孤儿 tool result。
- error message 不会进入请求。

对应实现主要在 [src/context.rs](../../src/context.rs) 和 [src/llm/attachments.rs](../../src/llm/attachments.rs)。

### 1.2 reasoning / thinking 的处理

TiDev 会把 assistant 的 reasoning 保存在 `Message.reasoning` 里，并且写入 SQLite。

当前行为是：

- reasoning 会参与 token 估算。
- reasoning 会出现在压缩 prompt 中，而且是截断后的版本。
- reasoning 不会直接进入普通 OpenAI / Anthropic 请求 payload。

这意味着 TiDev 会保留思考链作为压缩信号，但不会把它当成日常对话的固定输入。

### 1.3 tool result 的处理

Tool result 会被存成普通的 Tool 消息，内容保存在 `Message.content`，附件保存在 `Message.attachments`。

当前还有一层模型侧预览降级：

- 当 tool output 不大时，直接发送完整内容。
- 当 tool output 很大时，消息里只保留预览，完整输出会另外写入 `tool_events`，并通过 `message_id` 关联回对应的 tool result。
- 预览文本由 `tool_output_preview()` 生成首尾摘录：
  - 前 3000 个字符
  - 后 1000 个字符
  - 同时附带原始输出字符数

这只影响模型请求里看到的文本，不影响完整输出的持久化；UI 在展开 tool result 时会按 `message_id` 取回完整内容。

### 1.4 上下文压缩什么时候触发

TiDev 现在不是固定阈值，而是按模型配置动态计算压缩预算。

当前公式大致是：

- `reserved_tokens = max(max_output_tokens, context_window / 8, 4000)`
- `trigger_tokens = context_window - reserved_tokens`
- `retain_recent_tokens = max(默认保留窗口, reserved_tokens)`，再限制到 `trigger_tokens`

如果模型没有配置 `context_window`，会回退到旧的固定阈值：

- `prune_threshold_tokens = 24_000`
- `retain_recent_tokens = 12_000`

当可见消息的估算 token 数达到触发阈值时，就会启动压缩。

压缩切分时会从后往前保留最近消息，并且会做 tool 边界对齐，避免把一个 tool call / tool result 链切断。

### 1.5 压缩时发生什么

压缩是非破坏性的：旧消息不会被删除，仍然保存在会话历史里，只是后续请求不再发送它们。

压缩过程会：

- 把更早的消息送给模型做摘要。
- 摘要提示会要求保留目标、决策、文件路径、代码变更、工具结果、约束和未完成任务。
- 摘要输入里会包含消息内容、附件摘要、reasoning 摘要和 tool call 摘要。
- 生成后的摘要会保存在 `ContextManager.summary` 中，同时记录 `retained_from`。
- 后续请求只发送 `retained_from` 之后的消息，并把摘要作为 system message 放在最前面。

### 1.6 持久化和恢复

TiDev 现在会把压缩状态写回数据库，避免重启后丢失上下文摘要。

当前持久化字段是：

- `sessions.context_summary`
- `sessions.context_retained_from`

恢复路径是：

- 会话从数据库加载时，`ContextManager::from_state()` 会把摘要和保留位置恢复回来。
- 压缩完成后，`apply_context_compaction()` 会把新状态回写到数据库。
- 新会话、撤销、重做、丢弃 reverted branch 时，会清空上下文状态，避免旧摘要串到新上下文。

如果你在旧数据库上运行当前代码，需要确保 `sessions` 表包含这两个字段。

如果你想在界面里展开大 tool result，还需要 `tool_events` 表包含 `message_id`，以便从预览消息定位到完整输出。

## 2. 与 opencode 的对比

opencode 的策略更偏向“把完整历史尽量保留下来，再在需要时压缩老内容”。它和 TiDev 的差异主要体现在下面几点。

| 维度 | TiDev | opencode |
|---|---|---|
| reasoning / thinking | 存储并参与压缩，但不进入普通请求 | 会保留并发送回模型 |
| tool result | 保留完整消息，模型侧有大输出预览 | 会保留，压缩时可能把旧 tool result 内容替换成占位文本 |
| 压缩触发 | 依据模型 `context_window` 和输出预算动态计算 | 依据 token overflow 和保留缓冲区触发 |
| 压缩方式 | 模型生成摘要，保留最近窗口，非破坏性 | 也是摘要驱动，但更强调历史回放和压缩边界 |
| 压缩后的状态 | `summary` + `retained_from` 持久化 | 历史中会显式保留压缩边界和压缩标记 |
| 对 reasoning 的态度 | 保守，默认不送入普通请求 | 更激进，倾向保留并继续发送 |

直观上，opencode 更像“尽量完整地记住并回放上下文”，而 TiDev 更像“把上下文压缩成一个可持续续写的摘要窗口”。

## 3. 与 magic-code 的对比

magic-code 的策略更偏向“窗口化 + 分层降级”。它和 TiDev 的差异主要体现在压缩管线和大输出处理上。

| 维度 | TiDev | magic-code |
|---|---|---|
| reasoning / thinking | 存储，参与压缩，但不进普通请求 | 当前窗口内保留，跨 compact boundary 后会丢弃 |
| tool result | 作为 Tool 消息保留，模型侧做首尾预览 | 当前窗口内保留，超大结果会外化并用引用替代 |
| 压缩触发 | 单一的模型感知阈值 | 多个阈值：接近上限时自动压缩、90% 预警、接近硬上限时阻断 |
| 压缩阶段 | 统一摘要驱动 | snip、microcompact、autocompact 三段式 |
| 边界处理 | 保留 tool 链完整性，避免孤儿结果 | 以 compact boundary 作为显式分界 |
| 大输出策略 | 生成模型侧预览，保留首尾片段 | 会持久化结果并替换成引用或标记 |

直观上，magic-code 更像“把上下文当成一个带边界的窗口系统来管理”，而 TiDev 目前更像“一个单一摘要窗口 + 最近消息保留”的简化版。

## 4. 结论

TiDev 现在的策略介于两者之间：

- 比 opencode 更保守，不会把 reasoning 默认塞进普通请求。
- 比 magic-code 更简单，没有多阶段 snip / microcompact 管线。
- 但 TiDev 已经具备两个关键能力：
  - 上下文压缩是模型感知的，不再依赖固定 24k 阈值。
  - 压缩状态会持久化，重启后还能继续续写。

如果后续要继续优化，比较自然的方向有两个：

1. 把大 tool output 从“首尾预览”升级成“外部引用 + 可展开查看”。
2. 引入更细粒度的分层压缩，例如先做本地裁剪，再做摘要压缩，最后才调用模型做全局压缩。