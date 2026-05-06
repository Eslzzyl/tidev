# 存储层优化方案

## 背景

tidev 当前使用单文件 SQLite (`~/.local/share/tidev/sessions.sqlite3`) 存储所有数据。随着使用时间增长，数据库体积膨胀，session 切换出现可感知的延迟。

## 数据画像

基于当前 143MB 的数据库进行的实际测量。

### 各表空间分布

| 表 | 行数 | 空间 | 占比 |
|---|---|---|---|
| **messages** | 31,287 | **91.8 MB** | **64%** |
| **tool_events** | 12,457 | **45.1 MB** | **32%** |
| sessions 等 | ~6,500 | ~6 MB | 4% |

### messages 表列级分解 (91.8 MB)

| 列 | 空间 | 占总 DB 比例 | 性质 |
|---|---|---|---|
| **content** | 47.5 MB | 33% | 纯文本对话 |
| **reasoning** | 9.0 MB | 6% | 推理模型思维链 |
| tool_calls | 6.3 MB | 4% | JSON |
| metadata | 4.4 MB | 3% | JSON |
| file_diffs | 3.0 MB | 2% | JSON |
| attachments | 1.3 MB | <1% | JSON |
| 其他列 + SQLite 元数据 | ~20 MB | 14% | — |

### tool_events 表列级分解 (45.1 MB)

| 列 | 空间 | 性质 |
|---|---|---|
| **output_text** | 32.0 MB | 纯文本工具执行输出 |
| input_json | 4.2 MB | JSON |
| 其他 + 元数据 | ~9 MB | — |

### Session 特征

- 总计 388 个 session，377 个非空
- 平均每个 session 83 条消息
- 最长 session: 866 条消息，535KB content 文本
- 40% 的消息带有 tool_calls
- 39% 的消息带有 reasoning（推理模型使用频繁）
- tool_events.output_text 与 messages.content 存在数据重复

---

## 加载延迟根因分析

### 存储层耗时测量

对最长 session (866 条消息) 的实际基准测试：

| 阶段 | 耗时 | 说明 |
|---|---|---|
| SQLite 全表扫描 (25 列) | **~12 ms** | 使用 `idx_messages_session_created_at` 索引 |
| 3x JSON 反序列化 | **~3 ms** | attachments + tool_calls + metadata |
| UUID + DateTime 解析 | **~1 ms** | |
| 字符串分配 | **< 0.1 ms** | |
| **总计** | **~16 ms** | |

**结论：纯存储层不是瓶颈。** 866 条消息的全量读取 + 解析在 16ms 内完成。

### 延迟的真实来源（推测）

1. **Mutex 写写竞争** — `SessionStore` 为 `Arc<Mutex<SessionStore>>`，所有读写串行化。当 streaming 响应正在写入消息时，`load_conversation()` 需要等待锁释放。
2. **WAL 文件膨胀** — 首次查询需要扫描 WAL（上次观察到 WAL 达 4MB+）。
3. **加载后处理** — 系统提示组合、token 计数、tool 注册等环节的开销。
4. **UI 渲染** — 大量消息的 TUI 渲染本身需要时间。

---

## 推荐方案

### P0 — 立即执行

#### 方案 A：SQLite Pragma 调优

**当前配置（在 `SessionStore::open()` 中）：**

```rust
connection.pragma_update(None, "foreign_keys", "ON")?;
connection.pragma_update(None, "journal_mode", "WAL")?;
connection.busy_timeout(Duration::from_secs(5))?;
```

**缺失的关键配置：**

| Pragma | 当前值 | 推荐值 | 作用 |
|---|---|---|---|
| `mmap_size` | 0 (默认) | 268435456 (256MB) | SQLite 使用内存映射文件，OS 自动缓存热数据 |
| `cache_size` | 2000 pages (8MB) | -64000 (64MB) | 大 session 不需要频繁换页 |
| `synchronous` | FULL (2) | NORMAL (1) | WAL 模式下等价安全，写入延迟降低 10x |
| `temp_store` | FILE (0) | MEMORY (2) | 临时表和排序在内存中进行 |

**改动位置：** `src/storage/mod.rs` → `SessionStore::open()`

```rust
connection.pragma_update(None, "mmap_size", "268435456")?;    // 256MB
connection.pragma_update(None, "cache_size", "-64000")?;       // 64MB
connection.pragma_update(None, "synchronous", "NORMAL")?;      // WAL-safe
connection.pragma_update(None, "temp_store", "MEMORY")?;       // 内存临时表
```

**收益预估：**
- 冷启动 / 冷缓存场景的 session 加载延迟降低 30-50%
- 页面换出后再加载的延迟大幅降低
- 写入操作延迟降低（`synchronous = NORMAL`）
- 零风险，零 schema 改动

**工作量：** 1-2 天（改动 + 测试 + 验证）

---

### P1 — 短期

#### 方案 B：应用层 zstd 压缩

**目标列：**

| 表 | 列 | 当前大小 | 预估压缩后 | 节省 |
|---|---|---|---|---|
| messages | content | 47.5 MB | ~12 MB | 35.5 MB |
| messages | reasoning | 9.0 MB | ~2 MB | 7 MB |
| tool_events | output_text | 32.0 MB | ~8 MB | 24 MB |
| **合计** | | **88.5 MB** | **~22 MB** | **~66 MB** |

