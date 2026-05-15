# TiDev 记忆系统升级方案 & 实现状态

> 基于对 [agentmemory](https://github.com/rohitg00/agentmemory) v0.9.12 的逆向分析，在 tidev 中以 Rust 复刻。
>
> 更新时间：2026-05-15（Phase 1 ✅, Phase 2 ✅, Phase 3 ✅, Phase 4 ✅, Phase 6 ✅, 表合并 ✅, 隐私过滤 ✅）

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
│  ⑤ 每轮对话结束                                                   │
│     run_agent_loop_with_tools 返回 → summarize_session()          │
│       ├─ 1. 加载 compressed_observations                           │
│       ├─ 2. SUMMARY_SYSTEM + prompt → LLM API                     │
│       ├─ 3. 解析 XML → SessionSummary                              │
│       └─ 4. INSERT OR REPLACE session_summaries（每轮覆盖）       │
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

### 4.2 向量索引未持久化 [必须修复]

`VectorIndex` 纯内存，重启后清空。agentmemory 会将向量序列化到 KV 存储。

**影响**：重启后向量索引为空，且**不会重新 embedding 历史数据**（仅新产生的 `CompressedObservation` 会填充索引）。这意味着历史记忆的 embedding 表示永久丢失，向量搜索退化为纯 FTS5 搜索，embedding 基础设施形同虚设。

**此简化不可接受，必须解决。** 改造方案：
1. 在 `compressed_observations` 表中增加 `embedding BLOB` 列，持久化存储 embedding 向量
2. `MemoryStore` 启动时从数据库加载所有 embedding → `VectorIndex`（初始加载 ~2-5 秒）
3. 后续每次压缩完成时同时写入 DB + 内存索引

（agentmemory 的做法是将向量序列化到独立 KV 存储；tidev 直接复用 SQLite BLOB 列更简单。）

### 4.3 使用 std::thread::spawn 而非 tokio::spawn

rusqlite 的 bundled SQLite 内部使用 `RefCell` 做 statement cache，导致 `Connection: !Sync`，持有 `&Connection` 的 Future 是 `!Send`。无法在 `tokio::spawn` 中调用 `compress()`。

**方案**：`std::thread::spawn` + `block_on`。阻塞线程池中的空闲线程，不影响主循环。替代方案是改用 `tokio-rusqlite` 或所有 SQLite 操作包在 `spawn_blocking` 里，但 tidev 全项目共用 rusqlite，改动量大。

### 4.4 无隐私过滤

agentmemory 的 `observe.ts` 有 `stripPrivateData()` 过滤密码和 API key。tidev 未实现。敏感信息可能出现在 observation 记录中，但不会进入 system prompt。

### 4.5 审计不完整

只在 `add()` / `update()` / `delete()` 时自动审计。slot 操作和 compress 未审计。

### 4.6 PostToolFailure 未接入

`on_post_tool_failure()` 方法已定义但未接入执行路径。`ToolExecutionResult` 没有 error 字段，失败信息嵌在 `output` 中。可靠检测需要改造 `ToolExecutionResult` 结构体，属跨模块改动。

### 4.7 缺乏完善的降级措施 [待实现] ✅ Phase2（LLM 压缩降级已修复）

#### 4.7.1 LLM 压缩的降级 ✅ Phase2

已修复（2026-05-15）：`MemoryStore::compress()` 现在自动判断 LLM 可用性，不可用时走合成压缩路径。完整启发式规则已实现（类型推断、路径提取、重要性评分）。

tidev 的现状：

```
tidev:
  LLM client 可用?
    ├─ Yes → LLM 压缩（compress.rs:compress()）
    │    如果 LLM 调用失败 → log_warn + 自动降级到合成压缩
    │
    └─ No  → 合成压缩（compress_synthetic()），全功能可用
```

**剩余问题**：
1. 没有熔断器（连续 N 次失败后暂停 LLM 压缩一段时间）—— 可后续补
2. 没有 `isAutoCompressEnabled()` 配置开关 —— 当前行为等价于始终开启

#### 4.7.2 Embedding 的降级

agentmemory 的 embedding 降级：

```
agentmemory:
  createEmbeddingProvider() → detectEmbeddingProvider()
    ├─ 有 API key → 对应 provider（Gemini / OpenAI / Voyage / Cohere / OpenRouter）
    ├─ 无 API key 但有 @xenova/transformers → LocalEmbeddingProvider
    │    (Xenova/all-MiniLM-L6-v2, 384维, 纯本地)
    └─ 完全无 → null → HybridSearch 仅用 BM25
        搜索降级: hybrid(BM25+向量) → BM25 only → (无进一步降级)
```

tidev 的现状：

```
tidev:
  OpenAI API key 可用?
    ├─ Yes → OpenAIEmbedder（text-embedding-3-small, 1536维）
    └─ No  → embedder = None → search() 直接跳过 hybrid 走 FTS5
             无本地 embedding 选项
```

**问题**：
- 仅支持 OpenAI 一个 provider（已在 §4.1 记录）
- 没有本地 embedding 选项（如 ONNX / llama.cpp 等 Rust 生态方案）
- 但 tidev 的搜索降级链条本身是合理的：hybrid → FTS5 → LIKE

#### 4.7.3 Session 摘要的降级

agentmemory 在 `summarize.ts:75` 检测到 `provider.name === "noop"` 时直接跳过摘要生成。

tidev 的 `summarize_session()` 在 LLM 不可用时：`llm.ok_or_else` → `anyhow::bail!` → 调用者 `log_warn`。不会崩溃但浪费了一次 LLM 检测的时间。

#### 4.7.4 需要实现的降级措施清单

| 降级场景 | agentmemory | tidev | 优先级 |
|----------|-------------|-------|--------|
| 无 LLM API key | 合成压缩（规则驱动） | ✅ 合成压缩（Phase 2） | 已修复 |
| LLM API 调用失败 | 熔断器（3次→30秒） + FallbackChain | ✅ 自动降级到合成压缩 | 已修复 |
| 无 embedding API key | 本地 ONNX 模型（384维） | ⚠ 降级 FTS5（可接受） | 中 |
| embedding API 调用失败 | 忽略（不影响上游写入） | ✅ log_warn，不影响写入 | 中 |
| 摘要时无 LLM | 跳过 | ⚠ 报错但无害 | 低 |

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
- **blake3 而非 SHA256**：agentmemory 用 SHA256 做去重哈希。tidev 改用 blake3 — 性能更好、库更轻量。哈希仅用于本地去重，无兼容性问题。
- **SQLite TEXT 替代 zstd BLOB**：旧 MemoryStore 对 content 做 zstd 压缩，新版直接存 TEXT，简化查询。
- **无数据库迁移**：schema 从 v26→v27→v28，旧表被新表替换。用户需重建数据库。

---

## 7. 已知问题与设计缺陷

通过与 agentmemory 源码的对比分析，tidev 记忆系统存在以下问题。这些问题按照严重程度排列：

### 7.1 压缩观察和会话摘要不被消费 [严重] ✅ Phase1

已修复（2026-05-15）。`compose_system_prompt()` 现在在 `select_hot()` 后注入：
- 当前 session 的压缩观察（importance ≥ 5，限 8 条）
- 其他 session 的会话摘要（限 5 条）

agentmemory 中有一条完整的数据消费链，tidev 之前只实现了写入端，现在写入和注入端已闭环：

```
agentmemory:                            tidev:
─────────────                            ─────
observe → compress                        ✅ 相同
   ↓                                      ↓
写入搜索索引 + 向量索引                    ✅ 相同
   ↓                                      ↓
mem::context → 注入到 LLM system prompt    ✅ 已实现（Phase 1）
   ↓                                      ↓
mem::summarize → session_summaries         ⚠ 每轮触发（见 7.3）
   ↓                                      ↓
mem::context → 注入到 LLM                  ✅ 已实现（Phase 1）
   ↓                                      ↓
consolidation-pipeline → SemanticMemory    ❌ 缺失
```

**遗留问题**：会话摘要仍在每轮对话结束时触发（见 7.3），但至少摘要数据已被消费。

### 7.2 原始观察不应持久保留 [严重] ✅ 表合并

已修复（2026-05-15）。`observations` 表和 `observations_fts` 索引已删除，所有数据合并到单表 `compressed_observations`：
- `observe()` INSERT 一行，含 raw 字段
- `compress()` UPDATE 同一行，填充 compressed 字段，清空 `tool_input`/`tool_output`（NULL）
- 与 agentmemory 的"KV 覆盖"语义完全一致

### 7.3 会话摘要时机错误——每轮触发 [严重]

`src/agent/runtime.rs:1376`：
```rust
run_agent_loop_with_tools() 返回后 → summarize_session()
// 每轮用户消息都调用！
```

agentmemory 只在 `session::stopped` 时调用一次 `mem::summarize`。tidev 每轮都重新生成全量摘要（`INSERT OR REPLACE` 覆盖）。

**问题**：
1. **LLM 调用浪费**：如果一轮对话没有实质变化，仍然触发了一次 LLM 摘要
2. **摘要不完整**：后续轮次生成的摘要覆盖前一轮，但前一轮的摘要永不可见
3. **业务语义错误**：这本质上是"到目前为止的阶段性摘要"，不是"完整 session 的总结"

**根因**：tidev 没有 session 结束的判定机制（见 7.4），所以退而求其次每轮都触发。

### 7.4 缺少 session 结束标记 [严重]

tidev 的 sessions 表没有 `status` 或 `ended_at` 字段：

```sql
-- src/storage/schema.rs
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    parent_session_id TEXT,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- 没有 status: "active" | "completed" | "abandoned"
    -- 没有 ended_at
);
```

agentmemory 有明确的生命周期：

```
HTTP POST /session/start → status: "active"
  ↓
（对话可能持续数小时、数百轮）
  ↓
HTTP POST /session/end   → status: "completed", endedAt: now
  ↓
触发 event::session::stopped → mem::summarize
```

tidev 的 TUI 中，session 从创建后就一直存在，用户可以：
- 在当前 session 中不断发新消息（永久继续）
- 切换到其他 session 再切回来
- 重启程序后继续同一 session

**没有事件可以确定"这个 session 结束了"**。导致：
- `summarize_session()` 无法等到合适的时机调用
- 无法判断哪些 session 是"历史可引用的"（可能还在活跃编辑中）
- eviction 无法区分正常关闭的 session 和废弃的 session

**待设计方案**：
- 是否引入 "fork on new conversation" 模型？每次新对话 fork 出一个子 session
- 是否引入 `session.status` 字段，用户显式 close？
- 是否用超时判定（如 30 分钟无新消息视为结束）？

### 7.5 向量索引未持久化 [严重]（参见 §4.2）

重启后向量索引清空，且**历史数据不会被重新 embedding**。只有新产生的压缩观察会进入索引。这意味着：
- 重启后的前一段时间内向量搜索无结果
- 历史记忆永远丢失了向量表示
- FTS5 虽然可用，但语义搜索能力形同虚设

**方案**：在 `compressed_observations` 表中增加 `embedding BLOB` 列，启动时从 DB 批量加载。

### 7.6 搜索仅支持手动路径 ✅ Phase1（部分）

**Phase 1 已修复自动注入**：`compose_system_prompt()` 现在注入压缩观察（当前 session，重要性筛选）和会话摘要（其他 session）。但 `select_hot()` 仍仅按使用频率排序，不包含语义检索。

agentmemory 有两条搜索路径：

| 路径 | agentmemory | tidev |
|------|-------------|-------|
| 自动注入 | `mem::context` → 随 system prompt 注入 | ✅ 已实现压缩观察 + 会话摘要注入（Phase 1） |
| 手动调用 | MCP tools `memory_recall` / `memory_smart_search` | ✅ LLM 可调用 `memory search` |

tidev 的 `build_system_prompt()` 仍只用 `select_hot()` 取了 5 条按使用频率排序的记忆注入 prompt：

```rust
// src/agent/runtime.rs:209
memory_store.select_hot(&ws, 5, 800)
// SQL: ORDER BY usage_count DESC LIMIT 5
// 条件：LENGTH(content) >= 800
```

这不包含任何语义检索。如果 LLM 不主动调用 `memory search` 工具，它只能看到 5 条高频记忆 + pinned slots。agentmemory 的自动注入能提供跨 session、带重要性筛选的上下文。

### 7.7 缺乏完善的降级措施 [中]（参见 §4.7）✅ Phase2

已修复（2026-05-15）：
- `compress_synthetic()` 已完整实现：从骨架代码升级为完整的启发式规则引擎，含工具名推断、重要性评分、文件路径提取、概念推断
- `MemoryStore::compress()` 添加降级路径：LLM 不可用时自动走合成压缩，LLM 失败时自动降级

对比：

| 场景 | agentmemory | tidev |
|------|-------------|-------|
| 无 LLM API key | 合成压缩（自动降级，全功能可用） | ✅ 合成压缩（主动降级） |
| LLM API 临时故障 | 熔断 + 自动恢复 + 可选 FallbackChain | ✅ 自动降级到合成压缩 |
| 无 Embedding API key | 可装本地 ONNX 模型 | 降级 FTS5（可接受） |
| Embedding 故障 | 忽略错误，不影响写入 | 写入路径上 `embedder.embed()` 失败 → log_warn（可接受） |

详细分析见 §4.7。

### 7.8 `select_hot()` 排序策略过于简单 ✅ Phase3

已修复（2026-05-15）：`select_hot()` 改用复合排序公式：
```
score = importance * 0.5 + min(usage_count / 20, 1) * 0.3 + recency_bonus(7d) * 0.2
```

`select_hot()` 之前仅按 `usage_count` 降序取前 N 条（`src/memory/engine.rs:252`）：

```sql
ORDER BY usage_count DESC LIMIT 5
```

没有考虑：
- **recency**（最近访问时间）
- **importance**（由压缩 LLM 评分，1-10）
- **semantic relevance**（与当前任务的语义相关性）
- **retention score**（虽然 retention.rs 定义了评分公式但从未自动调用）

**影响**：一个项目积累大量记忆后，高频使用的旧记忆会一直占据 hot memory 槽位，新写入的重要记忆可能长时间无法被自动注入。

### 7.8 Retention scoring 未自动接入 ✅ Phase6

已修复（2026-05-15）：
- `remember()` 写入新记忆时自动计算 retention score
- `record_usage()` 更新使用计数时自动重新计算 retention score

仍然存在问题：

### 7.9 无自动跨 session 记忆整合（整合管线）

agentmemory 的 `mem::consolidate-pipeline` 在累积 ≥5 个 session summaries 后调用 LLM 提取跨 session 的知识事实（SemanticMemory）和流程模式（ProceduralMemory）。tidev 完全没有此功能。

### 7.10 PostToolFailure 未接入（已有 §4.6） ✅ Phase6

已修复（2026-05-15）：`execute_tool_calls()` 在工具返回错误时调用 `on_post_tool_failure()`。
- 检测条件：`sandbox_denied` 为 true，或 output 以 `"Error:"` / `"Tool task panicked"` 开头
- 记录为 `HookType::PostToolFailure` 观察，供记忆系统学习

### 7.11 审计不完整（已有 §4.5）

只在 `add()` / `update()` / `delete()` / `remember()` 时自动审计。slot 操作和 compress 未审计。

### 7.12 无隐私过滤（已有 §4.4） ✅ 隐私过滤

已修复（2026-05-15）。`build_compression_prompt()` 新增 `strip_sensitive()` 函数，在 LLM 调用前过滤：
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
  ├─ select_hot(5)  ← ORDER BY usage_count DESC，不含语义
  ├─ load_recent_compressed_observations(importance≥5, limit=8) ✅ Phase1
  ├─ load_other_session_summaries(limit=5) ✅ Phase1
  └─ render_pinned_slots()

每次工具调用:
  observe → INSERT INTO observations
     ↓（异步）
  compress → INSERT INTO compressed_observations  // raw 保留！
     ↓
  bm25Index.add()
  vectorIndex.add(embed())

每轮对话结束:
  summarize_session() → INSERT OR REPLACE session_summaries
  // 每次覆盖，LLM 调用浪费
  // ⚠ Phase1 修复了读回，但时机问题仍待解决

手动（可选）:
  LLM 调用 memory search 或 memory remember 工具
  ❌ 无整合管线
  ❌ retention scoring 定义了但从未自动调用
```
