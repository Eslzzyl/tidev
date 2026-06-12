# Windows TUI Ghost Characters (尾随单元格残留)

## 症状

tidev TUI 的消息区域刷新时，在 Windows 平台（Windows Terminal、WezTerm、Alacritty 等所有终端模拟器）上经常遗留一些鬼影字符。macOS 上无此问题。

## 根因

### 第一层：字符宽度差异

终端中 CJK 字符（如 `你`、`好`）、emoji 等占 2 个单元格宽度，但 ratatui 的 Paragraph 渲染器（`render_line`）只设置**首单元格**，**尾随单元格保持为 `Cell::EMPTY`**。

### 第二层：diff 正确处理了

v0.30.0 的 buffer diff 算法通过 `invalidated` 机制正确处理了 CJK→单宽度字符变化时的尾随单元格清除：

```rust
// ratatui-core 0.1.0 / buffer.rs : diff()
let mut invalidated: usize = 0;
// ...
if !current.skip && (current != previous || invalidated > 0) && to_skip == 0 {
    updates.push((x, y, &next_buffer[i]));
}
// ...
let affected_width = cmp::max(current.width(), previous.width());
invalidated = cmp::max(affected_width, invalidated).saturating_sub(1);
```

当一个宽字符被替换为窄字符时，`invalidated` 被设为 `max(新宽度, 旧宽度) - 1`，强制 emit 尾随单元格（写空格）来清除旧字符残留。

### 第三层：后端 draw 方法的光标跟踪 bug（真正原因）

清除空格的指令在 **ratatui-crossterm 后端**执行时被写到了**错误的位置**：

```rust
// ratatui-crossterm : Backend::draw()
let mut last_pos: Option<Position> = None;
for (x, y, cell) in content {
    // 关键：只有非相邻才 MoveTo
    if !matches!(last_pos, Some(p) if x == p.x + 1 && y == p.y) {
        queue!(self.writer, MoveTo(x, y))?;
    }
    last_pos = Some(Position { x, y });
    queue!(self.writer, Print(cell.symbol()))?;
}
```

| 步骤 | 动作 | 光标实际位置 | last_pos 记录 |
|------|------|-------------|--------------|
| 1 | `Print("你")` 在 (x, y) | (x+2, y) | (x, y) |
| 2 | 尾随格需要被清除，位置 (x+1, y) | — | — |
| 3 | 检查 `x+1 == last_pos.x + 1` → **true** | — | — |
| 4 | **跳过 MoveTo**，`Print(" ")` | (x+2, y) 写了空格（错误！） | — |

因为 `last_pos` 只记录单元格坐标 (x, y)，而打印双宽度字符后光标实际在 (x+2, y)，所以尾随格 (x+1, y) 被错误地判定为"相邻"，没有发 MoveTo，空格写到了 (x+2, y) 而非 (x+1, y)。旧字符右半从未被清除 → **鬼影**。

## 上游修复

### PR #2517

https://github.com/ratatui/ratatui/pull/2517

**标题**: `fix(backend): correct cursor positioning for wide characters in draw`

**作者**: zensh

**内容**: 将 `last_pos`（单元格坐标）替换为 `next_pos`（预测的光标实际位置），通过 `cell.cell_width()` 正确计算多宽度字符后的光标位置：

```rust
// PR #2517 的方案
let mut next_pos: Option<Position> = None;
for (x, y, cell) in content {
    // 只有光标实际位置不匹配才 MoveTo
    if !matches!(next_pos, Some(p) if x == p.x && y == p.y) {
        queue!(self.writer, MoveTo(x, y))?;
    }
    // 预测打印后的光标位置
    let width = cell.cell_width();
    next_pos = Some(Position {
        x: x + width,
        y,
    });
    queue!(self.writer, Print(cell.symbol()))?;
}
```

**影响范围**: 同时修复了 crossterm、termion、termwiz 三个后端的同类问题。

**合并时间**: 2026-04-29

### 发布状态

| 版本 | 是否包含修复 |
|------|------------|
| v0.30.0 (tidev 当前版本) | ❌ |
| v0.30.1 (2026-06-05) | ❌ |
| main branch | ❌（截至 2026-06-12 未合并） |

PR #2517 合并到 main 后也未被 cherry-pick 到任何 release 分支。

## 相关 issue

| Issue | 状态 | 说明 |
|-------|------|------|
| [#2213](https://github.com/ratatui/ratatui/issues/2213) | OPEN（2025-11） | 同根因：macOS 上 crossterm 的残留文字问题。用户确认换 Termwiz 后端就好了 |
| [#2186](https://github.com/ratatui/ratatui/issues/2186) | 已关闭 | Paragraph 滚动时的鬼影，根因不同，已在 0.30.0-beta 修复 |
| [crossterm#164](https://github.com/crossterm-rs/crossterm/issues/164) | OPEN（2019） | `ClearType::All` 在 Windows 上不清除滚动缓冲区 |
| [#1745](https://github.com/ratatui/ratatui/issues/1745) | 与 PR #2517/Fixed | CJK 字符间出现多余空白 |

## 为什么只在 Windows 出现

macOS 终端（Terminal.app、iTerm2、kitty）在收到单宽度字符覆盖时，会**自动清除右侧相邻格**的残留内容。Windows 上的 ConPTY 层不做这个处理，所以 ratatui-crossterm 后端的定位缺陷直接暴露为可见鬼影。

## 潜在修复方式（待处理）

### 推荐：上游 PR 合并后升级

```toml
# crates/tidev-tui/Cargo.toml
ratatui = "0.30.2"  # 或包含 PR #2517 的版本
```

### 备选：git 依赖 pin 到包含修复的 commit

```toml
ratatui = { git = "https://github.com/ratatui/ratatui", rev = "<包含 PR #2517 的 commit hash>" }
```

### 不推荐：tidev 层 workaround

在 tidev 层面可以规避，但代价是：
- 对内容区做额外全量重绘（性能开销 + 可能闪烁）
- 或手动拦截后端 draw 修复光标跟踪（侵入性强，维护成本高）

建议等待上游 PR #2517 合入发布版本后升级即可。
