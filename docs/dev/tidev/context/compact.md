# 上下文自动压缩机制

## 1. 触发时机

上下文压缩在以下两种情况下触发：

| 触发方式 | 说明 |
|---------|------|
| **自动触发** | 在每次 LLM 响应完成后调用 `schedule_context_compaction_for_session`（`src/app/mod.rs:1130`） |
| **自动触发** | 在用户提交 prompt 后、启动 assistant turn 前调用（`src/app/mod.rs:1241`） |
| **手动触发** | 用户输入 `/compact` 命令（`src/app/commands.rs:272`） |

### 2. 压缩判断逻辑 (`needs_compaction`)

```rust
// src/context.rs:108-124
pub fn needs_compaction(&self, conversation: &Conversation, model: &ActiveModel) -> bool {
    let (trigger_tokens, _) = self.compaction_budget_for_model(model);
    // ...
}
```

**关键参数：**
- `prune_threshold_tokens = 24_000`（默认触发阈值）
- `retain_recent_tokens = 12_000`（默认保留最近的 token 数）
- 模型特定：`trigger_tokens = context_window - reserved_tokens`
  - `reserved_tokens = max(max_output_tokens, context_window/8, 4000)`

### 3. 压缩执行流程 (`compact`)

```rust
// src/context.rs:235-331
pub async fn compact(&mut self, llm, model, conversation, manual, stream_ctx) -> Result<bool> {
    // 1. 获取所有可见消息
    let messages = conversation.visible_messages();
    
    // 2. 计算保留最近消息的截止点
    let split_index = self.choose_split_index(messages, retain_recent_tokens);
    
    // 3. 提取待压缩的消息块
    let compressed_chunk = &messages[..split_index];
    
    // 4. 构建压缩提示词
    let prompt = self.build_compression_prompt(compressed_chunk);
    
    // 5. 调用 LLM 生成摘要（支持流式或同步模式）
    let summary = llm.complete_with_messages(model, [system_msg, user_msg]).await;
    
    // 6. 保存状态
    self.summary = Some(summary.chars().take(8000).collect());
    self.retained_from = split_index;
}
```

### 4. 如何确定保留哪些信息

#### 4.1 保留策略 (`choose_split_index`)

```rust
// src/context.rs:337-353
fn choose_split_index(&self, messages: &[Message], retain_recent_tokens: usize) -> usize {
    let mut token_budget = retain_recent_tokens;
    let mut keep_from = messages.len();

    // 从后向前遍历，计算保留最近 N 个消息所需的 token
    for (index, message) in messages.iter().enumerate().rev() {
        let message_tokens = Self::message_tokens(message);
        if token_budget < message_tokens {
            keep_from = index + 1;
            break;
        }
        token_budget -= message_tokens;
        keep_from = index;
    }

    self.align_split_index_to_tool_boundary(messages, keep_from)
}
```

**保留规则：**
1. 从消息列表末尾向前计算，保留最近 `retain_recent_tokens` 个 token 的消息
2. 保留最近的消息（最新消息优先）
3. 对齐到工具边界：如果是 `Tool` 消息的起点，向上对齐以保持工具调用/结果的完整性

#### 4.2 被压缩消息的摘要提示词 (`build_compression_prompt`)

```rust
// src/context.rs:376-429
fn build_compression_prompt(&self, messages: &[Message]) -> String {
    let mut prompt = String::from(
        "Provide a detailed continuation summary for this coding conversation.\n\n",
    );
    
    // 添加已有的摘要（如果有）
    if let Some(summary) = &self.summary {
        prompt.push_str("Existing summary:\n");
        prompt.push_str(summary);
    }
    
    prompt.push_str("Messages to compress:\n");
    for message in messages {
        // 工具输出：使用 tool_output_preview()
        let content = if matches!(message.role, MessageRole::Tool) {
            tool_output_preview(message.tool_name, &message.content)
        } else {
            truncate(&message.content, 1500)  // 用户/assistant消息截取前1500字符
        };
        
        // 附件：调用 attachment.summary()
        // 推理过程：截取前240字符
        // 工具调用：截取前240字符
    }
    
    prompt.push_str(
        "\nFocus on: goals, decisions, file paths, code changes, active tasks, tool results, constraints, and anything needed to continue the work without re-reading prior context."
    );
}
```

### 5. 压缩摘要的格式要求

压缩系统提示（`src/prompts.rs:79-81`）：

```
You summarize coding context for continuation.
- Preserve the goal, decisions, file paths, constraints, tool results, and open tasks.
- Use short sections such as Goal, Decisions, Files, Tool Results, Open Tasks, and Constraints.
- Keep the summary dense and factual.
- Do not add filler, encouragement, or apologies.
- Prefer bullets over prose.
```

### 6. 持久化状态

压缩完成后，状态保存到数据库：

```rust
// src/storage/mod.rs:201-209
pub fn update_session_context_state(&self, session_id, summary, retained_from) {
    "UPDATE sessions SET context_summary = ?1, context_retained_from = ?2, updated_at = ?3 WHERE id = ?4"
}
```

字段：
- `context_summary`: 压缩生成的摘要文本（最大 8000 字符）
- `context_retained_from`: 保留消息的起始索引

### 7. 请求消息构建 (`build_request_messages`)

```rust
// src/context.rs:126-214
pub fn build_request_messages(&self, conversation, current_mode) -> Vec<Message> {
    let mut messages = Vec::new();
    
    // 如果有摘要，在最前面插入系统消息
    if let Some(summary) = &self.summary {
        messages.push(Message::new(System, format!("Context summary for continuation:\n{summary}")));
    }
    
    // 从 retained_from 索引开始，跳过保留消息
    for message in conversation.visible_messages().iter().skip(self.retained_from) {
        // ... 处理消息 ...
    }
}
```

### 总结

| 方面 | 实现 |
|------|------|
| **触发** | 自动（响应完成后）或手动（`/compact`） |
| **判断** | 基于模型的 context_window 和 token 阈值 |
| **保留** | 从后向前保留最近 N 个 token 的消息，对齐到工具边界 |
| **压缩** | LLM 根据提示词生成摘要 |
| **摘要内容** | 目标、决策、文件路径、代码变更、任务、工具结果、约束 |
| **格式** | 分节（Goal/Decisions/Files 等），用 bullet points |
| **长度限制** | 摘要最大 8000 字符；压缩提示词中内容截取 1500/240 字符 |