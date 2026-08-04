# tidev-tui 架构设计

> **2026-08-04 实现状态：**
> `tidev-tui` 已完成新架构迁移并成为唯一 TUI crate。Runtime、事件订阅、审批请求、
> 组件树和面板均由当前 crate 自己实现，旧 `tidev-tui-old` 已删除。

## 铁律

操作本项目【任何】文件时，必须严格遵守下面的铁律，没有任何例外：

- 总是使用简体中文提交信息
- **不得擅自对实现进行任何简化**。没有例外。任何违反都会导致工作前功尽弃。如果你觉得做不到，直接说然后停下
- **严禁使用任何 subagent**。没有例外

## 1. 现状与问题

### 1.1 规模

- 27,926 行代码，79 个 `.rs` 文件
- 24 个 `impl App` 块散落在 17 个文件中
- 多个 1000-2500 行的超大文件

### 1.2 核心问题

**问题 A：App 是上帝对象**

```rust
// 24 个 impl App 块散布在：
lib.rs                          → 构造、run()、事件循环
render/render.rs                → pub(crate) fn render()
render/chat_render/mod.rs       → 消息渲染
render/chat_dialog/dialogs.rs   → 对话框渲染
render/chat_dialog/panels.rs    → 面板渲染
input/event/keyboard.rs         → 键盘事件
input/event/mouse.rs            → 鼠标事件
input/event/actions.rs          → 动作处理
input/event/panels.rs           → 面板按键
input/event/scroll.rs           → 滚动处理
input/event/request.rs          → 请求处理
input/event/completion.rs       → 补全
// ... 等等
```

所有行为都挂在 `App` 上，导致：
- 难以追踪 `App` 有哪些方法
- 隐式耦合：所有 `impl App` 块通过 `use super::*` 引入全部模块
- 数据流不清晰

**问题 B：一个功能散落在三个目录**

```
ui/theme_panel.rs           → ThemePanelState (数据)
render/chat_dialog/panels.rs → App::render_theme_panel() (渲染)
input/event/panels.rs       → App::handle_theme_panel_key() (按键)
```

改一个面板需要横跨三个目录，认知负担极重。

**问题 C：超大文件**

| 文件 | 行数 | 问题 |
|---|---|---|
| `render/chat_render/mod.rs` | 2,453 | 消息渲染 + 布局逻辑全部混在一起 |
| `render/chat_render/tool.rs` | 1,653 | Tool call 渲染 |
| `render/chat_dialog/panels.rs` | 1,604 | 所有面板渲染集中在一个文件 |
| `input/event/mouse.rs` | 1,351 | 鼠标事件处理 |
| `render/chat_dialog/dialogs.rs` | 1,194 | 所有对话框渲染 |
| `render/render.rs` | 1,123 | 主 render() 函数 + 所有辅助组件 |

## 2. 设计原则

1. **每个组件自包含** — 状态、渲染、事件处理在一个地方，不分散
2. **不删功能** — 不简化、不合并、不删减任何现有行为
3. **性能不退化** — 虚拟化、缓存、dirty tracking 等现有优化全部保留，只强化不削弱
4. **组件通过 Action 通信** — 不直接访问其他组件的内部状态
5. **Runtime 是异步的唯一权威** — 组件不直接 spawn task，通过 Action 触发异步操作

## 3. 架构总览

实现集中在 `tidev-tui` crate 中。它通过 `tidev-core::Runtime` 获取后端能力，
组件之间通过 `Action` 通信，不再依赖旧 TUI crate。

```
┌─────────────────────────────────────────────┐
│                  main()                      │
├─────────────────────────────────────────────┤
│  Tui (终端层)                                │
│  • 持有 Terminal<CrosstermBackend>           │
│  • 事件轮询（crossterm + Runtime 双通道）     │
│  • 调用 App::update() / App::draw()           │
├─────────────────────────────────────────────┤
│  App (根组件)                                │
│  • 持有 Runtime（异步资源最终权威）            │
│  • 管理组件树                                │
│  • Action 路由 + 异步命令执行                 │
│  • `last_notice` 底部状态文字                 │
│  • `toast` 右上角瞬态弹窗（自动过期）          │
├─────────────────────────────────────────────┤
│  Component Tree                              │
│  MessageList + Composer (由 App 直接持有)     │
│  OverlayStack → 所有浮层（统一管理）           │
│  StatusBar (含 last_notice 状态文字)          │
└─────────────────────────────────────────────┘
```

