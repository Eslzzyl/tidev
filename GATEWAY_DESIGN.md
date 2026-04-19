# Tidev Gateway 设计文档

> **状态**: 规划中
> **版本**: 0.1.0
> **目标**: 为 Tidev 添加 Gateway 支持，实现 Telegram 等消息通道接入

---

## 1. 概述

本文档描述 Tidev Gateway 的架构设计，目标是将 Tidev 从纯 TUI 应用扩展为支持 HTTP/WebSocket Gateway 的多模式应用。

### 1.1 启动模式

```
tidev                 # TUI 模式（默认）
tidev gateway         # Gateway 模式
tidev web             # Web 模式（未来）
```

### 1.2 设计原则

1. **最小改动**：不改变现有 Cargo.toml 结构，Gateway 代码放在 `src/gateway/` 目录
2. **选择性复用**：优先从 zeroclaw 复制代码，注明来源和版本
3. **共享核心**：TUI 和 Gateway 共享项目核心组件

---

## 2. 架构设计

### 2.1 目录结构

```
tidev/
├── src/
│   ├── main.rs              # CLI 路由（TUI/Gateway）
│   ├── lib.rs               # 模块导出
│   ├── gateway/
│   │   ├── mod.rs           # Gateway 核心
│   │   ├── telegram.rs      # Telegram channel（复制自 zeroclaw）
│   │   ├── api.rs           # HTTP API
│   │   ├── ws.rs            # WebSocket
│   │   ├── sse.rs           # Server-Sent Events
│   │   └── rate_limit.rs    # Rate limiter
│   ├── config.rs            # 配置（扩展现有配置）
│   ├── app/                 # 现有 TUI 代码
│   ├── storage.rs           # 现有 SessionStore
│   └── llm.rs               # 现有 LlmClient
└── Cargo.toml               # 添加 gateway 依赖
```

### 2.2 架构图

```
┌─────────────────────────────────────────────────────────┐
│                     Tidev Binary                         │
├─────────────────────────────────────────────────────────┤
│  CLI Router                                             │
│  ├── tidev         → TUI Mode (src/app/)               │
│  ├── tidev gateway → Gateway Mode (src/gateway/)       │
│  └── tidev web     → Web Mode (未来)                   │
├─────────────────────────────────────────────────────────┤
│  共享核心                                               │
│  ├── SessionStore (src/storage.rs)                     │
│  ├── LlmClient (src/llm.rs)                            │
│  ├── AppConfig (src/config.rs)                         │
│  └── ToolRegistry (src/tooling.rs)                     │
├─────────────────────────────────────────────────────────┤
│  Gateway 模块 (src/gateway/)                           │
│  ├── Axum Router                                       │
│  ├── Telegram Channel                                  │
│  ├── WebSocket Handler                                 │
│  └── SSE Endpoint                                      │
└─────────────────────────────────────────────────────────┘
```

---

## 3. 依赖方案

### 3.1 新增依赖

```toml
# Cargo.toml
[dependencies]
# 现有依赖...

# Gateway 新增
axum = "0.8"
tower = "0.56"
tower-http = "0.59"
tokio-tungstenite = "0.26"
```

### 3.2 与 zeroclaw 的代码复用策略

| zeroclaw crate | 处理方式 | 说明 |
|----------------|----------|------|
| `zeroclaw-api` | 复制 traits | 复制 `Channel`, `SendMessage` 等 trait 定义到 `src/gateway/traits.rs` |
| `zeroclaw-gateway` | 复制核心 | 复制 Axum router、WS、SSE、rate limiter 到 `src/gateway/` |
| `zeroclaw-channels/src/telegram.rs` | 复制 | 复制到 `src/gateway/telegram.rs` |
| `zeroclaw-config` | 仅参考 | 不复制，扩展现有 `AppConfig` |
| `zeroclaw-runtime` | 不复用 | 太重，Tidev 已有自己的运行时 |
| `zeroclaw-memory` | 不复用 | Tidev 用 SQLite |

### 3.3 代码来源标注

所有复制的代码需要标注来源：

```rust
// src/gateway/telegram.rs
// 复制自: https://github.com/zeroclaw-labs/zeroclaw
// 来源: crates/zeroclaw-channels/src/telegram.rs
// 版本: zeroclaw v0.7.3
// 许可: MIT OR Apache-2.0
//
// 主要改动:
// - 移除 zeroclaw-api 依赖，使用本地 Channel trait
// - 替换 Config → AppConfig
// - 移除 pairing 相关逻辑
```

---

## 4. 模块设计

### 4.1 Gateway 核心 (src/gateway/mod.rs)

