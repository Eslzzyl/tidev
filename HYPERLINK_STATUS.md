# 超链接与 URL 感知换行 —— 实现与问题摘要

> 日期：2026-08-07
> 范围：`crates/tidev-tui`（聊天消息列表渲染管线）
> 状态：实现已完成并通过全部测试；功能尚未在真实会话中确认生效（见"当前问题状态"）

## 一、今日实现的内容

参考 codex（`../codex/codex-rs/tui/src/terminal_hyperlinks.rs`、`wrapping.rs`、`markdown_render.rs`）移植了两项能力：

### 1. OSC-8 超链接

- 新增模块 `crates/tidev-tui/src/hyperlink.rs`：
  - `HyperlinkLine` / `HyperlinkRange` —— 行 + 显示列范围 + 目标 URL 的元数据载体
  - `mark_buffer_hyperlinks` —— 帧渲染后向 `frame.buffer_mut()` 逐格注入 `\x1b]8;;url\x07…\x1b]8;;\x07`（依赖"预换行一逻辑行一屏行"不变量做直接行列映射）
  - `web_links_in_text` / `annotate_web_urls_in_line` —— 裸 URL 检测（含中文场景，见"已修复的问题"）
  - `remap_wrapped_line` —— 换行后链接列范围跨行重映射（宽字符安全）
  - `web_destination` / `osc8_hyperlink` / `strip_osc8` —— 目标过滤（仅 http/https）、序列生成、复制剥离
- 注入调用点：`crates/tidev-tui/src/components/chat/render/mod.rs` 的 `render_messages`（Paragraph 绘制后）
- 生效范围（均走聊天渲染管线）：
  - 用户消息：`render/cards.rs` `render_user_shell_card` → `render_text_body_lines` → markdown
  - 模型回复 / 思考内容：`render/cards.rs` `render_assistant_cards`、`render/thinking.rs`
  - 工具调用与输出：`render/blocks.rs`、`tool/mod.rs`、`tool/web.rs`、`tool/subagent.rs`
  - 系统 / 错误消息、重试提示、子代理卡、表格（`markdown/table.rs`）
  - 明确排除：代码块 / 内联代码、diff 内容、本地路径链接、mailto/ftp（与 codex 语义一致）
- 渲染管线从 `Vec<Line>` 切换为 `Vec<HyperlinkLine>`：`render_cache.rs`（LRU 值类型）、`render/blocks.rs`、`render/cards.rs`、`render/utils.rs`（`decorate_card_lines` 按前缀宽度位移链接列）、`render/thinking.rs`、`render/subagent.rs`
- markdown Writer 注解化：`markdown/mod.rs`（`push_text_spans` 三路分发、`pop_link` 注解可见 URL、`flush_current_line` remap；缓存改为 `Arc<MarkdownRender>`，`Deref<Target=Text>` 保持旧调用方兼容）
- 复制安全：`components/selection.rs` `extract_row_text` 剥离 OSC-8 序列（避免复制文本携带转义垃圾）

### 2. URL 感知换行

- `markdown/wrap.rs`：`adaptive_wrap_line` 三路分发（无 URL → 原样词界换行；混合 → `mixed_url_wrap_ranges`，URL 整体移动；纯 URL → URL 保留选项）
- 移植 codex 完整启发式：`is_url_like_token`、scheme 校验、裸域名 / IPv4 / localhost / 端口校验、装饰性 token 排除
- 两处有意的 tidev 适配（与 codex 的差异，均有测试锁定）：
  1. 超宽 URL 硬断而非输出超宽行（tidev 的 Paragraph 无 wrap 会截断超宽行）
  2. `mark_buffer_hyperlinks` 直接行列映射而非 scratch-buffer 重渲染
- `render/utils.rs` `wrap_text_lines` 改为 URL 感知（错误卡、重试提示、排队预览共用）

测试：全部通过（workspace 918 个测试，其中新增约 20 个：OSC-8 注入、中文 URL 检测、跨行 remap、表格位移、装饰位移端到端等）。

## 二、当前问题状态（用户实测现象与诊断）

用户实测（三个编译版本——修改前 / 反馈时 / 最新——行为完全一致，无可感知区别）：

