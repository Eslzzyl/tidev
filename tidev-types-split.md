# tidev-types 拆分实施计划（历史记录）

> 状态：已完成。本文保留迁移过程和历史目标，当前 crate 边界以
> tidev-target-roadmap.md 和 rewrite-plan/architecture.md 为准。

**状态**: 已完成（2026-08-03 实施，§8 验收标准全部通过；唯一行为变化：fixer Plan 检查生效于 core）
**日期**: 2026-08-03
**策略**: 一次性迁移（交付物无兼容垫片，无 tidev-types）
**执行对象**: Coding Agent。本文档是执行说明书：所有路径、类型名、函数名均经实际代码核实；实施时必须严格按 §7 步骤与 §8 验收标准执行。

## 1. 目标与范围

解散 `crates/tidev-types`，内容按归属拆分到对应 crate。拆分被迫触动的 agent 类型相关内容**直接做到目标态**（落在最终归属，不做临时安置）。本计划**不涉及**其他复杂重构（见 §9）。

约束（不可协商）：

- `tidev-agent` 只能依赖 `tidev-llm`，不能反向，也不能依赖其他 tidev crate。
- 拆分后依赖图无循环。
- 持久化类型的 serde JSON 格式不得改变（SQLite 已有历史数据、auth.json 已有配置）。
- 除 §5 明确列出的行为变化外，其余行为必须逐字节保持。

## 2. 现状盘点

tidev-types 共 3435 行、5 个模块：

| 模块 | 行数 | 内容 | 性质 |
|---|---|---|---|
| message.rs | 1420 | Message、ToolCall、BackendEvent 等 | 协议 + tidev UI 事件混杂 |
| reasoning.rs | 948 | ThinkingLevelType + 6 个厂商思考级别 | 含 LLM 请求塑形逻辑（extra_body 等） |
| tools.rs | 733 | 全量 ToolDefinition、权限、TodoItem、参数结构体、canonical_tool_name、tool_args! 宏 | tidev 工具类型 + 纯函数混杂 |
| agent_type.rs | 220 | AgentType / AgentDefinition / AgentOverride | tidev 产品概念 |
| prompts.rs | 106 | SessionMode、init_command_with_args | tidev 产品概念 |

⚠️ **注意：存在两个 prompts.rs**——`tidev-types/src/prompts.rs`（SessionMode + init_command_with_args，106 行）与 `tidev-agent/src/prompts.rs`（agent 系统提示词 + mode reminders，约 350 行）。二者去向不同，实施时勿混淆。

全仓 `tidev_types` 导入约 270 处（tui 76、core 53、llm 48、acp 34、tools 25、storage 16、agent 16、utils 2、config 1）。测试共 857 个单元测试 + acp/tui 集成测试，其中 tidev-types 内 98 个测试随代码迁移。

## 3. 拆分归属总表（最终版）

