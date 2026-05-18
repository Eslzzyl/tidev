# Memory System Audit

Audit date: 2026-05-18
Scope: `src/memory/` and directly related files

---

## HIGH — 需要尽快修复

### H1. 滞留年龄导致淘汰永不生效

**File:** `src/memory/evict.rs:17-23`

淘汰查询读取 `retention_scores` 表的 `age_days` 列，但这个值是**上次计算时**的快照，不是当前时间。

```sql
-- 当前（有 bug）：读取 retention_scores 中缓存的 age_days
DELETE FROM memories WHERE id IN (
  SELECT m.id FROM memories m
  JOIN retention_scores rs ON rs.entity_id = m.id
  WHERE m.active = 1 AND rs.score < 1.0 AND rs.age_days > 90
  --                       ^^^^^^^^ 缓存的年龄，不是当前时间
);
```

一条 200 天前的记忆如果 60 天前算过分（当时 `age_days = 60`），淘汰条件 `age_days > 90` **永远不满足**，这条记忆永远不会被淘汰。

对比同函数 lines 27-30 的另一条查询，它正确使用了 `julianday('now') - julianday(updated_at)` 实时计算。

**建议修复：** 在淘汰查询中改用 `julianday('now') - julianday(m.created_at)` 而非 `rs.age_days`，或在淘汰前先重算 retention score。

---

### H2. LLM 失败导致游标推进，永久丢失该聚类的洞察

**File:** `src/memory/reflect.rs:148-168`

反射流程按以下顺序处理每个聚类：
1. 从 DB 加载聚类
2. 调 LLM 合成洞察
3. 如果 LLM 成功，写入 insights 到 `memories` 表，推进游标
4. 如果 LLM 失败，`insights` 为空列表

问题在于步骤 4：代码进入保存循环（lines 171-197），0 个 insight 被"保存"。`all_saved` 保持 `true`，事务正常提交，游标**推进**。这个聚类再也不会被处理。

```rust
// lines 171-197: 0 insights → all_saved 保持 true → 游标推进
for insight in &insights {  // 空列表，循环不执行
    ...
    all_saved = true;
}
if all_saved {
    // 游标推进 —— 即使 insights 为空!
    ...
}
```

**建议修复：** 只有在 `insights` 非空时才推进游标。如果 LLM 失败，当前游标不更新，下一个周期重试。

---

### H3. FTS5 转义无效

**File:** `src/memory/search_index.rs:7-21`

FTS5 的反斜杠转义**仅在双引号字符串内有效**。当前代码：

```rust
query = query.replace("*", "\\*");
```

