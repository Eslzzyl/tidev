# Suggestion Backend Architecture

## 现状

三个弹窗（command palette / @-mention / snippet）的匹配排序逻辑所处位置不一致：

| 组件 | 计算/排序 | 状态 | 渲染 |
|------|-----------|------|------|
| @-mention | ✅ `tidev_search::FileSearchIndex::search()` | `AtMentionState`（TUI） | TUI |
| command palette | ❌ `command_palette.rs`（TUI） | `CommandPaletteState`（TUI） | TUI |
| snippet | ❌ `snippet.rs`（TUI） | `SnippetState`（TUI） | TUI |

## 目标

将 command palette 和 snippet 的匹配/排序逻辑也搬到后端，与 @-mention 一致。

## 方案

### 1. `tidev-core` 新增 `suggestions` 模块

**Command palette 后端 API：**

```rust
// tidev-core/src/suggestions/commands.rs

pub struct CommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
    pub action: CommandAction,
}

pub struct ScoredCommand {
    pub spec: &'static CommandSpec,
    pub score: i32,
}

pub fn suggest_commands(fragment: &str) -> Vec<ScoredCommand>;
```

`Runtime` 暴露方法：

```rust
impl Runtime {
    pub fn suggest_commands(&self, fragment: &str) -> Vec<ScoredCommand>;
    pub fn suggest_snippets(&self, fragment: &str) -> Vec<ScoredSnippet>;
}
```

**Snippet 后端 API：**

```rust
// tidev-core/src/suggestions/snippets.rs

pub struct ScoredSnippet {
    pub text: String,
    pub matched_indices: Vec<usize>,
    pub score: i64,
}

pub struct SnippetStore {
    // 从 ~/.config/tidev/snippets.txt 和 .tidev/snippets.txt 加载
}

impl SnippetStore {
    pub fn new(config_dir: &Path, workspace_root: &Path) -> Self;
    pub fn suggest(&self, query: &str) -> Vec<ScoredSnippet>;
}
```

### 2. TUI 侧简化

**CommandPaletteState：** 去掉 `COMMANDS` 静态列表和 `suggestions()` 模糊匹配，改为调用 `runtime.suggest_commands()`。只保留：

```rust
pub struct CommandPaletteState {
    pub visible: bool,
    pub suggestions: Vec<CommandSuggestion>,
    pub selected_index: usize,
}
```

**SnippetState：** 去掉文件加载和 `candidates()` 模糊匹配，改为调用 `runtime.suggest_snippets()`。只保留：

```rust
pub struct SnippetState {
    pub visible: bool,
    pub suggestions: Vec<ScoredSnippet>,
    pub selected_index: usize,
}
```

### 3. 已有模式参考

`AtMentionState`（`at_mention.rs`）已经走了这个模式：

```rust
// TUI 侧只做状态管理和委托
let file_suggestions = index.search(&query);  // 后端计算
self.suggestions = file_suggestions.into_iter().map(AtMentionSuggestion::from).collect();
```

新实现必须跟随相同模式。

## 优先级

低。目前 TUI 侧的实现功能完整，只是位置不对。建议在后续重构中处理，与 @-mention 统一。
