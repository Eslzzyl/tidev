# tidev web 实现计划

## 基本需求

- 一个基于 Svelte 5 的 web 前端（轻量、编译时优化）
- 响应式设计，适配宽屏和手机窄屏
- 适配目前 TUI 的大部分功能
- 通过 `tidev web` 命令启动 web 服务器

---

## 技术选型

### 通信协议：Server-Sent Events (SSE)

**选择理由**：
- 参考 OpenCode 架构，已被验证可靠
- 更好的网络穿透性（HTTP/1.1、兼容代理/防火墙）
- 浏览器原生支持自动重连
- 单向流模型完美匹配 LLM 场景

**对比 WebSocket**：
| 特性 | SSE | WebSocket |
|------|-----|-----------|
| 穿透性 | ✅ 标准 HTTP | ⚠️ 需协议升级 |
| 自动重连 | ✅ 原生支持 | ❌ 需手动实现 |
| 复杂度 | ✅ 单向简单 | ⚠️ 全双工状态管理 |
| 调试 | ✅ 常规 HTTP | ⚠️ 二进制帧 |

### 后端框架：Axum

**新增依赖**：
```toml
axum = { version = "0.8", features = ["tokio", "http2"] }
tower-http = { version = "0.6", features = ["fs", "cors", "trace"] }
tokio-stream = "0.1"
```

建议通过 cargo add 添加，确保使用最新版本。

### 前端框架：Svelte 5 + SvelteKit

**选择理由**：
- 编译时优化，运行时极小
- 响应式语法简洁（适合工具型应用）
- 与 TypeScript 集成良好

---

## 架构设计

### 后端模块结构

```
src/web/
├── mod.rs           # Web 模块入口，命令行解析
├── server.rs        # Axum 服务器启动
├── routes/
│   ├── mod.rs       # 路由聚合
│   ├── events.rs    # SSE 事件流 (/api/events)
│   ├── sessions.rs  # 会话 CRUD
│   ├── messages.rs  # 消息发送/获取
│   ├── models.rs    # 模型列表
│   ├── tools.rs     # 工具列表/权限
│   └── static.rs    # 前端静态文件
├── handlers/        # 请求处理逻辑（直接调用核心模块）
│   ├── mod.rs
│   ├── chat.rs      # 消息处理（调用 llm、tooling）
│   └── session.rs   # 会话管理（调用 storage）
├── event_bus.rs     # 内部事件总线 (tokio::sync::broadcast)
├── state.rs         # AppState 共享状态
└── auth.rs          # 简单认证（可选）
```

### 事件总线设计

**核心机制**：
- 使用 `tokio::sync::broadcast` 实现多播
- 每个 SSE 连接订阅一个 receiver
- LLM 流式输出、工具调用、权限请求均通过事件总线广播

**事件类型**：
```rust
enum AppEvent {
    MessageChunk { session_id, content },
    MessageComplete { session_id },
    ToolCall { session_id, tool_name, args },
    ToolResult { session_id, output },
    PermissionRequest { session_id, tool_name },
    Heartbeat,
}
```

### SSE 关键配置

**必须设置的 Headers**：
```
Cache-Control: no-cache, no-transform
X-Accel-Buffering: no          # 禁用 Nginx 缓冲
X-Content-Type-Options: nosniff
Content-Type: text/event-stream
```

**心跳机制**：每 10 秒发送一次 `{}` 保持连接

### API 设计

| 方法 | 端点 | 说明 |
|------|------|------|
| GET | `/api/events?session={id}` | SSE 事件流 |
| GET | `/api/sessions` | 列出会话 |
| POST | `/api/sessions` | 创建会话 |
| GET | `/api/sessions/{id}/messages` | 获取消息历史 |
| POST | `/api/sessions/{id}/messages` | 发送消息（触发 SSE 流） |
| POST | `/api/sessions/{id}/abort` | 中止生成 |
| GET | `/api/models` | 可用模型列表 |
| GET | `/api/tools` | 可用工具列表 |
| POST | `/api/permissions/grant` | 授权工具执行 |
| POST | `/api/permissions/deny` | 拒绝工具执行 |

### 为什么不使用 gateway::Channel

`gateway::Channel` trait 的设计目的是**多平台适配**（Telegram、QQ 等），每个渠道需要：
- 不同的消息格式转换
- 独立的会话存储和连接管理
- 平台特定的 API 适配

**Web 服务不同**：
- 单一 HTTP 端点，统一 JSON 格式
- 所有客户端共享同一个服务器实例
- 直接调用核心模块更简单清晰

**推荐架构**：HTTP Handler 直接调用核心模块

```rust
// handlers/chat.rs
pub async fn send_message(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Json(body): Json<SendMessageRequest>,
) -> Result<impl IntoResponse, AppError> {
    // 1. 直接从 storage 加载会话
    let session = state.store.get_session(session_id)?;
    
    // 2. 调用 LLM 客户端
    let llm = LlmClient::new()?;
    let stream = llm.chat_stream(&build_messages(&session, &body.content)).await?;
    
    // 3. 流式响应通过 event_bus 推送 SSE
    tokio::spawn(async move {
        for chunk in stream {
            state.event_bus.publish(AppEvent::MessageChunk {
                session_id,
                content: chunk.content,
            });
        }
    });
    
    Ok(Json({ "status": "ok" }))
}
```

