# TiDev 记忆系统升级方案 & 实现状态

> 基于对 [agentmemory](https://github.com/rohitg00/agentmemory) v0.9.12 的逆向分析，在 tidev 中以 Rust 复刻。
>
> 更新时间：2026-05-14（Phase 1 ✅, Phase 2 ✅, 集成修复 ✅）

---

## 目录

1. [核心架构与数据流](#1-核心架构与数据流)
2. [已实现功能清单](#2-已实现功能清单)
3. [未实现功能清单](#3-未实现功能清单)
4. [已知简化与妥协](#4-已知简化与妥协)
5. [数据模型映射](#5-数据模型映射)
6. [关键设计决策](#6-关键设计决策)

---

## 1. 核心架构与数据流

### 1.1 模块结构

```
src/memory/
├── mod.rs            — 模块声明 + 公共导出
├── types.rs          — MemoryEntry / HookType / ObservationType / MemorySlot / ...
├── engine.rs         — MemoryStore 主入口（SQLite 持久化，~590 行）
├── observe.rs        — 自动观察捕获（SHA256 去重 + 入库）
├── compress.rs       — LLM 压缩（COMPRESSION_SYSTEM + XML 解析）
├── search_index.rs   — BM25 内存索引 + FTS5 查询封装
├── dedup.rs          — SHA256 去重映射（LRU + 5 分钟 TTL）
├── remember.rs       — Jaccard 去重 + 版本链管理
├── sessions.rs       — LLM 会话摘要
├── audit.rs          — 不可变审计日志
├── slots.rs          — 记忆槽 CRUD + 8 默认槽 + 提示词渲染
├── embed.rs          — OpenAI Embeddings API 客户端
├── vector_index.rs   — 内存余弦相似度索引
├── hybrid_search.rs  — RRF BM25 + 向量融合搜索
├── retention.rs      — 时间衰减保存度评分
└── evict.rs          — 自动淘汰策略
```

### 1.2 端到端数据流

```
┌──────────────────────────────────────────────────────────────────┐
│                      工具执行流程                                   │
├──────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ① 工具执行前                                                     │
│     on_pre_tool_use() → observe(HookPayload{PreToolUse})         │
│                          └─ SHA256 去重 → INSERT observations     │
│                                                                   │
│  ② 工具执行                                                       │
│     (read / write / edit / bash / ...)                            │
│                                                                   │
│  ③ 工具执行后（成功）                                              │
│     on_post_tool_use() → observe(HookPayload{PostToolUse})        │
│                           ├─ SHA256 去重 → INSERT observations    │
│                           └─ 返回 New(id) → thread::spawn(500ms)  │
│                                              ↓                   │
│  ┌─────────────────────────────────────────────────────────────┐  │
│  │  ④ 后台异步压缩 (std::thread::spawn 上的 block_on)           │  │
│  │                                                             │  │
│  │  compress(obs_id)                                           │  │
│  │   ├─ 1. 打开独立 SQLite 连接，加载 RawObservation            │  │
│  │   ├─ 2. COMPRESSION_SYSTEM + prompt → LLM API                │  │
│  │   ├─ 3. 解析 XML 响应 → CompressedObservation               │  │
│  │   ├─ 4. 写入 compressed_observations 表                       │  │
│  │   ├─ 5. BM25 索引 add()                                      │  │
│  │   └─ 6. OpenAI Embeddings → 向量索引 add()                   │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ⑤ 会话结束                                                       │
│     run_agent_loop_with_tools 返回 → summarize_session()          │
│       ├─ 1. 加载 compressed_observations                           │
│       ├─ 2. SUMMARY_SYSTEM + prompt → LLM API                     │
│       ├─ 3. 解析 XML → SessionSummary                              │
│       └─ 4. 写入 session_summaries 表                              │
│                                                                   │
│  ⑥ 每小时定时器                                                   │
│     run_eviction()                                                │
│       ├─ 淘汰保存度 < 1.0 且超过 90 天的记忆                       │
│       ├─ 淘汰超过 30 天的旧版本                                    │
│       └─ 清理 retention_scores 表中已删除实体的记录                │
│                                                                   │
└──────────────────────────────────────────────────────────────────┘
```

### 1.3 搜索降级策略

```
search(query, workspace_root)
  │
  ├─ embedder 可用 + 在 tokio context 中 ?
  │   ├─ Yes → block_on(search_hybrid())
  │   │           ├─ BM25 索引查询
  │   │           ├─ 向量索引查询（query embedding）
  │   │           └─ RRF 融合 → 查 DB 返回 MemoryEntry
  │   └─ No  → ───────────────────────┘
  │
  ├─ FTS5 可用 ?
  │   ├─ Yes → FTS5 查询 (porter unicode61) → 查 DB
  │   └─ No  → ───────────────────────────┘
  │
  └─ LIKE 搜索（最终降级）
```

---

## 2. 已实现功能清单

### Phase 1 — 核心记忆引擎

| 功能 | 文件 | agentmemory 参考 | 算法/依赖 |
|------|------|-----------------|-----------|
| 自动观察捕获 | `observe.rs` | `src/functions/observe.ts` | PostToolUse + PreToolUse hook, SHA256 去重 |
| LLM 压缩 | `compress.rs` | `src/functions/compress.ts`, `src/prompts/compression.ts` | COMPRESSION_SYSTEM + XML 解析, 自动后台调度 |
| BM25 全文搜索 | `search_index.rs` | `src/state/search-index.ts` | FTS5 porter unicode61 (k1=1.2, b=0.75), + 内存索引用于 RRF |
| Jaccard 去重 | `remember.rs` | `src/state/schema.ts` (jaccardSimilarity), `src/functions/remember.ts` | >0.7 token 集相似度, 版本链 |
| SHA256 去重 | `dedup.rs` | `src/functions/dedup.ts` | blake3, LRU + 5 分钟 TTL |
| 记忆版本管理 | `remember.rs` | `src/functions/remember.ts` | parentId, supersedes, isLatest |
| 记忆 CRUD | `engine.rs` | `src/functions/remember.ts`, `src/functions/forget.ts` | remember / search / list / read / forget |
| 会话摘要 | `sessions.rs` | `src/functions/summarize.ts`, `src/prompts/summary.ts` | SUMMARY_SYSTEM, session 结束时自动触发 |
| 审计日志 | `audit.rs` | `src/functions/audit.ts` | 不可变追加, add/update/delete 自动记录 |

### Phase 2 — 语义搜索与记忆管理

| 功能 | 文件 | agentmemory 参考 | 算法/依赖 |
|------|------|-----------------|-----------|
| 记忆槽 | `slots.rs` | `src/functions/slots.ts` (DEFAULT_SLOTS + 7 个操作) | 8 默认槽, CRUD, system prompt 注入 |
| Embeddings API | `embed.rs` | `src/providers/embedding/index.ts` | OpenAI text-embedding-3-small, 复用 reqwest |
| 向量索引 | `vector_index.rs` | `src/state/vector-index.ts` | 余弦相似度 + BinaryHeap Top-K, 纯内存 |
| RRF 混合搜索 | `hybrid_search.rs` | `src/state/hybrid-search.ts`, `src/functions/smart-search.ts` | BM25 + 向量 RRF (k=60) 融合, 自动降级 FTS5 |
| 保存度评分 | `retention.rs` | `src/functions/retention.ts` | `importance * exp(-0.1 * age) + access_boost` |
| 自动遗忘 | `evict.rs` | `src/functions/evict.ts`, `src/functions/auto-forget.ts` | 每小时定时器, 淘汰 stale + 旧版本 |

---

## 3. 未实现功能清单

### 知识图谱（DV11）

最接近可用的未实现功能。schema 中 `graph_nodes` / `graph_edges` 表已就绪，缺失的是：

- **实体/关系抽取**：在压缩阶段调用 LLM 提取实体和关系（参考 `agentmemory/src/prompts/graph-extraction.ts`）
- **图查询**：BFS 遍历 + 排名
- **图谱统计**：节点/边计数

### Phase 3 功能（按优先级排列）

| 功能 | 文件 | 行数估计 | 复杂度 |
|------|------|---------|--------|
| 整合管线 | `consolidate.rs` | ~200 | 高 |
| 洞察/模式/教训 | `insights.rs` | ~200 | 高 |
| 导入导出 | `export.rs` | ~150 | 低 |
| 评估系统 | `eval/` | ~150 | 中 |

这些功能来自 agentmemory 但 tidev 目前无迫切需求。整合管线需要跨会话的模式检测 LLM 调用，洞察/模式/教训需要多次 LLM 分析和汇总。

---

## 4. 已知简化与妥协

### 4.1 只实现了 OpenAI Embeddings

agentmemory 支持 6 种 provider（Anthropic/Gemini/MiniMax/OpenRouter），tidev 仅实现 OpenAI `text-embedding-3-small`。原因是 tidev 已有 OpenAI key 配置和 reqwest 客户端，其他 provider 需额外配置且短期内无明确使用场景。

**影响**：无 OpenAI key 时向量搜索自动降级为 FTS5。

### 4.2 向量索引未持久化

`VectorIndex` 纯内存，重启后清空。agentmemory 会将向量序列化到 KV 存储。tidev 暂未实现持久化，但启动后首次压缩会自动重新填充（约 30 秒内）。

**影响**：重启后短暂时间内向量搜索不可用。

### 4.3 使用 std::thread::spawn 而非 tokio::spawn

rusqlite 的 bundled SQLite 内部使用 `RefCell` 做 statement cache，导致 `Connection: !Sync`，持有 `&Connection` 的 Future 是 `!Send`。无法在 `tokio::spawn` 中调用 `compress()`。

**方案**：`std::thread::spawn` + `block_on`。阻塞线程池中的空闲线程，不影响主循环。替代方案是改用 `tokio-rusqlite` 或所有 SQLite 操作包在 `spawn_blocking` 里，但 tidev 全项目共用 rusqlite，改动量大。

### 4.4 无隐私过滤

agentmemory 的 `observe.ts` 有 `stripPrivateData()` 过滤密码和 API key。tidev 未实现。敏感信息可能出现在 observation 记录中，但不会进入 system prompt。

### 4.5 审计不完整

只在 `add()` / `update()` / `delete()` 时自动审计。slot 操作和 compress 未审计。

### 4.6 PostToolFailure 未接入

`on_post_tool_failure()` 方法已定义但未接入执行路径。`ToolExecutionResult` 没有 error 字段，失败信息嵌在 `output` 中。可靠检测需要改造 `ToolExecutionResult` 结构体，属跨模块改动。

---

## 5. 数据模型映射

```
agentmemory                     tidev SQLite                Phase
────────────────────────────    ───────────────────────     ─────
mem:sessions                    sessions (已有)               P1
mem:obs:{sessionId}             observations                 P1
obsId + "_compressed"           compressed_observations       P1
mem:summaries                   session_summaries             P1
mem:audit                       audit_log                     P1
mem:memories                    memories (扩展 20 列)         P1
mem:index:bm25                  observations_fts +            P1
                                memories_fts (FTS5)
mem:graph:nodes                 graph_nodes                   P2
mem:graph:edges                 graph_edges                   P2
mem:slots                       memory_slots                  P2
mem:retention                   retention_scores              P2
```

详细列定义见 `src/storage/schema.rs`（当前版本 = 28）。

---

## 6. 关键设计决策

- **不依赖 iii-sdk**：agentmemory 的 WebSocket IPC 在单进程 Rust binary 中是多余开销。所有调用等价替换为：`registerFunction` → Rust trait, `state::get/set` → SQLite, `sdk.trigger` → 直接函数调用。
- **SQLite FTS5 替代自研 BM25**：FTS5 内建 BM25 排名（k1=1.2, b=0.75），零额外依赖。内存 BM25 索引为 RRF 融合保存。
- **单 binary 零外部依赖**：不需要 Docker / iii-engine / 额外数据库进程。
- **blake3 替代 SHA256**：性能更好、库更轻量。哈希仅用于本地去重，无兼容性问题。
- **SQLite TEXT 替代 zstd BLOB**：旧 MemoryStore 对 content 做 zstd 压缩，新版直接存 TEXT，简化查询。
- **无数据库迁移**：schema 从 v26→v27→v28，旧表被新表替换。用户需重建数据库。
