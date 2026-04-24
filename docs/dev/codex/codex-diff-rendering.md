# Codex 代码 Diff 渲染机制

> 本文档描述 tidev/codex 项目中代码差异（diff）的渲染实现原理。
> 主要针对 Rust TUI 终端界面中的 unified diff 可视化。

## 概述

Codex 使用 unified diff 格式进行代码变更展示，渲染核心位于 `codex-rs/tui/src/diff_render.rs`。该模块负责将 `FileChange` 协议消息转换为带行号、颜色高亮、语法高亮的终端文本输出。

## 核心数据结构

### DiffLineType

```rust
// codex/codex-rs/tui/src/diff_render.rs (第 105-110 行)
enum DiffLineType {
    Insert,   // 新增行 (+)
    Delete,   // 删除行 (-)
    Context,  // 上下文行 ( )
}
```

### DiffTheme

```rust
// codex/codex-rs/tui/src/diff_render.rs (第 119-123 行)
enum DiffTheme {
    Dark,   // 暗色终端
    Light,  // 亮色终端
}
```

### DiffColorLevel

```rust
// codex/codex-rs/tui/src/diff_render.rs (第 133-138 行)
enum DiffColorLevel {
    TrueColor,  // 24-bit 真彩色
    Ansi256,    // 256 色
    Ansi16,     // 16 色 (仅前景色)
}
```

### DiffRenderStyleContext

```rust
// codex/codex-rs/tui/src/diff_render.rs (第 187-192 行)
struct DiffRenderStyleContext {
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
}
```

## 渲染流程

### 1. 主入口：`render_change()`

位于 `diff_render.rs` 第 474-736 行，根据 `FileChange` 类型分发渲染：

```rust
// diff_render.rs (第 474-736 行)
fn render_change(
    change: &FileChange,
    out: &mut Vec<RtLine<'static>>,
    width: usize,
    lang: Option<&str>,
) {
    let style_context = current_diff_render_style_context();
    match change {
        FileChange::Add { content } => {
            // 整文件新增，逐行渲染为 Insert 类型
            for (i, raw) in content.lines().enumerate() {
                out.extend(push_wrapped_diff_line_inner_with_theme_and_color_level(
                    i + 1,
                    DiffLineType::Insert,
                    raw,
                    width,
                    line_number_width,
                    syntax_spans,
                    style_context.theme,
                    style_context.color_level,
                    style_context.diff_backgrounds,
                ));
            }
        }
        FileChange::Delete { content } => {
            // 整文件删除，逐行渲染为 Delete 类型
            // ... (同上，DiffLineType::Delete)
        }
        FileChange::Update { unified_diff, .. } => {
            // 解析 unified diff，遍历 hunks 和 lines
            if let Ok(patch) = diffy::Patch::from_str(unified_diff) {
                for h in patch.hunks() {
                    for l in h.lines() {
                        match l {
                            diffy::Line::Insert(text) => {
                                // Insert 行渲染
                            }
                            diffy::Line::Delete(text) => {
                                // Delete 行渲染
                            }
                            diffy::Line::Context(text) => {
                                // Context 行渲染
                            }
                        }
                    }
                }
            }
        }
    }
}
```

### 2. 核心渲染函数：`push_wrapped_diff_line_inner_with_theme_and_color_level()`

位于 `diff_render.rs` 第 838-938 行，每行渲染结构为：

```
┌──────────┬──────┬──────────────────────────────────────────┐
│  gutter  │ sign │              content                     │
│ (行号)   │ +/-  │  (语法高亮文本)                          │
└──────────┴──────┴──────────────────────────────────────────┘
```

```rust
// diff_render.rs (第 838-938 行)
fn push_wrapped_diff_line_inner_with_theme_and_color_level(
    line_number: usize,
    kind: DiffLineType,
    text: &str,
    width: usize,
    line_number_width: usize,
    syntax_spans: Option<&[RtSpan<'static>]>,
    theme: DiffTheme,
    color_level: DiffColorLevel,
    diff_backgrounds: ResolvedDiffBackgrounds,
) -> Vec<RtLine<'static>> {
    let gutter_width = line_number_width.max(1);
    let prefix_cols = gutter_width + 1;

    // 根据行类型确定样式
    let (sign_char, sign_style, content_style) = match kind {
        DiffLineType::Insert => (
            '+',
            style_sign_add(theme, color_level, diff_backgrounds),
            style_add(theme, color_level, diff_backgrounds),
        ),
        DiffLineType::Delete => (
            '-',
            style_sign_del(theme, color_level, diff_backgrounds),
            style_del(theme, color_level, diff_backgrounds),
        ),
        DiffLineType::Context => (' ', style_context(), style_context()),
    };

    let line_bg = style_line_bg_for(kind, diff_backgrounds);
    let gutter_style = style_gutter_for(kind, theme, color_level);

    // 语法高亮时保留高亮色，Delete 行叠加 DIM 修饰符
    if let Some(syn_spans) = syntax_spans {
        let styled: Vec<RtSpan<'static>> = syn_spans
            .iter()
            .map(|sp| {
                let style = if matches!(kind, DiffLineType::Delete) {
                    sp.style.add_modifier(Modifier::DIM)
                } else {
                    sp.style
                };
                RtSpan::styled(sp.content.clone().into_owned(), style)
            })
            .collect();

        let available_content_cols = width.saturating_sub(prefix_cols + 1).max(1);
        let wrapped_chunks = wrap_styled_spans(&styled, available_content_cols);

        // 组装行：gutter + sign + content
        // 续行缩进对齐 gutter 宽度
        // ...
    }
}
```