```
┌─────────────────────────────────────────────┐
│                  main()                      │
├─────────────────────────────────────────────┤
│  Tui (终端层)                                │
│  • 持有 Terminal<CrosstermBackend>           │
│  • 事件轮询（crossterm + Runtime 双通道）     │
│  • 调用 App::update() / App::draw()           │
├─────────────────────────────────────────────┤
│  App (根组件)                                │
│  • 持有 Runtime（异步资源最终权威）            │
│  • 管理组件树                                │
│  • Action 路由 + 异步命令执行                 │
│  • `last_notice` 底部状态文字                 │├─────────────────────────────────────────────┤
│  Component Tree                              │
│  MessageList + Composer (由 App 直接持有)     │
│  OverlayStack → 所有浮层（统一管理）           │
│  StatusBar                                   │
└─────────────────────────────────────────────┘
```

## 4. 组件树

```
App
├── MessageList                   ← 主要聊天消息列表（虚拟化 + 渲染缓存）
├── Composer                      ← 输入框（@mention、snippet、图片粘贴）
│                                   由 App 直接持有，可在 Welcome 和 Chat 页复用
├── OverlayStack                   ← 所有浮层，按 z-order 排列
│   ├── ImageViewer                ← 最顶层，Esc 关闭
│   ├── PanelLauncher              ← Ctrl+P 打开
│   ├── PermissionDialog           ← 工具执行权限
│   ├── QuestionDialog             ← LLM 提问
│   ├── WorkspaceBoundaryDialog    ← 工作区边界确认
│   ├── SensitiveFileDialog        ← 敏感文件访问
│   ├── ForkConfirmDialog          ← 分支确认
│   ├── UndoConfirmDialog          ← 撤销确认
│   ├── ConnectDialog              ← 连接 provider
│   ├── RenameDialog               ← 重命名会话
│   ├── SessionPanel               ← 会话列表
│   ├── SettingsPanel              ← 设置
│   ├── ThemePanel                 ← 主题选择
│   ├── ModelPanel                 ← 模型选择
│   ├── AgentsPanel                ← Agent 列表
│   ├── SkillsPanel                ← Skill 浏览
│   ├── SearchPanel                ← 搜索
│   ├── MessagePanel               ← 消息详情
│   └── Notifications              ← Toast 通知
└── StatusBar                      ← 底部状态栏

> **注意：** `CommandPalette`（`/command` 建议弹窗）不是独立的 Overlay，而是 `Composer` 内部的
> 三个内联弹窗之一（另外两个是 @mention 和 snippet），在 Composer 内部渲染为输入框上方的
> 弹出列表，不经过 OverlayStack。
```

### 4.1 关于 Panel vs Dialog

旧代码中 panel 和 dialog 的区分是**语义分类**，技术上没有差异：
- 都是 `Option<State>` 字段
- 都渲染为居中弹窗（`centered_rect()`）
- 都全拦截键盘输入（唯一例外：消息滚动）

新架构不做这种区分，统一为 **OverlayStack**。每个浮层组件在 `draw()` 中自行决定绘制位置（居中、靠下、靠边等）。架构层面只关心 z-order 和事件路由。

## 5. Component Trait

