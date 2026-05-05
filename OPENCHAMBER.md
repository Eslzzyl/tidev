# From OpenChamber to TiDev: 功能引入优先级分析

本文档基于对 [OpenChamber](https://github.com/openchamber/openchamber) 项目的深入调研，对比当前 TiDev Web 前端现状，按优先级罗列可引入的功能。

---

## 当前 TiDev Web 前端现状

| 维度 | 现状 |
|------|------|
| **框架** | React 19 + Vite + Tailwind CSS 4 + Zustand |
| **聊天** | 基础 SSE 流式聊天，消息轮次渲染，Markdown + tool call + diff 渲染 |
| **会话管理** | 左侧边栏列表，新建/删除/切换，基础信息展示 |
| **设置** | 极简：仅主题切换（light/dark/system） |
| **右侧边栏** | Token 用量统计、待办清单、diff 文件列表 |
| **文件系统** | 无独立文件浏览器 |
| **Git** | 无 |
| **终端** | 无 |
| **路由** | 无多视图路由 |

---

## P0: 高优先级 — 直接提升核心用户体验

这些功能填补了 tidev 最明显的空白，用户每天都会使用。

### 1. 多视图路由与导航

**参考**: OpenChamber `useRouter.ts`, `MainLayout.tsx`, `Header.tsx`

**现状**: TiDev 只有单页面聊天视图，通过 WelcomePage / ChatPanel 区分。无全局路由系统。

**建议引入**:
- **视图注册系统**: 以标签页/侧边栏导航切换聊天(Chat)、文件(Files)、Git、终端(Terminal)、设置(Settings)等视图
- **Header 导航栏**: 顶部恒驻导航栏，包含视图切换、项目名称、当前连接状态
- **URL hash/query 路由**: 支持 `#chat/session-id`、`#settings`、`#files` 等 URL 路由，支持浏览器前进/后退

**价值**: 高。为所有后续功能提供导航基础设施。

### 2. 文件浏览器 (FilesView)

**参考**: OpenChamber `views/FilesView.tsx` (3356 行), `components/layout/SidebarFilesTree.tsx`, `stores/fileStore.ts`, `components/icons/`

**现状**: TiDev 完全不支持文件浏览。用户无法浏览工作区文件结构。

**建议引入**:
- **目录树**: 左侧面板可展开/折叠的文件树，带文件类型图标
- **代码编辑器**: 集成 CodeMirror 或类似编辑器（语法高亮、行号、主题）
- **文件操作**: 创建/重命名/删除文件和目录
- **Git 状态标识**: 文件旁显示 git 状态标记（新增/修改/删除）
- **多标签页**: 多个文件可同时在标签页中打开
- **右键菜单**: 文件/文件夹的上下文操作菜单
- **搜索功能**: 工作区文件搜索

**价值**: 极高。AI 编码助手用户频繁需要查看和编辑文件。

### 3. Markdown / 消息渲染增强

**参考**: OpenChamber `chat/MarkdownRendererImpl.tsx`, `chat/message/parts/*`

**现状**: TiDev 使用基础的 `react-markdown` + `rehype-highlight`。功能有限。

**建议引入**:
- **LaTeX / KaTeX 数学公式渲染**
- **Mermaid 图表渲染**
- **表格增强**: 可排序列、CSV/TSV 复制、下载
- **大型代码块虚拟化**: 超过阈值时使用虚拟滚动
- **推理过程折叠 (ReasoningPart)**: 可折叠的"thinking"块，带实时计时器
- **代码块增强**: 语言标签、复制按钮、下载按钮、HTML 预览切换
- **流式渲染性能优化**: 流式过程中的节流渲染、延迟语法高亮
- **图像内联渲染**

**价值**: 高。直接影响用户阅读 AI 回复的体验。

### 4. Diff 预览增强

**参考**: OpenChamber `views/DiffView.tsx`, `views/PierreDiffViewer.tsx`, `chat/DiffPreview.tsx`

**现状**: TiDev 有基础 diff 渲染（`DiffRenderer.tsx`），支持 unified 模式。

**建议引入**:
- **并排 (Side-by-side) 模式**: 超过一定宽度时并排显示新旧代码
- **文件级折叠**: 每个 diff 文件可展开/折叠
- **语法高亮**: diff 中的代码语法高亮
- **行级注释**: 可在 diff 行上添加注释
- **展开/折叠所有**: 批量控制
- **大文件降级**: 内容超过阈值时降低高亮质量或懒加载
- **内联 (Inline) diff**: 在聊天消息中渲染紧凑型 diff

**价值**: 高。用户审查 AI 修改的核心交互。

### 5. 会话管理增强

**参考**: OpenChamber `session/SessionSidebar.tsx` (1818 行)

**现状**: TiDev 有基础会话列表，仅支持新建/删除/切换。

**建议引入**:
- **会话搜索/过滤**: 按标题、日期、项目搜索会话
- **会话分组**: 按项目/目录分组显示
- **会话重命名**: 双击重命名
- **会话状态指示器**: 显示流式/等待/错误/完成等实时状态
- **草稿会话**: 未发送消息的草稿状态
- **与会话文件夹**: 自定义文件夹组织会话，拖拽排序
- **归档/删除**: 带确认的归档和删除

**价值**: 高。用户每天频繁操作会话。

---

## P1: 中优先级 — 显著提升工作效率

### 6. Git 集成 (GitView)

**参考**: OpenChamber `views/GitView.tsx` (2309 行), `stores/useGitStore.ts`, `stores/useGitIdentitiesStore.ts`, `components/session/BranchPickerDialog.tsx`

**现状**: TiDev 无任何 git 功能。

**建议引入**:
- **更改展示**: 暂存/未暂存文件列表，状态指示器，文件类型图标
- **提交**: 提交信息输入、暂存文件摘要、作者支持、修改提交
- **历史**: 提交日志、分支图可视化、按提交展开 diff
- **分支管理**: 创建/切换/删除分支，上游跟踪
- **同步操作**: push/pull/fetch 带状态反馈
- **冲突解决**: 交互式冲突解决 UI
- **Stash**: Stash/unstash 管理
- **GitHub 集成**: PR 创建工作流、PR 状态检查

**价值**: 中-高。AI 编码场景中 git 操作频繁（创建分支、提交 AI 修改），但不是每天必须。

### 7. 终端 (TerminalView)

**参考**: OpenChamber `views/TerminalView.tsx` (1166 行), `components/terminal/TerminalViewport.tsx`, `stores/useTerminalStore.ts`

**现状**: TiDev 无终端功能。

**建议引入**:
- **xterm.js 集成**: 完整终端模拟器
- **多标签页终端**: 排序、添加、关闭、重排
- **SSE 流连接**: 实时终端流，带重试和指数退避
- **全屏模式**: 切换全屏终端
- **快捷键**: Esc、Tab、Enter 等虚拟键（移动端支持）

**价值**: 中。用户可通过聊天发送 bash 命令，但独立终端提供更灵活的操作。

### 8. 权限请求 UI (PermissionCard)

**参考**: OpenChamber `chat/PermissionCard.tsx`, `chat/PermissionRequest.tsx`, `stores/permissionStore.ts`

**现状**: TiDev 后端支持工具权限控制，但前端无可视化权限请求 UI。

**建议引入**:
- **权限请求卡片**: 工具调用权限时弹出审批/拒绝/始终允许的卡片
- **权限设置持久化**: 记住用户的权限选择
- **权限状态指示器**: 当前会话的权限状态和授权历史

**价值**: 中-高。对于安全性敏感的用户来说很重要。

### 9. Tool Call 渲染增强

**参考**: OpenChamber `chat/message/parts/ToolPart.tsx` (2670 行)

**现状**: TiDev 有基础 ToolCallRow 展示，按工具类型展示不同内容。

**建议引入**:
- **折叠/展开**: 工具调用默认折叠，点击展开显示详细输入输出
- **JSON 树视图**: 对 JSON 输出用可折叠的树形结构展示
- **持续时间显示**: 每个工具调用的耗时
- **文件类型图标**: 根据工具名称和参数显示对应图标
- **状态动画**: 运行中的工具调用显示加载动画

**价值**: 中。改善用户对 AI 工具调用过程的可见性。

---

## P2: 较低优先级 — 锦上添花

### 10. 设置面板扩展

**参考**: OpenChamber `views/SettingsView.tsx` (818 行), `components/sections/*`

**现状**: TiDev 设置面板极简，仅主题切换。

**建议引入**:
- **字体设置**: UI 字体、等宽字体、字号
- **Diff 布局设置**: 动态/内联/并排
- **行为设置**: 自定义提示词、回复风格预设
- **Provider 配置**: LLM provider 管理（API Key、Endpoint、模型列表）
- **键盘快捷键**: 查看和自定义快捷键
- **关于页面**: 版本信息、更新检查

**价值**: 中。属于"用了就回不去"的体验改进，但不是核心功能缺口。

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

### 第一阶段 (近期，高 Impact/Effort 比)
1. **多视图路由** — 基础设施，一切的基础
2. **文件浏览器** — AI 编码核心体验
3. **Markdown 渲染增强** — 直接提升体验
4. **Diff 预览增强** — 代码审查核心
5. **会话管理增强** — 日常使用优化

### 第二阶段 (中期)
6. **Git 集成** — 显著的效率提升
7. **终端集成** — 独立操作能力
8. **权限请求 UI** — 安全体验
9. **Tool Call 渲染增强** — 透明度提升
10. **设置面板扩展** — 自定义能力

### 第三阶段 (长期)
11. 语音、多模型并发、内联注释、计划视图、技能市场等

---

*文档生成日期: 2026-05-05*
