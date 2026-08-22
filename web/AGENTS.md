# tidev Web 前端指导

本目录（`web/`）是 tidev 的 Web 前端：一个独立的 Vite + React + TypeScript 单页
应用（包名 `web-react`），由 pnpm 管理，不属于 Cargo workspace。后端集成与静态
资源托管由 `crates/tidev-web` 负责。操作本目录时同样遵守仓库根目录 `AGENTS.md`
的全部铁律。

## 技术栈

- React 19 + TypeScript + Vite（构建基于 rolldown），PWA 由 vite-plugin-pwa 提供。
- 样式使用 Tailwind CSS v4；客户端状态用 Zustand，服务端数据用 TanStack Query。
- 编辑器为 CodeMirror 6（按语言动态分包）；Markdown 渲染链路为 react-markdown +
  remark-gfm/remark-math + rehype-highlight/rehype-katex，另支持 mermaid 图表。
- 终端视图基于 restty（内嵌 WASM 运行时）。
- 测试用 vitest（jsdom + Testing Library），lint 用 oxlint，格式化用 oxfmt。

## 与 Rust 后端的边界

- 前端只通过 HTTP/WebSocket 与 `tidev-web` 服务通信：REST 走 `/api/*`，事件流走
  `/api/events`（SSE），终端走 WebSocket PTY，认证使用 Bearer token。
  后端类型对应 `src/types/api.ts` 与 `src/types/chat.ts`。
- 日常入口：在仓库根目录运行 `cargo run -- web`，浏览器访问
  `http://127.0.0.1:26502/`。Rust 服务会拉起 Vite 并代理浏览器请求，`/api`
  由 Rust 直接提供；Vite 的 `5173` 只是内部开发端口，不是正常入口。
- pnpm 或前端依赖不可用时，Rust 服务保持运行并返回诊断 fallback 页面，
  `/api` 不受影响。
- 发布构建：`cargo build --release` 会执行 `pnpm install --frozen-lockfile`
  和 `pnpm build`，并把压缩后的 `web/dist` 嵌入二进制
  （见 `crates/tidev-web/build.rs`），用户无需单独部署 web 目录。
- 依赖解析以 `pnpm-lock.yaml` 为准；本项目刻意不固定顶层 `packageManager`
  版本，最低 pnpm 要求通过 `engines.pnpm` 表达。

## 目录结构

- `src/api/`：REST 客户端（client.ts）与 SSE 事件流（events.ts）。
- `src/stores/`：Zustand 全局状态（会话、UI、终端、认证、文件等）。
- `src/hooks/`：TanStack Query 封装（sessionQueries、gitQueries、statsQueries、
  workspaceQueries、useChatRuntime 等）与通用 hooks。
- `src/components/`：界面组件，按 chat / views（含 git）/ settings / renderers /
  ui / layout 分组。
- `src/lib/codemirror/`：CodeMirror 配置与语言扩展；`src/lib/` 下还有
  queryClient 与 gitGraph 等纯逻辑模块。
- `src/terminal/`：restty 终端的 WebSocket 连接与传输层。
- `src/i18n/`：i18next 多语言配置与语言资源。
- `src/types/`：API 与聊天消息类型定义；`src/utils/`：通用工具函数。

## 常用命令

在 `web/` 下直接运行，或从仓库根目录加 `--dir web` 前缀：

- `pnpm install --frozen-lockfile` —— 安装依赖
- `pnpm dev` —— Vite 开发服务器（5173，`/api` 代理到 127.0.0.1:26502）
- `pnpm build` —— `tsc -b` 类型检查 + Vite 构建到 `dist/`
- `pnpm lint` / `pnpm lint:fix` —— oxlint 检查与自动修复
- `pnpm format` / `pnpm format:check` —— oxfmt 格式化与检查
- `pnpm test` / `pnpm test:watch` —— vitest 单元测试

## 工作约束

- 代码与注释一律使用英文（继承根 `AGENTS.md` 铁律）。
- 修改前端代码后至少运行 `pnpm lint` 与 `pnpm test`；涉及类型的改动确保
  `pnpm build` 通过。
- 不要绕过 `/api` 边界直接访问后端端口或文件系统；新增后端能力先在
  `crates/tidev-web` 暴露 API，再在前端消费。
