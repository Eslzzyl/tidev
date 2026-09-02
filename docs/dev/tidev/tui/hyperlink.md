# OSC-8 超链接与 URL 感知换行

> 日期：2026-08-07
> 范围：`crates/tidev-tui` 聊天消息列表渲染管线
> 状态：实现完成并通过测试；功能尚未在真实会话中确认生效（见"已知问题"）

## 实现概要

参考 codex（`codex-rs/tui/src/terminal_hyperlinks.rs`、`wrapping.rs`、`markdown_render.rs`）移植了两项能力：

### OSC-8 超链接

- `crates/tidev-tui/src/hyperlink.rs`：`HyperlinkLine`/`HyperlinkRange` 元数据载体、`mark_buffer_hyperlinks` 帧渲染后注入 `\x1b]8;;url\x07…\x1b]8;;\x07`、裸 URL 检测（含中文场景）、跨行重映射（宽字符安全）、目标过滤（仅 http/https）。
- 注入调用点：`components/chat/render/mod.rs` 的 `render_messages`。
- 生效范围：用户消息、模型回复/思考内容、工具调用与输出、系统/错误消息、子代理卡、表格；明确排除代码块/内联代码、diff 内容、本地路径链接、mailto/ftp。
- 渲染管线从 `Vec<Line>` 切换为 `Vec<HyperlinkLine>`（`render_cache.rs` LRU 值类型随之调整）。
- 复制安全：`components/selection.rs` 的 `extract_row_text` 剥离 OSC-8 序列。

### URL 感知换行

- `markdown/wrap.rs`：`adaptive_wrap_line` 三路分发（无 URL / 混合 / 纯 URL），URL 整体移动。
- 两处有意适配（与 codex 不同，均有测试锁定）：超宽 URL 硬断而非输出超宽行；`mark_buffer_hyperlinks` 直接行列映射而非 scratch-buffer 重渲染。

## 已知问题

用户实测（三个编译版本行为一致）时所有 URL 形态表现与终端默认行为相同，诊断为：**消息未进入会话**（停留在 composer 输入框或排队预览区，纯文本渲染、无 OSC-8）时，渲染差异不可见；叠加终端自动 URL 检测掩盖了 OSC-8 的存在。跨行长 URL 第二行无链接，恰是 OSC-8 覆盖范围——可据此判断序列是否输出。

## 验证清单

1. 确认终端响应 OSC-8：`printf '\e]8;;https://example.com\x07open me\e]8;;\x07\n'`，"open me" 非 URL 形态，hover 有提示即支持。
2. 消息必须真正进入会话（聊天主体出现带 `┃` 前缀的卡片）。
3. 发送 `测试 [链接](https://example.com/a)`，预期渲染为 `测试 链接 (https://example.com/a)` 且 label 与 URL 均可 hover；窗口调窄发长 URL，hover 跨行第二行应有链接提示。