# TiDev 文件管理功能改进计划

本文档基于当前实现与 [OpenChamber](https://github.com/openchamber/openchamber) 的对比分析，制定后续优化计划。

---

## 当前已完成的文件管理功能

| 功能 | 状态 | 说明 |
|------|------|------|
| 文件树浏览 | ✅ | 递归展开/折叠，懒加载，文件类型图标 |
| 文件搜索 | ✅ | 300ms debounce 搜索 |
| 文件读取 | ✅ | `GET /api/fs/read` |
| **文件写入** | ✅ | **`POST /api/fs/write`** |
| **文件创建/删除/重命名** | ✅ | **`POST /api/fs/create`, `POST /api/fs/rename`, `DELETE /api/fs/remove`** |
| CodeMirror 编辑器 | ✅ | Lezer 语法高亮，20+ 语言 |
| 编辑/只读切换 | ✅ | 不支持脏状态时确认切换 |
| Ctrl+S 保存 | ✅ | 通过 CustomEvent 通信 |
| 脏状态跟踪 | ✅ | "Modified" 徽章 + 保存按钮 |
| Light/Dark 主题 | ✅ | 跟随系统/用户设置 |
| **右键上下文菜单** | ✅ | **新建文件/目录、重命名、删除、复制路径** |
| **对话框系统** | ✅ | **创建对话框、重命名对话框、删除确认对话框** |
| **Toast 通知** | ✅ | **成功/错误/警告/信息，自动消失** |
| **文件树底部按钮** | ✅ | **新建文件、新建目录、刷新按钮** |
| **多标签页** | ✅ | **同时打开多个文件，标签栏切换，关闭/激活/脏状态指示** |
| **Git 状态徽章** | ✅ | **文件树显示 M/A/D/? 状态，脚注显示当前分支名** |
| **多格式预览** | ✅ | **图片内联渲染、Markdown 预览、JSON 树视图** |

---

## 改进计划

### Phase 1 — 基础文件操作 (P0) ✅ 已完成

**目标**: 补齐文件 CRUD 能力，完善编辑体验 **✅ 已完成**

#### 1.1 后端: 文件创建/重命名/删除 API ✅

已新增 3 个后端端点：

| 端点 | 方法 | 说明 |
|------|------|------|
| `/api/fs/create` | POST | 创建文件或目录，请求体 `{ path, type: "file"|"directory" }` |
| `/api/fs/rename` | POST | 重命名/移动文件，请求体 `{ path, new_path }` |
| `/api/fs/remove` | DELETE | 删除文件或空目录，请求体 `{ path }` |

**安全考虑**:
- 复用 `resolve_path()` 进行路径合法性校验
- 删除目录时先检查是否为空（或者加 `recursive` 参数）
- 重命名时检查目标路径是否在 workspace 内

**参考**: OpenChamber `server/lib/routes/files.ts`

**文件**: `src/web/routes/fs.rs`, `src/web/routes/mod.rs` ✅

#### 1.2 前端: API 客户端扩展 ✅

```typescript
api.createFile(path, type)   → POST /api/fs/create
api.renameFile(path, newPath) → POST /api/fs/rename
api.removeFile(path)          → DELETE /api/fs/remove
```

**文件**: `web/src/api/client.ts`, `web/src/types/api.ts`

#### 1.3 前端: 右键上下文菜单 ✅

在 `FileTree.tsx` 中添加右键菜单，支持以下操作：

| 操作 | 逻辑 |
|------|------|
| 新建文件 | 弹出文件名输入对话框，调用 `createFile` |
| 新建目录 | 同上 |
| 重命名 | 内联编辑文件名，调用 `renameFile` |
| 删除 | 确认对话框，调用 `removeFile` |
| 复制路径 | 复制文件路径到剪贴板 |
| 在文件管理器打开 | 打开系统文件管理器（需后端 `open` 命令） |

**组件设计**:
```
ContextMenu           — 浮动菜单容器（定位、动画）
├── MenuItem          — 单个菜单项
├── MenuSeparator     — 分隔线
└── ConfirmDialog     — 删除确认
```

**文件**: `web/src/components/ui/ContextMenu.tsx`, `web/src/stores/useFileStore.ts` (新增 `createFile`, `renameFile`, `deleteFile` 方法) ✅

#### 1.4 前端: 菜单按钮 ✅

文件树底部增加"新建"按钮（+图标），提供与右键菜单相同的操作，保证键盘可访问性。

**文件**: `web/src/components/views/FileTree.tsx`, `web/src/components/views/FilesView.tsx` ✅

#### 1.5 新增: Toast 通知系统 ✅

文件操作需要反馈（成功/失败），当前没有通知机制。新建轻量级 toast 组件：

```typescript
// 使用方式
toast.success("File created");
toast.error("Failed to delete file");
```

**文件**: `web/src/components/ui/Toast.tsx`, `web/src/stores/useToastStore.ts` ✅

---

### Phase 2 — 多标签页支持 (P1) ✅ 已完成

**目标**: 支持同时打开多个文件，标签栏切换，持久化标签状态 ✅ 已完成

#### 2.1 存储层改造 ✅

`useFileStore` 改为多文件存储：

```typescript
interface OpenFile {
  path: string;
  content: string;
  language: string | null;
  isDirty: boolean;
  originalContent: string;
}

openFiles: OpenFile[];        // 所有打开的标签
activeFilePath: string | null; // 当前激活的标签
```

核心变更：
- `openFilePath`/`openFileContent`/`openFileLanguage`/`isDirty`/`originalContent` → 统一为 `openFiles: OpenFile[]` + `activeFilePath`
- `openFile(path)` → 加载文件并加入标签列表，已打开则直接切换
- `closeFile(path)` → 关闭指定标签，自动选择下一个
- `updateFileContent(path, content)` → 按路径更新指定文件内容
- 重命名/删除时自动更新或关闭对应标签

**文件**: `web/src/stores/useFileStore.ts` ✅

#### 2.2 标签栏组件 ✅

`FileTabs` 组件已创建，类似 IDE 的标签栏，水平排列：

```
┌──────────┬──────────┬──────┬───────────────┐
│ App.tsx ●│ styles…  │ +    │  <file tabs>  │
└──────────┴──────────┴──────┴───────────────┘
```

- 激活的文件高亮
- 脏文件显示圆点（●）
- 每个标签有关闭按钮（hover 时显示）
- 标签过多时水平滚动
- 默认显示文件名，title 展示完整路径

**文件**: `web/src/components/views/FileTabs.tsx` ✅

#### 2.3 FilesView 布局调整 ✅

FilesView 右侧面板更新为：

```
┌──────────────────────────────────────────┐
│ FileTabs (标签栏)                        │
├──────────────────────────────────────────┤
│ CodeMirrorEditor (当前标签内容)          │
└──────────────────────────────────────────┘
```

**文件**: `web/src/components/views/FilesView.tsx`

---

### Phase 3 — Git 状态集成 (P1) ✅ 已完成

**目标**: 在文件树中显示文件的 Git 状态（M/A/D/?），让用户直观看到修改 ✅ 已完成

#### 3.1 后端

`GET /api/git/status` 已存在，返回格式：
```json
{
  "branch": "main",
  "sha": "abc123",
  "files": [
    { "path": "src/main.rs", "status": "M", "staged": true },
    { "path": "new_file.ts", "status": "?", "staged": false }
  ]
}
```

无需后端改动。

#### 3.2 前端: Git Store ✅

新建 `useGitFileStore` 缓存 Git 状态：
- 文件树加载时自动拉取 Git 状态
- 缓存已解析的 `path → status` 映射
- 提供刷新方法

```typescript
interface GitFileStore {
  statusMap: Record<string, { status: string; staged: boolean }>;
  loading: boolean;
  refresh: () => Promise<void>;
}
```

**文件**: `web/src/stores/useGitFileStore.ts` ✅

#### 3.3 前端: FileTree 添加徽章 ✅

在 `FileTree.tsx` 的树节点旁显示：

| 状态 | 徽章 | 颜色 |
|------|------|------|
| Modified (staged) | `M` | 绿色 |
| Modified (unstaged) | `M` | 橙色 |
| Added | `A` | 绿色 |
| Deleted | `D` | 红色 |
| Untracked | `?` | 灰色 |
| Renamed | `R` | 蓝色 |

**文件**: `web/src/components/views/FileTree.tsx`

---

### Phase 4 — 多格式文件预览 (P2) ✅ 已完成

**目标**: 根据文件类型自动选择最佳预览方式 ✅ 已完成

#### 4.1 CodeViewer 添加多模式渲染 ✅

当前 `CodeViewer.tsx` 对所有文件都用 CodeMirror 渲染。改为按扩展名分发：

```
文件打开
├── .png/.jpg/.gif/.svg → img 内联渲染
├── .md/.markdown → MarkdownRenderer 预览 + CodeMirror 编辑切换
├── .json → 结构化 JSON 树视图 + CodeMirror 编辑切换
├── 其他文本文件 → CodeMirrorEditor（默认）
```

**组件拆分**:
- `CodeViewer.tsx` — 降级为路由/分发层
- `ImagePreview.tsx` — 图片预览
- `MarkdownPreview.tsx` — Markdown 预览
- `JsonTreeView.tsx` — JSON 树（可复用已有的 `JsonTreeView` 在 `web/src/components/ui/` 中）

**文件**: `web/src/components/views/CodeViewer.tsx`, `web/src/components/views/ImagePreview.tsx`, `web/src/components/views/MarkdownPreview.tsx` ✅

#### 4.2 工具栏按钮 ✅

在 CodeViewer 的标题栏添加预览/编辑切换按钮（类似 OpenChamber 的浮动工具栏）：
- 自动换行切换
- 跳转到行
- 全屏
- 在系统应用中打开

**文件**: `web/src/components/views/CodeViewer.tsx`

---

### Phase 5 — 体验优化 (P2)

#### 5.1 跳转到行对话框

CodeMirror 已有行号，但缺少跳转输入。新增 `GoToLineDialog` 组件：
- 快捷键 `Ctrl+G` / `Cmd+G`
- 输入行号后自动滚动到指定行
- 显示总行数

**文件**: `web/src/components/ui/GoToLineDialog.tsx`

#### 5.2 文件搜索优化

当前搜索使用 300ms debounce + `/api/files/search` 端点。改进方向：
- 增加搜索缓存（避免重复查询相同关键词）
- 搜索结果中加入文件类型过滤
- 高亮匹配文本片段

**文件**: `web/src/stores/useFileStore.ts` 或新建 `web/src/stores/useFileSearchStore.ts`

#### 5.3 文件树性能优化

当前文件树对大目录（数千个文件）性能可能不佳：
- 虚拟滚动（使用 `@tanstack/react-virtual`）
- 懒加载子树（已有，但可优化）
- 文件树节点增量更新（避免全量重建）

---

### Phase 6 — 高级功能 (P3)

#### 6.1 多文件编辑体验

- 对比查看（2 个文件并排对比）
- 文件变更侧边栏（AI 修改的文件列表）

#### 6.2 拖拽操作

- 文件/目录拖拽移动
- 将文件拖入聊天输入框自动插入 `@-mention`

#### 6.3 文件监控

- WebSocket/SSE 推送文件变更事件
- 编辑器外修改文件时自动刷新内容（需要后端 `inotify`/`kqueue` 支持）

---

## 实施路线图

| 阶段 | 功能 | 预估工作量 | 依赖 |
|------|------|-----------|------|
| **Phase 1** | 文件 CRUD + 右键菜单 + Toast | ✅ **已完成** | 无 |
| **Phase 2** | 多标签页 | ✅ **已完成** | Phase 1 |
| **Phase 3** | Git 状态徽章 | ✅ **已完成** | 无（后端 API 已存在） |
| **Phase 4** | 多格式预览 | ✅ **已完成** | 无 |
| **Phase 5** | 跳转到行/搜索优化 | 1-2 天 | 无 |
| **Phase 6** | 拖拽/文件监控 | 3-5 天 | Phase 1-2 完成 |

---

## 不建议近期引入的功能

| 功能 | 原因 |
|------|------|
| **内联代码评论** | 复杂度高（CodeMirror Block Widget + createPortal），tidev 目前单用户无协作需求 |
| **远程隧道/WebDAV** | tidev 定位为本地工具，远程访问场景极少 |
| **文件历史版本** | 依赖后端存储，超出文件管理范围（可用 Git 替代） |