### 3. 智能折行：`wrap_styled_spans()`

位于 `diff_render.rs` 第 951-1020 行，处理超长行折行：

```rust
// diff_render.rs (第 951-1020 行)
fn wrap_styled_spans(spans: &[RtSpan<'static>], max_cols: usize) -> Vec<Vec<RtSpan<'static>>> {
    let mut result: Vec<Vec<RtSpan<'static>>> = Vec::new();
    let mut current_line: Vec<RtSpan<'static>> = Vec::new();
    let mut col: usize = 0;

    for span in spans {
        let style = span.style;
        let text = span.content.as_ref();
        let mut remaining = text;

        while !remaining.is_empty() {
            // 按 Unicode 字符宽度计算
            let mut byte_end = 0;
            let mut chars_col = 0;

            for ch in remaining.chars() {
                // Tab 扩展为 4 列
                let w = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 0 });
                if col + chars_col + w > max_cols {
                    break;
                }
                byte_end += ch.len_utf8();
                chars_col += w;
            }

            // 超宽字符强制换行，避免无限循环
            if byte_end == 0 {
                if !current_line.is_empty() {
                    result.push(std::mem::take(&mut current_line));
                }
                let ch_len = remaining.chars().next().unwrap().len_utf8();
                current_line.push(RtSpan::styled(remaining[..ch_len].to_string(), style));
                col = ch.width().unwrap_or(if ch == '\t' { TAB_WIDTH } else { 1 });
                remaining = &remaining[ch_len..];
                continue;
            }

            let (chunk, rest) = remaining.split_at(byte_end);
            current_line.push(RtSpan::styled(chunk.to_string(), style));
            col += chars_col;
            remaining = rest;

            if col >= max_cols {
                result.push(std::mem::take(&mut current_line));
                col = 0;
            }
        }
    }

    if !current_line.is_empty() || result.is_empty() {
        result.push(current_line);
    }

    result
}
```

## 主题色板

### 硬编码调色板

位于 `diff_render.rs` 第 60-75 行：

```rust
// Dark theme (暗色终端)
const DARK_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (33, 58, 43);    // #213A2B 绿色
const DARK_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (74, 34, 29);    // #4A221D 红色

// Light theme (亮色终端 - GitHub 风格)
const LIGHT_TC_ADD_LINE_BG_RGB: (u8, u8, u8) = (218, 251, 225); // #dafbe1
const LIGHT_TC_DEL_LINE_BG_RGB: (u8, u8, u8) = (255, 235, 233); // #ffebe9
const LIGHT_TC_ADD_NUM_BG_RGB: (u8, u8, u8) = (172, 238, 187);  // #aceebb
const LIGHT_TC_DEL_NUM_BG_RGB: (u8, u8, u8) = (255, 206, 203);  // #ffcecb
const LIGHT_TC_GUTTER_FG_RGB: (u8, u8, u8) = (31, 35, 40);      // #1f2328

// 256 色调色板索引
const DARK_256_ADD_LINE_BG_IDX: u8 = 22;
const DARK_256_DEL_LINE_BG_IDX: u8 = 52;
const LIGHT_256_ADD_LINE_BG_IDX: u8 = 194;
const LIGHT_256_DEL_LINE_BG_IDX: u8 = 224;
const LIGHT_256_ADD_NUM_BG_IDX: u8 = 157;
const LIGHT_256_DEL_NUM_BG_IDX: u8 = 217;
const LIGHT_256_GUTTER_FG_IDX: u8 = 236;
```

### 样式选择逻辑

位于 `diff_render.rs` 第 1136-1304 行：