| 内容 | 去向 | 说明 |
|---|---|---|
| message.rs 协议类型：Message、MessageRole、MessageAttachment、ToolCall、ToolExecutionResult、ToolMetadata、AssistantTurn、QueuedUserMessage、FileChangeInfo、tool_output_preview、COMPACTION_MESSAGE_LABEL | **tidev-llm**（`pub mod message`） | 整个文件迁入，见 §4-3 |
| message.rs 的 BackendEvent | **tidev-llm 临时**（随 message.rs 同迁） | 见 §4-3 |
| reasoning.rs 全部（ThinkingLevel、DeepSeekV4/Qwen35/Glm/Gpt5/MiniMax/ClaudeEffort、ThinkingLevelType） | **tidev-llm**（`pub mod reasoning`） | 协议 + LLM 逻辑归 LLM 层 |
| SessionMode（tidev-types/src/prompts.rs） | **tidev-llm**（`pub mod mode`） | 见 §4-1；**本次不改名** |
| init_command_with_args（tidev-types/src/prompts.rs） | **tidev-core**（`pub mod prompts`） | 使用方 acp、tui 均依赖 core |
| 精简 ToolDefinition | 已在 **tidev-llm**（types.rs），不动 | 顺带消除与全量版的重复 |
| 全量 ToolDefinition、ToolPermission、ToolOrigin、TodoItem | **tidev-tools**（`pub mod types`） | 工具类型归工具 crate |
| tool_args! 宏 + ReadArgs/WriteArgs/EditArgs/TaskArgs/SkillArgs/QuestionArgs/QuestionInfo | **tidev-tools**（types 模块） | 参数结构体由宏生成，宏随迁 |
| canonical_tool_name（tools.rs 纯函数） | **tidev-utils**（`pub mod tool_name`） | 见 §4-2 |
| McpTarget（tools.rs） | **删除** | 全仓零使用（config 另有 McpConfig），死代码 |
| FileReadStamp（tools.rs） | **删除** | 全仓零使用（D-009 跳过文件读取追踪），死代码 |
| AgentType / AgentDefinition / AgentOverride（tidev-types/src/agent_type.rs） | **tidev-core**（`pub mod agent_type`） | **最终归属**（用户决策） |
| create_agent / create_all_agents / create_sub_agents（tidev-agent/src/agent_type.rs） | **tidev-core**（agent_type 模块） | 引用 AgentType，必须同迁；当前无 tidev-agent 之外调用方（已核实） |
| agent 系统提示词（tidev-agent/src/prompts.rs 中 system_prompt / default_system_prompt / general / explorer / librarian / oracle / fixer 提示词及测试） | **tidev-core**（agent_type 模块） | create_agent 调用 system_prompt，必须同迁；core 的 compose_system_prompt（agent_ctx.rs:61）正在调用它，迁后变内部引用 |
| mode reminders（tidev-agent/src/prompts.rs 中 mode_reminder / plan_mode_reminder / build_mode_reminder / plan_switch_reminder / build_switch_reminder） | 留 **tidev-agent** | loop_.rs 直接调用；只依赖 SessionMode（→ llm） |
| ApiType（tidev-config/src/types.rs） | 合并进 **tidev-llm**（types.rs） | 见 §5-6 |

## 4. 强制约束（为什么）

1. **SessionMode 必须进协议层（tidev-llm）**。tidev-tools 的 exec/task 使用它，若进 tidev-core 则 tools→core 循环；若进 tidev-tools 则 tidev-agent 无法使用（只能依赖 llm）。Message.mode 字段（`Option<SessionMode>`）与 task.rs 的 mode 参数都引用它。归入 `tidev_llm::mode`，保留原名与方法（all/as_str/toggle/title/description + serde）。
2. **canonical_tool_name 必须进 tidev-utils**。tidev-utils/src/path.rs（:291、:331）自身使用它，若留 tidev-tools 则 utils→tools→utils 循环。迁入后 tidev-utils 变为零内部依赖叶子。
3. 历史上 BackendEvent 曾临时留在 tidev-llm，以避免迁移阶段的依赖循环。后续 P1 已完成 LlmEvent/AgentEvent/BackendEvent 三层拆分，当前 BackendEvent 在 tidev-core。
4. **agent_type → core 的连锁**（本次范围，见 §5）：task 工具（tidev-tools 唯一引用 AgentType 处）必须解耦；`AgentLoopConfig.definition: AgentDefinition` 字段必须换成 `system_prompt: String`（agent 无法引用 core 的类型）；trait 的 `tools()` 必须改用 `tidev_llm::ToolDefinition`（agent 无法依赖 tools）。
5. **ShellOutput 暂无独立处理**。仍是 BackendEvent 的一个变体（tidev-tools 的 `ToolContext.event_tx: Option<UnboundedSender<BackendEvent>>`，builtin/mod.rs:35），随事件拆分再本地化到 tidev-tools（§9）。

## 5. 连带改动细则（agent 相关，做到目标态）

### 5.1 task 工具解耦（crates/tidev-tools/src/builtin/task.rs，62 行）

