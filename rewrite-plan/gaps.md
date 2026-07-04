# 功能实现差距跟踪

本文档记录已知的未实现功能，按影响范围分级。

---

## P0 — 核心功能缺失

### 1. 子代理不支持 per-agent 模型配置（`/model`）

**当前状态：** tidev-config 层方法已实现，tidev-core 层子代理创建时已接入。仅缺 tidev-tui 的 `/model` 命令 + Model Panel UI。

**仍缺失：**

| 层 | 具体缺失 |
|----|----------|
| tidev-tui | `/model` 命令注册 + Model Panel 多 Tab UI + 模型选择 + 思考等级选择 |
| tidev-tui | 配置变更后通过 Runtime 通知 tidev-core 刷新模型 |

**涉及文件：**
- `crates/tidev-tui/` — 新命令 + 新面板

**旧版参考：**
- `_archive/v0.6.x/crates/tidev-tui/src/ui/model_panel.rs` — Model Panel 完整实现（约 270 行）

---

## P1 — 已知功能缺口

### 2. 图片 base64 编码

✅ 已完成，见 `rewrite-plan/issues/image-base64-encoding.md`。

`MessageAttachment::Image` 存储原始字节 `data: Vec<u8>`，各 LLM provider 在请求构建时自行编码。

---

## P2 — 阶段 4 未完成

### 3. tidev-tui 未接入 Runtime（阶段 4）

architecture.md 阶段 4 计划：
> tidev-tui 接入 Runtime，删除直接持有的资源

当前 tidev-tui 尚未启动重写。接入 Runtime 后：
- TUI 不再直接持有 `SessionStore`、`LlmClient`、`ToolRegistry` 等资源
- 通过 `Runtime::submit_prompt()` 提交用户消息
- 通过 `Runtime::event_rx()` 接收事件
- 通过 `Runtime::perm_rx()` 处理权限审批
