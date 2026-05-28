# 消息渲染机制与滚动约束 (Message Rendering & Scrolling Constraints)

本文档描述了 tidev 中会话消息的渲染实现原理，以及在维护 UI 相关代码时必须遵守的核心约束，以防止出现滚动条计算偏差导致消息显示不全的问题。

tidev 的聊天界面渲染由 `src/app/render/render_chat.rs` 负责。为了支持长会话和流畅的滚动体验，渲染流程遵循以下逻辑：
1. **手动换行 (Manual Word Wrap)**：
   - 系统**不依赖** Terminal UI 库（如 `ratatui`）的自动换行功能。
   - 所有文本内容在渲染阶段之前，都会根据当前视口（Viewport）的实际像素宽度，通过 `render_markdown_text_with_width_and_cwd` 或 `word_wrap_line` 进行精准的手动预折行。
   - 这样做的目的是为了准确掌控每一条消息在垂直方向上占据的确切行数。

2. **虚拟化布局计算**：
   - 渲染系统维护一个逻辑上的 `total_lines` 计数，它是所有预折行后的行数总和。
   - 滚动位置（`message_scroll_offset`）和视口底部（`max_scroll`）的准确性完全依赖于 `total_lines` 的计算值。

3. **虚拟化渲染的滚动偏移计算**：
   - 当消息数量超过阈值（当前为 20 条）时，系统启用虚拟化渲染以提升性能。
   - 虚拟化渲染只渲染视口附近的可见块（包含上下各 5 行缓冲区）。
   - **关键**：`render_scroll` 必须正确计算为 `scroll - first_block_start`，以跳过缓冲区中不需要显示的行。
   - 如果第一个可见块的 `start_line` 大于 `scroll`，需要在渲染内容前添加空白行作为填充。

## 关键约束：UI 组件配置

在 `src/app/render/render_chat.rs` 中使用 `ratatui::widgets::Paragraph` 渲染消息文本时，**严禁开启 `.wrap()` 属性**。

### 为什么不能使用 `.wrap()`？

如果对 `Paragraph` 开启了自动换行：
- **行数不一致**：当某些不受 Markdown 渲染器控制的内容（如长路径、无空格的超长字符串等）意外超过可用宽度时，`ratatui` 会在底层进行二次折行。
- **滚动条失效**：二次折行会导致物理渲染的行数**多于**代码逻辑计算出的 `total_lines`。
- **后果**：用户滚动到底部时，由于代码计算的 `max_scroll` 小于实际物理高度，视口将永远无法触达消息的最末尾，造成“前半段正常，后半段卡在视口外”的显示故障。

### 正确配置示例

```rust
// 必须保持不带 .wrap() 的状态
let paragraph = Paragraph::new(text)
    .style(Style::default().bg(palette.background).fg(palette.text))
    .scroll((render_scroll as u16, 0));
```

## 维护注意事项

1. **新组件宽度校验**：
   - 在添加新的会话组件（如 Tool Call 预览、附件摘要等）时，必须确保其生成的每行文本宽度不会超过容器宽度，或者使用专用的 `shorten()` 函数进行截断。
2. **文本宽度感知**：
   - 任何会导致垂直高度增加的改动，都必须同步反应在 `total_lines` 的统计逻辑中。
3. **样式与 Padding**：
   - `decorate_card_lines` 等负责背景填充的函数会增加左右 Padding，计算 `body_width` 时务必预留出这些空间，确保内部文字不会因为 Padding 挤压而触发超出宽度的边缘情况。

## 故障排查

如果再次发现消息无法滚动到底部或滚动时有"死区"（滚动无响应的区域）：
1. 检查 `src/app/render/render_chat.rs` 中是否意外引入了 `.wrap()`。
2. 检查是否有新的 UI 元素（如 Tool Result 标题）生成的行宽超过了 `content_width` 但未被截断。
3. 验证 `messages_text` 函数中对 `total_lines` 的累加逻辑是否覆盖了所有新增的卡片装饰行。
4. **虚拟化渲染**：检查 `render_scroll` 的计算是否正确。在虚拟化渲染中，如果第一个可见块在 `scroll` 之前开始，需要正确计算 `render_scroll = scroll - first_block_start`；如果第一个可见块在 `scroll` 之后开始，需要添加空白行填充。