```rust
// Gateway 配置
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    pub require_pairing: bool,
    pub max_body_size: usize,
    pub request_timeout_secs: u64,
}

// Gateway 状态
pub struct GatewayState {
    pub tx: mpsc::Sender<ChannelMessage>,
    pub rate_limiter: GatewayRateLimiter,
    pub session_backend: SessionStore,
}

// 启动 Gateway
pub async fn run_gateway(config: GatewayConfig, state: GatewayState) -> anyhow::Result<()> {
    // 使用 Axum 构建路由
    let app = Router::new()
        .route("/health", get(health))
        .route("/api/health", get(api_health))
        .route("/api/chat", post(chat))
        .route("/ws", get(ws_handler))
        .route("/sse", get(sse_handler))
        .with_state(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

### 4.2 Telegram Channel (src/gateway/telegram.rs)

复制自 `zeroclaw/crates/zeroclaw-channels/src/telegram.rs`

主要实现：
- `impl Channel for TelegramChannel`
- `async fn send(&self, message: &SendMessage)`
- `async fn listen(&self, tx: Sender<ChannelMessage>)`
- Draft streaming 支持
- 附件处理（图片、文档、音频）

### 4.3 HTTP API (src/gateway/api.rs)

```rust
// 健康检查
async fn health() -> impl IntoResponse;
async fn api_health() -> impl IntoResponse;

// 聊天接口
async fn chat(Json(payload): Json<ChatRequest>) -> impl IntoResponse;

// 管理接口
async fn admin_shutdown() -> impl IntoResponse;
async fn get_pairing_code() -> impl IntoResponse;
```

### 4.4 WebSocket (src/gateway/ws.rs)

```rust
async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket))
}

async fn handle_socket(socket: WebSocket) -> Result<(), ws::Error> {
    // 实现 WebSocket 握手和消息处理
}
```

### 4.5 Rate Limiter (src/gateway/rate_limit.rs)

复制自 `zeroclaw/crates/zeroclaw-gateway/src/auth_rate_limit.rs`

```rust
pub struct SlidingWindowRateLimiter {
    window: Duration,
    max_requests: u32,
    storage: Mutex<HashMap<String, Instant>>,
}

impl SlidingWindowRateLimiter {
    pub fn new(max_requests: u32, window: Duration, max_keys: usize) -> Self;
    pub fn allow(&self, key: &str) -> bool;
}
```

---

## 5. 配置设计

### 5.1 配置结构

```rust
// src/config.rs 扩展

pub struct GatewayConfig {
    /// Gateway 监听地址
    pub host: String,
    /// Gateway 监听端口
    pub port: u16,
    /// 是否需要配对
    pub require_pairing: bool,
    /// 最大请求体大小
    pub max_body_size: usize,
    /// 请求超时（秒）
    pub request_timeout_secs: u64,
}

pub struct TelegramConfig {
    /// Telegram Bot Token
    pub bot_token: Option<String>,
    /// 是否仅监听 @mention
    pub mention_only: bool,
    /// 流式模式
    pub stream_mode: StreamMode,
}
```

### 5.2 配置加载

```toml
# config.toml

[gateway]
host = "127.0.0.1"
port = 8080
require_pairing = true

[telegram]
bot_token = "your-bot-token"
mention_only = false
```

---

## 6. 实现阶段

### Phase 1: Gateway 基础 (1-2天)

1. 创建 `src/gateway/mod.rs` 基础框架
2. 实现 CLI 路由 (`tidev gateway` 命令)
3. 添加 Axum 依赖到 Cargo.toml
4. 实现基础 `/health` 端点
5. 复制 zeroclaw rate limiter

### Phase 2: Telegram 接入 (2-3天)

1. 复制 `zeroclaw-channels/src/telegram.rs` 到 `src/gateway/telegram.rs`
2. 实现 `Channel` trait
3. 添加 Telegram 配置项
4. 实现长轮询 `listen()` 方法
5. 测试 Telegram Bot API 集成

### Phase 3: API 集成 (2-3天)

1. 实现 `/api/chat` 端点
2. 对接 Tidev `LlmClient`
3. 实现 Webhook → LLM → Telegram 消息流
4. 添加 WebSocket 支持

### Phase 4: 测试与文档 (1-2天)

1. 编写单元测试
2. 编写集成测试
3. 更新 README
4. 添加示例配置

---

## 7. 与 zeroclaw 的主要差异

| 方面 | zeroclaw | Tidev |
|------|----------|-------|
| Crate 结构 | workspace 多 crate | 单一 crate |
| Gateway 位置 | 独立 crate | `src/gateway/` |
| 配对机制 | 完整 pairing 系统 | 简化版或使用现有认证 |
| 会话存储 | zeroclaw-memory | SQLite (现有) |
| 运行时 | 独立守护进程 | Tidev App |
| 消息格式 | zeroclaw ChannelMessage | Tidev Message |

---

## 8. 风险与缓解

| 风险 | 级别 | 缓解措施 |
|------|------|----------|
| 消息格式差异 | 中 | 定义 Tidev 自己的 `ChannelMessage` 类型 |
| WebSocket 兼容性 | 低 | tower-http 已覆盖主流场景 |

---

## 9. 参考资料

- [zeroclaw](https://github.com/zeroclaw-labs/zeroclaw) - 源代码参考
- [Telegram Bot API](https://core.telegram.org/bots/api) - Telegram 接口文档
- [axum](https://docs.rs/axum/) - HTTP framework
- [tokio-tungstenite](https://docs.rs/tokio-tungstenite/) - WebSocket

---

## 10. 变更历史

| 日期 | 版本 | 变更内容 |
|------|------|----------|
| 2024-01-01 | 0.1.0 | 初始文档 |