FTS5 在外面遇到 `\*` 时，`\` 无特殊意义，`*` 仍然被解释为前缀操作符。

此外 `-`（NOT）、`+`（AND）、`NEAR`、`( )` 等操作符完全未转义。用户输入例如 `"error -warning"` 会被 FTS5 解释为"匹配含有 warning_NOT_error 的文档"而非字面搜索，这可能是意外的行为。

更糟的是 FTS5 语法错误被静默吞掉：

```rust
// engine.rs:250
let fts_results = fts5_search_memories(&db, query, workspace_root, 20).unwrap_or_default();
```

没有日志记录，FTS5 问题极难调试。

**建议修复：** 将整个查询用双引号包裹（内部的双引号转义为 `""`），或剥离/转义 FTS5 特殊运算符。

---

### H4. Jaccard 空集返回 1.0

**File:** `src/memory/remember.rs:15-16`

```rust
fn jaccard_similarity(a: &str, b: &str) -> f64 {
    let set_a: HashSet<&str> = a.split_whitespace()
        .filter(|w| w.len() >= 3).collect();
    let set_b: HashSet<&str> = b.split_whitespace()
        .filter(|w| w.len() >= 3).collect();

    if set_a.is_empty() && set_b.is_empty() {
        return 1.0;  // ← BUG
    }
    // ...
}
```

两个字符串如 `"a b c"` 和 `"x y z"` 的所有 token 都因 `< 3 字符` 被过滤掉，两个 set 都为空，返回 `1.0`。这两条完全无关的内容会被当作**完全相同**，新的会 supersede 旧的所有记忆。

**触发条件：** 任何只包含短词（≤2 字符）的记忆内容，例如缩写、单字母变量等。

**建议修复：** 当两个 set 都为空时返回 `0.0`（不相似），或者保留原始字符串再做一次比较。

---

## MEDIUM

### M1. 持久化失败导致无限重试同一个聚类

**File:** `src/memory/reflect.rs:199-206`

如果 DB 写入失败（例如约束冲突），事务回滚：

```rust
if all_saved {
    // 推进游标
} else {
    db.execute_batch("ROLLBACK")?;
    // 不推进游标 —— 正确
}
```

但如果没有推进游标，下一个周期会再次处理**同一个**聚类。如果错误是持久的，该聚类之后的所有聚类**永远得不到处理**。

**建议修复：** 添加跳过计数：连续失败 N 次后跳过该聚类并记录错误，避免阻塞后续聚类。

---

### M2. 并发写入可创建重复图节点/边

**File:** `src/memory/graph.rs:40-63, 134-151`

`upsert_node` 和 `upsert_edge` 都采用先查询后插入的模式：

```rust
fn upsert_node(db: &Connection, node_type: &str, label: &str) -> Result<String> {
    // 1. 检查是否存在
    let existing = db.query_row(
        "SELECT id FROM graph_nodes WHERE node_type = ?1 AND label = ?2",
        ...
    ).ok();

    if let Some(id) = existing {
        return Ok(id);
    }

    // 2. 插入（此时另一个线程可能已经插入了同一条）
    let id = Uuid::new_v4().to_string();
    db.execute(
        "INSERT INTO graph_nodes (id, node_type, label) VALUES (?1, ?2, ?3)",
        ...
    )?;
    Ok(id)
}
```

Schema 中没有 UNIQUE 约束（`schema.rs:244-250`）：

```sql
CREATE TABLE IF NOT EXISTS graph_nodes (
    id TEXT PRIMARY KEY,
    node_type TEXT NOT NULL,
    label TEXT NOT NULL
    -- 没有 UNIQUE(node_type, label)
);
```

如果两个线程并发调用 `upsert_node("concept", "React")`，两个线程都查到不存在，都执行 INSERT，创建两个重复节点。

**建议修复：** 在 schema 中添加 `UNIQUE(node_type, label)` 和 `UNIQUE(source_id, target_id, relation)`。

---

### M3. 悬空数据未清理

**File:** `src/memory/evict.rs`

| 表 | 当前 | 问题 |
|---|---|---|
| `graph_nodes` | ❌ 未清理 | 记忆软删除后，从中提取的图节点/边残留在库中 |
| `graph_edges` | ❌ 未清理 | 同上 |
| `session_summaries` | ❌ 未清理 | 注释写着 "Remove stale sessions"（line 12）但代码不存在 |
| `retention_scores` | ✅ 正确清理 | 关联记忆删除时同步删除 |

**建议修复：** 在 eviction 中添加清理步骤，删除关联已软删除记忆的图节点/边，以及无对应 session 的 session_summaries。

---

### M4. `ensure_defaults` 每次 Prompt 注入都执行

**File:** `src/memory/slots.rs:264-289`

`render_pinned`（每次对话都调用）内部调用 `ensure_defaults`，执行 8 条 `INSERT OR IGNORE`：

```rust
pub fn render_pinned(&self, project: &str) -> Result<String> {
    self.ensure_defaults(project);  // ← 每次 prompt 注入都调用
    // ... 只读查询 pinned slots
}
```

`ensure_defaults` 插入 8 个默认 slot，包括 unpinned 的 `session_patterns` 和 `self_notes`（这些之后会被 `render_pinned` 过滤掉）。

首次启动后这些记录已存在，`INSERT OR IGNORE` 只是空操作。但每次对话都执行 8 条 SQL 是冗余的。

**建议修复：** 将 `ensure_defaults` 移到启动时（`run.rs` 或 `start_background_tasks`），或添加内存缓存标记来判断是否已初始化。

---

## LOW

### L1. 每次启动重建 FTS5 索引

**File:** `src/memory/engine.rs:79-91`

```rust
fn rebuild_fts5_if_needed(&self) -> Result<()> {
    // ...
    db.execute("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')", [])?;
    Ok(())
}
```

对外部内容表 FTS5 执行 `'rebuild'` 会重建整个索引。对于 10k+ 条记忆的大存储，每次启动都重建很慢。可以检查一条记录来判断是否首次启动即可。

---

### L2. `update()` 不更新 FTS5

**File:** `src/memory/engine.rs:207-227`

`MemoryStore::update()` 更新 `memories` 表但**不更新** `memories_fts`。虽然外部内容表在查询时会自动同步，但修改后的内容在下一次 `'rebuild'`（重启）之前不会出现在搜索结果中。

**建议修复：** update 后在 FTS5 上执行 `INSERT OR REPLACE` 同步变更。

---

### L3. 后台任务无优雅关闭

**File:** `src/memory/mod.rs:39-57, 62-93, 100-133`

三个后台任务都运行无限 `loop { interval.tick().await; ... }`，没有 cancellation token。应用关闭时，正在执行的 LLM 调用或 SQL 事务可能被中断。

---

### L4. Lesson 衰减过激进

**File:** `src/memory/lessons.rs:133-144`

```rust
pub fn decay_lessons(db: &Connection, project: &str) -> Result<usize> {
    db.execute(
        "UPDATE memories SET strength = MAX(0.0, strength - 0.02)
         WHERE memory_type = 'lesson' AND ...",
        ...
    )?;
}
```

每次调用的衰减量为 0.02。如果后台任务每小时运行一次，一条 strength=1.0 的 lesson 在大约 50 次调用（~2 天）后就衰减到 0.0。结合 `recall_lessons` 的 `min_confidence` 过滤，lesson 实际上有 2 天半衰期。

---

### L5. 图边权重运行平均值有偏

**File:** `src/memory/graph.rs:145`

```rust
let merged = (old_weight + weight) / 2.0;
```

运行平均值意味着后写入的观测权重偏高。例如序列 w1, w2, w3：
- Obs 1: `w1`
- Obs 2: `(w1 + w2) / 2`
- Obs 3: `w1/4 + w2/4 + w3/2`

最近一次观测占 50% 权重。更合理的做法是使用计数加权平均或指数移动平均。

---

## 修复建议优先级

| 优先级 | 问题 | 工作量估计 | 影响范围 |
|--------|------|-----------|----------|
| P0 | **H1 evict age_days** | 1 行 SQL 改动 | 所有记忆永不被淘汰，DB 持续膨胀 |
| P0 | **H2 reflect 游标泄漏** | 加个条件判断 | LLM 失败时洞察永久丢失 |
| P0 | **H4 Jaccard 空集** | 1 行改动 | 短文本记忆被意外删除 |
| P1 | **H3 FTS5 转义** | ~10 行改 escape 逻辑 | 含特殊字符的搜索误匹配或报错 |
| P1 | **M1 reflect 无限重试** | 加跳过计数 | 持久 DB 错误阻塞所有后续聚类 |
| P1 | **M2 graph UNIQUE** | Schema + 逻辑改动 | 并发写入产生重复节点/边 |
| P1 | **M3 悬空数据清理** | evict 加代码 | 图/摘要数据持续增长 |
| P2 | **M4 ensure_defaults** | 移调用位置 | 每次 prompt 多余 8 条 SQL |
| P2 | **L1/L2 FTS5** | ~20 行 | 启动慢 + 搜索结果过时 |
| P3 | **L3-L5** | 各 5-30 行 | 健壮性/可维护性改进 |