现状：`:8` 导入 AgentType；`:44-47` `AgentType::parse` 校验；`:50-56` Plan 模式拒绝 fixer；`:58-61` 结果字符串用 `agent_type.display_name()`。

改造：

- 删除 `use tidev_types::agent_type::AgentType;`（:8）。SessionMode 导入（:9）改为 `tidev_llm::mode::SessionMode`；TaskArgs/ToolDefinition/ToolPermission（:10）改为 `crate::types::{...}`。
- 新增常量与归一化辅助函数，**与 `AgentType::parse` 行为逐字节等价**（parse 语义：trim + to_ascii_lowercase + strip_prefix('@')，接受 explorer/librarian/oracle/fixer 四个名字，不接受 "general"）：

```rust
/// Mirrors `tidev_core::agent_type::AgentType::parse` accepted names.
/// Keep in sync when agent types are added/renamed (see tidev-core agent_type.rs).
const SUBAGENT_TYPES: &[&str] = &["explorer", "librarian", "oracle", "fixer"];

fn normalize_subagent_type(s: &str) -> Option<&'static str> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.strip_prefix('@').unwrap_or(&s);
    SUBAGENT_TYPES.iter().find(|t| **t == s).copied()
}
```

- `:38-42` 的 `ensure!(!subagent_type_str.is_empty(), ...)` 保留（消息逐字不变）；`:44-47` 校验改为：`let subagent_type = normalize_subagent_type(&args.subagent_type).ok_or_else(|| anyhow::anyhow!("unknown subagent type '{subagent_type_str}': expected one of explorer, librarian, oracle, fixer"))?;`（错误消息逐字保留；`subagent_type_str` 用 `args.subagent_type.trim()` 的原样值，保持与现有一致）。
- `:50-56` 的 fixer Plan 检查**整段删除**（移到 core，见 §5.2）。
- `:58-61` 结果字符串：`Ok(format!("Started {subagent_type} subagent task '{description}'"))`（`subagent_type` 是归一化后的规范名，等价于原 `agent_type.display_name()` 的输出）。
- `definitions()` 的工具描述文本不变（其中已硬编码四个子代理名，与常量表一致）。

### 5.2 fixer Plan 检查移入 core（方案 B，用户确认的行为变化）

位置：`crates/tidev-core/src/agent_ctx.rs` 的 `execute_task_tool`（:1350 起）。在 `:1377` 的 `AgentType::parse` 之后插入：

```rust
// Plan mode rejects delegation to fixer subagents (they perform writes).
// Moved here from tidev-tools task.rs: the main loop intercepts all task
// calls in execute_tools, so this check only takes effect in core.
if spawner.mode == SessionMode::Plan && agent_type == AgentType::Fixer {
    anyhow::bail!(
        "Task delegation to fixer subagent rejected: Plan mode is read-only and does not allow write operations. \
        You may delegate to read-only subagents (explorer, librarian, oracle) in plan mode. \
        Switch to build mode to use the fixer subagent."
    );
}
```

注意：`SubagentConfig` **没有** mode 字段；`SubagentSpawner` **有** `mode: SessionMode`（:1332），用 `spawner.mode`。错误消息与 task.rs 原文逐字一致。**这是本次唯一的意图性行为变化**：原检查位于主循环不可达的工具代码中（execute_tools 在 :733 拦截全部 task 调用），移入 core 后 Plan 模式委托 fixer 会真正被拒绝。用户已确认（方案 B）。

### 5.3 AgentLoopConfig.definition → system_prompt: String

- `crates/tidev-agent/src/context.rs:28`：结构体删除 `definition: crate::AgentDefinition` 字段，新增 `pub system_prompt: String`。tidev-agent 对 AgentDefinition 的引用全部消失（lib.rs re-export 同步删除，见 5.5）。
- `crates/tidev-agent/src/loop_.rs:68`：`let system_prompt = config.definition.system_prompt.clone();` → `let system_prompt = config.system_prompt.clone();`。已核实 definition 在 loop 中仅此一处使用。
- 两处构造点：
  - `crates/tidev-core/src/runtime.rs:676-720`：`agent_def` 变量仅用于喂给 loop_config（:720），删除该变量，直接 `system_prompt: system_prompt.clone()`（:656 已有同名变量）。
  - `crates/tidev-core/src/agent_ctx.rs:1494-1504`（子代理）：`agent_def` 同时用于 child_model 的 system_prompt（:1392），保留变量，loop_config 改 `system_prompt: agent_def.system_prompt.clone()`。