| 现象 | 诊断 | 关键代码位置 |
|---|---|---|
| markdown 链接（`[text](url)`）显示为原始格式，未渲染 | 文本未经过 markdown 渲染管线。聊天管线三版都会渲染 markdown 链接，因此该现象只能发生在**消息未进入会话**时（停留在 composer 输入框，或排队预览） | 消息进入会话的唯一途径：`app/backend_events.rs` `BackendEvent::UserMessageCreated`（约 155 行）；发送流程：`app/actions.rs` `ChatAction::SendMessage`（约 642 行）；排队逻辑：`actions.rs` `has_active_request` → `pending_prompt_queue`（约 684 行） |
| 所有 URL（含 mailto/ftp）表现完全一致 | 终端自动 URL 检测（如 WezTerm 默认 `hyperlink_rules`）对所有 URL 形态文本附加 hover/点击效果，**不需要程序输出任何序列**；与 OSC-8 无关 | 无（终端侧行为） |
| 跨行长 URL 第二行无链接 | 终端自动检测按行匹配、不跨行；**跨行第二行恰是 OSC-8 的覆盖范围**——第二行无链接说明 OSC-8 序列未输出（文本未经过 `render_messages` 的 `mark_buffer_hyperlinks`） | `components/chat/render/mod.rs` `render_messages` |
| 中文路径（`https://example.com/文档`）不入链 | 终端自动检测不覆盖非 ASCII 路径；OSC-8 未生效时无兜底 | 同上 |
| 三个版本行为一致 | 与以上一致：测试内容从未经过本次改动的渲染管线 | — |

**结论**：功能实现已完成且单元测试覆盖充分，但用户测试时文本很可能停留在 composer 输入框或排队预览（`app/drawing.rs` `render_queued_prompts`，纯文本渲染、无 OSC-8），从未进入会话消息列表；叠加终端自动检测掩盖了差异。**待用户确认消息真正进入会话后，在支持 OSC-8 的终端（WezTerm / iTerm2 / kitty / Ghostty 等）复测。**

## 三、已修复的问题（本次会话内）

- 中文文本紧贴 URL 时链接识别失败（如 `URL：https://example.com/bare`、`见https://example.com/a`）：原 `web_links_in_text` 按空白分词后仅修剪 token 首尾标点，`URL：https://…` 被整体当做一个 token 导致 `Url::parse` 失败
  - 修复：`hyperlink.rs` `web_links_in_text` 改为在 token 内扫描 `scheme://` 起点（多字节安全回溯）+ 中文标点（`。，、；！？`）尾部修剪 + 中文括号（`（）【】》…`）配平处理；`markdown/wrap.rs` `trim_url_token` 同步支持中文标点
  - 新增测试：`hyperlink.rs` `detects_urls_glued_to_cjk_text`、`trims_cjk_trailing_punctuation`

## 四、待验证事项

1. 确认终端响应 OSC-8：`printf '\e]8;;https://example.com\x07open me\e]8;;\x07\n'` —— "open me" 非 URL 形态，hover 有提示即支持
2. 确认消息真正进入会话：发送后聊天主体区域出现带 `┃` 前缀的卡片、模型有回复（检查后端 / 模型配置是否可用）
3. 最短链路测试：发送 `测试 [链接](https://example.com/a)`，预期渲染为 `测试 链接 (https://example.com/a)` 且 label 与 URL 均可 hover；窗口调窄发长 URL，hover 跨行第二行应有链接提示

## 五、修改文件清单

新增：
- `crates/tidev-tui/src/hyperlink.rs`（核心模块）

修改：
- `crates/tidev-tui/src/markdown/wrap.rs`（URL 感知换行）
- `crates/tidev-tui/src/markdown/mod.rs`（Writer 注解化、`MarkdownRender` 缓存）
- `crates/tidev-tui/src/markdown/table.rs`（表格链接传播）
- `crates/tidev-tui/src/components/chat/render/mod.rs`（`render_messages` 注入 OSC-8）
- `crates/tidev-tui/src/components/chat/render/blocks.rs` / `cards.rs` / `thinking.rs` / `subagent.rs` / `utils.rs`（管线切换 `Vec<HyperlinkLine>`）
- `crates/tidev-tui/src/components/chat/render_cache.rs`（LRU 值类型）
- `crates/tidev-tui/src/components/chat/tool/mod.rs` / `tool/web.rs` / `tool/subagent.rs`（工具输出）
- `crates/tidev-tui/src/components/selection.rs`（复制剥离 OSC-8）
- `crates/tidev-tui/src/components/overlays/skills.rs`（缓存类型适配）
- `crates/tidev-tui/src/lib.rs`（模块注册）
