# 测试覆盖改善计划

## 现状

参见 [`gaps.md`](./gaps.md) 中的详细分析。概览：

| 指标 | 数值 |
|------|------|
| 项目总代码量 | ~76,798 LOC |
| 测试函数总数 | 283 |
| 零测试 crate | `tidev-logging` |
| 严重不足 | `tidev-core` (3,332 LOC / 4 tests)、`tidev-llm` (5,738 LOC / 12 tests) |
| CI 测试 | **没有** — 仅有 release.yml，无 `cargo test` 步骤 |
| 覆盖率工具 | 无 |
| 集成测试 | 无 |
| 文档测试 | 所有代码示例标记为 `ignore`，不编译不运行 |
| Mock 策略 | 无 — 所有测试依赖真实 I/O |

---

## 指导原则

### 按层选择测试策略，不搞一刀切

| 层 | 策略 | 理由 |
|----|------|------|
| **纯函数**（压缩/解析/格式化/序列化） | 直接单元测试，无需 mock | 确定性，快 |
| **SQLite 存储** | 真实数据库 + `tempfile` | SQLite 行为确定，tempfile 自动清理，比 mock 更可靠 |
| **文件系统操作**（snapshot/undo/attachment） | 真实 FS + `tempfile::TempDir` | 同上 |
| **HTTP/LLM API** | **Mock HTTP 层** | 无法控制远端；错误码、重试、超时、限流等场景必须 mock 才能覆盖 |
| **核心编排层**（runtime/context/session） | **Trait + Mock** | 正确性高度依赖外部行为注入，只有 mock 能覆盖所有路径 |
| **日志** | 纯函数验证 + 文件输出验证 | LogConfig 过滤、ANSI 格式化，简单可测 |

### 其他原则

- 不引入 `proptest`、`insta` 等额外框架 — 保持轻量
- 不提前拆分 inline test module — 只在 `mod tests` 膨胀到影响编译缓存时再外提
- 新增功能必须附带测试
- 每次提交保持 `cargo test --workspace` 通过

---

## Phase 1 — 基础设施

### 1.1 CI 测试流水线

新建 `.github/workflows/ci.yml`：

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: cargo test --workspace
      - run: cargo clippy --workspace -- -D warnings
```

### 1.2 新建 `tidev-test-utils` 共享 crate

从 archive 抽取可复用的测试 helper，避免各 crate 重复定义。

结构：
```
crates/tidev-test-utils/
  Cargo.toml
  src/
    lib.rs              # re-exports
    conversation.rs     # test_conversation() builder
    model.rs            # test_model() / test_active_model() builder
    temp.rs             # TempDir + TempGuard helpers
    fixture.rs          # serde_json::json!() fixture 加载
```

Dev-dependencies（只对 test 可见）：
- `tempfile`
- `tidev-llm`、`tidev-tools` 和 `tidev-core` 的协议、工具及应用数据
- `uuid`
- `chrono`
- `serde_json`

**注意：** 这不是生产 crate，只用 `[dev-dependencies]` 引入，不出现在 workspace 的 `default-members` 中。

---

## Phase 2 — 关键 crate 补齐

按严重程度排序。每个 crate 标记了目标 test 数量和关键测试场景。

### 2.1 `tidev-storage`（当前 4 → 目标 ~25）

依赖：`tidev-test-utils` + `tempfile`

文件：`src/lib.rs`（追加 `#[cfg(test)] mod tests`）

场景：
- Session 完整 CRUD round-trip（create → load → update → delete）
- Parent-child 关系（创建子 session，查询隔离）
- Workspace 隔离（不同 workspace 不互相污染）
- Message 追加 + 查询 + 流式更新 content/tool_calls
- Message 压缩字段 round-trip（content / reasoning / tool_calls 经过 zstd）
- 工具输出保存 + 过期删除
- Session 模糊搜索（title LIKE + UUID 精确匹配）
- Export to JSONL / SQLite
- 数据库维护（VACUUM / ANALYZE 不报错）
- Migration 版本管理 + 幂等性（in-memory SQLite）

参考 `last-full` 标签：`crates/tidev-storage/src/tests.rs`（23 个测试的完整模式）

### 2.2 `tidev-core`（当前 4 → 目标 ~35）

这是重写后的核心 crate，但测试覆盖率最低。

#### `context.rs`

依赖：`tidev-test-utils`（提供 `test_conversation()` + `test_model()`）

测试：
- 消息追加不超过 context window
- 超过限制时的 compaction 策略
- Orphan tool call 处理
- Revert marker 之后的消息可见性
- System prompt 注入

参考 `last-full` 标签：`crates/tidev-engine/src/context.rs`

#### `session.rs`

纯单元测试，无 I/O。

测试：
- Visible messages 过滤（revert 之前/之后）
- Tool output preview truncation
- `BackendEvent` session ID 传递
- `AssistantTurn` upsert
- Compaction prior-state 解析
- `ToolMetadata` JSON round-trip

参考 `last-full` 标签：`crates/tidev-session/src/session.rs`