- `queued_messages` 字段及其文档注释（context.rs:42-46）不动（QueuedUserMessage 随 message.rs 迁入 llm，导入路径同步改）。

### 5.4 trait 的 tools() 改用 tidev_llm::ToolDefinition

- `crates/tidev-agent/src/context.rs:21`：`use tidev_types::tools::ToolDefinition` → `use tidev_llm::ToolDefinition`（trait 签名 `fn tools(&self) -> Vec<ToolDefinition>`，:125）。
- 已核实该 trait 方法**无任何调用方**，涟漪仅限 impl：
- `crates/tidev-core/src/agent_ctx.rs:446-448`：`fn tools(&self) -> Vec<ToolDefinition> { self.tools.clone() }` → `self.tools.iter().map(crate::context::to_llm_tool_def).collect()`（`to_llm_tool_def` 已存在于 tidev-core/src/context.rs:25，pub(crate)，注意 agent_ctx.rs:470 已有同款内联转换，可复用）。

### 5.5 tidev-agent 瘦身

- 删除 `crates/tidev-agent/src/agent_type.rs` 整个文件（工厂迁 core）。
- `crates/tidev-agent/src/lib.rs`：删除 `pub mod agent_type;`（:7）、`pub use agent_type::{create_agent, create_all_agents, create_sub_agents};`（:13）、`pub use tidev_types::agent_type::{AgentDefinition, AgentOverride, AgentType};`（:19）。
- `crates/tidev-agent/src/prompts.rs`：删除 AgentType 相关部分——`system_prompt`（:11）、`default_system_prompt`（:22）、`general_system_prompt`（:50）及 explorer/librarian/oracle/fixer 提示词函数、`:6` 的 AgentType 导入、`:299` 起的相关测试（`:299-341` 中引用 AgentType 的部分）。保留 SessionMode 相关部分——`mode_reminder`（:27）、`plan_mode_reminder`（:263）、`build_mode_reminder`（:271）、`plan_switch_reminder`（:279）、`build_switch_reminder`（:287）及其测试。拆完后检查模块内无残留交叉引用。
- `crates/tidev-agent/src/context.rs` 与 `loop_.rs` 的全部 tidev_types 导入改为 tidev_llm（message/mode/reasoning + BackendEvent 仍从 `tidev_llm::message` 取）。
- `Cargo.toml`：移除 tidev-types，新增 tidev-llm。

### 5.6 ApiType 合并（tidev-config → tidev-llm）

- `crates/tidev-llm/src/types.rs` 的 ApiType（现无 serde）：增加 `#[derive(Serialize, Deserialize)]`，**逐字保留** tidev-config 版的变体重命名（`#[serde(rename = "openai_chat_completions")]` / `"openai_responses"` / `"anthropic"` / `"google_gemini"`）与 `#[default]`；增加 `impl std::fmt::Display`（tidev-config/src/types.rs:32-40 原文）；`parse()` 换成 tidev-config 版（大小写不敏感 + 别名：`openai`/`chat`、`responses`、`claude`、`gemini`/`google`，tidev-config/src/types.rs:21-29 原文）。保留 llm 版现有 `as_str()`。
- `crates/tidev-config/src/types.rs`：删除本地定义，改为 `pub use tidev_llm::ApiType;`。`tidev-config/src/lib.rs:36` 的 re-export 不变（路径兼容）。
- `crates/tidev-core/src/agent_ctx.rs:115-123`：删除 `to_llm_api_type` 转换函数；`to_llm_provider_config`（:125）的 `api_type: to_llm_api_type(model.api_type)` 改为 `api_type: model.api_type`（ActiveModel.api_type 现在是 llm 的 ApiType，经 config re-export）。
- `crates/tidev-core/src/registry.rs:276`：`tidev_config::types::ApiType::OpenAiChatCompletions` 经 config re-export 仍可用，可保持或改 `tidev_llm::ApiType`。