```rust
/// 每个组件的初始化上下文（不变资源）
pub(crate) struct InitContext<'a> {
    pub config: &'a tidev_config::AppConfig,
    pub auth: &'a tidev_config::AuthStore,
    pub workspace_root: &'a Path,
}

/// 每帧传入的共享上下文（只读）
pub(crate) struct DrawContext<'a> {
    pub palette: ThemePalette,
    pub focused: bool,
    pub chat_context: Option<&'a ChatContext>,
}

/// Action 处理时的上下文
pub(crate) struct UpdateContext<'a> {
    pub runtime: &'a mut tidev_core::Runtime,
    pub palette: &'a ThemePalette,
}

pub(crate) trait Component {
    /// 初始化（config、workspace_root 等不变资源）
    fn init(&mut self, ctx: &InitContext) -> Result<()> { Ok(()) }

    /// 键盘事件：返回 Some(action) 表示已消费
    fn handle_key_event(&mut self, key: KeyEvent) -> Option<Action> { None }

    /// 鼠标事件：传入 area 用于 hit-test
    fn handle_mouse_event(&mut self, mouse: MouseEvent, area: Rect) -> Option<Action> { None }

    /// 处理 Action，返回可能的后续 Action（链式处理）
    fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> { vec![] }

    /// 纯渲染（不修改状态）
    fn draw(&mut self, frame: &mut Frame, rect: Rect, ctx: &DrawContext);

    // ── Dirty tracking ──

    /// 是否需要重新渲染
    fn is_dirty(&self) -> bool { true }

    /// 标记为已渲染
    fn mark_clean(&mut self) {}

    // ── Overlay 支持 ──

    /// 是否是浮层组件（放在 OverlayStack 中）
    fn is_overlay(&self) -> bool { false }

    /// 浮层的 z-order（值越大越靠上）
    fn z_order(&self) -> u8 { 0 }

    /// 是否阻塞下层组件的事件传递
    fn blocks_input(&self) -> bool { false }
}
```

### 5.1 关键设计决策

| 决策 | 理由 |
|---|---|
| `handle_key_event` 返回 `Option<Action>` | `None` = "未消费，传给下一个"——精确实现优先链 |
| `update` 接受 `&Action`（借用） | 同一个 Action 可广播给多个组件（如 ThemeChanged → Chat + 所有面板） |
| `draw` 不返回 Action | 渲染不产生副作用，避免 borrow checker 问题 |
| 不设 `handle_events` 泛型方法 | TUI 需要分别处理键盘/鼠标（鼠标需要 Rect 做 hit-test） |
| 粒度控制在**面板/对话框级别** | 不把 Component trait 用在每个小 widget 上，避免虚函数开销 |

## 6. Action 分层枚举

### 6.1 领域子 Action

```rust
/// 会话管理
pub(crate) enum SessionAction {
    Create,
    Select(Uuid),
    Delete(Uuid),
    Rename(Uuid, String),
    Fork(Uuid),
    Undo,
    Redo,
    Compact,
    // 异步操作返回
    Loaded(Result<Vec<SessionSummary>>),
    Deleted(Result<()>),
}

/// 聊天操作
pub(crate) enum ChatAction {
    SendMessage { text: String, attachments: Vec<Attachment> },
    CancelGeneration,
    ScrollTo(Uuid),
    ScrollDelta(isize),
    ToggleToolResult(Uuid),
    ToggleImage(Uuid),
    // 流式渲染
    StreamDelta { message_id: Uuid, delta: String },
    StreamEnd(Uuid),
}

/// 浮层管理
pub(crate) enum OverlayAction {
    Open(OverlayKind),
    Close(OverlayKind),
    CloseTop,
    CloseAll,
}

/// 主题
pub(crate) enum ThemeAction {
    Set(ThemeName),
    Toggle,
    Preview(ThemeName),    // 预览不改配置
}

/// 面板启动器
pub(crate) enum LauncherAction {
    Open,
    Close,
    Select(usize),
    Execute(PanelAction),
}

/// 搜索
pub(crate) enum SearchAction {
    SwitchProvider(String),
    SaveApiKey { provider: String, key: String, is_cx: bool },
}

/// LLM 提供商连接
pub(crate) enum ConnectAction {
    SaveApiKey { provider_id: String, key: String },
    PruneOrphans,
}

/// 异步命令结果
pub(crate) enum CommandAction {
    Response { id: Uuid, result: Result<Box<[u8]>> },
}
```

### 6.2 顶层 Action

```rust
pub(crate) enum Action {
    // ── 生命周期 ──
    Tick,
    Render,
    Resize(u16, u16),
    Quit,

    // ── 领域 ──
    Session(SessionAction),
    Chat(ChatAction),
    Overlay(OverlayAction),
    Theme(ThemeAction),
    Launcher(LauncherAction),
    Search(SearchAction),
    Connect(ConnectAction),
    Command(CommandAction),
    // ── 内部 ──
    Noop,
    Error(String),
}
```

