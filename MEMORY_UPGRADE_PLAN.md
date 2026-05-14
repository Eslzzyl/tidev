# TiDev 记忆系统升级方案

> 基于对 [agentmemory](https://github.com/rohitg00/agentmemory) v0.9.12 项目的完整逆向分析，制定的精准复刻计划。
>
> 文档日期：2026-05-14

---

## 目录

1. [AgentMemory 项目全景](#1-agentmemory-项目全景)
2. [功能完整目录](#2-功能完整目录)
3. [架构深度分析](#3-架构深度分析)
4. [iii-sdk 依赖分析](#4-iii-sdk-依赖分析)
5. [TiDev 当前现状对比](#5-tidev-当前现状对比)
6. [复刻架构设计](#6-复刻架构设计)
7. [完整数据模型映射](#7-完整数据模型映射)
8. [分期实施路线图](#8-分期实施路线图)
9. [附录：关键文件参考](#9-附录关键文件参考)

---

## 1. AgentMemory 项目全景

### 1.1 项目定位

AgentMemory 是一个为 AI 编码代理设计的持久化记忆系统，基于 **iii-engine** 的三个核心原语（Worker/Function/Trigger）构建。它捕获代理的每一次操作，通过 LLM 压缩为结构化摘要，并提供混合搜索（BM25 + 向量 + 知识图谱）进行检索。

### 1.2 关键统计

| 指标 | 数值 | 说明 |
|------|------|------|
| MCP 工具 | 44（默认 8 个可见） | `AGENTMEMORY_TOOLS=all` 显示全部 |
| REST 端点 | 104 | 覆盖核心、记忆、图谱、团队等 |
| iii 函数 | 50+ | 注册在 iii-engine 上的命名函数 |
| 钩子类型 | 12 | session_start → session_end |
| 技能 | 4 | remember/recall/forget/session-history |
| 测试用例 | 699+ | vitest 单元测试 |
| 版本 | v0.9.12 | TypeScript ESM |
| 生产依赖 | 6 | iii-sdk、Anthropic SDK、zod 等 |

### 1.3 代码仓库结构

```
agentmemory/
├── src/
│   ├── index.ts              # 主入口，注册所有 50+ 函数
│   ├── types.ts              # 864 行类型定义（Session/Observation/Memory/…）
│   ├── config.ts             # 配置加载（环境变量 → ProviderConfig）
│   ├── version.ts            # 版本号
│   ├── logger.ts             # 日志
│   ├── cli.ts                # CLI 入口（43547 行，含所有 MCP 工具注册）
│   ├── auth.ts               # HMAC-SHA256 认证
│   ├── functions/            # 50+ iii 函数实现（每个文件一个函数族）
│   ├── state/                # 状态管理（KV 封装 + 搜索索引 + 向量索引）
│   ├── providers/            # LLM 提供商（Anthropic/Gemini/MiniMax/OpenRouter）
│   ├── mcp/                  # MCP 服务器（工具注册 + 路由 + 独立模式）
│   ├── triggers/             # REST API（104 个 HTTP 端点）
│   ├── hooks/                # 钩子脚本（独立 Node.js 脚本）
│   ├── prompts/              # LLM 提示词模板（压缩/摘要/图谱）
│   ├── eval/                 # 评估系统（质量评分/指标/校验/自纠正）
│   ├── telemetry/            # 遥测
│   ├── health/               # 健康监控
│   ├── replay/               # 回放系统
│   ├── viewer/               # Web 查看器
│   └── utils/                # 工具函数
├── packages/mcp/             # 独立 MCP 包（仅 7 个工具，降级模式）
├── plugin/skills/            # Claude Code 技能
├── integrations/             # 第三方集成
├── benchmark/                # 基准测试（100k 加载/LongMemEval/Scale 等）
├── test/                     # 699+ 测试用例
└── docker-compose.yml        # iii-engine Docker 部署
```

---

## 2. 功能完整目录

### 2.1 44 个 MCP 工具

| # | 工具名 | 功能描述 | 实现文件 | 依赖 |
|---|--------|----------|----------|------|
| 1 | `memory_recall` | 搜索历史会话观察 | `mcp/server.ts` | 搜索索引 |
| 2 | `memory_compress_file` | 压缩 markdown 文件减少 token | `functions/compress-file.ts` | LLM |
| 3 | `memory_save` | 显式保存重要记忆 | `functions/remember.ts` | 去重/向量 |
| 4 | `memory_file_history` | 获取特定文件的历史观察 | `functions/search.ts` | 搜索索引 |
| 5 | `memory_observations` | 列出当前会话的观察 | `functions/search.ts` | KV 存储 |
| 6 | `memory_context` | 获取当前会话上下文 | `functions/context.ts` | 搜索/LLM |
| 7 | `memory_consolidate` | 运行记忆整合管线 | `functions/consolidate.ts` | LLM |
| 8 | `memory_insights` | 从模式中综合洞察 | `functions/insights.ts` | LLM |
| 9 | `memory_patterns` | 列出已学习的模式 | `functions/patterns.ts` | 搜索 |
| 10 | `memory_lessons` | 获取关键教训 | `functions/lessons.ts` | LLM |
| 11 | `memory_profile` | 获取代理性能画像 | `functions/profile.ts` | KV 存储 |
| 12 | `memory_relations` | 查询实体关系 | `functions/relations.ts` | 图谱 |
| 13 | `memory_timeline` | 获取事件时间线 | `functions/timeline.ts` | 搜索 |
| 14 | `memory_smart_search` | 混合搜索（语义 + 关键词） | `functions/smart-search.ts` | 向量/BM25 |
| 15 | `memory_graph_query` | 查询知识图谱 | `functions/graph.ts` | LLM |
| 16 | `memory_graph_stats` | 图谱统计 | `functions/graph.ts` | 图谱 |
| 17 | `memory_team_share` | 向团队共享记忆 | `functions/team.ts` | 团队存储 |
| 18 | `memory_team_feed` | 团队活动流 | `functions/team.ts` | 团队存储 |
| 19 | `memory_team_profile` | 团队成员画像 | `functions/team.ts` | 团队存储 |
| 20 | `memory_audit` | 查询审计日志 | `functions/audit.ts` | 审计存储 |
| 21 | `memory_governance_delete` | 按隐私规则删除数据 | `functions/governance.ts` | 治理存储 |
| 22 | `memory_governance_bulk` | 批量治理操作 | `functions/governance.ts` | 治理存储 |
| 23 | `memory_snapshots` | 列出文件系统快照 | `functions/snapshot.ts` | 快照存储 |
| 24 | `memory_snapshot_create` | 创建文件树快照 | `functions/snapshot.ts` | 快照存储 |
| 25 | `memory_snapshot_restore` | 从快照恢复 | `functions/snapshot.ts` | 快照存储 |
| 26 | `memory_next` | 获取下一个最重要的操作 | `functions/frontier.ts` | LLM |
| 27 | `memory_lease` | 获取/释放/续租独占锁 | `functions/leases.ts` | 租约存储 |
| 28 | `memory_routine_run` | 实例化冻结的工作流例程 | `functions/routines.ts` | 例程存储 |
| 29 | `memory_signal_send` | 向另一个代理发送信号 | `functions/signals.ts` | 信号存储 |
| 30 | `memory_signal_read` | 读取代理信号 | `functions/signals.ts` | 信号存储 |
| 31 | `memory_checkpoint_create` | 创建操作检查点 | `functions/checkpoints.ts` | 检查点存储 |
| 32 | `memory_checkpoint_resolve` | 解决检查点 | `functions/checkpoints.ts` | 检查点存储 |
| 33 | `memory_checkpoint_list` | 列出检查点 | `functions/checkpoints.ts` | 检查点存储 |
| 34 | `memory_mesh_register` | 在网格中注册代理 | `functions/mesh.ts` | 网格存储 |
| 35 | `memory_mesh_list` | 列出网格代理 | `functions/mesh.ts` | 网格存储 |
| 36 | `memory_mesh_sync` | 与网格同步状态 | `functions/mesh.ts` | 网格存储 |
| 37 | `memory_mesh_receive` | 接收网格更新 | `functions/mesh.ts` | 网格存储 |
| 38 | `memory_mesh_export` | 导出网格状态 | `functions/mesh.ts` | 网格存储 |
| 39 | `memory_flow_compress` | 压缩观察流 | `functions/flow-compress.ts` | LLM |
| 40 | `memory_branch_detect` | 检测 git 分支上下文 | `functions/branch-aware.ts` | Git |
| 41 | `memory_branch_worktrees` | 列出 git 工作树 | `functions/branch-aware.ts` | Git |
| 42 | `memory_branch_sessions` | 获取每个分支的会话 | `functions/branch-aware.ts` | 搜索 |
| 43 | `memory_insight_list` | 列出综合洞察 | `functions/insights.ts` | LLM |
| 44 | `memory_slot_list` | 列出所有记忆槽 | `functions/slots.ts` | KV 存储 |
| 45 | `memory_slot_get` | 读取单个槽 | `functions/slots.ts` | KV 存储 |
| 46 | `memory_slot_create` | 创建新槽 | `functions/slots.ts` | KV 存储 |
| 47 | `memory_slot_append` | 追加到槽 | `functions/slots.ts` | KV 存储 |
| 48 | `memory_slot_replace` | 替换槽内容 | `functions/slots.ts` | KV 存储 |
| 49 | `memory_slot_delete` | 删除槽 | `functions/slots.ts` | KV 存储 |
| 50 | `memory_slot_reflect` | 反思槽内容 | `functions/slots.ts` | LLM |

### 2.2 50+ iii 核心函数

#### 观察与记忆

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::observe` | `functions/observe.ts`（281 行） | 处理钩子发来的观察数据 | SHA256 去重（5 分钟 TTL），隐私过滤 |
| `mem::remember` | `functions/remember.ts`（228 行） | 显式保存记忆 | Jaccard 相似度去重（>0.7 版本链），keyed mutex |
| `mem::forget` | `functions/forget.ts` | 删除记忆 | 软删除 |
| `mem::evict` | `functions/evict.ts` | 按条件淘汰记忆 | 重要性 + 访问频率排序 |
| `mem::enrich` | `functions/enrich.ts` | 用元数据丰富观察 | LLM 提取 |

#### 搜索与检索

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::search` | `functions/search.ts`（352 行） | 全文搜索 | BM25（k1=1.2, b=0.75），同义词扩展 |
| `mem::smart-search` | `functions/smart-search.ts` | 混合搜索 | BM25 + 向量余弦 + RRF 融合 |
| `mem::context` | `functions/context.ts` | 构建会话上下文 | 按 token 预算 + 重要性 + 时效性 |
| `mem::file-index` | `functions/file-index.ts` | 索引文件内容 | BM25 索引 |
| `mem::timeline` | `functions/timeline.ts` | 事件时间线 | 按时间戳排序 |

#### 压缩与整合

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::compress` | `functions/compress.ts`（266 行） | 压缩观察为结构化摘要 | LLM + XML 解析 + 质量评分 + 自纠正 |
| `mem::compress-file` | `functions/compress-file.ts` | 压缩 markdown 文件 | 保留标题/URL/代码块 |
| `mem::compress-synthetic` | `functions/compress-synthetic.ts` | 合成压缩（无 LLM 时） | 规则引擎 |
| `mem::flow-compress` | `functions/flow-compress.ts` | 压缩观察流 | 滑动窗口 + LLM |
| `mem::consolidate` | `functions/consolidate.ts` | 记忆整合 | 跨会话模式检测 + LLM |
| `mem::consolidate-pipeline` | `functions/consolidation-pipeline.ts` | 完整整合管线 | 多阶段处理 |

#### 知识图谱

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::graph-extract` | `functions/graph.ts` | 从观察提取实体/关系 | LLM + NER |
| `mem::graph-query` | `functions/graph.ts` | 查询图谱 | 图遍历 + 排名 |
| `mem::graph-stats` | `functions/graph.ts` | 图谱统计 | 节点/边计数 |
| `mem::temporal-graph` | `functions/temporal-graph.ts` | 时序图操作 | 时间窗口过滤 |

#### 综合洞察

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::crystallize` | `functions/crystallize.ts` | 结晶化观察 | LLM 模式提取 |
| `mem::reflect` | `functions/reflect.ts` | 反思会话 | LLM 分析 |
| `mem::lessons` | `functions/lessons.ts` | 提取教训 | LLM + 模式匹配 |
| `mem::insights` | `functions/insights.ts` | 综合洞察 | LLM + 跨会话分析 |
| `mem::patterns` | `functions/patterns.ts` | 学习模式 | 频次分析 + LLM |

#### 会话管理

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::summarize` | `functions/summarize.ts` | 会话摘要 | LLM |
| `mem::session-start` | `mcp/server.ts` | 会话开始 | 初始化等 |
| `mem::session-end` | `functions/summarize.ts` | 会话结束 | 自动摘要 + 反思 |

#### 治理与审计

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::governance-delete` | `functions/governance.ts` | 隐私删除 | 范围匹配 |
| `mem::governance-bulk` | `functions/governance.ts` | 批量治理 | 条件过滤 |
| `mem::audit` | `functions/audit.ts` | 审计日志 | 不可变追加 |
| `mem::access-tracker` | `functions/access-tracker.ts` | 访问追踪 | 时间窗口+频次 |

#### 并发控制

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::lease-acquire` | `functions/leases.ts` | 获取租约 | 超时 + 重试 |
| `mem::lease-release` | `functions/leases.ts` | 释放租约 | 所有权验证 |
| `mem::lease-renew` | `functions/leases.ts` | 续租 | 心跳 |
| `mem::keyed-mutex` | `state/keyed-mutex.ts` | 键级互斥锁 | Promise 队列 |

#### 代理间通信

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::signal-send` | `functions/signals.ts` | 发送信号 | 队列 |
| `mem::signal-read` | `functions/signals.ts` | 读取信号 | 出队 |
| `mem::checkpoint-create` | `functions/checkpoints.ts` | 创建检查点 | 状态快照 |
| `mem::checkpoint-resolve` | `functions/checkpoints.ts` | 解决检查点 | 状态验证 |
| `mem::checkpoint-list` | `functions/checkpoints.ts` | 列出检查点 | 按会话 |

#### 多代理网格

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::mesh-register` | `functions/mesh.ts` | 注册入网格 | 身份+能力声明 |
| `mem::mesh-list` | `functions/mesh.ts` | 列出网格代理 | 网格视图 |
| `mem::mesh-sync` | `functions/mesh.ts` | 同步状态 | 增量同步 |
| `mem::mesh-receive` | `functions/mesh.ts` | 接收更新 | 事件订阅 |
| `mem::mesh-export` | `functions/mesh.ts` | 导出状态 | 序列化 |

#### 工作流

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::action-create` | `functions/actions.ts` | 创建操作项 | DAG 图 |
| `mem::action-update` | `functions/actions.ts` | 更新操作 | 状态转换 |
| `mem::action-list` | `functions/actions.ts` | 列出操作 | 过滤/排序 |
| `mem::action-edge` | `functions/actions.ts` | 创建依赖边 | DAG 边 |
| `mem::frontier` | `functions/frontier.ts` | 前沿操作 | 拓扑排序 |
| `mem::routine-create` | `functions/routines.ts` | 创建例程 | 冻结工作流 |
| `mem::routine-run` | `functions/routines.ts` | 运行例程 | 状态机 |
| `mem::routine-status` | `functions/routines.ts` | 例程状态 | 进度追踪 |

#### 记忆槽

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::slot-*`（7 个） | `functions/slots.ts` | 槽管理 | KV CRUD + LLM reflect |
| `mem::sketch-*`（4 个） | `functions/sketches.ts` | 草图管理 | KV |
| `mem::sentinel-*`（5 个） | `functions/sentinels.ts` | 哨兵监控 | 条件检查 + LLM |

#### 其他

| 函数 ID | 文件 | 功能 | 算法/依赖 |
|---------|------|------|-----------|
| `mem::retention` | `functions/retention.ts` | 保存评分 | 时间衰减 + 访问频率 |
| `mem::auto-forget` | `functions/auto-forget.ts` | 自动遗忘 | 阈值触发 |
| `mem::verify` | `functions/verify.ts` | 验证一致性 | 校验和 |
| `mem::diagnostics` | `functions/diagnostics.ts` | 诊断 | 健康检查 |
| `mem::migrate` | `functions/migrate.ts` | 数据迁移 | 版本升级 |
| `mem::replay` | `functions/replay.ts` | 回放 | 重放事件 |
| `mem::export-import` | `functions/export-import.ts` | 导入导出 | JSON 序列化 |

### 2.3 12 个钩子类型

| 钩子 | 触发时机 | 用途 | 负载 |
|------|----------|------|------|
| `session_start` | 会话开始时 | 初始化会话，加载上下文 | sessionId, project, cwd, timestamp |
| `prompt_submit` | 用户提交提示词 | 捕获用户输入 | sessionId, prompt, timestamp |
| `pre_tool_use` | 工具调用前 | 捕获预期操作 | sessionId, toolName, toolInput |
| `post_tool_use` | 工具成功后 | 捕获结果 | sessionId, toolName, toolInput, toolOutput |
| `post_tool_failure` | 工具失败后 | 捕获错误 | sessionId, toolName, error |
| `pre_compact` | 压缩前 | 准备压缩 | sessionId, observationCount |
| `subagent_start` | 子代理启动 | 追踪子代理 | sessionId, subagentId, task |
| `subagent_stop` | 子代理停止 | 追踪完成 | sessionId, subagentId, result |
| `notification` | 通知时 | 捕获通知 | sessionId, message, level |
| `task_completed` | 任务完成 | 追踪完成 | sessionId, taskId, result |
| `stop` | 代理停止 | 会话清理 | sessionId |
| `session_end` | 会话结束时 | 总结、反思 | sessionId, duration |

### 2.4 12 个钩子脚本

`src/hooks/` 目录下的独立 Node.js 脚本，通过 stdin 接收 JSON 负载，通过 HTTP 调用 REST API：

| 脚本 | 用途 |
|------|------|
| `mark-milestone.js` | 标记会话里程碑 |
| `update-progress.js` | 更新进度信息 |
| `save-memory.js` | 自动保存重要记忆 |
| `track-file.js` | 追踪文件变更 |
| `detect-pattern.js` | 检测行为模式 |
| `summarize-chunk.js` | 分块汇总 |
| `update-graph.js` | 更新知识图谱 |
| `check-goals.js` | 检查目标达成 |
| `suggest-actions.js` | 建议下一步操作 |
| `report-insight.js` | 报告洞察 |
| `log-metrics.js` | 记录指标 |
| `auto-tag.js` | 自动标签 |

### 2.5 104 个 REST 端点

按路径分组：

```
# 核心 (15)
GET    /agentmemory/health
GET    /agentmemory/config-flags
POST   /agentmemory/observe
GET    /agentmemory/context
POST   /agentmemory/search
POST   /agentmemory/compress-file
POST   /agentmemory/replay/load
GET    /agentmemory/replay/sessions
POST   /agentmemory/replay/import
POST   /agentmemory/session/start
POST   /agentmemory/session/end
POST   /agentmemory/summarize
GET    /agentmemory/sessions
GET    /agentmemory/observations
GET    /agentmemory/file-context

# 记忆 (20+)
POST   /agentmemory/remember
POST   /agentmemory/forget
POST   /agentmemory/consolidate
GET    /agentmemory/patterns
POST   /agentmemory/generate-rules
POST   /agentmemory/migrate
POST   /agentmemory/evict
GET    /agentmemory/smart-search
GET    /agentmemory/timeline
GET    /agentmemory/profile
GET    /agentmemory/export
POST   /agentmemory/import
GET    /agentmemory/relations
POST   /agentmemory/evolve
POST   /agentmemory/auto-forget
GET    /agentmemory/memories
GET    /agentmemory/memory-by-id
GET    /agentmemory/semantic-list
GET    /agentmemory/procedural-list
GET    /agentmemory/relations-list

# 知识图谱 (3)
POST   /agentmemory/graph-query
GET    /agentmemory/graph-stats
POST   /agentmemory/graph-extract

# 团队 (4)
POST   /agentmemory/team-share
GET    /agentmemory/team-feed
GET    /agentmemory/team-profile
(还有更多团队端点在 api.ts 中)

# 治理 (2)
POST   /agentmemory/governance-delete
POST   /agentmemory/governance-bulk

# 快照 (3)
GET    /agentmemory/snapshots
POST   /agentmemory/snapshot-create
POST   /agentmemory/snapshot-restore

# 操作/工作流 (15+)
POST   /agentmemory/action-create
POST   /agentmemory/action-update
GET    /agentmemory/action-list
GET    /agentmemory/action-get
POST   /agentmemory/action-edge
GET    /agentmemory/frontier
GET    /agentmemory/next
POST   /agentmemory/routine-create
GET    /agentmemory/routine-list
POST   /agentmemory/routine-run
GET    /agentmemory/routine-status
POST   /agentmemory/lease-acquire
POST   /agentmemory/lease-release
POST   /agentmemory/lease-renew

# 信号 (2)
POST   /agentmemory/signal-send
GET    /agentmemory/signal-read

# 检查点 (3)
POST   /agentmemory/checkpoint-create
POST   /agentmemory/checkpoint-resolve
GET    /agentmemory/checkpoint-list

# 网格 (5)
POST   /agentmemory/mesh-register
GET    /agentmemory/mesh-list
POST   /agentmemory/mesh-sync
POST   /agentmemory/mesh-receive
GET    /agentmemory/mesh-export

# 记忆槽 (7)
GET    /agentmemory/slot-list
GET    /agentmemory/slot-get
POST   /agentmemory/slot-create
POST   /agentmemory/slot-append
POST   /agentmemory/slot-replace
POST   /agentmemory/slot-delete
POST   /agentmemory/slot-reflect

# Claude Bridge (2)
GET    /agentmemory/claude-bridge-read
POST   /agentmemory/claude-bridge-sync

# 其他 (15+)
POST   /agentmemory/flow-compress
GET    /agentmemory/branch-detect
GET    /agentmemory/branch-worktrees
GET    /agentmemory/branch-sessions
GET    /agentmemory/viewer
POST   /agentmemory/vision-search
GET    /agentmemory/diagnostics
GET    /agentmemory/facets
POST   /agentmemory/verify
POST   /agentmemory/sentinel-*
POST   /agentmemory/sketch-*
```

### 2.6 4 个 Skills（Claude Code 技能）

| 技能 | 文件 | 功能 |
|------|------|------|
| `remember` | `plugin/skills/remember/SKILL.md` | "记住这个"——用户要求保存记忆时触发 |
| `recall` | `plugin/skills/recall/SKILL.md` | "搜索记忆"——用户要求回忆时触发 |
| `forget` | `plugin/skills/forget/SKILL.md` | "忘记这个"——用户要求删除记忆时触发 |
| `session-history` | `plugin/skills/session-history/SKILL.md` | "查看历史"——用户要求查看会话历史时触发 |

---

## 3. 架构深度分析

### 3.1 AgentMemory 整体架构

```
┌─────────────────────────────────────────────────────────────────┐
│                    agentmemory (TypeScript)                       │
│                                                                   │
│  src/functions/*.ts                                              │
│    registerFunction("mem::observe", handler)                     │
│    registerFunction("mem::compress", handler)                    │
│    registerFunction("mem::remember", handler)                    │
│    registerFunction("mem::search", handler)                      │
│    50+ functions in total                                        │
│         │                                                        │
│         ▼                                                        │
│  src/state/kv.ts ← StateKV 封装                                  │
│    kv.get(scope, key)  ──► sdk.trigger("state::get", payload)   │
│    kv.set(scope, key, v)──► sdk.trigger("state::set", payload)  │
│    kv.list(scope)       ──► sdk.trigger("state::list", payload)  │
│    kv.delete(scope,key) ──► sdk.trigger("state::delete",payload) │
│         │                                                        │
│         ▼  WebSocket (port 49134)                                │
├─────────────────────────────────────────────────────────────────┤
│                    iii-engine (Rust, Docker)                      │
│                                                                   │
│  核心模块:                                                        │
│  ├── StateModule: 文件级 SQLite (./data/state_store.db)          │
│  │   ├── state::get    → SELECT value FROM kv WHERE scope=? AND   │
│  │   │                    key=?                                   │
│  │   ├── state::set    → INSERT OR REPLACE INTO kv ...           │
│  │   ├── state::list   → SELECT * FROM kv WHERE scope=?          │
│  │   └── state::delete → DELETE FROM kv WHERE scope=? AND key=?  │
│  ├── QueueModule: 内置任务队列                                    │
│  ├── PubSubModule: 本地发布订阅                                   │
│  ├── CronModule: 定时任务调度                                     │
│  ├── HTTPModule: REST API 路由 (port 3111)                        │
│  ├── StreamModule: 流式数据 (port 3112)                           │
│  └── MetricsModule: Prometheus 指标 (port 9464)                   │
│                                                                   │
│  Worker 管理:                                                     │
│  ├── iii-http      (接收 HTTP 请求 → 路由到注册的函数)           │
│  ├── iii-state     (SQLite 状态存储)                              │
│  ├── iii-queue     (任务队列)                                     │
│  ├── iii-pubsub    (发布订阅)                                     │
│  ├── iii-cron      (定时任务)                                     │
│  ├── iii-stream    (流式数据)                                     │
│  ├── iii-observability (指标和日志)                               │
│  └── iii-exec      (执行 agentmemory 的 dist/index.mjs)          │
└─────────────────────────────────────────────────────────────────┘
```

### 3.2 数据流详细路径

以「用户调用工具 → 记忆被保存」为例：

```
1. 用户/代理调用工具 (如 write file)
         │
2. iii-engine 触发 post_tool_use 钩子
         │
3. agentmemory 收到钩子负载 → mem::observe 函数
         │
4. observe.ts:
   ├── 验证 payload (sessionId, hookType, timestamp)
   ├── 去重检查 (SHA256 hash, 5 分钟 TTL)
   ├── 隐私过滤 (stripPrivateData)
   ├── 创建 RawObservation 对象
   ├── kv.set("mem:obs:{sessionId}", obsId, rawObservation)
   │       └── WebSocket → iii-engine → SQLite INSERT
   ├── 触发自动压缩 (mem::compress)
   │       └── WebSocket → iii-engine 路由 → mem::compress 函数
   └── 记录去重 hash
         │
5. compress.ts:
   ├── 构建压缩 prompt (COMPRESSION_SYSTEM + buildCompressionPrompt)
   ├── provider.compress(prompt) → LLM 返回 XML
   ├── parseCompressionXml(xml) → CompressedObservation
   │       ├── type: "file_write"
   │       ├── title: "Created main.rs"
   │       ├── facts: ["Added error handling", "…"]
   │       ├── narrative: "Created the main entry point…"
   │       ├── concepts: ["file_write", "rust", "error-handling"]
   │       ├── files: ["src/main.rs"]
   │       └── importance: 7
   ├── kv.set("mem:obs:{sessionId}", obsId + "_compressed", compressed)
   ├── 添加到 BM25 搜索索引
   ├── 添加向量嵌入 (if embedder available)
   ├── 质量评分 (scoreCompression)
   └── 存储指标 (MetricsStore)
```

### 3.3 iii-engine 的解耦设计

关键洞察：**iii-engine 不包含任何记忆逻辑**。它纯粹提供：

| 服务 | 等价于 |
|------|--------|
| KV 存储（get/set/list/delete） | SQLite 表 |
| 函数注册与路由 | `match` 分发或 `HashMap<String, Handler>` |
| 函数间调用（sdk.trigger） | 直接函数调用 |
| HTTP 路由 | 不需要（tidev 用内置 tool 替代） |
| 定时任务 | Tokio Cron |
| 发布订阅 | Tokio broadcast channel |
| 队列 | Tokio mpsc channel |

所有的**核心算法**（去重、BM25、RRF、Jaccard、LLM 压缩提示词、图谱提取、整合管线）都在 agentmemory 的 TypeScript 代码中实现，不在 iii-engine 里。

---

## 4. iii-sdk 依赖分析

### 4.1 iii-sdk 是什么

`iii-sdk`（npm 包 `^0.11.2`）是一个 **WebSocket 客户端**，连接远程 iii-engine（Docker 容器，端口 49134）。它的接口极其简单：

```typescript
// iii-sdk 暴露的核心类型
interface ISdk {
  registerFunction(name: string, handler: Function): void;
  trigger<T>(opts: { function_id: string; payload: unknown }): Promise<T>;
}

// agentmemory 的使用方式（70+ 文件都只用了这三个模式）
Pattern A: sdk.registerFunction("mem::xxx", handler)  // 注册
Pattern B: sdk.trigger({ function_id: "state::get", payload: {scope, key} })  // KV 操作
Pattern C: sdk.trigger({ function_id: "mem::search", payload: {query, limit} })  // 函数间调用
```

### 4.2 70+ 文件的使用统计

对 agentmemory 中所有导入 `iii-sdk` 的文件进行分析：

| 使用模式 | 文件数 | 涉及文件 |
|----------|--------|----------|
| 仅 `registerFunction` + kv.xxx | 68 | 所有 functions/*.ts, triggers/api.ts, mcp/server.ts |
| 仅 `ISdk` 类型导入 | 1 | `state/kv.ts` |
| 仅 `sdk.trigger` 调用 | 2 | `health/monitor.ts`, `cli.ts` |
| 使用 `TriggerAction` | 1 | `triggers/events.ts` |

**没有任何一个功能函数的算法逻辑依赖 iii-sdk 特有的能力。** 所有 iii-sdk 调用都可以被替换为：

| 原始 iii-sdk 调用 | Rust 等价实现 |
|-------------------|---------------|
| `sdk.registerFunction(name, fn)` | `registry.insert(name, Arc::new(fn))` |
| `sdk.trigger({function_id: "state::get", payload})` | `sqlite::query("SELECT value FROM kv WHERE scope=? AND key=?")` |
| `sdk.trigger({function_id: "mem::search", payload})` | `self.search_engine.search(query, limit)` |
| `sdk.trigger({function_id: "mem::compress", payload})` | `self.compress(raw_observation).await` |
| KV 的 scope 命名空间 | SQLite 表的分组字段 |

### 4.3 不需要 iii-sdk 的理由

1. **架构不匹配**：iii-sdk 是为远程 Docker 进程通信设计的 WebSocket 客户端，在单进程 Rust binary 中使用是多余的 IPC 开销。

2. **零算法贡献**：iii-sdk 不包含任何记忆、搜索或压缩算法——所有算法都在 agentmemory 的 TypeScript 源码中。

3. **运维负担**：使用 iii-sdk 意味着必须同时运行 iii-engine Docker 容器，违背了 tidev "单 binary 零外部依赖" 的设计目标。

4. **功能退化**：agentmemory 的 `packages/mcp/` 独立模式已经证明了这一点——没有 iii-engine 时只能提供 7/44 个工具，且数据仅存内存不持久。

5. **有更好的替代**：tidev 已有 SQLite 存储层、LLM provider（OpenAI/Anthropic）、PostToolUse hook 系统，这些都是 iii-engine 提供功能的超集。

---

## 5. TiDev 当前现状对比

### 5.1 现有记忆系统（src/memory/types.rs，453 行）

```rust
pub struct MemoryEntry {
    pub id: Uuid,
    pub workspace_root: String,
    pub memory_type: MemoryType,  // User | Project | Feedback | Reference
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source_session_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub usage_count: i64,
    pub active: bool,
}
```

现有 tools（src/tooling/builtin/memory.rs）：
| 操作 | 功能 | 实现 |
|------|------|------|
| store | 保存记忆 | SQLite INSERT + zstd 压缩 |
| update | 更新记忆 | SQLite UPDATE |
| search | 搜索 | SQL LIKE '%keyword%' |
| list | 列出 | SELECT active=1 ORDER BY usage |
| read | 读取 | SELECT by id |
| delete | 软删除 | SET active=0 |

### 5.2 功能差距矩阵

| 能力 | AgentMemory | TiDev 当前 | 差距 |
|------|-------------|------------|------|
| **自动观察捕获** | 12 种钩子自动记录 | 仅手动 `memory store` | 🔴 缺失 |
| **LLM 压缩** | 结构化 XML 摘要 | 原始文本存储 | 🔴 缺失 |
| **BM25 全文搜索** | BM25（k1=1.2, b=0.75） | SQL LIKE | 🔴 低效 |
| **向量嵌入** | 6 种 provider + 余弦相似度 | 不支持 | 🔴 缺失 |
| **RRF 混合搜索** | BM25 + 向量 + 图谱融合 | 无 | 🔴 缺失 |
| **Jaccard 去重** | >0.7 相似度版本链 | 无 | 🔴 缺失 |
| **SHA256 去重** | 5 分钟 TTL 窗口 | 无 | 🔴 缺失 |
| **重要性评分** | LLM 评分（1-10） | 仅 usage_count | 🔴 不足 |
| **知识图谱** | 实体/关系提取+图查询 | 无 | 🔴 缺失 |
| **记忆版本管理** | parentId/supersedes/isLatest | 无 | 🔴 缺失 |
| **概念标签** | LLM 提取 concepts[] | 手写 tags[] | 🟡 简陋 |
| **文件关联** | files[] 数组 | 无 | 🔴 缺失 |
| **会话摘要** | LLM 总结 | 无 | 🔴 缺失 |
| **整合管线** | 跨会话模式检测 | 无 | 🔴 缺失 |
| **自动遗忘** | TTL + 重要性衰减 | 永远保留 | 🔴 缺失 |
| **评估系统** | 质量评分 + 指标追踪 | 无 | 🔴 缺失 |
| **内存槽** | 命名槽（可追加/替换） | 无 | 🔴 缺失 |
| **审计日志** | 不可变操作记录 | 无 | 🔴 缺失 |
| **操作/工作流** | DAG + 状态机 | 无 | 🔴 缺失 |
| **多代理协调** | 信号/检查点/网格 | 不适用 | ⚪ 非目标 |

### 5.3 现有基础设施复用

| 现有能力 | 位置 | 可替代 iii-engine 的什么 |
|----------|------|-------------------------|
| SQLite 存储 | `src/storage/` | iii-engine StateModule（KV 存储） |
| PostToolUse 钩子 | `src/hooks/engine.rs` | agentmemory 的 12 种钩子 |
| LLM Provider（OpenAI/Anthropic） | `src/llm/` | 压缩、摘要、图谱提取 |
| 函数式 tool 系统 | `src/tooling/builtin/` | MCP 工具注册 |
| MCP 客户端 | `src/mcp.rs` | 对外暴露记忆检索工具 |
| TUI 面板 | `src/tui/` | 记忆浏览 |
| 异步运行时（Tokio） | Cargo.toml | 定时任务、队列、并发 |

---

## 6. 复刻架构设计

### 6.1 总体架构

```
┌──────────────────────────────────────────────────────────────────┐
│                        tidev (Rust)                               │
│                                                                    │
│  src/memory/  ← 新模块，完整复刻 agentmemory                      │
│  ├── mod.rs           ← MemoryEngine 主入口                        │
│  ├── types.rs         ← 完整数据模型（全部 20+ 类型）              │
│  ├── engine.rs        ← 核心引擎（KV → SQLite 映射）              │
│  ├── observe.rs       ← 自动观察捕获（复用 hooks）                │
│  ├── compress.rs      ← LLM 压缩（复用 llm provider）             │
│  ├── remember.rs      ← 记忆保存（含 Jaccard 去重）               │
│  ├── forget.rs        ← 遗忘/淘汰                                  │
│  ├── search.rs        ← 搜索（BM25 + 向量 + RRF）                 │
│  ├── search-index.rs  ← BM25 算法实现                              │
│  ├── vector-index.rs  ← 向量索引（余弦相似度）                    │
│  ├── dedup.rs         ← 去重（SHA256 + Jaccard）                  │
│  ├── graph.rs         ← 知识图谱                                   │
│  ├── slots.rs         ← 记忆槽                                     │
│  ├── sessions.rs      ← 会话管理/摘要                              │
│  ├── consolidate.rs   ← 整合管线                                   │
│  ├── insights.rs      ← 洞察/模式/教训                             │
│  ├── audit.rs         ← 审计日志                                   │
│  ├── actions.rs       ← 操作/工作流                                │
│  ├── leases.rs        ← 租约/并发控制                              │
│  ├── signals.rs       ← 代理间信号                                  │
│  ├── checkpoints.rs   ← 检查点                                     │
│  ├── timelines.rs     ← 时间线                                     │
│  ├── profile.rs       ← 画像/统计                                  │
│  ├── export.rs        ← 导入导出                                   │
│  ├── retention.rs     ← 保存评分/自动遗忘                          │
│  └── governance.rs    ← 治理/隐私                                  │
│                                                                    │
│  数据流路径:                                                        │
│  Hook 触发 → memory_engine.observe()                                │
│                   ↓                                                  │
│            memory_engine.compress_async()                           │
│                   ↓                                                  │
│            BM25 index.add() + Vector index.add()                    │
│                   ↓                                                  │
│            SQLite 持久化                                             │
│                                                                    │
│  LLM 请求 → memory_engine.llm_compress(observation)               │
│               ↓                                                     │
│          LlmClient.complete(COMPRESSION_SYSTEM, prompt)             │
│               ↓                                                     │
│          parse_compression_xml → CompressedObservation              │
└──────────────────────────────────────────────────────────────────┘
```

### 6.2 核心接口设计

```rust
/// 记忆引擎——所有功能的统一入口
pub struct MemoryEngine {
    db: MemoryDb,                    // SQLite 持久化
    bm25: Bm25Index,                 // BM25 全文索引（内存 + SQLite 持久化）
    vector: VectorIndex,             // 向量索引（内存）
    embedder: Option<Embedder>,      // 嵌入提供商
    llm: LlmClient,                  // LLM 客户端（压缩/摘要/图谱）
    config: MemoryConfig,            // 配置
}

impl MemoryEngine {
    // === 观察捕获 ===
    async fn observe(&self, payload: HookPayload) -> Result<ObservationId>;
    
    // === 压缩 ===
    async fn compress(&self, observation_id: &str) -> Result<CompressedObservation>;
    async fn compress_file(&self, path: &Path) -> Result<()>;
    async fn flow_compress(&self, session_id: &str) -> Result<Vec<CompressedObservation>>;
    
    // === 记忆 ===
    async fn remember(&self, content: &str, opts: RememberOpts) -> Result<Memory>;
    async fn forget(&self, id: &str) -> Result<()>;
    async fn evict(&self, criteria: EvictCriteria) -> Result<Vec<MemoryId>>;
    async fn consolidate(&self, session_ids: &[&str]) -> Result<ConsolidationReport>;
    
    // === 搜索 ===
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    async fn smart_search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>>;
    async fn context(&self, session_id: &str, token_budget: usize) -> Result<ContextBlocks>;
    async fn file_history(&self, path: &str, limit: usize) -> Result<Vec<CompressedObservation>>;
    
    // === 知识图谱 ===
    async fn graph_extract(&self, observation_id: &str) -> Result<Vec<GraphNode>>;
    async fn graph_query(&self, query: &str) -> Result<GraphQueryResult>;
    async fn graph_stats(&self) -> Result<GraphStats>;
    
    // === 会话 ===
    async fn summarize_session(&self, session_id: &str) -> Result<SessionSummary>;
    async fn timeline(&self, session_id: &str) -> Result<Vec<TimelineEvent>>;
    async fn patterns(&self, session_id: &str) -> Result<Vec<Pattern>>;
    async fn lessons(&self, session_id: &str) -> Result<Vec<Lesson>>;
    async fn insights(&self, session_ids: &[&str]) -> Result<Vec<Insight>>;
    
    // === 记忆槽 ===
    fn slot_list(&self, scope: SlotScope) -> Result<Vec<MemorySlot>>;
    fn slot_get(&self, label: &str) -> Result<Option<MemorySlot>>;
    fn slot_create(&self, slot: NewSlot) -> Result<MemorySlot>;
    fn slot_append(&self, label: &str, content: &str) -> Result<()>;
    fn slot_replace(&self, label: &str, content: &str) -> Result<()>;
    fn slot_delete(&self, label: &str) -> Result<()>;
    async fn slot_reflect(&self, label: &str) -> Result<String>;
    
    // === 操作/工作流 ===
    fn action_crud(...) -> ...;
    fn routine_crud(...) -> ...;
    
    // === 审计 ===
    fn audit_log(&self, opts: AuditQuery) -> Result<Vec<AuditEntry>>;
    
    // === 治理 ===
    fn governance_delete(&self, criteria: GovernanceCriteria) -> Result<()>;
    fn governance_bulk(&self, ops: Vec<GovernanceOp>) -> Result<()>;
    
    // === 同步结构（独立 binary 不包含）===
    // signals(), checkpoints(), mesh(), leases() — 保留接口，以空实现占位
}
```

### 6.3 事件驱动架构

agentmemory 大量使用异步事件驱动。在 Rust 中用 Tokio 等价实现：

```rust
// 对应 iii-engine 的 PubSub + Queue
pub enum MemoryEvent {
    ObservationCreated { id: ObservationId, session_id: String },
    ObservationCompressed { id: ObservationId, compressed: CompressedObservation },
    MemorySaved { id: MemoryId, memory: Memory },
    MemoryForgotten { id: MemoryId },
    SessionEnded { id: String },
    ConsolidationNeeded { session_id: String },
    // ... 更多事件
}

// 事件总线——使用 Tokio broadcast
pub struct EventBus {
    tx: broadcast::Sender<MemoryEvent>,
    rx: broadcast::Receiver<MemoryEvent>,
}

impl EventBus {
    async fn schedule_compression(&self, obs_id: &str) {
        // 延迟 500ms 后触发压缩（等待可能的后续操作到来）
        let tx = self.tx.clone();
        let id = obs_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = tx.send(MemoryEvent::CompressionNeeded { id });
        });
    }
}
```

---

## 7. 完整数据模型映射

### 7.1 AgentMemory KV Schema → TiDev SQLite Schema

agentmemory 的 KV 存储是无模式的（scope + key + JSON value）。在 SQLite 中，每个 scope 映射为一张表，以实现类型安全和查询效率。

```
AgentMemory KV Schema                    TiDev SQLite Schema
═══════════════════                      ══════════════════

mem:sessions                            sessions 表（已有）
  └── sessionId → Session                 ├── id TEXT PK
                                          ├── project TEXT
                                          ├── cwd TEXT
                                          ├── started_at TEXT
                                          ├── ended_at TEXT
                                          ├── status TEXT
                                          ├── observation_count INT
                                          ├── model TEXT
                                          ├── tags TEXT (JSON)
                                          ├── first_prompt TEXT
                                          └── summary TEXT

mem:obs:{sessionId}                     observations 表
  └── obsId → RawObservation              ├── id TEXT PK
                                          ├── session_id TEXT FK
                                          ├── timestamp TEXT
                                          ├── hook_type TEXT
                                          ├── tool_name TEXT
                                          ├── tool_input TEXT
                                          ├── tool_output TEXT
                                          ├── user_prompt TEXT
                                          ├── assistant_response TEXT
                                          ├── modality TEXT
                                          └── image_data TEXT

▸ 压缩观察存在同一 scope 下               compressed_observations 表
  obsId + "_compressed"                     ├── id TEXT PK (same as observation id)
  → CompressedObservation                   ├── observation_id TEXT FK
                                            ├── session_id TEXT
                                            ├── type TEXT
                                            ├── title TEXT
                                            ├── subtitle TEXT
                                            ├── facts TEXT (JSON array)
                                            ├── narrative TEXT
                                            ├── concepts TEXT (JSON array)
                                            ├── files TEXT (JSON array)
                                            ├── importance INT
                                            ├── confidence REAL
                                            └── modality TEXT

mem:memories                            memories 表（需要扩展）
  └── id → Memory                         现有:
                                          ├── id TEXT PK
                                          ├── workspace_root TEXT
                                          ├── memory_type TEXT
                                          ├── title TEXT
                                          ├── content BLOB (zstd)
                                          ├── tags TEXT (JSON)
                                          ├── source_session_id TEXT
                                          ├── created_at TEXT
                                          ├── updated_at TEXT
                                          ├── usage_count INT
                                          └── active INT

                                          新增字段:
                                          ├── concepts TEXT (JSON array)
                                          ├── files TEXT (JSON array)
                                          ├── strength REAL
                                          ├── importance INT
                                          ├── version INT
                                          ├── parent_id TEXT
                                          ├── supersedes TEXT (JSON array)
                                          ├── related_ids TEXT (JSON array)
                                          ├── is_latest INT
                                          └── forget_after TEXT

mem:summaries                           session_summaries 表
  └── sessionId → SessionSummary          ├── session_id TEXT PK FK
                                          ├── project TEXT
                                          ├── created_at TEXT
                                          ├── title TEXT
                                          ├── narrative TEXT
                                          ├── key_decisions TEXT (JSON)
                                          ├── files_modified TEXT (JSON)
                                          ├── concepts TEXT (JSON)
                                          └── observation_count INT

mem:graph:nodes                         graph_nodes 表
  └── nodeId → GraphNode                  ├── id TEXT PK
                                          ├── type TEXT
                                          ├── label TEXT
                                          ├── properties TEXT (JSON)
                                          └── created_at TEXT

mem:graph:edges                         graph_edges 表
  └── edgeId → GraphEdge                  ├── id TEXT PK
                                          ├── source_id TEXT FK
                                          ├── target_id TEXT FK
                                          ├── relation TEXT
                                          ├── weight REAL
                                          ├── properties TEXT (JSON)
                                          ├── created_at TEXT
                                          └── session_id TEXT

mem:slots                               memory_slots 表
  └── label → MemorySlot                  ├── label TEXT PK
                                          ├── content TEXT
                                          ├── size_limit INT
                                          ├── description TEXT
                                          ├── pinned INT
                                          ├── read_only INT
                                          ├── scope TEXT
                                          ├── project TEXT
                                          ├── created_at TEXT
                                          └── updated_at TEXT

mem:slots:global                        同上，scope 字段区分
  └── label → MemorySlot

mem:actions                             actions 表
  └── actionId → Action                   ├── id TEXT PK
                                          ├── session_id TEXT
                                          ├── title TEXT
                                          ├── description TEXT
                                          ├── status TEXT
                                          ├── priority INT
                                          ├── dependencies TEXT (JSON)
                                          ├── created_at TEXT
                                          └── updated_at TEXT

mem:action-edges                        action_edges 表
  └── edgeId → ActionEdge                  ├── id TEXT PK
                                           ├── source_id TEXT FK
                                           ├── target_id TEXT FK
                                           └── type TEXT

mem:leases                              leases 表
  └── resourceId → Lease                  ├── resource_id TEXT PK
                                          ├── holder_id TEXT
                                          ├── acquired_at TEXT
                                          ├── expires_at TEXT
                                          └── renewals INT

mem:signals                             signals 表
  └── signalId → Signal                   ├── id TEXT PK
                                          ├── from TEXT
                                          ├── to TEXT
                                          ├── type TEXT
                                          ├── payload TEXT (JSON)
                                          ├── status TEXT
                                          ├── created_at TEXT
                                          └── delivered_at TEXT

mem:checkpoints                         checkpoints 表
  └── checkpointId → Checkpoint           ├── id TEXT PK
                                          ├── action_id TEXT
                                          ├── state_snapshot TEXT
                                          ├── created_at TEXT
                                          └── resolved_at TEXT

mem:routines                            routines 表
  └── routineId → Routine                 ├── id TEXT PK
                                          ├── name TEXT
                                          ├── definition TEXT (JSON)
                                          ├── created_at TEXT
                                          └── updated_at TEXT

mem:routine-runs                        routine_runs 表
  └── runId → RoutineRun                  ├── id TEXT PK
                                          ├── routine_id TEXT FK
                                          ├── status TEXT
                                          ├── progress TEXT
                                          ├── started_at TEXT
                                          └── completed_at TEXT

mem:audit                               audit_log 表
  └── entryId → AuditEntry                ├── id TEXT PK
                                          ├── timestamp TEXT
                                          ├── operation TEXT
                                          ├── entity_type TEXT
                                          ├── entity_id TEXT
                                          ├── actor TEXT
                                          ├── details TEXT (JSON)
                                          └── session_id TEXT

mem:access                              access_log 表
  └── entryId → AccessEntry               ├── id TEXT PK
                                          ├── entity_type TEXT
                                          ├── entity_id TEXT
                                          ├── accessed_at TEXT
                                          └── session_id TEXT

mem:retention                           retention_scores 表
  └── entityId → RetentionScore           ├── entity_id TEXT PK
                                          ├── entity_type TEXT
                                          ├── importance REAL
                                          ├── access_frequency REAL
                                          ├── age_days REAL
                                          ├── score REAL
                                          └── computed_at TEXT

mem:metrics                             function_metrics 表
  └── functionId → Metrics                ├── function_id TEXT PK
                                          ├── total_calls INT
                                          ├── success_count INT
                                          ├── failure_count INT
                                          ├── avg_latency_ms REAL
                                          └── avg_quality_score REAL

mem:team:*（3 个子scope）                team_* 表（单用户不启用）
mem:mesh                                 mesh_* 表（单进程不启用）
mem:sketches                             sketches 表
mem:sentinels                            sentinels 表
mem:crystals                             crystals 表
mem:lessons                              lessons 表
mem:insights                             insights 表
mem:config                               config 表
mem:health                               health 表（运行时状态，不入库）
```

### 7.2 全文搜索替换方案

agentmemory 在内存中维护 BM25 索引和向量索引，定期持久化到 iii-engine 的 KV 存储。

在 tidev 中，使用 **SQLite FTS5** 替代自研 BM25：

```sql
-- 全文搜索虚拟表
CREATE VIRTUAL TABLE observations_fts USING fts5(
    tool_name, tool_input, tool_output,
    title, narrative, facts,
    concepts, files,
    content='observations',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

CREATE VIRTUAL TABLE memories_fts USING fts5(
    title, content, tags, concepts, files,
    content='memories',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

-- 查询
SELECT m.*, rank FROM memories_fts f
JOIN memories m ON m.rowid = f.rowid
WHERE memories_fts MATCH ?1
ORDER BY rank LIMIT ?2;
```

**保留内存 BM25 索引**用于 RRF 融合时需要归一化分数。实现方式：

```rust
pub struct Bm25Index {
    entries: HashMap<String, Bm25Entry>,  // obs_id → tokenized entry
    inverted_index: HashMap<String, HashMap<String, usize>>,  // term → {doc_id → tf}
    total_docs: usize,
    total_doc_length: f64,
    k1: f64,  // 1.2
    b: f64,   // 0.75
}

impl Bm25Index {
    fn add(&mut self, id: &str, text: &str);
    fn remove(&mut self, id: &str);
    fn search(&self, query: &str, limit: usize) -> Vec<(String, f64)>;
    fn bm25_score(&self, term: &str, doc_id: &str, doc_length: f64) -> f64 {
        // BM25 = IDF * (tf * (k1+1)) / (tf + k1 * (1 - b + b * docLen/avgDocLen))
    }
}
```

### 7.3 向量索引实现

```rust
pub struct VectorIndex {
    vectors: HashMap<String, (Vec<f32>, String)>,  // obs_id → (embedding, session_id)
    dimensions: usize,
}

impl VectorIndex {
    fn add(&mut self, id: &str, session: &str, embedding: Vec<f32>);
    fn remove(&mut self, id: &str);
    
    fn search(&self, query: &[f32], limit: usize) -> Vec<(String, f64)> {
        // 余弦相似度 + 最小堆 top-K
        let mut heap: BinaryHeap<Reverse<(f64, String)>> = BinaryHeap::new();
        for (id, (emb, _)) in &self.vectors {
            let sim = cosine_similarity(query, emb);
            heap.push(Reverse((sim, id.clone())));
            if heap.len() > limit { heap.pop(); }
        }
        heap.into_sorted_vec().into_iter()
            .map(|Reverse((s, id))| (id, s))
            .rev()
            .collect()
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum();
    let norm_b: f32 = b.iter().map(|x| x * x).sum();
    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 { 0.0 } else { (dot / denom) as f64 }
}
```

### 7.4 RRF 融合搜索

```rust
pub struct HybridSearch {
    bm25: Bm25Index,
    vector: VectorIndex,
    bm25_weight: f64,   // 0.4
    vector_weight: f64, // 0.6
    graph_weight: f64,  // 0.3
    rrf_k: f64,         // 60
}

impl HybridSearch {
    fn search(&self, query: &str, limit: usize, embedding: Option<&[f32]>) -> Vec<HybridResult> {
        // 1. BM25 搜索结果
        let bm25_results = self.bm25.search(query, limit * 2);
        
        // 2. 向量搜索结果
        let vector_results = embedding.map(|emb| self.vector.search(emb, limit * 2))
            .unwrap_or_default();
        
        // 3. 知识图谱检索（如果有）
        // ...
        
        // 4. RRF 融合
        let mut scores: HashMap<String, ScoreComponents> = HashMap::new();
        
        for (i, (id, score)) in bm25_results.iter().enumerate() {
            let rrf = 1.0 / (self.rrf_k + i as f64);
            scores.entry(id.clone()).or_default()
                .combined += rrf * self.bm25_weight;
            scores.get_mut(id).unwrap().bm25 = Some(*score);
        }
        
        for (i, (id, score)) in vector_results.iter().enumerate() {
            let rrf = 1.0 / (self.rrf_k + i as f64);
            scores.entry(id.clone()).or_default()
                .combined += rrf * self.vector_weight;
            scores.get_mut(id).unwrap().vector = Some(*score);
        }
        
        // 5. 排序取 top-K
        let mut results: Vec<_> = scores.into_iter().collect();
        results.sort_by(|a, b| b.1.combined.partial_cmp(&a.1).unwrap());
        results.into_iter().take(limit).map(|(id, s)| HybridResult {
            id, score: s.combined, components: s
        }).collect()
    }
}
```

---

## 8. 分期实施路线图

### 8.1 总览

```
Phase 0 ── 基础设施搭建（当前 tidev 已有）
  ├── SQLite 存储层 ✓
  ├── LLM Provider（OpenAI/Anthropic） ✓
  ├── PostToolUse 钩子系统 ✓
  ├── 工具注册系统 ✓
  └── TUI 框架 ✓

Phase 1 ── 核心记忆引擎（复刻 agentmemory 60% 价值）
  ├── 完整数据模型与数据库迁移
  ├── DV1: 自动观察捕获（复用 hooks）
  ├── DV2: LLM 压缩管道（复用 LLM provider）
  ├── DV3: BM25 全文搜索（FTS5 + 内存索引）
  ├── DV4: 记忆去重（SHA256 + Jaccard）
  ├── DV5: 核心 CRUD（扩展现有 store/update/search/list/read/delete）
  ├── DV6: 会话管理 + 摘要
  └── DV7: 审计日志

Phase 2 ── 语义搜索与知识图谱（复刻 agentmemory 85% 价值）
  ├── DV8: OpenAI Embeddings API
  ├── DV9: 内存向量索引
  ├── DV10: RRF 混合搜索融合
  ├── DV11: 知识图谱（实体/关系提取 + 图查询）
  ├── DV12: 重要性评分 + 保存度计算
  ├── DV13: 自动遗忘 + 淘汰策略
  └── DV14: 记忆槽

Phase 3 ── 高级能力（完整复刻）
  ├── DV15: 整合管线（跨会话模式检测）
  ├── DV16: 洞察/模式/教训提取
  ├── DV17: 操作 DAG + 前沿计算
  ├── DV18: 工作流例程
  ├── DV19: 导入导出
  ├── DV20: 治理/隐私
  ├── DV21: 评估系统（质量评分 + 指标）
  ├── DV22: 并发控制（租约/互斥锁）
  └── DV23: 代理间通信（信号/检查点）

Phase 4 ── 工具暴露与 UI（面向用户交付）
  ├── 升级现有 memory tool 为完整接口
  ├── 新增 MCP 工具（按需暴露）
  ├── 升级 TUI 记忆面板
  ├── 系统提示词注入增强
  └── 性能优化与基准测试
```

### 8.2 每个功能的详细实现说明

#### DV1: 自动观察捕获

**文件**：`src/memory/observe.rs`

**对应源码**：`agentmemory/src/functions/observe.ts`（281 行）

**逻辑**：
1. 接收 `HookPayload`（sessionId, hookType, timestamp, toolName, toolInput, toolOutput）
2. 验证 payload 完整性
3. SHA256 去重检查（5 分钟 TTL 窗口）
4. 隐私过滤（去除密码、API key 等敏感字段）
5. 创建 `RawObservation`，写入 `observations` 表
6. 触发异步压缩（tokio::spawn，延迟 500ms）
7. 更新 `sessions.observation_count`

**数据结构**：
```rust
pub struct HookPayload {
    pub session_id: Uuid,
    pub hook_type: HookType,
    pub timestamp: DateTime<Utc>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
}

pub struct RawObservation {
    pub id: Uuid,
    pub session_id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub hook_type: HookType,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_output: Option<String>,
    pub user_prompt: Option<String>,
    pub assistant_response: Option<String>,
    pub modality: Modality,
    pub image_data: Option<String>,
}
```

**去重算法**（`src/memory/dedup.rs`）：
```rust
pub struct DedupMap {
    entries: LruCache<String, Instant>,
}

impl DedupMap {
    pub fn compute_hash(&self, session_id: &str, tool_name: &str, input: &str) -> String {
        let input = truncate(input, 500);
        let raw = format!("{}:{}:{}", session_id, tool_name, input);
        blake3::hash(raw.as_bytes()).to_hex()[..32].to_string()
    }

    pub fn check_duplicate(&mut self, hash: &str) -> bool {
        if let Some(expires) = self.entries.get(hash) {
            if *expires > Instant::now() {
                return true; // 重复
            }
        }
        false
    }

    pub fn record(&mut self, hash: String) {
        self.entries.put(hash, Instant::now() + Duration::from_secs(300)); // 5 分钟 TTL
    }
}
```

**与 tidev hooks 集成**：在 `src/hooks/engine.rs` 的 `on_post_tool_use()` 末尾调用 `memory_engine.observe(payload)`。

---

#### DV2: LLM 压缩管道

**文件**：`src/memory/compress.rs`

**对应源码**：`agentmemory/src/functions/compress.ts`（266 行）

**逻辑**：
1. 读 `RawObservation`（从数据库）
2. 构建压缩 prompt
3. 调用 `llm_client.complete(COMPRESSION_SYSTEM, prompt)` 
4. 解析 LLM 返回的 XML（`<type>`, `<title>`, `<facts>`, `<narrative>`, `<concepts>`, `<files>`, `<importance>`）
5. 验证字段完整性（type + title 必须存在）
6. 创建 `CompressedObservation`，写入 `compressed_observations` 表
7. 添加至 BM25 搜索索引
8. 向量嵌入（若 embedder 可用）
9. 质量评分（事实覆盖率、重要性合理性）

**LLM 压缩提示词**（翻译自 `agentmemory/src/prompts/compression.ts`）：
```
You are an observation compression system. Extract structured information
from the following tool call observation.

Tool: {tool_name}
Input: {tool_input}
Output: {tool_output}

Respond with XML exactly in this format:
<type>file_read|file_write|file_edit|command_run|search|web_fetch|conversation|error|decision|discovery|subagent|notification|task|image|other</type>
<title>A brief title (max 80 chars)</title>
<subtitle>Optional context (max 120 chars)</subtitle>
<facts>
  <fact>Atomic fact 1</fact>
  <fact>Atomic fact 2</fact>
</facts>
<narrative>One paragraph narrative</narrative>
<concepts>
  <concept>concept1</concept>
  <concept>concept2</concept>
</concepts>
<files>
  <file>/path/to/file</file>
</files>
<importance>5</importance>
```

**回退策略**：当 LLM 不可用时，使用 `compress_synthetic()` 用规则引擎生成简化摘要。

---

#### DV3: BM25 全文搜索

**文件**：`src/memory/search-index.rs`

**对应源码**：`agentmemory/src/state/search-index.ts`

**双重实现**：
1. SQLite FTS5（主查询路径，持久化）
2. 内存 BM25 索引（用于 RRF 融合分数归一化）

**BM25 参数**（与 agentmemory 一致）：
- k1 = 1.2（词频饱和度）
- b = 0.75（文档长度归一化）

**查询扩展**（`src/memory/query-expansion.rs`）：
- 同义词扩展（权重 0.7）
- CJK 分词支持（通过 `tiny-segmenter` 算法的 Rust 实现或 jieba-rs）

---

#### DV4: 记忆去重

**文件**：`src/memory/dedup.rs`（见 DV1） + `src/memory/remember.rs`

**两种去重机制**：

1. **操作级去重**（SHA256 + 5 分钟 TTL）：防止同一工具调用被多次记录
2. **内容级去重**（Jaccard 相似度 > 0.7）：防止存储内容相似的记忆

**Jaccard 相似度**（翻译自 `agentmemory/src/state/schema.ts:68-77`）：
```rust
pub fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace()
        .filter(|t| t.len() > 2).collect();
    let set_b: HashSet<&str> = b.split_whitespace()
        .filter(|t| t.len() > 2).collect();
    
    if set_a.is_empty() && set_b.is_empty() { return 1.0; }
    if set_a.is_empty() || set_b.is_empty() { return 0.0; }
    
    let intersection = set_a.intersection(&set_b).count();
    intersection as f64 / (set_a.len() + set_b.len() - intersection) as f64
}
```

---

#### DV5: 完整 CRUD

**文件**：`src/memory/engine.rs`

扩展当前的 `MemoryStore`，增加：

```rust
// 新增操作
fn add_tags(&self, id: &str, tags: &[&str]) -> Result<()>;
fn remove_tags(&self, id: &str, tags: &[&str]) -> Result<()>;
fn add_files(&self, id: &str, files: &[&str]) -> Result<()>;
fn add_concepts(&self, id: &str, concepts: &[&str]) -> Result<()>;
fn merge(&self, ids: &[&str]) -> Result<MemoryEntry>;  // 合并多条记忆
fn get_version_chain(&self, id: &str) -> Result<Vec<MemoryEntry>>;  // 版本链
```

**版本管理**（对应 agentmemory 的 `parentId/supersedes/isLatest`）：
```rust
pub fn remember_with_dedup(&self, new: &NewMemory) -> Result<MemoryEntry> {
    let existing = self.search_by_content(&new.content)?;
    let mut version = 1;
    let mut parent_id = None;
    let mut supersedes = vec![];
    
    for mem in &existing {
        let sim = jaccard_similarity(&new.content, &mem.content);
        if sim > 0.7 {
            supersedes.push(mem.id);
            parent_id = Some(mem.id);
            version = mem.version + 1;
            self.set_latest_flag(&mem.id, false)?; // 标记旧版本为非最新
        }
    }
    
    // 保存新版本
    let entry = MemoryEntry {
        version,
        parent_id,
        supersedes: serde_json::to_string(&supersedes)?,
        is_latest: true,
        .. // 其他字段
    };
    self.add(&entry)
}
```

---

#### DV6: 会话管理 + 摘要

**文件**：`src/memory/sessions.rs`

**对应源码**：`agentmemory/src/functions/summarize.ts`

**逻辑**：
1. 会话开始时：创建 Session 记录
2. 会话过程中：每个观察自动关联到会话
3. 会话结束时：触发 LLM 摘要生成
4. 会话摘要结构（对应 `SessionSummary`）：
   - project, title, narrative
   - keyDecisions[]：提取的关键决策
   - filesModified[]：变更文件列表
   - concepts[]：涉及的概念
   - observationCount：观察总数

---

#### DV7: 审计日志

**文件**：`src/memory/audit.rs`

**对应源码**：`agentmemory/src/functions/audit.ts`

```rust
pub fn record_audit(&self, operation: AuditOp, entity_type: &str, entity_id: &str) {
    // 不可变追加，仅 INSERT
    db.execute("INSERT INTO audit_log (...) VALUES (...)", params![
        Uuid::new_v4(),
        Utc::now().to_rfc3339(),
        operation.as_str(),
        entity_type,
        entity_id,
        // actor, details, session_id
    ])?;
}

// 查询支持：按时间/操作/实体分页
pub fn query_audit(&self, q: AuditQuery) -> Result<Vec<AuditEntry>> {
    // SELECT ... WHERE timestamp BETWEEN ? AND ?
    //   AND (operation = ? OR ? IS NULL)
    //   AND (entity_type = ? OR ? IS NULL)
    // ORDER BY timestamp DESC LIMIT ? OFFSET ?
}
```

---

#### DV8: OpenAI Embeddings API

**文件**：`src/llm/embeddings.rs`（新增）

**对应源码**：`agentmemory/src/providers/embedding/`

```rust
pub trait EmbeddingProvider: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
}

pub struct OpenAIEmbedder {
    client: reqwest::Client,
    api_key: String,
    model: String,   // "text-embedding-3-small"
    dimensions: usize, // 1536
}

impl OpenAIEmbedder {
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            dimensions: match model {
                "text-embedding-3-small" => 1536,
                "text-embedding-3-large" => 3072,
                "text-embedding-ada-002" => 1536,
                _ => 1536,
            },
        }
    }
    
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // POST https://api.openai.com/v1/embeddings
        // 复用现有的 reqwest client 和 api_key 配置
        let resp = self.client.post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&json!({
                "model": self.model,
                "input": text,
            }))
            .send()?;
        // 解析响应，提取向量
    }
}
```

**vector_embed 输入裁剪**（对应 agentmemory 的 `EMBED_MAX_CHARS = 16_000`）：
```rust
const EMBED_MAX_CHARS: usize = 16_000;

fn clip_embed_input(text: &str) -> &str {
    if text.len() <= EMBED_MAX_CHARS { text }
    else { &text[..EMBED_MAX_CHARS] }
}
```

---

#### DV9: 内存向量索引

**文件**：`src/memory/vector-index.rs`

**对应源码**：`agentmemory/src/state/vector-index.ts`

**核心数据结构**：
```rust
pub struct VectorIndex {
    vectors: HashMap<String, VectorEntry>,
    dimensions: usize,
}

struct VectorEntry {
    embedding: Vec<f32>,
    session_id: String,
    created_at: DateTime<Utc>,
}

impl VectorIndex {
    pub fn add(&mut self, id: &str, session: &str, embedding: Vec<f32>);
    pub fn remove(&mut self, id: &str);
    pub fn search(&self, query: &[f32], limit: usize) -> Vec<(String, f64)>;
    pub fn session_count(&self, session_id: &str) -> usize;
    pub fn clear_session(&mut self, session_id: &str);
    pub fn size(&self) -> usize;
}
```

**维度守卫**（对应 agentmemory 的 `withDimensionGuard`）：
```rust
impl VectorIndex {
    pub fn add(&mut self, id: &str, session: &str, embedding: Vec<f32>) -> Result<()> {
        if embedding.len() != self.dimensions {
            bail!("Embedding dimension mismatch: expected {}, got {}",
                  self.dimensions, embedding.len());
        }
        self.vectors.insert(id.to_string(), VectorEntry {
            embedding,
            session_id: session.to_string(),
            created_at: Utc::now(),
        });
        Ok(())
    }
}
```

---

#### DV10: RRF 混合搜索

**文件**：`src/memory/hybrid-search.rs`

**对应源码**：`agentmemory/src/state/hybrid-search.ts`

**实现细节**已在 7.4 节描述。

**额外特性**：可选的 **交叉编码器重排序**（cross-encoder rerank），对应 agentmemory 的 `ms-marco-MiniLM-L-6-v2`。在 tidev 中可以通过 MCP 实现（或暂时跳过）。

---

#### DV11: 知识图谱

**文件**：`src/memory/graph.rs`

**对应源码**：`agentmemory/src/functions/graph.ts`

**核心流程**：
1. 在压缩阶段，LLM 提取实体和关系
2. 实体存储到 `graph_nodes` 表
3. 关系存储到 `graph_edges` 表
4. 查询时支持：按实体名/关系类型/时间范围

**图谱提取 prompt**：
```
Extract entities and relationships from this observation:

{compressed_observation}

Respond in JSON:
{
  "entities": [
    {"name": "...", "type": "file|function|concept|person|tool|...", "metadata": {}}
  ],
  "relations": [
    {"source": "...", "target": "...", "type": "uses|modifies|creates|depends_on|..."}
  ]
}
```

---

#### DV12-DV23

详见各阶段详细设计（在 Phase 3 实施前补充）。

### 8.3 预计代码量统计

| 模块 | 预计行数 | 复杂度 | 依赖 |
|------|---------|--------|------|
| 数据模型 + 数据库迁移 | ~200 | 低 | SQLite |
| DV1 自动观察捕获 | ~150 | 低 | hooks 系统 |
| DV2 LLM 压缩 | ~200（+prompt） | 中 | LLM Provider |
| DV3 BM25 搜索 | ~150 | 中 | FTS5 + 内存索引 |
| DV4 去重 | ~100 | 低 | LRU cache |
| DV5 完整 CRUD | ~200 | 低 | SQLite |
| DV6 会话管理 | ~150 | 低 | SQLite |
| DV7 审计日志 | ~80 | 低 | SQLite |
| DV8 Embeddings API | ~150 | 低 | reqwest |
| DV9 向量索引 | ~100 | 低 | 纯算法 |
| DV10 RRF 混合搜索 | ~120 | 中 | 纯算法 |
| DV11 知识图谱 | ~250 | 高 | LLM Provider |
| DV12 重要性/保存度 | ~100 | 中 | 纯计算 |
| DV13 自动遗忘 | ~80 | 中 | SQLite |
| DV14 记忆槽 | ~150 | 低 | SQLite |
| DV15 整合管线 | ~200 | 高 | LLM Provider |
| DV16 洞察/模式/教训 | ~200 | 高 | LLM Provider |
| DV17 操作 DAG | ~300 | 高 | 纯算法 |
| DV18 工作流例程 | ~250 | 高 | 纯算法 |
| DV19 导入导出 | ~150 | 低 | JSON |
| DV20 治理 | ~100 | 低 | SQLite |
| DV21 评估系统 | ~150 | 中 | 纯算法 |
| DV22 租约/互斥 | ~100 | 中 | 纯算法 |
| DV23 信号/检查点 | ~150 | 中 | SQLite |
| Tool 暴露 | ~200 | 低 | tooling 系统 |
| TUI 面板升级 | ~300 | 中 | TUI 系统 |
| **总计** | **~3,830** | | |

---

## 9. 附录：关键文件参考

### 9.1 AgentMemory 关键源码文件索引

| 功能 | 文件路径（agentmemory） | 行数 |
|------|------------------------|------|
| 主入口 | `src/index.ts` | 532 |
| 类型定义 | `src/types.ts` | 864 |
| KV 定义 | `src/state/schema.ts` | 78 |
| KV 封装 | `src/state/kv.ts` | 47 |
| 观察捕获 | `src/functions/observe.ts` | 281 |
| 压缩 | `src/functions/compress.ts` | 266 |
| 记忆保存 | `src/functions/remember.ts` | 228 |
| 搜索 | `src/functions/search.ts` | 352 |
| 智能搜索 | `src/functions/smart-search.ts` | ~200 |
| 混合搜索 | `src/state/hybrid-search.ts` | ~200 |
| BM25 索引 | `src/state/search-index.ts` | ~200 |
| 向量索引 | `src/state/vector-index.ts` | ~100 |
| 去重 | `src/functions/dedup.ts` | ~50 |
| 会话摘要 | `src/functions/summarize.ts` | ~200 |
| 整合 | `src/functions/consolidate.ts` | ~300 |
| 整合管线 | `src/functions/consolidation-pipeline.ts` | ~200 |
| 知识图谱 | `src/functions/graph.ts` | ~400 |
| 图谱检索 | `src/functions/graph-retrieval.ts` | ~200 |
| 洞察 | `src/functions/insights.ts` | ~150 |
| 教训 | `src/functions/lessons.ts` | ~150 |
| 结晶化 | `src/functions/crystallize.ts` | ~200 |
| 反思 | `src/functions/reflect.ts` | ~150 |
| 模式 | `src/functions/patterns.ts` | ~150 |
| 保存度 | `src/functions/retention.ts` | ~150 |
| 访问追踪 | `src/functions/access-tracker.ts` | ~150 |
| 审计 | `src/functions/audit.ts` | ~100 |
| 记忆槽 | `src/functions/slots.ts` | ~200 |
| 操作 | `src/functions/actions.ts` | ~300 |
| 前沿 | `src/functions/frontier.ts` | ~200 |
| 例程 | `src/functions/routines.ts` | ~250 |
| 租约 | `src/functions/leases.ts` | ~150 |
| 信号 | `src/functions/signals.ts` | ~150 |
| 检查点 | `src/functions/checkpoints.ts` | ~150 |
| 网格 | `src/functions/mesh.ts` | ~300 |
| 团队 | `src/functions/team.ts` | ~200 |
| 治理 | `src/functions/governance.ts` | ~150 |
| 自动遗忘 | `src/functions/auto-forget.ts` | ~100 |
| 淘汰 | `src/functions/evict.ts` | ~100 |
| 文件索引 | `src/functions/file-index.ts` | ~200 |
| 时间线 | `src/functions/timeline.ts` | ~100 |
| 画像 | `src/functions/profile.ts` | ~150 |
| 导入导出 | `src/functions/export-import.ts` | ~200 |
| 质量评估 | `src/eval/quality.ts` | ~100 |
| 指标存储 | `src/eval/metrics-store.ts` | ~100 |
| 自纠正 | `src/eval/self-correct.ts` | ~100 |
| 验证器 | `src/eval/validator.ts` | ~50 |
| MCP 工具注册 | `src/mcp/tools-registry.ts` | 923 |
| MCP 服务器 | `src/mcp/server.ts` | ~500 |
| MCP 独立模式 | `src/mcp/standalone.ts` | ~200 |
| REST API | `src/triggers/api.ts` | ~800 |
| 压缩提示词 | `src/prompts/compression.ts` | ~100 |
| 嵌入提供商 | `src/providers/embedding/index.ts` | ~100 |
| CLI | `src/cli.ts` | 43,547 |
| 配置 | `src/config.ts` | ~300 |

### 9.2 TiDev 现有代码参考

| 功能 | 文件路径（tidev） | 行数 |
|------|--------------------|------|
| 记忆存储 | `src/memory/types.rs` | 453 |
| 记忆工具 | `src/tooling/builtin/memory.rs` | 328 |
| 数据库 schema | `src/storage/schema.rs` | 410 |
| 压缩存储 | `src/storage/compression.rs` | 95 |
| LLM 客户端 | `src/llm/mod.rs` | ~246 |
| OpenAI provider | `src/llm/openai.rs` | ~200 |
| Anthropic provider | `src/llm/anthropic.rs` | ~200 |
| Hook 引擎 | `src/hooks/engine.rs` | ~155 |
| Hook 配置 | `src/hooks/config.rs` | ~71 |
| MCP 实现 | `src/mcp.rs` | 622 |
| MCP 配置 | `src/config/mcp.rs` | ~44 |
| 工具注册 | `src/tooling/registry.rs` | 519 |
| 代理运行时 | `src/agent/runtime.rs` | ~600 |
| TUI 记忆面板 | `src/tui/ui/memory_panel.rs` | 541 |

---

## 关于 iii-sdk 的最终结论

**不需要、也不应该依赖 iii-sdk。**

原因总结：

1. **iii-sdk 只是一个 WebSocket 客户端**，它在 agentmemory 中的全部作用可以归纳为三个 API 调用的封装：`kv.get/set/list`（映射到 SQLite），`registerFunction`（映射到 Rust trait/enum），`sdk.trigger`（映射到直接函数调用）。

2. **零算法贡献**：所有记忆算法（去重、BM25、RRF、Jaccard、压缩提示词、图谱提取）都在 agentmemory 的 TypeScript 源码中，不依赖 iii-sdk。

3. **运维成本不可接受**：使用 iii-sdk 意味着必须运行 `iiidev/iii:0.11.2` Docker 容器（2 个容器、4 个端口、持久卷），这与 tidev "单 Binary 零外部依赖"的核心设计哲学冲突。

4. **性能劣化**：通过 WebSocket + JSON 序列化访问同一台机器的 SQLite，比 Rust 函数直接调用 SQLite 慢 100-1000 倍。

5. **功能退化风险**：agentmemory 自己的 `packages/mcp/` 已经证明——没有 iii-engine 时只能提供降级服务。

复刻方案将所有 iii-sdk 的 WebSocket 远程调用替换为 Rust 原生调用，效果完全相同，而性能更优、资源更少、运维更简单。