### 5.7 死代码删除

`tools.rs` 的 McpTarget、FileReadStamp 删除。**实施时先 grep 全仓复核**（含测试、build.rs）：`grep -rn "McpTarget\|FileReadStamp" crates src` 应仅剩 tidev-types 自身。

## 6. 目标依赖图与模块布局

```
tidev-llm（叶子，只依赖外部 crate）
  pub mod message（含 BackendEvent 临时，带 TODO 注释）
  pub mod reasoning
  pub mod mode（SessionMode）
  mod types（ApiType 合并后 / LlmProviderConfig / 精简 ToolDefinition）+ 现有 provider 实现
tidev-utils（叶子）
  pub mod tool_name（canonical_tool_name）
tidev-agent ──→ tidev-llm（唯一内部依赖）
  context.rs（trait / AgentLoopConfig / ApprovedTool / TuiRequest 等）+ loop_.rs + prompts.rs（仅 mode reminders）
tidev-tools ──→ llm, utils, config, instructions
  pub mod types（全量工具类型）+ builtin/（task.rs 已解耦）+ todo_persistence.rs
tidev-config ──→ llm（ApiType re-export + reasoning）
tidev-storage ──→ llm, tools（Message 持久化、TodoItem）
tidev-core ──→ agent, tools, llm, config, storage, snapshot, instructions, logging, search, utils
  pub mod agent_type（AgentType/AgentDefinition/AgentOverride + 工厂 + agent 系统提示词）
  pub mod prompts（init_command_with_args）
  agent_ctx.rs（compose_system_prompt / execute_task_tool fixer 检查 / CoreContext）/ runtime.rs / registry.rs / context.rs 等
tidev-tui ──→ core, llm, tools, config, utils, search
tidev-acp ──→ core, llm, config, utils
tidev（bin）──→ core, config, storage, tui, acp, utils（不变）
```

模块路径设计原则：**保持与现状同构的路径**，让导入迁移可机械替换（`tidev_types::message` → `tidev_llm::message`、`tidev_types::tools` → `tidev_tools::types`、`tidev_types::agent_type` → `tidev_core::agent_type`、`tidev_types::reasoning` → `tidev_llm::reasoning`、`tidev_types::prompts::SessionMode` → `tidev_llm::mode::SessionMode`、`tidev_types::prompts::init_command_with_args` → `tidev_core::prompts::init_command_with_args`、`tidev_types::tools::canonical_tool_name` → `tidev_utils::tool_name::canonical_tool_name`）。

## 7. 实施步骤（一次性迁移，按序执行）

本计划为一次性迁移：步骤之间允许工作区处于未编译状态，**最终交付必须全绿且无 tidev-types**。执行时如确需中途编译验证，可自行临时把 tidev-types 改为 re-export 垫片（不属于交付物，最终必须删除）。

### 步骤 1：tidev-llm 吸收协议层

1. 新建 `crates/tidev-llm/src/message.rs`：将协议消息类型迁入 tidev-llm；产品字段和 BackendEvent 后续按目标路线图分别净化、迁移。
2. 新建 `crates/tidev-llm/src/reasoning.rs`：`tidev-types/src/reasoning.rs` 整文件迁入（948 行 + 34 测试），一字不改。
3. 新建 `crates/tidev-llm/src/mode.rs`：`tidev-types/src/prompts.rs` 的 SessionMode 部分迁入（枚举 + 全部方法 + 相关测试），一字不改。
4. `crates/tidev-llm/src/lib.rs`：声明 `pub mod message; pub mod reasoning; pub mod mode;`。
5. ApiType 合并（§5.6 的前半部分）：types.rs 的 ApiType 增加 serde/Display/config 版 parse。
6. `crates/tidev-llm/src/types.rs`：`thinking_level: tidev_types::reasoning::ThinkingLevelType` 等内部引用改为 `crate::reasoning::`。
7. `Cargo.toml` 移除 tidev-types。llm 变叶子（仅外部 crate 依赖）。