zstd level 3 对英文文本的典型压缩比：3x-5x。
解压速度 ~500 MB/s，解压一个 session 的全部 content + reasoning（~500KB）只需 ~1ms。
LICENSE：BSD-2-Clause / Apache-2.0 OR MIT，与 tidev 的 Apache-2.0 兼容。

**实现方式：**

1. 依赖添加（`Cargo.toml`）：
   ```toml
   zstd = { version = "0.13", default-features = false }
   ```

2. 新增压缩辅助类型：
   ```rust
   // src/storage/compression.rs
   use zstd::stream::{encode_all, decode_all};
   
   /// 用 zstd level 3 压缩文本
   pub fn compress_text(text: &str) -> Vec<u8> {
       encode_all(std::io::Cursor::new(text), 3).unwrap()
   }
   
   /// 解压文本
   pub fn decompress_text(data: &[u8]) -> String {
       let bytes = decode_all(std::io::Cursor::new(data)).unwrap();
       String::from_utf8(bytes).unwrap()
   }
   ```

3. Schema 变更（`SCHEMA_SQL` + 版本号）：
   - `content` TEXT → BLOB
   - `reasoning` TEXT → BLOB
   - `output_text` TEXT → BLOB (tool_events 表)

4. 读写路径修改：
   - `append_message()` / `update_message()` → 写入前压缩
   - `load_messages()` → 读出后解压
   - `append_tool_event()` / `load_tool_event_output()` → 同样处理

5. 数据迁移：
   - 新增 `SCHEMA_VERSION = 22`
   - 在 `SessionStore::open()` 中检测旧版本，执行迁移 SQL：
     ```sql
     ALTER TABLE messages RENAME TO messages_old;
     CREATE TABLE messages ... (新 schema);
     INSERT INTO messages SELECT ... FROM messages_old;
     DROP TABLE messages_old;
     ```

**关于调试便利性的处理（三选一）：**

- **选项一（推荐）：** 提供 CLI 子命令 `tidev db decode-message <id>` 和 `tidev db dump-session <id>`，输出解压后可读内容。
- **选项二：** 注册 SQLite 自定义函数 `zstd_decode(blob) → text`，可在 `sqlite3` CLI 中直接查询：
  ```rust
  connection.create_scalar_function("zstd_decode", 1, |ctx| {
      let blob = ctx.get::<Vec<u8>>(0)?;
      let text = decompress_text(&blob);
      Ok(text)
  })?;
  ```
- **选项三：** 轻量列（id, role, created_at, total_tokens, tool_call_id, tool_name 等）保持 TEXT 明文。日常调试 `SELECT id, role, total_tokens FROM messages` 完全不受影响。

**工作量：** 3-5 天

---

### P2 — 中期

#### 方案 C：减少锁竞争

**问题：** `SessionStore` 是 `Arc<std::sync::Mutex<SessionStore>>`，读写串行化。

**方案 C-1：RwLock（推荐）**

```rust
// 用 tokio::sync::RwLock 替代 std::sync::Mutex
store: Arc<tokio::sync::RwLock<SessionStore>>,
```

**方案 C-2：读写分离连接**

```rust
pub struct SessionStore {
    write_conn: Mutex<Connection>,   // DDL / DML
    read_conn: Connection,           // SELECT only
}
```

读操作用独立的 `read_conn`，无需锁。
写操作需要 `write_conn` 的互斥锁。

WAL 模式下读连接能看到写连接已提交的最新数据，一致性天然满足。

**方案 C-3：连接池（r2d2 + r2d2_sqlite）**

更重，但适合高并发场景（如 web 前端 + TUI 同时使用）。

**工作量：** 3-5 天

---

### P3 — 长期

#### 方案 D：Tool Events 去重

**问题：** tool 调用的输出同时存储在：
- `messages.content`（tool result message 的 `content` 字段 → 约 47.5 MB）
- `tool_events.output_text`（工具执行的原始输出 → 约 32.0 MB）

**改进方向：**
- `tool_events.output_text` 保留完整的原始执行输出
- `messages.content` 中不再重复存储完整 output，改为存储摘要或截断版本
- UI 渲染 tool result 时，从 `tool_events` 表按 `tool_call_id` 实时查询

**约束：** 需要改动 TUI 渲染逻辑 + 导出/还原逻辑。且 `content` 字段可能被 LLM 重新读取（作为历史消息的一部分），截断会丢失信息。需要仔细设计。

**工作量：** 2-3 天

---

## 推荐路线图

```
Week 1 ─── 方案 A (Pragma 调优)
             ├── 改动 src/storage/mod.rs
             ├── 回归测试
             └── 验证延迟改善

Week 2-3 ─ 方案 B (zstd 压缩)
             ├── 添加 zstd 依赖
             ├── 实现压缩辅助层
             ├── 修改读写路径
             ├── Schema 迁移逻辑
             ├── CLI 调试工具
             └── 回归测试

Week 4 ─── 方案 C (锁竞争)
             ├── RwLock 或读写分离实现
             ├── 验证并发正确性
             └── 性能对比测试

待定 ──── 方案 D (去重)
             └── 需评估对 LLM context 的影响
```

## 不做的事情

- **不引入第二个存储引擎**（如 LMDB/Sled/Parquet）— 当前 SQLite 的查询性能足够，复杂度和收益不成正比
- **不做消息级懒加载 / 分页** — UI 需要全量消息供用户查阅和历史分析
- **不删除 compact 过的旧消息内容** — 用户在 UI 中需要看到完整的对话历史
- **不加运行时迁移框架** — 保持现有模式：更新 `SCHEMA_SQL` + 用户重建数据库
