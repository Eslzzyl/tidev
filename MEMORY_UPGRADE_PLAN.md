# TiDev 记忆系统升级方案 & 实现状态

> 基于对 [agentmemory](https://github.com/rohitg00/agentmemory) v0.9.12 的逆向分析，在 tidev 中以 Rust 复刻。
>
> 更新时间：2026-05-15（Phase 1 ✅, Phase 2 ✅, Phase 3 ✅, Phase 4 ✅, Phase 6 ✅, Phase 7 ✅, 表合并 ✅, 隐私过滤 ✅, Session 巡检 ✅, 整合管线 ✅）

---

## 目录

1. [核心架构与数据流](#1-核心架构与数据流)
2. [已实现功能清单](#2-已实现功能清单)
3. [未实现功能清单](#3-未实现功能清单)
4. [已知简化与妥协](#4-已知简化与妥协)
5. [数据模型映射](#5-数据模型映射)
6. [关键设计决策](#6-关键设计决策)
7. [已知问题与设计缺陷](#7-已知问题与设计缺陷)

---

## 1. 核心架构与数据流

### 1.1 模块结构

```
src/memory/
├── mod.rs            — 模块声明 + 公共导出
├── types.rs          — MemoryEntry / HookType / ObservationType / MemorySlot / ...
├── engine.rs         — MemoryStore 主入口（SQLite 持久化，~590 行）
├── observe.rs        — 自动观察捕获（blake3 去重 + 入库）
├── compress.rs       — LLM 压缩（COMPRESSION_SYSTEM + XML 解析）
├── search_index.rs   — BM25 内存索引 + FTS5 查询封装
├── dedup.rs          — blake3 去重映射（LRU + 5 分钟 TTL）
├── remember.rs       — Jaccard 去重 + 版本链管理
├── sessions.rs       — LLM 会话摘要
├── slots.rs          — 记忆槽 CRUD + 8 默认槽 + 提示词渲染
├── consolidate.rs    — 跨 session 整合管线（语义事实 + 可复用流程）
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
│                          └─ blake3 去重 → INSERT observations     │
│                                                                   │
│  ② 工具执行                                                       │
│     (read / write / edit / bash / ...)                            │
│                                                                   │
│  ③ 工具执行后（成功）                                              │
│     on_post_tool_use() → observe(HookPayload{PostToolUse})        │
│                           ├─ blake3 去重 → INSERT observations    │
│                           └─ 返回 New(id) → thread::spawn(500ms)  │
│                                              ↓                   │
│  ┌─────────────────────────────────────────────────────────────┐  │
│  │  ④ 后台异步压缩 (std::thread::spawn 上的 block_on)           │  │
│  │                                                             │  │
│  │  compress(obs_id)                                           │  │
│  │   ├─ 1. 打开独立 SQLite 连接，加载 RawObservation            │  │
│  │   ├─ 2. COMPRESSION_SYSTEM + prompt → LLM API                │  │
│  │   ├─ 3. 解析 XML 响应 → CompressedObservation               │  │
│  │   ├─ 4. 写入 compressed_observations 表（原始观察保留！）    │  │
│  │   ├─ 5. BM25 索引 add()                                      │  │
│  │   └─ 6. OpenAI Embeddings → 向量索引 add()                   │  │
│  └─────────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ⑤ 后台巡检（每 60 秒）                                            │
│     inactivity_check task → find_inactive_sessions()               │
│       ├─ 查找 parent_session_id IS NULL（用户 session）             │
│       ├─ 过滤 status = 'active' 且 updated_at < now - 300s        │
│       ├─ 排除前台活跃 session                                       │
│       ├─ 过滤有 compressed_observations 的 session                  │
│       ├─ 标记 status = 'completed', ended_at = now                 │
│       └─ 调用 summarize_session() → INSERT OR REPLACE              │
│                                                                   │
│  ⚠ 缺失：无整合管线（SemanticMemory / ProceduralMemory）          │
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
| 自动观察捕获 | `observe.rs` | `src/functions/observe.ts` | PostToolUse + PreToolUse hook, blake3 去重 |
| LLM 压缩 | `compress.rs` | `src/functions/compress.ts`, `src/prompts/compression.ts` | COMPRESSION_SYSTEM + XML 解析, 自动后台调度 |
| 合成压缩降级 ✅ Phase2 | `compress.rs` | `src/functions/compress.ts` (synthetic) | 启发式规则（工具名推断+路径提取+重要性评分）, LLM 不可用/失败时自动降级 |
| BM25 全文搜索 | `search_index.rs` | `src/state/search-index.ts` | FTS5 porter unicode61 (k1=1.2, b=0.75), + 内存索引用于 RRF |
| Jaccard 去重 | `remember.rs` | `src/state/schema.ts` (jaccardSimilarity), `src/functions/remember.ts` | >0.7 token 集相似度, 版本链 |
| blake3 去重 | `dedup.rs` | `src/functions/dedup.ts` | blake3, LRU + 5 分钟 TTL |
| 记忆版本管理 | `remember.rs` | `src/functions/remember.ts` | parentId, supersedes, isLatest |
| 记忆 CRUD | `engine.rs` | `src/functions/remember.ts`, `src/functions/forget.ts` | remember / search / list / read / forget |
| 会话摘要 | `sessions.rs` | `src/functions/summarize.ts`, `src/prompts/summary.ts` | SUMMARY_SYSTEM, session 结束时自动触发 |
| 审计日志 | `audit.rs` | `src/functions/audit.ts` | 不可变追加, add/update/delete 自动记录 |
| 压缩观察注入 LLM ✅ Phase1 | `engine.rs`, `runtime.rs` | `src/functions/context.ts` | `compose_system_prompt()` 中读取 compressed_observations 注入 |
| 会话摘要注入 LLM ✅ Phase1 | `engine.rs`, `runtime.rs` | `src/functions/context.ts` | `compose_system_prompt()` 中读取 session_summaries 注入 |

### Phase 2 — 语义搜索与记忆管理

| 功能 | 文件 | agentmemory 参考 | 算法/依赖 |
|------|------|-----------------|-----------|
| 记忆槽 | `slots.rs` | `src/functions/slots.ts` (DEFAULT_SLOTS + 7 个操作) | 8 默认槽, CRUD, system prompt 注入 |
| Embeddings API | `embed.rs` | `src/providers/embedding/index.ts` | OpenAI text-embedding-3-small, 复用 reqwest |
| 向量索引 | `vector_index.rs` | `src/state/vector-index.ts` | 余弦相似度 + BinaryHeap Top-K, 纯内存 |
| RRF 混合搜索 | `hybrid_search.rs` | `src/state/hybrid-search.ts`, `src/functions/smart-search.ts` | BM25 + 向量 RRF (k=60) 融合, 自动降级 FTS5 |
| 保存度评分 | `retention.rs` | `src/functions/retention.ts` | `importance * exp(-0.1 * age) + access_boost` |
| 自动遗忘 | `evict.rs` | `src/functions/evict.ts`, `src/functions/auto-forget.ts` | 每小时定时器, 淘汰 stale + 旧版本 |
| select_hot 复合排序 ✅ Phase3 | `engine.rs` | — | `importance*0.5 + usage_count*0.3 + recency_bonus*0.2` |
| Retention 自动集成 ✅ Phase6 | `engine.rs` | `src/functions/retention.ts` | `remember()` + `record_usage()` 中自动计算 |
| PostToolFailure 接入 ✅ Phase6 | `runtime.rs` | `src/functions/observe.ts` | 检测 `sandbox_denied` / `Error:` 前缀 |

### Phase 7 — Session 生命周期管理 + 整合管线

| 功能 | 文件 | 说明 |
|------|------|------|
| session status 字段 | `schema.rs` | sessions 表增加 `status` + `ended_at` |
| 后台不活跃巡检 | `run.rs` | 每 60s 检查 inactive session 并 summarize |
| 退出零阻塞 | `run.rs` | 退出时只 cancel 任务，不调用 LLM |
| web fork 修正 | `sessions.rs` | web fork 改用 `create_session()`（无 parent） |
| 整合管线 | `consolidate.rs` | 从 summaries 提取跨 session 事实，从 patterns 提取可复用流程 |
| 后台整合调度 | `run.rs` | 每 30 分钟运行一次整合管线 |
| Facts + Procedures 注入 | `runtime.rs` | `compose_system_prompt()` 中注入 consolidated knowledge |

---

## 3. 未实现功能清单

### 知识图谱（DV11）

最接近可用的未实现功能。schema 中 `graph_nodes` / `graph_edges` 表已就绪，缺失的是：

- **实体/关系抽取**：在压缩阶段调用 LLM 提取实体和关系（参考 `agentmemory/src/prompts/graph-extraction.ts`）
- **图查询**：BFS 遍历 + 排名
- **图谱统计**：节点/边计数

### 未实现功能

| 功能 | 复杂度 | 说明 |
|------|--------|------|
| 知识图谱（DV11） | 高 | schema 中 `graph_nodes` / `graph_edges` 表已就绪，缺失实体抽取、图查询、图统计 |
| 洞察/模式/教训反射 | 高 | 从概念聚类中合成 insight（参考 agentmemory 的 `mem::reflect`），依赖整合管线但尚未实现 |
| 导入导出 | 低 | `storage/mod.rs` 已有 session 级 SQLite export/import，但记忆系统级别的导出文件 `export.rs` 不存在 |
| 评估系统 | 中 | agentmemory 的自我评估功能，对单进程工具来说需求不高 |

已实现：
- 整合管线 ✅（`consolidate.rs`，2026-05-15）

---

## 4. 已知简化与妥协

### 4.1 只实现了 OpenAI Embeddings

agentmemory 支持 6 种 provider（Anthropic/Gemini/MiniMax/OpenRouter），tidev 仅实现 OpenAI `text-embedding-3-small`。原因是 tidev 已有 OpenAI key 配置和 reqwest 客户端，其他 provider 需额外配置且短期内无明确使用场景。

**影响**：无 OpenAI key 时向量搜索自动降级为 FTS5。

### 4.2 向量索引持久化 ✅ Phase 4

`compressed_observations` 表包含 `embedding BLOB` 列，持久化存储 OpenAI embedding 向量：

- **启动时**：`MemoryStore::load_embeddings_from_db()` 从 DB 批量加载所有 embedding → 内存 `VectorIndex`
- **写入时**：每次压缩完成，embedding 同时写入 DB + 索引
- **无丢失风险**：重启后向量索引从 DB 重建，历史数据仍然可被语义搜索

agentmemory 的做法是将向量序列化到独立 KV 存储；tidev 直接复用 SQLite BLOB 列更简单。

### 4.3 使用 std::thread::spawn 而非 tokio::spawn

rusqlite 的 bundled SQLite 内部使用 `RefCell` 做 statement cache，导致 `Connection: !Sync`，持有 `&Connection` 的 Future 是 `!Send`。无法在 `tokio::spawn` 中调用 `compress()`。

**方案**：`std::thread::spawn` + `block_on`。阻塞线程池中的空闲线程，不影响主循环。替代方案是改用 `tokio-rusqlite` 或所有 SQLite 操作包在 `spawn_blocking` 里，但 tidev 全项目共用 rusqlite，改动量大。

### 4.4 无隐私过滤

agentmemory 的 `observe.ts` 有 `stripPrivateData()` 过滤密码和 API key。tidev 未实现。敏感信息可能出现在 observation 记录中，但不会进入 system prompt。

### 4.5 审计日志已移除

审计模块（`AuditService`）在 agentmemory 中用于 WebSocket IPC 调试，tidev 为单进程 binary 无此需求。2026-05-15 已移除：
- `AuditEntry` 类型、`AuditService`、`audit_query()`、`audit_log` 表全部删除
- 共移除约 130 行代码 + 1 张 DB 表 + 2 个索引

### 4.6 PostToolFailure 未接入

`on_post_tool_failure()` 方法已定义但未接入执行路径。`ToolExecutionResult` 没有 error 字段，失败信息嵌在 `output` 中。可靠检测需要改造 `ToolExecutionResult` 结构体，属跨模块改动。

### 4.7 降级措施 ✅ Phase2

#### 4.7.1 LLM 压缩的降级 ✅ Phase2

`MemoryStore::compress()` 自动判断 LLM 可用性：

```
LLM client 可用?
  ├─ Yes → LLM 压缩（compress.rs:compress()）
  │    如果 LLM 调用失败 → log_warn + 自动降级到合成压缩
  └─ No  → 合成压缩（compress_synthetic()），全功能可用
```

合成压缩（`compress_synthetic()`）是完整的启发式规则引擎：工具名推断、路径提取、重要性评分。

**剩余问题**：
1. 没有熔断器（连续 N 次失败后暂停 LLM 压缩一段时间）
2. 没有 `isAutoCompressEnabled()` 配置开关

#### 4.7.2 Embedding 的降级

```
OpenAI API key 可用?
  ├─ Yes → OpenAIEmbedder（text-embedding-3-small, 1536维）
  └─ No  → embedder = None → search() 直接跳过 hybrid 走 FTS5
           无本地 embedding 选项
```

搜索降级链条：hybrid(BM25+向量) → FTS5 → LIKE

**问题**：
- 仅支持 OpenAI 一个 provider
- 没有本地 embedding 选项（如 ONNX / llama.cpp 等 Rust 生态方案）

#### 4.7.3 Session 摘要的降级

`summarize_session()` 在 LLM 不可用时：`llm.ok_or_else` → `anyhow::bail!` → 调用者 `log_warn`。
不会崩溃但浪费了一次 LLM 检测的时间。

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

详细列定义见 `src/storage/schema.rs`（当前版本 = 29）。

sessions 表新增列：
- `status TEXT DEFAULT 'active'` — `'active'` / `'completed'`
- `ended_at TEXT` — session 结束时间戳

---

## 6. 关键设计决策

- **不依赖 iii-sdk**：agentmemory 的 WebSocket IPC 在单进程 Rust binary 中是多余开销。所有调用等价替换为：`registerFunction` → Rust trait, `state::get/set` → SQLite, `sdk.trigger` → 直接函数调用。
- **SQLite FTS5 替代自研 BM25**：FTS5 内建 BM25 排名（k1=1.2, b=0.75），零额外依赖。内存 BM25 索引为 RRF 融合保存。
- **单 binary 零外部依赖**：不需要 Docker / iii-engine / 额外数据库进程。
- **blake3 而非 SHA256**：agentmemory 用 SHA256 做去重哈希。tidev 改用 blake3 — 性能更好、库更轻量。哈希仅用于本地去重，无兼容性问题。
- **SQLite TEXT 替代 zstd BLOB**：旧 MemoryStore 对 content 做 zstd 压缩，新版直接存 TEXT，简化查询。
- **无数据库迁移**：schema 从 v26→v27→v28→v29，旧表被新表替换。用户需重建数据库。
- **后台巡检替代 agentmemory 的 session::stopped event**：agentmemory 依赖客户端显式调用 session 结束事件，tidev 作为单进程 TUI 没有 HTTP 端点，改用后台定期巡检 + 不活跃超时（5 分钟）判定 session 结束。退出不阻塞 LLM 调用。
- **`parent_session_id` 仅用于 subagent**：web fork 已修正为使用 `create_session()`（无 parent），与 TUI fork 一致。`parent_session_id IS NOT NULL` 等价于 subagent session，在巡检中自然排除。

---

## 7. 已知问题与设计缺陷

通过与 agentmemory 源码的对比分析，tidev 记忆系统存在以下问题。这些问题按照严重程度排列：

### 7.1 压缩观察和会话摘要已注入 LLM System Prompt ✅ Phase1

`compose_system_prompt()` 在 `select_hot()` 后注入：
- 当前 session 的压缩观察（importance ≥ 5，限 8 条）
- 其他 session 的会话摘要（限 5 条）

数据消费链完整闭环：

```
observe → compress → 写入搜索索引 + 向量索引
  ↓
mem::context → 注入到 LLM system prompt
  ↓
后台巡检 → summarize_session → session_summaries
  ↓
mem::context → 注入到 LLM
```

### 7.2 原始观察不持久保留——与 compressed_observations 表合并 ✅ 表合并

`observations` 表和 `observations_fts` 索引已删除，所有数据合并到单表 `compressed_observations`：
- `observe()` INSERT 一行，含 raw 字段
- `compress()` UPDATE 同一行，填充 compressed 字段，清空 `tool_input`/`tool_output`（NULL）
- 与 agentmemory 的"KV 覆盖"语义完全一致

### 7.3 会话摘要时机——后台巡检触发 ✅ Phase7

**已修复（2026-05-15）**。移除了 `run_agent_loop_with_tools()` 返回后的 `summarize_session()` 调用（`src/agent/runtime.rs:1451`）。

新的触发机制：**后台巡检任务**（`src/tui/core/run.rs`）每 60 秒运行一次，查找满足以下条件的 session：
- `parent_session_id IS NULL`（用户 session，不含 subagent）
- `status = 'active'`
- `updated_at < now - 300s`（超过 5 分钟无活动）
- `id !=` 当前前台 session
- 有 compressed_observations（确实有内容）

找到后逐个标记 `status = 'completed'` + `ended_at = now`，然后调用 `summarize_session()`。

**与 agentmemory 的对比**：

| 方面 | agentmemory | tidev |
|------|-------------|-------|
| 触发时机 | 显式 `session::stopped` event | 后台巡检（不活跃超时） |
| 调用次数 | 整个 session 生命周期一次 | 每次进入后台 + 超时后一次 |
| 退出处理 | client 先发 end 再断开 | 不阻塞，下次启动巡检自动捡起 |
| 子 session | 独立管理 | `parent_session_id IS NULL` 自然排除 |

```rust
// src/tui/core/run.rs — 后台巡检任务
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let ids = store.find_inactive_sessions(&cutoff, current)?;
                for id in ids {
                    store.set_session_status(id, "completed")?;
                    mem_store.summarize_session(id, &ws).await?;
                }
            }
            _ = cancel_token.cancelled() => break,
        }
    }
});
```

### 7.4 缺少 session 结束标记 ✅ Phase7

**已修复（2026-05-15）**。sessions 表增加了 `status` 和 `ended_at` 字段：

```sql
-- src/storage/schema.rs (SCHEMA_VERSION = 29)
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT REFERENCES sessions(id) ON DELETE CASCADE,
    ...
    status TEXT NOT NULL DEFAULT 'active',
    ended_at TEXT,
    ...
);
```

**判定策略**：不使用 agentmemory 的显式 `session::stopped` event 模型。改为**后台不活跃巡检**：

```
用户切换到 session B
  → session A 进入后台
  → background_check 每 60s 运行
  → 发现 A 的 updated_at 超过 300s 未更新
  → 且 A 不是当前前台 session
  → 标记 A.status = 'completed' + ended_at = now
  → summarize_session(A)

用户切回 A（超时前）：
  → A 仍然是 'active'
  → 巡检排除当前 session，不会误触发

用户切回 A（超时后）：
  → A 已是 'completed'
  → 可以继续发消息（与当前 TUI UX 一致）
  → 再次离开时会重新触发 summarize
```

**边界情况处理**：

| 场景 | 行为 |
|------|------|
| 快速切换 A→B→A | 巡检跳过当前 session，A 不会被总结 |
| 多个 session 陆续进入后台 | 每个独立检查，互不干扰 |
| 应用退出 | cancel 巡检任务，不阻塞；重启后巡检捡起 |
| 应用崩溃 | 下次启动后台捡起未总结的 session |
| Subagent 子 session | `parent_session_id IS NOT NULL` 天然排除 |
| 用户 fork session | 与 TUI 一致，用 `create_session()` 无 parent |

### 7.5 向量索引持久化 [已修复] ✅ Phase 4（参见 §4.2）

重启后向量索引从 DB 的 `embedding BLOB` 列重建，历史 embedding 不会丢失。
- 启动时 `load_embeddings_from_db()` 批量加载 ~2-5 秒
- 每次压缩完成时 embedding 同时写入 DB + 内存索引
- 语义搜索在重启后立即可用

### 7.6 自动注入不含语义检索

`select_hot()` 使用复合排序公式：
```
score = importance * 0.5 + min(usage_count / 20, 1) * 0.3 + recency_bonus(7d) * 0.2
```

```rust
// src/memory/engine.rs:315-327
ORDER BY
    importance * 0.5 +
    LEAST(usage_count / 20.0, 1.0) * 0.3 +
    CASE WHEN updated_at >= datetime('now', '-7 days') THEN 0.2 ELSE 0.0 END
DESC
```

但仍不包含语义检索。自动注入仅包含：
- 5 条按复合分数排序的高频/重要/近期记忆
- 8 条当前 session 的压缩观察（Phase 1）
- 5 条其他 session 的摘要（Phase 1）
- 5 条 consolidated facts（整合管线）
- 3 条 reusable procedures（整合管线）
- pinned slots

agentmemory 的 `mem::context` 在注入时做了语义搜索，tidev 暂时没有。

### 7.7 降级措施完善 ✅ Phase2

LLM 压缩不可用时自动走合成压缩：
- 无 LLM API key → 合成压缩（规则驱动，全功能可用）
- LLM API 临时故障 → 自动降级到合成压缩，log_warn

| 场景 | 行为 |
|------|------|
| 无 LLM API key | 合成压缩（主动降级） |
| LLM API 临时故障 | 自动降级到合成压缩 |
| 无 Embedding API key | 降级 FTS5（可接受） |
| Embedding 故障 | log_warn，不影响写入 |

### 7.8 `select_hot()` 复合排序 + Retention 自动集成 ✅ Phase3 + Phase6

`select_hot()` 使用复合排序公式（Phase 3）：
```
score = importance * 0.5 + min(usage_count / 20, 1) * 0.3 + recency_bonus(7d) * 0.2
```

Retention scoring 在以下时机自动计算（Phase 6）：
- `remember()` 写入新记忆时
- `record_usage()` 更新使用计数时

### 7.9 整合管线 ✅ Consolidation

`ConsolidationService`（`src/memory/consolidate.rs`）每 30 分钟后台运行：

- **Tier 1 语义整合**：从 ≥5 个 session summaries 调用 LLM 提取跨 session 事实，存入 `memories` 表（`memory_type = fact`, `tags` 含 `consolidated`）
- **Tier 2 流程提取**：从 ≥3 个 pattern/workflow 记忆中提取可复用流程，存入 `memories` 表（`memory_type = pattern`, `tags` 含 `procedure`）
- **Cursor 机制**：通过 `meta` 表记录已处理的最后 summary/memory ID，避免重复提取

注入到 `compose_system_prompt()`：
- `## Consolidated Project Knowledge` — facts（limit 5）
- `## Reusable Procedures` — procedures（limit 3）

agentmemory 还有 `reflect`（洞察合成）未实现。

### 7.10 PostToolFailure 接入 ✅ Phase6

`execute_tool_calls()` 在工具返回错误时调用 `on_post_tool_failure()`：
- 检测条件：`sandbox_denied` 为 true，或 output 以 `"Error:"` / `"Tool task panicked"` 开头
- 记录为 `HookType::PostToolFailure` 观察，供记忆系统学习

### 7.11 审计日志已移除（§4.5）

审计模块已于 2026-05-15 移除。所有 `AuditService::record()` 调用使用 `let _ = ` 静默忽略错误，且 `audit_query()` 无任何调用者。`audit_log` 表甚至不在主 `SCHEMA_SQL` 中，从未被正确创建过。移除后减少 ~130 行死代码和 1 张 DB 表。

### 7.12 隐私过滤 ✅ 隐私过滤

`build_compression_prompt()` 在 LLM 调用前过滤敏感信息：
- OpenAI / Anthropic API keys
- GitHub tokens
- Bearer tokens / Authorization headers
- AWS access keys
- SSH private key blocks
- 通用 `password`/`secret`/`api_key`/`token` 模式

---

## 8. 与 agentmemory 的数据流对比（完整版）

```
agentmemory 数据流（完整）:
==========================

session::started
  └─ mem::context
       ├─ 读取其他 session 的 summaries（如果有）
       ├─ 读取其他 session 的 compressed_observations（importance≥5）
       ├─ 读取 pinned slots
       └─ 返回 <agentmemory-context>...</agentmemory-context>
            → 注入到 LLM system prompt

每次工具调用:
  observe → kv.set(raw)
     ↓（同步或异步）
  compress → kv.set(compressed)  // 覆盖 raw！
     ↓
  bm25Index.add(compressed)
  vectorIndex.add(embed(compressed))

session::stopped（客户端显式调用）:
  └─ mem::summarize
       ├─ 读取所有 compressed_observations
       └─ kv.set(session_summary)  // 仅一次

手动（可选）:
  mem::consolidate-pipeline
  ├─ 读取 ≥5 个 session_summaries → LLM → SemanticMemory
  └─ 读取 pattern memories → LLM → ProceduralMemory
      ↓
  mem::context 也可以读取 SemanticMemory（通过 ProjectProfile）


tidev 数据流（当前）:
=====================

compose_system_prompt（每轮）:
  ├─ select_hot(5)  ← 复合排序（重要+高频+近期）
  ├─ load_recent_compressed_observations(importance≥5, limit=8)
  ├─ load_other_session_summaries(limit=5)
  ├─ load_consolidated_facts(limit=5) ✅ Consolidation
  ├─ load_consolidated_procedures(limit=3) ✅ Consolidation
  └─ render_pinned_slots()

每次工具调用:
  observe → INSERT INTO compressed_observations (raw)
     ↓（异步，std::thread::spawn）
  compress → UPDATE compressed_observations (compressed fields)
     ↓
  bm25Index.add() + vectorIndex.add(embed())

后台巡检（每 60s）✅ Phase7:
  find_inactive_sessions()
  ├─ 跳过 parent_session_id IS NOT NULL（subagent）
  ├─ 跳过 status != 'active'
  ├─ 跳过 updated_at 在超时时间内
  ├─ 跳过当前前台 session
  └─ 逐个: set_status('completed') + summarize_session()

整合管线（每 1800s）✅ Consolidation:
  Tier 1: ≥5 summaries → LLM → <fact> → memories(type=fact)
  Tier 2: ≥3 patterns  → LLM → <procedure> → memories(type=pattern)

手动（可选）:
  LLM 调用 memory search 或 memory remember 工具
```