### 步骤 2：tidev-utils 吸收 canonical_tool_name

1. 新建 `crates/tidev-utils/src/tool_name.rs`：`tidev-types/src/tools.rs` 的 `canonical_tool_name`（:575）+ 相关测试迁入。
2. `crates/tidev-utils/src/path.rs`（:291、:331）：`tidev_types::tools::canonical_tool_name` → `crate::tool_name::canonical_tool_name`。
3. `Cargo.toml` 移除 tidev-types。utils 变叶子。

### 步骤 3：tidev-tools 吸收工具类型 + task 解耦

1. 新建 `crates/tidev-tools/src/types.rs`：`tidev-types/src/tools.rs` 迁入——TodoItem、ToolPermission、ToolOrigin、ToolDefinition、tool_args! 宏 + ReadArgs/WriteArgs/EditArgs/TaskArgs/SkillArgs/QuestionArgs/QuestionInfo、相关测试（12 个）；**删除 McpTarget、FileReadStamp**（§5.7）。
2. `crates/tidev-tools/src/lib.rs`：声明 `pub mod types;`。
3. 内部导入改路径：`builtin/mod.rs:15` 的 `use tidev_types::tools::{QuestionArgs, SkillArgs, ToolDefinition, ToolPermission, canonical_tool_name}` 拆为 `use crate::types::{...}` + `use tidev_utils::tool_name::canonical_tool_name;`；builtin/web/mod.rs、builtin/search.rs、builtin/file.rs 的 `tidev_types::tools::canonical_tool_name` → `tidev_utils::tool_name::canonical_tool_name`；builtin/mod.rs:230 的 task 派发不变。
4. task.rs 解耦（§5.1）。
5. `Cargo.toml`：移除 tidev-types，新增 tidev-llm（utils/config/instructions 已有）。

### 步骤 4：tidev-agent 瘦身

1. 删除 `src/agent_type.rs`（内容迁 core，见步骤 5）。
2. `src/lib.rs`：删除 agent_type 模块声明与两处 re-export（§5.5）。
3. `src/prompts.rs`：删除 AgentType 相关函数与测试（§5.5），保留 mode reminders。
4. `src/context.rs`：AgentLoopConfig.definition → system_prompt（§5.3）；trait 的 ToolDefinition 改 `tidev_llm::ToolDefinition`（§5.4）；全部 tidev_types 导入改 tidev_llm。
5. `src/loop_.rs`：`config.system_prompt`（§5.3）；全部 tidev_types 导入改 tidev_llm。
6. `Cargo.toml`：移除 tidev-types，新增 tidev-llm。

### 步骤 5：tidev-core 接收 agent 相关内容

1. 新建 `src/agent_type.rs`：迁入 tidev-types/src/agent_type.rs（AgentType/AgentDefinition/AgentOverride，220 行 + 5 测试）+ tidev-agent/src/agent_type.rs 的工厂（create_agent/create_all_agents/create_sub_agents）+ tidev-agent/src/prompts.rs 的 agent 系统提示词（system_prompt/default_system_prompt/general/explorer/librarian/oracle/fixer + 相关测试）。工厂内 `crate::prompts::system_prompt` 引用随迁后改 `crate::agent_type::system_prompt`（或同模块内部调用）。
2. 新建 `src/prompts.rs`：`init_command_with_args`（tidev-types/src/prompts.rs:51）+ 测试迁入。
3. `src/lib.rs`：声明 `pub mod agent_type; pub mod prompts;`；**保留**现有对 tidev_agent 的 re-export（ApprovedTool 等，:23-25）；`PendingToolApproval`（:33-37）的 `tidev_types::message::ToolCall` / `tidev_types::prompts::SessionMode` 引用改 tidev_llm 路径。
4. `src/agent_ctx.rs`：
   - `:26` 导入改 `crate::agent_type::{AgentDefinition, AgentType}`；`:58`（SubagentSpawner 字段）的 `tidev_types::agent_type::` 引用同步改。
   - `:61` `tidev_agent::prompts::system_prompt(agent_type)` → `crate::agent_type::system_prompt(agent_type)`。
   - 删除 `to_llm_api_type`（:115-123），`to_llm_provider_config` 直用 `model.api_type`（§5.6）。
   - `tools()` impl（:446-448）加 to_llm_tool_def 转换（§5.4）。
   - `execute_task_tool` 加 fixer Plan 检查（§5.2，用 `spawner.mode`）。
   - 子代理 loop_config（:1494-1504）：`definition: agent_def` → `system_prompt: agent_def.system_prompt.clone()`。
   - 其余 `tidev_types::` 导入改 tidev_llm / tidev_tools::types。