```rust
// 背景样式 - 上下文行不设置背景
fn style_line_bg_for(kind: DiffLineType, diff_backgrounds: ResolvedDiffBackgrounds) -> Style {
    match kind {
        DiffLineType::Insert => diff_backgrounds.add.map_or_else(Style::default, |bg| Style::default().bg(bg)),
        DiffLineType::Delete => diff_backgrounds.del.map_or_else(Style::default, |bg| Style::default().bg(bg)),
        DiffLineType::Context => Style::default(),
    }
}

// Insert 内容样式
fn style_add(theme: DiffTheme, color_level: DiffColorLevel, diff_backgrounds: ResolvedDiffBackgrounds) -> Style {
    match (theme, color_level, diff_backgrounds.add) {
        (_, DiffColorLevel::Ansi16, _) => Style::default().fg(Color::Green),
        (DiffTheme::Light, _, Some(bg)) => Style::default().bg(bg),
        (DiffTheme::Dark, _, Some(bg)) => Style::default().fg(Color::Green).bg(bg),
        (DiffTheme::Light, _, None) => Style::default(),
        (DiffTheme::Dark, _, None) => Style::default().fg(Color::Green),
    }
}

// Delete 内容样式 - 同上，红色
fn style_del(...) -> Style { ... }
```

## 主题检测与适配

### 终端背景检测

位于 `diff_render.rs` 第 1030-1043 行：

```rust
fn diff_theme_for_bg(bg: Option<(u8, u8, u8)>) -> DiffTheme {
    if let Some(rgb) = bg && is_light(rgb) {
        return DiffTheme::Light;
    }
    DiffTheme::Dark
}

fn diff_theme() -> DiffTheme {
    diff_theme_for_bg(default_bg())
}
```

### Windows Terminal 特殊处理

位于 `diff_render.rs` 第 1089-1115 行：

```rust
fn diff_color_level_for_terminal(
    stdout_level: StdoutColorLevel,
    terminal_name: TerminalName,
    has_wt_session: bool,
    has_force_color_override: bool,
) -> DiffColorLevel {
    // Windows Terminal 检测到 WT_SESSION，自动提升到 TrueColor
    if has_wt_session && !has_force_color_override {
        return DiffColorLevel::TrueColor;
    }

    let base = match stdout_level {
        StdoutColorLevel::TrueColor => DiffColorLevel::TrueColor,
        StdoutColorLevel::Ansi256 => DiffColorLevel::Ansi256,
        StdoutColorLevel::Ansi16 | StdoutColorLevel::Unknown => DiffColorLevel::Ansi16,
    };

    // 非 WT_SESSION 环境下，已知 Windows Terminal 也提升到 TrueColor
    if stdout_level == StdoutColorLevel::Ansi16
        && terminal_name == TerminalName::WindowsTerminal
        && !has_force_color_override
    {
        DiffColorLevel::TrueColor
    } else {
        base
    }
}
```

## 语法高亮策略

### Update Diffs (Hunk 级高亮)

位于 `diff_render.rs` 第 607-621 行：

```rust
// 每个 hunk 作为整体高亮，保持 syntect 解析器状态跨行
let hunk_syntax_lines = diff_lang.and_then(|language| {
    let hunk_text: String = h.lines()
        .iter()
        .map(|line| match line {
            diffy::Line::Insert(text)
            | diffy::Line::Delete(text)
            | diffy::Line::Context(text) => *text,
        })
        .collect();
    let syntax_lines = highlight_code_to_styled_spans(&hunk_text, language)?;
    // 确保高亮行数与 hunk 行数匹配
    (syntax_lines.len() == h.lines().len()).then_some(syntax_lines)
});
```

### Add/Delete (文件级高亮)

位于 `diff_render.rs` 第 483-513 行：

```rust
FileChange::Add { content } => {
    // 整文件内容一次性高亮
    let syntax_lines = lang.and_then(|l| highlight_code_to_styled_spans(content, l));
    for (i, raw) in content.lines().enumerate() {
        let syn = syntax_lines.as_ref().and_then(|sl| sl.get(i));
        // ... 渲染逻辑
    }
}
```

### Delete 行特殊处理

位于 `diff_render.rs` 第 882-887 行：

```rust
let styled: Vec<RtSpan<'static>> = syn_spans.iter().map(|sp| {
    let style = if matches!(kind, DiffLineType::Delete) {
        // Delete 行叠加 DIM 修饰符，语法色不会覆盖删除提示
        sp.style.add_modifier(Modifier::DIM)
    } else {
        sp.style
    };
    RtSpan::styled(sp.content.clone().into_owned(), style)
}).collect();
```

## 文件清单

| 文件路径 | 描述 |
|----------|------|
| `codex/codex-rs/tui/src/diff_render.rs` | 核心 Diff 渲染模块 (~2480 行) |
| `codex/codex-rs/tui/src/render/highlight.rs` | 语法高亮实现 |
| `codex/codex-rs/tui/src/terminal_palette.rs` | 终端调色板工具 |
| `codex/codex-rs/tui/src/color.rs` | 颜色工具函数 |

## 相关协议

`FileChange` 协议定义位于 `codex-protocol` crate，定义三种变更类型：

- `FileChange::Add { content }` - 新增文件
- `FileChange::Delete { content }` - 删除文件
- `FileChange::Update { unified_diff, move_path }` - 修改文件（含 unified diff）