分层的好处：
- 编译速度：修改一个子 Action 不会触发整个枚举的重新编译
- match 时可读性高：`Action::Chat(ChatAction::ScrollDelta(n))`
- 可在不同抽象层级传递

## 7. 事件路由模型

### 7.1 路由策略

```
用户输入 → Tui (轮询 crossterm)
          │
          ▼
     App::handle_event(event)
          │
          ├── 全局快捷键 (Ctrl+D=退出, Ctrl+X=Leader)
          │
          ├── Leader 模式 → leader key 处理器
          │
          ├── 消息滚动 → 即使在浮层中也工作（硬编码例外）
          │
          ├── OverlayStack 顶层优先
          │   └── 顶层 handle_key_event() return Some → 消费
          │   └── return None → 下一层
          │   └── blocks_input() == true → 停止传递
          │
          ├── MessageList (键盘滚动、展开/折叠)
          │
          └── Composer (输入、补全、粘贴)
```

### 7.2 App 层的显式路由

```rust
impl App {
    fn handle_key_event(&mut self, key: KeyEvent) -> Result<Action> {
        // 1. 全局快捷键（不受浮层影响）
        if let Some(action) = self.handle_global_key(key) {
            return Ok(action);
        }

        // 2. 消息滚动（不受浮层影响，硬编码例外）
        if self.handle_message_scroll_key(key) {
            return Ok(Action::Noop);
        }

        // 3. 浮层栈：顶层优先，遇到 blocks_input 则停止
        for overlay in self.overlays.iter_mut().rev() {
            if let Some(action) = overlay.handle_key_event(key) {
                return Ok(action);
            }
            if overlay.blocks_input() {
                return Ok(Action::Noop);
            }
        }

        // 4. 聊天主区域
        self.chat.handle_key_event(key)
            .map_or(Ok(Action::Noop), |a| Ok(a))
    }
}
```

## 8. 渲染管线 + Dirty Tracking

### 8.1 每帧流程

```
┌───────────────────────────────────────────────┐
│  1. Tui::poll_events()                         │
│     ├── crossterm 事件 → App::handle_event()    │
│     ├── Runtime 事件 → App::handle_backend()    │
│     └── 收集 Action → App::update()            │
│                                                │
│  2. App::update(action)                        │
│     ├── 广播 Action 到所有活跃组件              │
│     ├── 收集后续 Action（递归处理，防无限循环）   │
│     ├── 执行 Command（异步，如 DB 查询）         │
│     └── 标记受影响组件 dirty                    │
│                                                │
│  3. App::draw(frame)                           │
│     ├── 布局计算（仅当 dirty 或 resize）         │
│     ├── 对每个 dirty 组件 → draw()              │
│     ├── 组件自己决定是否重算缓存                 │
│     └── terminal.draw() — ratatui diff flush    │
└───────────────────────────────────────────────┘
```

### 8.2 Dirty Marking

```rust
struct App {
    message_list: Option<MessageList>,
    composer: Option<Composer>,
    // ...
}

// App 在 draw() 中分别检查并渲染
impl App {
    fn draw(&mut self, frame: &mut Frame) {
        if let Some(ref mut ml) = self.message_list {
            if ml.is_dirty() {
                ml.draw(frame, message_area, &draw_ctx);
            }
        }
        if let Some(ref mut comp) = self.composer {
            if comp.is_dirty() {
                comp.draw(frame, composer_area, &draw_ctx);
            }
        }
    }
}
```
### 8.3 保留的现有优化

| 优化 | 位置 | 保留方式 |
|---|---|---|
| `MessageLayoutIndex` | 虚拟滚动 | 移到 `MessageList` 组件内 |
| `MessageRenderCache` (LRU) | 消息渲染缓存 | 移到 `MessageList` 组件内 |
| `MARKDOWN_RENDER_CACHE` (内容哈希) | markdown 渲染 | 保持全局（纯函数） |
| Parallel block computation (rayon) | 布局计算 | 保留，改用独立 thread pool |
| `centered_rect()` 等辅助函数 | 通用布局 | 保留为独立工具函数 |

### 8.4 新增优化