5. `src/runtime.rs`：`:44` 的 `use tidev_agent::{AgentDefinition, TuiRequest}` 改 `use crate::agent_type::AgentDefinition`（TuiRequest 仍从 tidev_agent 取）；`:663`、`:677` 的 `tidev_types::agent_type::AgentType` → `crate::agent_type::AgentType`；删除 `agent_def` 变量（:676-684），loop_config（:718-727）改 `system_prompt: system_prompt.clone()`。
6. `src/context.rs`（ContextManager）：`tidev_types::message::{BackendEvent, Message, MessageRole}` → `tidev_llm::message::{...}`；`tidev_types::tools::ToolDefinition` → `tidev_tools::types::ToolDefinition`；`to_llm_tool_def` 保留原位。
7. `src/registry.rs`：`:276` ApiType 引用（§5.6）；`tidev_types::` 导入改路径（BackendEvent/ToolCall/ToolExecutionResult → tidev_llm；ToolDefinition/ToolPermission → tidev_tools::types；SessionMode → tidev_llm::mode；canonical_tool_name → tidev_utils::tool_name）。
8. `src/mcp.rs`、`src/session.rs`、`src/undo.rs`、`src/system_info.rs`、`src/message_buf.rs`、`src/attachment.rs`、`src/lib.rs`：`tidev_types::` 导入统一改路径。
9. `Cargo.toml`：移除 tidev-types（llm/tools/agent 均已依赖）。

### 步骤 6：其余 crate 导入迁移

- **tidev-tui**（76 处，最大头）：`tidev_types::message` → `tidev_llm::message`（52 处）；`tidev_types::prompts` → SessionMode 用 `tidev_llm::mode::SessionMode`、init_command_with_args 用 `tidev_core::prompts::init_command_with_args`（10 处）；`tidev_types::tools` → `tidev_tools::types` / canonical_tool_name 用 `tidev_utils::tool_name`（8 处）；`tidev_types::reasoning` → `tidev_llm::reasoning`（4 处）；`tidev_types::agent_type` → `tidev_core::agent_type`（2 处，overlays.rs）。Cargo.toml：-types，+llm，+tools。
- **tidev-acp**（34 处）：message/prompts(SessionMode)/reasoning → tidev_llm；init_command_with_args → tidev_core::prompts；canonical_tool_name → tidev_utils::tool_name。Cargo.toml：-types，+llm。
- **tidev-storage**（16 处）：message 类型 → tidev_llm::message；TodoItem（:2046-2469）→ tidev_tools::types。Cargo.toml：-types，+llm，+tools。
- **tidev-config**（1 处）：reasoning re-export（src/reasoning.rs:5）→ tidev_llm::reasoning；types.rs 的 ApiType 改为 re-export（§5.6）。Cargo.toml：-types，+llm。
- 检查 `tidev-acp/tests`、`tidev-tui/tests` 集成测试中的 tidev_types 引用。

### 步骤 7：收尾

