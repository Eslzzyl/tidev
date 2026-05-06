# From OpenChamber to TiDev: 功能引入优先级分析

本文档基于对 [OpenChamber](https://github.com/openchamber/openchamber) 项目的深入调研，对比当前 TiDev Web 前端现状，按优先级罗列可引入的功能。

---

## 当前 TiDev Web 前端现状

| 维度 | 状态 (2026-05-06 CodeMirror 更新) |
|------|------------------------|
| **框架** | React 19 + Vite + Tailwind CSS 4 + Zustand |
| **路由与导航** | ✅ **已完成**: Hash 路由 (#chat/#files/#settings/#terminal/#git), Header 导航栏, 视图注册系统 |
| **聊天** | 基础 SSE 流式聊天，消息轮次渲染，Markdown + tool call + diff 渲染 |
| **文件浏览器** | ✅ **已完成**: 目录树(递归展开/折叠)、文件搜索、文件读取/写入 API |
| **代码编辑器** | ✅ **已完成**: CodeMirror 6 编辑器，语法高亮(Lezer AST)，编辑/只读切换，Ctrl+S 保存，脏状态跟踪，light/dark 主题跟随 |
| **Markdown 渲染** | ✅ **已完成**: KaTeX 数学公式、Mermaid 图表、代码块增强(语言标签/复制/下载)、图片内联渲染 |
| **Diff 预览** | ✅ **已完成**: 并排/统一模式、语法高亮、文件级折叠/展开、大文件降级渲染 |
| **会话管理** | ✅ **已完成**: 搜索/过滤、内联重命名(双击)、状态指示器(流式动画) |
| **设置** | ✅ **已完成**: 字体、Diff 布局、行为设置，主题切换（light/dark/system），**完整页面视图** |
| **右侧边栏** | Token 用量统计、待办清单、diff 文件列表 |
| **Git** | ✅ **已完成 (见下方注意事项)**: GitView 面板(更改/历史/分支)、Git API 端点(10个)、提交/push/pull/stash |
| **终端** | ✅ **已完成 (见下方注意事项)**: xterm.js + portable-pty PTY 终端，多标签页，**浅色/深色主题跟随** |
| **Tool Call 渲染** | ✅ **已完成**: JSON 树视图、持续时间显示、状态动画、websearch/webfetch 分类 |
| **权限请求 UI** | ✅ **已完成**: PermissionCard 组件、权限 store、SSE 事件监听 |

---

## P0: 高优先级 — 直接提升核心用户体验

这些功能填补了 tidev 最明显的空白，用户每天都会使用。

### 1. 多视图路由与导航 ✅ 已完成

**参考**: OpenChamber `useRouter.ts`, `MainLayout.tsx`, `Header.tsx`

**实现**:
- **Hash 路由系统**: `web/src/lib/router.ts` — 解析 `#chat`, `#files`, `#settings` URL hash
- **Header 导航栏**: `web/src/components/layout/Header.tsx` — 顶部恒驻导航栏，包含视图切换标签页
- **视图注册**: `App.tsx` 根据 `activeTab` 状态渲染 ChatPanel / FilesView / SettingsView
- **URL 双向同步**: hash change 事件 ↔ zustand store 状态
- **设置面板迁移**: 从 modal 迁移为完整 SettingsView 页面视图

**文件**: `web/src/lib/router.ts`, `web/src/stores/useUIStore.ts`, `web/src/components/layout/Header.tsx`, `web/src/components/views/SettingsView.tsx`

### 2. 文件浏览器 (FilesView) ✅ 已完成

**参考**: OpenChamber `views/FilesView.tsx`, `SidebarFilesTree.tsx`, `fileStore.ts`

**实现**:
- **后端 API**: `GET /api/fs/list?path=` (目录列表), `GET /api/fs/read?path=` (文件读取) — 带安全路径解析
- **文件树**: `web/src/components/views/FileTree.tsx` — 递归展开/折叠目录树，文件类型图标
- **代码查看器**: `web/src/components/views/CodeViewer.tsx` — 语言检测、语法高亮、文件头、复制/关闭
- **文件搜索**: 集成已有 `/api/files/search` 端点
- **文件 Store**: `web/src/stores/useFileStore.ts` — 树形数据管理、加载状态

**文件**: `src/web/routes/fs.rs`, `src/web/routes/mod.rs`, `src/web/error.rs`, `web/src/stores/useFileStore.ts`, `web/src/components/views/FilesView.tsx`, `web/src/components/views/FileTree.tsx`, `web/src/components/views/CodeViewer.tsx`, `web/src/components/ui/CodeMirrorEditor.tsx`, `web/src/lib/codemirror/theme.ts`, `web/src/lib/codemirror/languageByExtension.ts`

### 3. Markdown / 消息渲染增强 ✅ 已完成

**参考**: OpenChamber `chat/MarkdownRendererImpl.tsx`, `chat/message/parts/*`

**实现**:
- **KaTeX 数学公式**: 集成 `remark-math` + `rehype-katex`
- **Mermaid 图表**: 动态加载 Mermaid 库，支持所有图表类型，失败降级 + 重试
- **代码块增强**: 语言标签显示、复制按钮、下载按钮
- **图片内联渲染**: 自定义 img 组件，点击展开/收缩，带加载动画
- **推理过程折叠**: `ThinkingBlock` 增加实时计时器（显示消耗秒数）
- **组件化**: `CodeBlock` 和 `Mermaid` 作为独立子组件

**文件**: `web/src/components/renderers/MarkdownRenderer.tsx`, `web/src/components/renderers/ThinkingBlock.tsx`

### 4. Diff 预览增强 ✅ 已完成

**参考**: OpenChamber `views/DiffView.tsx`, `PierreDiffViewer.tsx`, `chat/DiffPreview.tsx`

**实现**:
- **并排模式**: 宽度超过 768px 时并排显示新旧代码
- **文件级折叠**: `CollapsibleDiffFile` 组件 — 带文件头、展开/折叠
- **批量控制**: `DiffCollapseProvider` — 展开/折叠所有文件
- **语法高亮**: 使用 highlight.js 对 diff 代码进行语法着色
- **大文件降级**: 超过 100 行时禁用逐行语法高亮
- **统一/内联模式**: 窄屏时自动切换为 inline 布局

**文件**: `web/src/components/renderers/DiffRenderer.tsx`

### 5. 会话管理增强 ✅ 已完成

**参考**: OpenChamber `session/SessionSidebar.tsx`

**实现**:
- **会话搜索/过滤**: 按标题实时过滤，搜索输入框
- **内联重命名**: 双击会话标题进入编辑模式，Enter 确认，Escape 取消
- **会话状态指示器**: 当前活动会话 + 流式进行中时显示 ping 动画
- **操作按钮**: hover 时显示重命名和删除按钮
- **保留已有特性**: 新建/删除/切换、草稿会话

**文件**: `web/src/components/layout/LeftSidebar.tsx`

---

## P1: 中优先级 — 显著提升工作效率 ✅ 已完成 (2026-05-06)

### 6. Git 集成 (GitView) ✅ 已完成

**参考**: OpenChamber `views/GitView.tsx` (2309 行), `stores/useGitStore.ts`, `stores/useGitIdentitiesStore.ts`, `components/session/BranchPickerDialog.tsx`

**现状**: ✅ 已完成

**实现**:
- **后端 API** (`src/web/routes/git.rs`): 10 个端点 — status/branches/log/commit/branch create/branch delete/push/pull/stash/stash-pop，调用系统 git
- **GitView 面板** (`web/src/components/views/GitView.tsx`): 三个标签页 — Changes（暂存/未暂存 + 提交）、History（提交列表）、Branches（创建/删除/切换）
- **顶部栏**: 分支名、SHA、ahead/behind、push/pull/stash 按钮
- **Hash 路由**: `#git` 注册

**文件**: `src/web/routes/git.rs`, `web/src/components/views/GitView.tsx`, `web/src/api/client.ts`, `web/src/types/api.ts`

### 7. 终端 (TerminalView) ✅ 已完成

**现状**: ✅ 已完成

**实现**:
- **xterm.js 集成**: `@xterm/xterm` + `@xterm/addon-fit` 全功能终端模拟器
- **PTY 后端**: `portable-pty` crate 创建真实 PTY，shell 交互运行
- **SSE 流**: broadcast channel 推送 PTY 输出，按 session_id 过滤
- **多标签页**: 创建/关闭/切换
- **颜色主题**: 跟随前端浅色/深色主题 (`DARK_THEME` / `LIGHT_THEME`)
- **API**: start/input/resize/events(SSE)/close

**文件**: `src/web/terminal.rs`, `src/web/routes/terminal.rs`, `web/src/components/views/TerminalView.tsx`, `web/src/stores/useTerminalStore.ts`
### 8. 权限请求 UI (PermissionCard) ✅ 已完成

**现状**: ✅ 已完成

**实现**:
- **PermissionCard 组件** (`web/src/components/chat/PermissionCard.tsx`): 工具权限请求卡片，Allow once / Always allow / Deny
- **权限 Store** (`web/src/stores/usePermissionStore.ts`): 待审批管理、auto-accept 持久化、SSE 事件处理
- **SSE/UI 集成**: useSSE 监听 `permission.request`，ChatPanel 展示 `PermissionArea`

**⚠️ 注意事项**: 后端 `auto_approve_permissions: false` 下需要权限的工具直接被拒绝（不触发 permission.request 事件）。前端组件已就绪，需后端启用 permission channel 才能激活完整流程。

**文件**: `web/src/components/chat/PermissionCard.tsx`, `web/src/stores/usePermissionStore.ts`, `web/src/hooks/useSSE.ts`
### 9. Tool Call 渲染增强 ✅ 已完成

**现状**: ✅ 已完成

**实现**:
- **JSON 树视图** (`web/src/components/ui/JsonTreeView.tsx`): 可折叠 JSON 树形展示，类型彩色编码
- **持续时间显示**: 实时计时器 + 完成耗时
- **状态动画**: 运行中 spinner + 实时时间
- **新增分类**: `websearch`/`webfetch` 工具图标和颜色

**文件**: `web/src/components/ui/JsonTreeView.tsx`, `web/src/components/renderers/ToolCallRow.tsx`

---

## P2: 较低优先级 — 锦上添花

### 10. 设置面板扩展 ✅ 已完成
**参考**: OpenChamber `views/SettingsView.tsx` (818 行), `components/sections/*`

**现状**: ✅ 已完成

**实现**:
- **字体设置**: UI 字体、等宽字体、字号滑块
- **Diff 布局**: 并排/内联选择
- **行为设置**: Enter to send 开关
- 所有设置 localStorage 持久化

**文件**: `web/src/stores/useUIStore.ts`, `web/src/components/views/SettingsView.tsx`

### 11. 多轮次对话的分组与 Timeline

**参考**: OpenChamber `chat/TimelineDialog.tsx`, `chat/turn/*`, `hooks/useTimelineStaging.ts`

**现状**: TiDev 消息以扁平列表展示。

**建议引入**:
- **Turn 分组**: 用户消息 + 对应 AI 回复作为一个 Turn 展示
- **Timeline 视图**: 可展开的时间轴查看所有 Turn
- **历史导航**: 在 Turn 之间快速跳转
- **撤回状态提示**: 当用户撤回消息后显示分支提示

**价值**: 中。对于复杂对话有帮助。

### 12. 会话状态和用量实时统计

**参考**: OpenChamber `components/chat/StatusRow.tsx`, `stores/useQuotaStore.ts`, `components/sections/quota/`

**现状**: TiDev 在右侧边栏有 token 统计，但不是实时状态条。

**建议引入**:
- **状态行**: 聊天底部显示实时状态（流式、token 速率、估算用时）
- **用量面板**: 会话级别的 token 用量统计（输入/输出/缓存读取/缓存写入）
- **TPS 显示**: 实时 token 每秒速度
- **请求计数**: 当前会话的 API 请求次数

**价值**: 中。帮助用户了解性能和成本。

### 13. 会话文件变更列表与预览

**参考**: OpenChamber `chat/ChangedFilesList.tsx`, `chat/TurnChangedFilesDropdown.tsx`, `chat/PendingChangesBar.tsx`

**现状**: TiDev 在右侧边栏有基础 diff 文件列表。

**建议引入**:
- **每个 Turn 的文件变更**: 在每条 AI 回复旁显示本次修改的文件列表
- **变更摘要**: 文件新增/修改/删除的状态摘要
- **待发送变更栏**: 在输入框上方显示待应用的 git 变更

**价值**: 中。对于代码审查场景有帮助。

### 14. 虚拟滚动优化

**参考**: OpenChamber `@tanstack/react-virtual` + `ChatContainer`, `chat/MessageList.tsx`

**现状**: TiDev 消息列表使用基本滚动，大量消息时性能可能下降。

**建议引入**:
- **虚拟列表**: 对消息列表使用虚拟滚动，只渲染可见区域
- **自动滚动智能判断**: 用户手动向上滚动时停止自动滚动，底部显示"回到最新"按钮
- **大型消息性能优化**: 超大代码块使用虚拟化渲染

**价值**: 中。会话历史很长时影响显著。

---

## P3: 可长期规划的功能

### 15. 语音输入/输出

**参考**: OpenChamber `voice/*`, `hooks/useBrowserVoice.ts`, `hooks/useServerTTS.ts`, `hooks/useSayTTS.ts`

**建议引入**:
- **浏览器语音识别**: 点击麦克风按钮开始语音输入
- **TTS 朗读**: AI 回复可朗读
- **对话模式**: 连续语音交互
- **多语言支持**: 10+ 语言选择

**价值**: 低-中。对移动端和 accessibility 场景有价值。

### 16. 多模型并发执行 (Multi-Run)

**参考**: OpenChamber `multirun/*`, `stores/useMultiRunStore.ts`

**建议引入**:
- **并发执行**: 同一条提示词同时在多个模型上运行
- **结果对比**: 并排展示不同模型的结果

**价值**: 低。高级用户功能，评估模型时使用。

### 17. 内联代码注释 (Comments)

**参考**: OpenChamber `comments/*`, `stores/useInlineCommentDraftStore.ts`

**建议引入**:
- **行级注释**: 在 diff 或代码上添加注释
- **评论草稿**: 在发送前编辑评论

**价值**: 低。代码审查协作场景才有需求。

### 18. 计划/规划视图 (PlanView)

**参考**: OpenChamber `views/PlanView.tsx`, `hooks/usePlanDetection.ts`

**建议引入**:
- **计划文件编辑**: `.plan.md` 文件的预览/编辑
- **从计划创建任务**: "改进"或"实现"计划

**价值**: 低。特定的 workflow 场景。

### 19. 计划任务 (Scheduled Tasks)

**参考**: OpenChamber `components/session/ScheduledTasksDialog.tsx`, `server/lib/scheduled-tasks/`

**建议引入**:
- **定时执行**: 在指定时间自动执行任务
- **任务管理**: 查看、编辑、删除计划任务

**价值**: 低。后台自动化场景。

### 20. 技能市场 (Skills Catalog)

**参考**: OpenChamber `stores/useSkillsCatalogStore.ts`, `stores/useSkillsStore.ts`, `server/lib/skills-catalog/`

**建议引入**:
- **技能浏览**: 浏览可用技能
- **安装/卸载**: 一键安装技能
- **自定义技能**: 用户编写自己的技能

**价值**: 低。生态扩展功能。

### 21. 隧道与远程访问

**参考**: OpenChamber `server/lib/tunnels/*`, `components/sections/tunnels/`

**建议引入**:
- **Cloudflare Tunnel 集成**: 通过隧道分享本地服务
- **QR 码分享**: 手机扫描二维码访问

**价值**: 低。远程协作场景。

### 22. WebAuthn/Passkey 认证

**参考**: OpenChamber `auth/SessionAuthGate.tsx`, `server/lib/ui-auth/`

**建议引入**:
- **密码认证**: 密码保护 Web 界面
- **Passkey 支持**: WebAuthn 无密码登录
- **设备信任**: "信任此设备"选项

**价值**: 低。仅当需要对外暴露服务时有用。

### 23. 桌面端集成

**参考**: OpenChamber `packages/desktop/`, `packages/electron/`

**建议引入**:
- **Tauri 桌面打包**: 将 web 前端打包为桌面应用
- **桌面特有功能**: 本地文件系统 API、系统托盘、通知

**价值**: 低。当前 tidev 定位为终端 + Web 辅助界面。

### 24. PWA 支持

**参考**: OpenChamber `src/sw.ts`, `src/pwa.d.ts`, `hooks/usePwaInstallPrompt.ts`, `hooks/usePwaDetection.ts`

**建议引入**:
- **Service Worker**: 离线缓存支持
- **可安装**: PWA 安装提示
- **推送通知**: 后台任务完成通知

**价值**: 低。辅助体验提升。

---

## 总体建议

### 第一阶段 (已完成 — 2026-05-06)
1. ✅ **多视图路由** — 基础设施，一切的基础
2. ✅ **文件浏览器** — AI 编码核心体验
3. ✅ **Markdown 渲染增强** — 直接提升体验
4. ✅ **Diff 预览增强** — 代码审查核心
5. ✅ **会话管理增强** — 日常使用优化

### 第二阶段 (已完成 — 2026-05-06)
6. ✅ **Git 集成** — 显著的效率提升
7. ✅ **终端集成** — 独立操作能力
8. ✅ **权限请求 UI** — 安全体验 (前端完成，需后端配合)
9. ✅ **Tool Call 渲染增强** — 透明度提升
10. ✅ **设置面板扩展** — 自定义能力

### 第三阶段 (长期)
11. 语音、多模型并发、内联注释、计划视图、技能市场等

---

## OpenChamber 代码编辑/浏览架构深度分析

本附录基于对 OpenChamber `packages/ui/src/` 的实际源码调研，详细分析其代码编辑和浏览的实现方案，供后续优化 tidev 代码浏览功能时参考。

### 核心组件结构

OpenChamber 的代码浏览/编辑由三个核心组件构成，分工明确：

| 组件 | 位置 | 代码量 | 职责 |
|------|------|--------|------|
| `CodeMirrorEditor` | `components/ui/CodeMirrorEditor.tsx` | 418 行 | 核心编辑器，封装 CodeMirror 6 |
| `FilesView` | `components/views/FilesView.tsx` | **3356 行** | 文件查看器（最大组件） |
| `SidebarFilesTree` | `components/layout/SidebarFilesTree.tsx` | 968 行 | 侧边栏文件树 |

辅助文件：

| 文件 | 职责 |
|------|------|
| `lib/codemirror/languageByExtension.ts` (175 行) | 扩展名→CodeMirror 语言映射（同步 + 异步动态加载） |
| `lib/codemirror/flexokiTheme.ts` (550 行) | CodeMirror 6 完整自定义主题 |
| `stores/useFilesViewTabsStore.ts` | 标签页状态管理（按 workspace root 分组，持久化） |
| `stores/useFileSearchStore.ts` | 文件搜索缓存 |
| `components/ui/GoToLineDialog.tsx` | 跳转到行对话框 |
| `components/comments/CodeMirrorCommentWidgets.tsx` | 内联评论的 CodeMirror widget |

### 代码编辑器：CodeMirror 6

OpenChamber **只使用 CodeMirror 6**，未集成 Monaco Editor。

#### CodeMirrorEditor 组件设计

**Props 接口：**
```typescript
type CodeMirrorEditorProps = {
  value: string;
  onChange: (value: string) => void;
  extensions?: Extension[];
  className?: string;
  readOnly?: boolean;
  lineNumbersConfig?: ...;
  highlightLines?: { start: number; end: number };
  blockWidgets?: BlockWidgetDef[];
  onViewReady?: (view: EditorView) => void;
  onViewDestroy?: () => void;
  enableSearch?: boolean;
  searchOpen?: boolean;
  onSearchOpenChange?: (open: boolean) => void;
};
```

**关键设计模式：**
- **Compartment 模式**: 使用 CodeMirror 的 `Compartment` 动态开关/配置功能：
  - `lineNumbersCompartment` — 行号显示配置
  - `editableCompartment` — 只读/编辑切换
  - `externalExtensionsCompartment` — 外部传入的扩展
  - `highlightLinesCompartment` — 行高亮范围
  - `blockWidgetsCompartment` — 块级 Widget
  - `searchCompartment` — 搜索面板
- **Block Widget 系统**: 通过 `WidgetType` 和 `createPortal` 在 CodeMirror 中渲染 React 组件（用于内联评论）
- **搜索面板**: 使用 CodeMirror 内置搜索，打补丁改为图标按钮

#### 语言检测

双重系统：
- **`languageByExtension(filePath)`** (同步) — 静态导入常用语言：JS/TS, JSON, CSS, HTML, Markdown, Python, Shell
- **`loadLanguageByExtension(filePath)`** (异步) — 动态加载不常用语言：C++, Go, Rust, SQL, XML, YAML, Elixir

#### 主题系统

`flexokiTheme.ts` 从应用主题动态生成 CodeMirror 主题：
- `EditorView.theme({...})` — CSS 级主题（字体、颜色、间距、光标、选区等）
- `HighlightStyle.define(...)` — 通过 Lezer tags 定义语法高亮颜色
- 将应用 `theme.colors.syntax` 映射到 CM6 CSS 类（`.cm-keyword`, `.cm-string`, `.cm-comment` 等）

### 外部依赖

OpenChamber 重度依赖 CodeMirror 6 生态，共 **18 个 `@codemirror/*` 包** + `@lezer/highlight` + `codemirror-lang-elixir`：

| 类别 | 包 | 用途 |
|------|-----|------|
| **核心** | `@codemirror/view` (6.39.13 pinned), `@codemirror/state` (^6.5.4), `@codemirror/language` (6.12.2 pinned) | 编辑器框架 |
| **功能** | `@codemirror/commands`, `@codemirror/autocomplete`, `@codemirror/search`, `@codemirror/lint` | 命令/补全/搜索/检查 |
| **语言** | `@codemirror/lang-javascript`, `-python`, `-rust`, `-go`, `-cpp`, `-css`, `-html`, `-json`, `-markdown`, `-sql`, `-xml`, `-yaml` | 12 种语言支持 |
| **扩展** | `@codemirror/language-data`, `@codemirror/legacy-modes` | 动态加载更多语言 |
| **额外** | `codemirror-lang-elixir` | Elixir 语言支持 |

此外，diff 渲染使用 `@pierre/diffs` (1.1.0-beta.13)，在只读模式中提供 Shiki 语法高亮。

### 文件树 (SidebarFilesTree)

**数据结构：**
```typescript
type FileNode = {
  name: string;
  path: string;
  type: 'file' | 'directory';
  extension?: string;
  relativePath?: string;
};
```

**关键实现细节：**
- **数据源**: 通过 `opencodeClient.listDirectory()` 调用 OpenCode 服务端 API 获取目录列表
- **排序**: 目录优先，然后按名称字母序（`sortNodes`）
- **状态**: `useFilesViewTabsStore` 管理展开/折叠状态（按路径 key）
- **懒加载**: 目录展开时按需加载；`loadedDirsRef` 跟踪已加载的目录避免重复请求
- **Git 集成**: 通过 `useGitStatus` 显示文件状态徽章（M=修改, A=新增, D=删除, ?=未跟踪）
- **上下文菜单**: 右键菜单支持：打开文件、在文件管理器中显示、复制路径、复制内容、新建文件/文件夹、重命名、删除
- **对话框操作**: 新建/重命名/删除通过 runtime API 调用 + toast 通知
- **文件类型图标**: 使用 `<FileTypeIcon>` 组件（不是直接使用 remixicon）

### 文件查看器 (FilesView) — 3356 行

这是 OpenChamber **最大的组件**，承担了多种职责。

**标签管理 (useFilesViewTabsStore)：**
- 按 workspace root 分组：`byRoot: Record<string, RootTabsState>`
- 每个 root 存储：`openPaths: string[]`, `selectedPath`, `expandedPaths`, `touchedAt`
- 限制最多 20 个 workspace roots
- 路径归一化处理跨平台路径（Windows UNC, 驱动器号, Unix 路径）
- **持久化** 通过 zustand persist middleware

**标签栏渲染：**
- 从 `openPaths` 渲染标签
- 每个标签显示：`FileTypeIcon` + 文件名 + 关闭按钮
- 激活的文件高亮
- 标签栏水平滚动

**文件渲染模式（FilesView 内部的多态渲染）：**

| 模式 | 检测条件 | 渲染方式 |
|------|---------|---------|
| 加载中 | content === null | Spinner |
| 错误 | error set | 红色错误消息 |
| 图片 | `.png/.jpg/.gif/.svg` 等 | `<img>` 标签 |
| Markdown | `.md` | `<SimpleMarkdownRenderer>` + 预览/编辑切换按钮 |
| HTML | `.html` | `<iframe sandbox>` 预览 + CodeMirror 编辑 |
| JSON | `.json` | `<JsonTreeView>` 结构化树形展示 |
| Shiki 高亮 | 只读模式 | `@pierre/diffs` 的 `renderShikiFileView` |
| **CodeMirror** | 默认（文本文件） | `CodeMirrorEditor` 组件 |

**工具栏：**
浮动工具栏（hover 显示）：编辑/预览切换、自动换行、跳转到行、全屏、复制、系统应用打开等。

**快捷键：**
- `Ctrl/Cmd+S` → 保存文件
- `Ctrl/Cmd+F` → 打开搜索

### 当前 tidev 与 OpenChamber 对比

| 维度 | tidev 当前 (CodeMirror) | OpenChamber (CodeMirror) |
|------|-------|-------------|
| **编辑器引擎** | ✅ **CodeMirror 6**（完整 IDE 框架） | CodeMirror 6（完整 IDE 框架） |
| **语法高亮** | ✅ **Lezer 解析器**（精准 AST，覆盖 20+ 语言） | Lezer 解析器（精准 AST，覆盖 15+ 语言） |
| **文件编辑** | ✅ **读写**，Ctrl+S 保存，脏状态跟踪 | ✅ 读写，Ctrl+S 保存 |
| **多标签页** | ❌ 单文件 | ✅ 多标签，标签栏，持久化 |
| **右键菜单** | ❌ | ✅ 创建/重命名/删除/复制 |
| **Git 状态** | ❌ | ✅ M/A/D/? 徽章 |
| **图片预览** | ❌ | ✅ 内联渲染 |
| **Markdown 预览** | ❌ | ✅ 预览/编辑切换 |
| **JSON 树视图** | ❌ | ✅ 结构化浏览 |
| **文件搜索** | 基本（300ms debounce） | 缓存 + debounce + 类型过滤 |
| **块级 Widget** | ❌ | ✅ 支持 React 组件嵌入（评论） |
| **行高亮** | ❌ | ✅ 指定范围高亮 |
| **搜索面板** | ✅ **CodeMirror 内置搜索** | ✅ CodeMirror 内置搜索 |
| **跳转到行** | ❌ | ✅ GoToLineDialog |
| **主题** | ✅ **完整的 CodeMirror 主题**（light/dark 自适应） | 完整的 CodeMirror 主题（550 行） |

### 优化建议（分优先级）

#### P1 — 当前差距最大的功能缺口

1. **多标签页支持**
   - 修改 `useFileStore` 增加 `openPaths: string[]` + `selectedPath`
   - 创建标签栏组件（水平滚动、激活高亮、关闭按钮）
   - 不需要 CodeMirror，当前 CodeViewer 可继续使用
   - 后端需要: 无需改动（已有 `/api/fs/read`）

2. **文件编辑 + 保存**
   - 后端: 新增 `POST /api/fs/write` 端点（接收路径+内容）
   - 前端: CodeViewer 添加编辑模式切换（`contentEditable` 或 `<textarea>`）
   - 快捷键: `Ctrl+S` 保存
   - 复杂度: 小，但需要处理脏状态提示

3. **文件操作（创建/重命名/删除）**
   - 后端: 新增 `POST /api/fs/create`（文件/目录）、`POST /api/fs/rename`、`DELETE /api/fs/remove`
   - 前端: 文件树添加右键菜单 + 创建按钮
   - 复杂度: 中

#### P2 — 用户体验优化

4. **Git 状态徽章**
   - 后端: 新增 `GET /api/git/status` 端点
   - 前端: FileTree 节点旁显示 M/A/D/? 标识
   - 复杂度: 中（需添加 git 集成）

5. **图片/ Markdown/ JSON 预览**
   - 按文件扩展名自动切换渲染模式
   - 图片: `<img>` 标签内联展示
   - Markdown: 使用已有的 `MarkdownRenderer` 组件
   - JSON: 简单格式化 + `<pre>` 展示
   - 复杂度: 小

#### P3 — 编辑器升级

6. ✅ **接入 CodeMirror 6 （已完成 2026-05-06）**
   - 替换自制的 `CodeViewer.tsx` 为 `CodeMirrorEditor` 组件
   - 获得真正的 Lezer AST 语法高亮、搜索、行号、折叠
   - 预加载常用语言包（JS/TS, Python, Rust, JSON, Markdown, CSS, HTML, SQL, XML, YAML, C++）
   - 其他语言通过 `@codemirror/language-data` 动态加载
   - 自定义主题适配 tidev 的 light/dark 主题
   - 新增编辑模式：读写切换、Ctrl+S 保存、脏状态跟踪
   - 复杂度: 大 ✅ 已完成

### 不建议直接照搬的设计

1. **FilesView 的 3356 行单体组件** — 承担了标签栏、工具栏、7 种渲染模式、注释系统等太多职责。tidev 应该拆分为更小的组件：`FileTabs`, `FileToolbar`, `FilePreview`, `CodeEditor` 等。

2. **全部 18 个 codemirror 包** — 完整引入会增加约 500KB+ bundle 体积。tidev 采用了折中方案：预加载 11 个最常用语言包，其余通过 `@codemirror/language-data` 动态加载，平衡了体积和体验。

3. **内联评论系统** — CodeMirror 的 Block Widget + `createPortal` 实现很精巧但复杂度高。tidev 在早期不需要这个功能。

### 结论

当前 tidev 的代码编辑已从自制 CodeViewer 升级为 **CodeMirror 6**，获得了完整的 IDE 级编辑器体验：

- ✅ **语法高亮**: Lezer 精准 AST 解析，覆盖 20+ 种语言
- ✅ **文件编辑**: 编辑/只读切换，Ctrl+S 保存，脏状态跟踪
- ✅ **搜索面板**: CodeMirror 内置搜索
- ✅ **主题跟随**: light/dark 自适应主题

后续仍有优化空间：

- **多标签页**: 类似 OpenChamber 的标签栏（打开多个文件标签）
- **文件操作**: 右键菜单创建/重命名/删除
- **Git 状态**: 文件树 Git 状态徽章
- **多格式预览**: 图片/Markdown/JSON 直接内联预览