- **增量流式更新**：streaming 期间只渲染新增内容，不清缓存
- **布局缓存**：按组件缓存 Rect 分配，resize 时失效
- **独立 rayon thread pool**：避免与 tokio 争抢全局线程池

## 9. 异步协调

### 9.1 原则

**Runtime 是异步的唯一权威。** 组件不 spawn tokio task。

```rust
// 错误示范：组件直接 spawn
fn update(&mut self, action: &Action) -> Vec<Action> {
    if matches!(action, Action::Session(SessionAction::Load)) {
        tokio::spawn(async { /* ... */ });  // ❌
    }
}

// 正确做法：通过 Action 触发
fn update(&mut self, action: &Action, ctx: &UpdateContext) -> Vec<Action> {
    if matches!(action, Action::Session(SessionAction::Load)) {
        vec![Action::Session(SessionAction::Load)]  // → App 看到后执行
    }
}
```

### 9.2 App 侧的异步执行

```rust
impl App {
    fn process_actions(&mut self, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Session(SessionAction::Load) => {
                    let tx = self.action_tx.clone();
                    tokio::spawn(async move {
                        match load_sessions().await {
                            Ok(sessions) => tx.send(Action::Session(
                                SessionAction::Loaded(Ok(sessions))
                            )),
                            Err(e) => tx.send(Action::Session(
                                SessionAction::Loaded(Err(e))
                            )),
                        }
                    });
                }
                Action::Session(SessionAction::Loaded(result)) => {
                    // 广播结果给需要知道的组件
                    self.broadcast(action);
                }
                // ...
            }
        }
    }
}
```

## 10. 模块文件结构

```
tidev-tui/src/
├── lib.rs                      ← 模块声明 + run() 函数
├── app.rs                      ← App 根组件 + OverlayStack + Action 路由 + draw
├── action.rs                   ← Action 枚举分层定义（含 PanelAction 本地定义）
├── component.rs                ← Component trait + 辅助类型
├── context.rs                  ← InitContext, DrawContext, UpdateContext
├── tui.rs                      ← Tui 终端层（setup/teardown + 事件轮询）
├── utils.rs                    ← centered_rect + render_scrollbar + strip_system_reminder_tags + paste_from_clipboard
├── ansi.rs                     ← strip_ansi（从 tidev-tui 拷贝）
│
├── components/
│   ├── mod.rs
│   ├── overlay_stack.rs        ← OverlayStack 容器
│   └── overlays/               ← 所有浮层组件（每个文件一个）
│       ├── mod.rs
│       ├── theme.rs            ✅ 已完成
│       ├── agents.rs           ✅ 已完成
│       ├── skills.rs           ✅ 已完成（含 markdown 预览）
│       ├── settings.rs         ✅ 已完成
│       ├── search.rs           ✅ 已完成
│       ├── message.rs          ✅ 已完成
│       ├── model.rs            ✅ 已完成
│       ├── session.rs          ✅ 已完成
│       ├── connect.rs          ✅ 已完成（含 ProviderPicker + ApiKey 两阶段）
│       ├── rename.rs           ✅ 已完成（带 Ctrl+V 粘贴）
│       ├── fork.rs             ✅ 已完成
│       ├── undo.rs             ✅ 已完成
│       ├── image.rs            ✅ 已完成（Picker 缓存）
│       ├── permission.rs       ✅ 已完成
│       ├── question.rs         ✅ 已完成
│       ├── workspace.rs        ✅ 已完成
│       ├── sensitive.rs        ✅ 已完成
│       ├── command_palette.rs  ✅ 已完成
│       ├── panel_launcher.rs   ✅ 已完成│
├── markdown/                   ← 从 tidev-tui 完整拷贝（含 syntax highlighting）
│   ├── mod.rs
│   ├── highlight.rs
│   ├── styles.rs
│   ├── wrap.rs
│   ├── table.rs
│   ├── line.rs
│   └── links.rs
│
└──── (theme/, chat_context/ 等均为 tidev-tui 自有模块)
```

render/ 和 input/ 目录不再存在，功能已并入组件。

## 11. 迁移路线

> **实现状态：** TUI 迁移已完成，二进制入口使用当前 `tidev-tui` 架构。