#### `runtime.rs`

需要 mock `LlmClient` 和 `ToolExecutor`。

方法：为 `LlmClient` 和 `ToolExecutor` 定义 trait（当前是具体 struct），生产代码用 real impl，测试用 mock impl。mock impl 返回预设响应，不涉及网络。

测试：
- 正常流：用户消息 → LLM 响应 → 工具调用 → 工具结果 → LLM 继续
- 所有工具被拒绝时 LLM 的行为
- LLM 返回空响应
- LLM 重试（注入临时错误）
- 指令注入
- Session 自动保存（检查 storage 层调用）
- Context compaction 触发

参考 `last-full` 标签：`crates/tidev-engine/src/agent/runtime/tests.rs`

#### `undo.rs`

真实 git 仓库操作。

测试：
- Snapshot 创建 + 内容比较
- Revert 到指定 snapshot
- Redo
- 更改检测（文件修改/新增/删除）
- 非 git 目录的行为

参考 `last-full` 标签：`crates/tidev-tui/src/core/undo.rs`

#### `registry.rs`

测试：
- 工具注册 + 查找
- 按名称/分类过滤
- 并发注册安全性

### 2.3 `tidev-llm`（当前 12 → 目标 ~30）

Mock HTTP 层。使用 `wiremock` 或 `httpmock` crate。

#### `think_parser.rs`（纯函数）

- 解析 `thinking` / `think` 标签（多种格式）
- 嵌套标签
- 不包含 think 标签的普通内容
- 空内容

#### `error.rs`（纯函数）

- `classify_anyhow_error` 对超时/限流/认证错误的分类
- Retryable / non-retryable 分类
- `backoff_delay` 计算

#### `tool_call_format.rs`（纯函数）

- Tool call XML 生成
- XML 解析回 `ToolCall`
- 转义处理

#### `attachments.rs`（纯函数）

- 图片 base64 编码
- 附件大小计算

#### `types.rs`（纯函数）

- `LlmProviderConfig` 序列化/反序列化
- `ApiType` 验证
- `ToolDefinition` 构造

#### Provider 模块（需要 mock HTTP）

- OpenAI chat completions 响应解析
- Anthropic 流式分块解析
- Gemini 响应解析
- 各种错误码的 mock 覆盖（400/429/500/超时）

### 2.4 `tidev-logging`（当前 0 → 目标 ~8）

纯函数 + 文件输出验证。

- `LogConfig` 解析 + 过滤规则
- 日志级别过滤
- ANSI 格式生成
- 文件日志滚动（验证文件创建 + 大小限制）
- `tidev-config` 默认值集成

---

## Phase 3 — 提升质量

### 3.1 集成测试

在 `tests/` 目录（根 workspace 或 `tidev-core/tests/`）下添加：

- CLI 参数解析 + 子命令分发
- 完整 session 生命周期：创建 → 追加消息 → 存储 → 查询 → 导出
- 配置文件加载 + 合并

### 3.2 覆盖率工具

```bash
# 安装
cargo install cargo-llvm-cov

# CI 中使用
cargo llvm-cov --workspace --lcov --output-path lcov.info
```

CI 中生成覆盖率报告，上传到 Codecov。

### 3.3 关键公共 API 文档测试

逐步把 ` ```ignore` 改为 ` ```rust`，优先覆盖：
- `tidev-llm` 中 `Message` / `ToolCall` 的构造 + 序列化
- `tidev-storage` 中 `SessionStore` 的基本使用
- `tidev-config` 中配置加载
- `tidev-llm` 中 `LlmClient::new` 创建

---

## 工作量估算

| Phase | 内容 | 预估人天 |
|-------|------|---------|
| 1.1 | CI 流水线 | 0.5 |
| 1.2 | `tidev-test-utils` crate | 1 |
| 2.1 | `tidev-storage` 测试 | 1 |
| 2.2 | `tidev-core` 测试 | 3 |
| 2.3 | `tidev-llm` 测试 | 2 |
| 2.4 | `tidev-logging` 测试 | 0.5 |
| 3.1 | 集成测试 | 1 |
| 3.2 | 覆盖率工具 | 0.5 |
| 3.3 | 文档测试 | 0.5 |
| **合计** | | **10 人天** |

---

## 阻力和注意事项

1. **`tidev-core` 的 trait 提取** — 当前 `Runtime` 直接使用具体 `LlmClient` 和 `SessionStore`，提取 trait 需要改生产代码接口。虽然工作量不大，但需要 review 确保不引入回归
2. **`wiremock` 依赖** — 不需要引入网络层 mock 框架。可以简单地在测试中构造 `LlmProviderConfig` + mock 响应体，通过自定义 `reqwest::Client` 的 `Interceptor` 或直接绕过 HTTP 层测试解析逻辑。如果路径简单，就 mock 纯解析逻辑
3. **`tidev-tui` 的测试** — 当前 TUI 已完成迁移；后续覆盖率工作应继续补充
   交互流程和 Runtime 边界测试，不再维护已删除的旧 crate 测试。
