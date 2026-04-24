# OpenCode UI 设计

本文档总结了 `opencode` 中的终端用户界面（TUI）架构和布局，基于 `opencode-design.md` 与 `opencode/packages/opencode/src/cli/cmd/tui/` 源码。

## 1. 技术栈与架构

- 核心渲染：`@opentui/core` + `@opentui/solid`
- UI 框架：SolidJS
- TUI 入口：`opencode/packages/opencode/src/cli/cmd/tui/app.tsx`
- 插件扩展：`TuiPluginRuntime` 插件运行时、`Slot` 插槽机制
- 全局状态：Route、Theme、Sync、Keybind、Dialog、Toast、Local、KV、SDK 等

## 2. 主入口与 Provider 层级

`app.tsx` 负责启动终端渲染器、检测终端背景、初始化插件、并将主应用挂载到终端。

主要 Provider 层级：

- `ArgsProvider`
- `ExitProvider`
- `KVProvider`
- `ToastProvider`
- `RouteProvider`
- `TuiConfigProvider`
- `SDKProvider`
- `ProjectProvider`
- `SyncProvider`
- `ThemeProvider`
- `LocalProvider`
- `KeybindProvider`
- `PromptStashProvider`
- `DialogProvider`
- `CommandProvider`
- `FrecencyProvider`
- `PromptHistoryProvider`
- `PromptRefProvider`

该层级确保 UI 在不同模块间统一共享状态、命令、路由、主题和插件能力。

### 2.1 App 主入口布局

`app.tsx` 内部的 `App` 组件本身也是一个全屏容器：

- 根节点为 `box`，宽度和高度与终端一致
- 背景色使用 `theme.background`
- 支持鼠标右键复制、终端选择复制逻辑
- 提供 `TuiPluginRuntime.Slot("app")`，允许插件插入全局顶层 UI
- 内部通过 `Show` 控制 `ready()` 渲染，启动阶段显示 `StartupLoading`

### 2.2 路由与页面渲染

- `App` 使用 `route.data.type` 决定渲染页面：
  - `home` 渲染 `<Home />`
  - `session` 渲染 `<Session />`
- `plugin` 路由通过 `routeView(route.data.id)` 动态查找插件渲染函数
- 如果插件路由无对应渲染，则显示 `PluginRouteMissing`
- `App` 还负责根据当前路由更新终端标题，以及处理命令面板中的会话、模型、Agent 切换等入口

## 3. Session 页面总体布局

`Session` 页面是 TUI 的核心交互面板，定义在 `opencode/packages/opencode/src/cli/cmd/tui/routes/session/index.tsx`。

### 3.1 总体布局结构

- 外层容器：`<box flexDirection="row">`
- 主区：左侧主消息区域与输入区域
- 侧边栏：右侧可选信息面板 `Sidebar`

布局示意：

  ┌──────────────────────────────────────────────────────────┐
  │                       Session 页面                         │
  │  ┌──────────────────────────────┐  ┌─────────────────────┐  │
  │  │                              │  │     Sidebar         │  │
  │  │  消息流 + 工具结果区域         │  │  (宽度固定 42 cols) │  │
  │  │                              │  │                     │  │
  │  │                              │  │                     │  │
  │  │                              │  │                     │  │
  │  │                              │  │                     │  │
  │  └──────────────────────────────┘  └─────────────────────┘  │
  │  ┌──────────────────────────────────────────────────────┐  │
  │  │ Prompt / 权限 / 问题 / 子 Agent Footer                │  │
  │  └──────────────────────────────────────────────────────┘  │
  └──────────────────────────────────────────────────────────┘

### 3.2 具体区域划分

1. 主消息区
   - 使用 `scrollbox` 渲染会话消息列表
   - 支持滚动、粘性底部、可选滚动条
   - 单条消息通过 `UserMessage` / `AssistantMessage` 渲染
   - 每条消息采用左侧边框线分隔，增加视觉层次

2. 底部控制区
   - `PermissionPrompt`：权限请求确认
   - `QuestionPrompt`：问题询问流程
   - `SubagentFooter`：当前为子会话时显示子会话导航和控制
   - `Prompt`：用户输入框，始终位于底部

3. 侧边栏 `Sidebar`
   - 宽度固定为 `42`
   - 上部显示会话标题、分享链接
   - 中部为可扩展内容区
   - 底部为插件插槽页脚

### 3.3 侧边栏显示逻辑

侧边栏通过 `sidebarVisible()` 决定是否显示：

- `sidebarOpen()` 为 `true` 时显示
- 否则当 `sidebar` 配置为 `auto` 且终端宽度足够时显示
- 否则隐藏

宽屏与窄屏行为：

- 宽屏：侧边栏作为固定右侧面板显示
- 窄屏：侧边栏以绝对定位覆盖层显示，带半透明遮罩

### 3.4 主区宽度计算

主区宽度依据终端宽度动态计算：