| 阶段 | 内容 | 风险 | 状态 |
|---|---|---|---|---|
| 1 | 定义 `Component` trait、`Action` 枚举、context 类型 | 低 | ✅ 已完成 |
| 2 | 迁移**自包含叶子组件**（ThemePanel, SkillsPanel, AgentsPanel） | 低 | ✅ 已完成 |
| 3 | **OverlayStack + App 最小路由**（使阶段 2 组件可运行） | 低 | ✅ 已完成 |
| 4 | 迁移剩余面板 | 中 | ✅ 已完成 |
|   | · SettingsPanel | 低 | ✅ 已完成 |
|   | · SearchPanel | 中 | ✅ 已完成 |
|   | · MessagePanel | 中 | ✅ 已完成 |
|   | · ModelPanel | 中 | ✅ 已完成 |
|   | · SessionPanel | 中 | ✅ 已完成 |
| 5a | 迁移简单对话框（ForkConfirm, UndoConfirm, RenameDialog） | 低 | ✅ 已完成 |
|   | · ForkConfirmDialog | 低 | ✅ 已完成 |
|   | · UndoConfirmDialog | 低 | ✅ 已完成 |
|   | · RenameDialog（自包含 text buffer + Ctrl+V 粘贴） | 低 | ✅ 已完成 |
| 5b | 迁移 Notifications/Toast + ImageViewer + ConnectDialog | 中 | ✅ 已完成 |
|   | · last_notice + toast（App 内联渲染） | 低 | ✅ 已完成 |
|   | · ImageViewer（Picker 缓存） | 中 | ✅ 已完成 |
|   | · ConnectDialog（ProviderPicker + ApiKey，粘贴支持） | 中 | ✅ 已完成 |
| 5c | 迁移安全对话框（WorkspaceBoundary, SensitiveFile） | 中 | ✅ 已完成 |
| 5d | 迁移工具执行对话框（Permission, Question） | **高** | ✅ 已完成 |
| 6 | 提取 **Chat** 组件（MessageList + 渲染管线，~6000 行） | **高** | ✅ 已完成 |
| 7 | 提取 **Composer** 组件（1135 行的输入处理 + @mention + /command + snippet + paste） | **高** | ✅ 已完成 |
| 8 | 全部迁移完成并清理旧实现 | — | ✅ 已完成 |
### 11.1 当前状态

- `cargo run` → `tidev-tui`，当前架构
- 旧 TUI 实现已删除

每迁移一个组件，新 crate 中对应功能即可用。

## 12. 性能保障清单

- [x] **Dirty tracking** — 避免全量重算
- [x] **消息虚拟化** — 只渲染可见消息（现有，保留）
- [x] **渲染缓存** — LRU cache + 内容哈希（现有，保留）
- [x] **并行 markdown 渲染** — rayon（保留，改用独立 thread pool 避免 tokio 争抢）
- [x] **布局缓存** — 按组件缓存 Rect 分配
- [x] **增量流式更新** — streaming 期间只渲染新增内容，不清缓存
- [x] **纯渲染不修改状态** — `draw()` 无副作用，避免 borrow checker 问题
- [ ] **组件粒度不穿透到 widget 级别** — 避免虚函数开销
- [ ] **不每帧 clone ChatContext** — 通过 `&DrawContext` 传递引用
- [ ] **LayoutIndex 增量更新** — streaming 时不触发全量 `update_message_layout_index`

## 13. Chat 组件迁移评估（2026-07-07）

### 13.1 规模

旧代码中 Chat 渲染相关代码约 **6,000 行**：

| 旧文件 | 行数 | 功能 |
|--------|------|------|
| `render/chat_render/mod.rs` | 2,456 | 主渲染管线、布局索引、滚动、缓存、流式 |
| `render/chat_render/tool.rs` | 1,653 | Tool call / tool result / subagent 卡片 |
| `render/chat_render/content.rs` | 819 | 消息内容渲染、BlockComputation 并行计算 |
| `render/chat_render/utils.rs` | 541 | 辅助函数 |
| `render/chat_render/tests.rs` | 394 | 测试 |
| `core/state.rs` (布局索引部分) | ~100 | MessageLayoutIndex, MessageBlock |
| **合计** | **~6,000** | |