1. 删除 `crates/tidev-types` 目录。
2. 根 `Cargo.toml`：workspace members 移除 `"crates/tidev-types"`。
3. `grep -rn "tidev_types" crates src` 确认零残留（_archive 除外）。
4. `cargo check --workspace` 全绿。
5. `cargo test --workspace` 全绿（857 单测 + 集成测试）。
6. `cargo tree` 验证依赖方向（§6 图；tidev-agent 仅 llm）。

## 8. 验收标准

- 工作区无 tidev-types 目录与任何 `tidev_types` 引用。
- `cargo check --workspace`、`cargo test --workspace` 全部通过。
- `cargo tree` 确认：tidev-agent 的 tidev 内部依赖仅有 tidev-llm；tidev-llm、tidev-utils 为叶子；无循环。
- auth.json 等既有数据仍可解析（ApiType 的 serde rename 逐字未变）。
- 消息存储 JSON 格式与拆分前一致（Message 结构体逐字段未变；迁移过程未改动任何字段/serde 属性）。
- **行为变化清单（仅此一项，用户已确认方案 B）**：Plan 模式委托 fixer 子代理现在会被 core 的 execute_task_tool 拒绝（原检查位于主循环不可达的工具代码中）。其余行为逐字节保持。
- task 工具输出与错误消息与拆分前一致（含大小写、@ 前缀输入的规范化输出）。

## 9. 本次明确不做（范围外）

1. 事件三层拆分（LlmEvent / AgentEvent / BackendEvent）、事件去 session_id、ShellOutput 本地化到 tidev-tools——BackendEvent 暂留 tidev-llm（带 TODO 注释）。
2. tidev-agent 内核扩充（ContextManager / ToolRegistry / MessageBuffer 迁入、默认 Runtime、Tool trait 通用化）——ContextManager 等仍留 tidev-core。
3. AgentContext trait 瘦身（inject_instructions / append_instruction_sources / update_message_content 仍留 trait）、循环注入逻辑迁移、TuiRequest/TuiResponse 移出 tidev-agent。
4. SessionMode 重命名为 Mode；mode reminders 移出 tidev-agent。
5. task 工具重构（子代理能力由内核 ToolContext 提供）——agent_type 已落 core，此项不再依赖本次改动。
6. 其余此前讨论过的后续重构项。

## 10. 风险与注意事项（执行 agent 必读）

- **serde 兼容是承重墙**：Message/TodoItem 的全部字段与 serde 属性、ApiType 的 `#[serde(rename)]` 必须逐字保留。跨 crate 移动不改变序列化；合并 ApiType 时手滑改 rename 会静默破坏 auth 数据（唯一"弄坏用户数据"的途径）。
- **任务名与 parse 等价性**：task.rs 的 `normalize_subagent_type` 必须与 `AgentType::parse` 语义一致（trim/lowercase/strip @；不含 general）。常量表与 core 的 AgentType 存在重复，属已知漂移风险，用注释 + 测试兜底（可加一个断言四个名字与错误消息一致的测试）。
- **两个 prompts.rs**：tidev-types/src/prompts.rs（106 行）与 tidev-agent/src/prompts.rs（约 350 行）去向不同，迁移时逐文件核对。
- **无垫片的一次改**：约 270 处导入在同一变更内完成；tui 是最大头，建议脚本机械替换（sed：`tidev_types::message`→`tidev_llvm::message` 等按 §6 映射表）后人工复核。注意 sed 替换顺序（先长路径后短路径，如 `tidev_types::prompts::SessionMode` 需特殊处理）。
- **BackendEvent 临时驻留 tidev-llm** 是明知状态，必须保留 TODO 注释，避免后续误当作协议类型。
- **死代码删除前复核**：McpTarget / FileReadStamp 再 grep 一次全仓（含测试与集成测试）。
- **测试随代码迁移**：98 个 tidev-types 测试、tidev-agent prompts/agent_type 测试按归属迁移，不删除。
- 拆分后 llm 与 utils 为叶子：后续新增共享类型必须先问归属，避免 tidev-types 死灰复燃。