- `contentWidth = dimensions().width - (sidebarVisible() ? 42 : 0) - 4`

即：总宽度减去侧边栏宽度和左右边距，保证主区与侧边栏不会重叠。

## 4. 详细布局元素

### 4.1 Prompt 输入区

`Prompt` 是页面底部的核心交互组件，定义在 `opencode/packages/opencode/src/cli/cmd/tui/component/prompt/index.tsx`。

- 可见性 `visible` 与禁用状态 `disabled` 可控
- 支持命令面板、历史输入、补全、粘贴、提交
- 右侧插槽 `session_prompt_right` 可插入额外操作组件
- 支持根据最新用户消息自动切换 `agent` 和 `model`

### 4.2 Sidebar

`Sidebar` 以面板形式渲染：

- 背景色 `theme.backgroundPanel`
- 常驻宽度 `42`
- 内部使用 `scrollbox`，并支持纵向滚动条样式
- 通过插件插槽注入标题、内容、页脚

### 4.3 Footer 状态栏

`Footer` 在页面底部显示全局状态，定义在 `opencode/packages/opencode/src/cli/cmd/tui/routes/session/footer.tsx`。

- 显示当前工作目录
- 显示连接状态：LSP 数量、MCP 状态、权限提醒
- 未连接时显示 `/connect` 提示
- 使用行内文本与状态符号简洁呈现

### 4.4 页面弹窗与对话层

页面还可触发多种弹出层：

- `DialogTimeline`：会话历史与分支操作
- `DialogMessage`：消息操作与复用
- `DialogForkFromTimeline`：从历史消息创建分支
- `DialogConfirm` / `DialogAlert`：确认与警告弹窗

这些弹窗通常覆盖主视图，但不改变基本布局。

### 4.5 Dialog 布局

`DialogProvider` 负责全局对话框层，渲染在根视图之上：

- 由一个绝对定位的 `box` 包裹，`zIndex=3000`
- 该容器占满整个终端尺寸，并捕获点击与键盘事件
- 使用 `Dialog` 组件渲染实际内容，对话框本身居中显示
- 背景为半透明遮罩，顶部 `paddingTop` 约为屏幕高度的四分之一

`Dialog` 组件的布局规则：

- 支持三种宽度：
  - `medium`：60 列
  - `large`：88 列
  - `xlarge`：116 列
- 最大宽度不超过终端宽度减 2
- 对话框本身背景为 `theme.backgroundPanel`
- 点击遮罩区会触发关闭
- 使用 Escape / Ctrl+C 关闭活动弹窗，同时保留文本选择行为

这种对话框层设计保证：

- 弹窗不会破坏根页面布局
- 用户可以快速关闭对话框
- 对话框布局在宽屏和窄屏下都有固定尺寸约束

## 5. 交互与响应式设计

### 5.1 响应式侧边栏

- 宽屏：侧边栏固定显示，主区保留足够宽度
- 窄屏：侧边栏作为覆盖模态出现，避免压缩主内容

### 5.2 键盘与终端友好

- 支持终端复制、选择和鼠标点击
- 保持 Ctrl+C、Esc 等终端惯用操作
- 提供快捷键面板与命令式输入

### 5.3 会话驱动视图

整个 UI 围绕 `sessionID` 组织：

- 会话消息与状态关联当前 `sessionID`
- 输入、权限、问题、子 Agent 等都基于当前会话
- 可通过会话内导航与历史操作调整上下文

### 5.4 插件扩展插槽

OpenCode UI 大量使用 `TuiPluginRuntime.Slot`：

- 侧边栏标题、内容、页脚
- 输入框右侧扩展区域
- 会话视图中的替换或插入组件

这让插件能在布局关键位置扩展功能，而无需改动基础页面结构。

## 6. 关键文件索引

- `opencode/packages/opencode/src/cli/cmd/tui/app.tsx`：TUI 启动与 Provider 层级
- `opencode/packages/opencode/src/cli/cmd/tui/routes/home.tsx`：Home 页面入口
- `opencode/packages/opencode/src/cli/cmd/tui/routes/session/index.tsx`：Session 页面布局
- `opencode/packages/opencode/src/cli/cmd/tui/routes/session/sidebar.tsx`：侧边栏
- `opencode/packages/opencode/src/cli/cmd/tui/routes/session/footer.tsx`：底部状态栏
- `opencode/packages/opencode/src/cli/cmd/tui/component/prompt/index.tsx`：Prompt 输入组件
- `opencode/packages/opencode/src/cli/cmd/tui/component/dialog-command.tsx`：命令面板
- `opencode/packages/opencode/src/cli/cmd/tui/ui/toast.tsx`：通知组件

## 7. 总结

OpenCode 的 UI 设计强调：

- 终端友好的 SolidJS TUI 实现
- 分区明确的主内容与侧边栏布局
- 响应式宽屏/窄屏显示策略
- 插件驱动的扩展性
- 统一上下文管理与命令式交互