### 13.2 已具备的条件

| 基础设施 | 位置 | 状态 |
|---------|------|------|
| Markdown 渲染管线 | `crates/tidev-tui/src/markdown/` (2,465 行) | ✅ 已完整拷贝 |
| Message 类型 | `tidev-llm::message::Message` | ✅ 存在 |
| ChatContext | `tidev_tui_old::chat_context::ChatContext` | ✅ 存在 |
| Component trait | `component.rs` | ✅ 存在 |
| 全局 MARKDOWN_RENDER_CACHE | `markdown/mod.rs` LazyLock | ✅ 存在 |

### 13.3 旧代码的设计问题（需在迁移中修复）

**问题 1：`messages_text()` 的 God Return Type**

```rust
type MessagesTextResult = (
    Text<'static>,                // 渲染行
    usize,                        // 总行数
    Vec<ToolResultCardRange>,     // 工具卡片范围
    Vec<(Uuid, usize, usize)>,    // 用户卡片范围
    Vec<SelectableRegionRange>,   // 可选中区域
    bool,                         // 是否流式中
    usize,                        // 滚动偏移修正
    Vec<InlineRunningCardRange>,  // 运行中卡片范围
);
```

一个函数返回 8 个值，调用方需记忆每个位置的语义。**应替换为命名结构体。**

**问题 2：`render_message_block_to_lines` 12 个参数**

加了 `#[allow(clippy::too_many_arguments)]`。6 个 `&mut Vec<...>` 收集器应收进一个 `RenderOutput` 结构体。

**问题 3：三重遍历消息**

```
Pass 1 (sequential): 确定 block 边界 (BlockInfo)
Pass 2 (rayon):      并行计算每个 block 行数
Pass 3 (sequential): 渲染每个 block 的行内容（重新计算了 Pass 2 已算过的内容）
```

Pass 2 算出的行数在 Pass 3 中没有被复用。这是历史演化导致的性能浪费。

**问题 4：RefCell 内变异性**

```rust
self.message_layout_index.borrow_mut()
```

旧架构中因 ratatui draw closure 限制导致的 hack。新架构 `Component::draw(&mut self)` 不需要。

**问题 5：所有状态散落在 App 上**

```
message_layout_index        → 应归 MessageList
message_render_cache        → 应归 MessageList  
expanded_tool_results       → 应归 MessageList
expanded_tool_outputs       → 应归 MessageList
message_scroll_offset       → 应归 MessageList
message_follow_tail         → 应归 MessageList
selectable_regions          → 应归 MessageList
```

### 13.4 不需要改的行为

| 模块 | 处理方式 |
|------|---------|
| `MessageLayoutIndex` 增量更新逻辑（dirty_messages、contains_streaming_messages） | **保留原样** |
| `MessageRenderCache` LRU 淘汰策略 | **保留原样** |
| Rayon 并行 block 计算 | **保留**，改用独立 thread pool |
| Tool call 卡片展开/折叠/图片切换 | **保留**，交互行为不变 |
| SelectableRegion 追踪 | **保留**，鼠标点击交互 |
| 流式增量标记逻辑（streaming 期间不清缓存） | **保留** |
| MARKDOWN_RENDER_CACHE 全局缓存 | **保留**，纯函数 |

### 13.5 建议的文件结构

```
src/components/
└── chat/
    ├── mod.rs           ← MessageList Component（容器 + 滚动 + BackendEvent 分发）
    ├── layout_index.rs  ← MessageBlock + MessageLayoutIndex（增量更新）
    ├── render_cache.rs  ← MessageRenderCache + key/value 类型
    ├── render.rs        ← messages_text() → render_message_block_to_lines() 管线
    ├── streaming.rs     ← StreamingBuffer（Delta 累积 + 同步）
    └── tool.rs          ← tool call 卡片渲染（对应旧 tool.rs）
```

### 13.6 迁移原则

- **结构重组，行为保留。** 渲染逻辑、缓存策略、并行计算、流式增量全部原样迁移。
- **只改代码归属**：从 `impl App` 方法改为 `impl MessageList` 方法。Composer 是独立 Component，由 App 直接持有。**没有 ChatScreen 这个组件。**