---

## 前端架构

### 项目结构

```
packages/web/
├── src/
│   ├── lib/
│   ├── api.ts          # REST API 客户端
│   ├── sse.ts          # EventSource 封装（自动重连）
│   └── stores/
│       ├── session.ts  # 当前会话状态
│       ├── messages.ts # 消息列表
│       └── ui.ts       # UI 状态（主题、面板）
├── components/
│   ├── ChatPanel/      # 聊天主面板
│   ├── SessionList/    # 会话侧边栏
│   ├── MessageInput/   # 输入框（支持 @ 提及）
│   ├── MessageBubble/  # 消息气泡
│   ├── ToolCall/       # 工具调用展示
│   ├── PermissionDialog/ # 权限确认弹窗
│   ├── ModelSelector/  # 模型选择
│   └── SettingsPanel/  # 设置面板
├── routes/
│   ├── +page.svelte    # 主页面
│   └── +layout.svelte  # 根布局
├── app.html
├── static/
├── svelte.config.js
└── vite.config.ts
```

### SSE 客户端封装

**关键功能**：
- 自动重连（浏览器原生支持）
- 事件类型路由（`message.chunk`, `tool.call` 等）
- 连接状态指示器

**示例**：
```typescript
class EventSourceClient {
  private es: EventSource | null = null;
  private reconnectAttempts = 0;
  
  connect(sessionId: string) {
    this.es = new EventSource(`/api/events?session=${sessionId}`);
    
    this.es.addEventListener("message.chunk", (e) => {
      const data = JSON.parse(e.data);
      messagesStore.appendChunk(data);
    });
    
    this.es.addEventListener("tool.call", (e) => {
      const data = JSON.parse(e.data);
      permissionStore.showDialog(data);
    });
    
    this.es.onerror = () => {
      console.log("连接中断，自动重连中...");
    };
  }
}
```

### 响应式设计

**断点设计**：
- 移动端：< 768px（单栏，底部输入）
- 平板：768px - 1024px（可折叠侧边栏）
- 桌面：> 1024px（双栏，左侧会话列表）

**移动端适配要点**：
- 会话列表为抽屉式侧边栏
- 输入框固定在底部
- 消息气泡宽度自适应

---

## 关键注意事项

### 1. 工具权限处理

**挑战**：TUI 可以阻塞等待用户确认，Web 需要异步处理。

**方案**：
- 工具调用时发送 `PermissionRequest` 事件
- 前端显示模态对话框
- 用户选择后 POST 到 `/api/permissions/grant` 或 `/deny`
- 后端继续或中断 LLM 流程

### 2. 消息流式渲染

**优化**：
- 使用虚拟列表（大量消息时）
- Markdown 增量渲染（避免频繁重排）
- 代码块语法高亮（使用 highlight.js 或 shiki）

### 3. 文件引用（@）

**实现**：
- 输入框监听 `@` 字符
- 弹出文件选择器（基于当前工作区）
- 支持模糊搜索文件名
- 显示为可删除的标签

### 4. 会话恢复

**机制**：
- 页面刷新后从 `/api/sessions` 恢复列表
- 自动连接到最后活跃的会话
- URL 携带 `?session=uuid` 支持直接分享

### 5. 静态文件嵌入

**生产构建**：
- 前端构建输出到 `packages/web/dist/`
- Rust 编译时通过 `include_dir!` 嵌入
- Axum 使用 `tower_http::services::ServeDir` 或内存服务

### 6. 安全性

**建议**：
- 默认只绑定 `127.0.0.1`（本地访问）
- 可选简单 token 认证（通过 query param 或 header）
- CORS 配置为只允许同源（或配置指定域名）

---

## 开发计划

### 阶段 1：后端 MVP（1 周）
- [ ] Axum 服务器基础框架
- [ ] SSE 事件流实现
- [ ] 会话 CRUD API（直接调用 storage 模块）
- [ ] 消息发送/接收 API（直接调用 llm、tooling 模块）
- [ ] 事件总线集成（tokio::sync::broadcast）

### 阶段 2：前端基础（1 周）
- [ ] SvelteKit 项目初始化
- [ ] SSE 客户端封装
- [ ] 会话列表组件
- [ ] 聊天面板组件
- [ ] 消息输入组件

### 阶段 3：核心功能（1 周）
- [ ] 消息流式渲染
- [ ] Markdown 渲染 + 代码高亮
- [ ] 模型选择面板
- [ ] 工具权限对话框
- [ ] 文件引用（@）功能

### 阶段 4：完善与优化（3-5 天）
- [ ] 响应式适配（移动端）
- [ ] 设置面板
- [ ] 静态文件嵌入
- [ ] 构建流程集成
- [ ] 端到端测试

**总计：约 2.5-3 周**

---

## 参考资源

- OpenCode SSE 实现：`opencode/packages/opencode/src/server/routes/instance/event.ts`
- Axum SSE 文档：https://docs.rs/axum/latest/axum/response/sse/index.html
- Svelte 5 文档：https://svelte.dev/docs
