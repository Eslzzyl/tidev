# SQLite 透明压缩研究报告

本文档记录了对 tidev 项目引入 SQLite 透明压缩功能的可行性研究。

**研究日期**: 2025-04-18  
**研究目标**: 评估为 tidev 存储层添加 zstd 透明压缩的可行性、潜在问题和实现方案  
**结论**: 技术上可行，建议作为可选功能在未来引入

---

## 目录

1. [背景与动机](#背景与动机)
2. [候选库对比](#候选库对比)
3. [可行性分析](#可行性分析)
4. [依赖兼容性](#依赖兼容性)
5. [性能影响评估](#性能影响评估)
6. [实现方案](#实现方案)
7. [风险与注意事项](#风险与注意事项)
8. [结论与建议](#结论与建议)

---

## 背景与动机

### tidev 存储层现状

tidev 使用 SQLite 作为会话持久化存储，当前数据库结构：

- **位置**: `~/.local/share/tidev/sessions.sqlite3`
- **大小**: 约 2.9 MB（27 个会话，1056 条消息，505 个工具事件）
- **可压缩内容**: 
  - `messages.content` - LLM 响应内容
  - `messages.reasoning` - 推理过程
  - `tool_events.input_json` - 工具调用输入
  - `tool_events.output_text` - 工具调用输出
- **可压缩比例**: 约 64% 的数据为文本内容

### 引入压缩的预期收益

1. **存储节省**: 预计减少 50-70% 存储空间
2. **I/O 减少**: 更小的数据量意味着更少的磁盘 I/O
3. **缓存效率**: 压缩数据占用更少内存缓存

### 评估的两个 Submodule

项目中已添加两个 git submodule 用于评估：

- `sqlite-zstd/` - Rust 实现的行级透明压缩
- `sqlite_zstd_vfs/` - C++ 实现的页级 VFS 压缩

---

## 候选库对比

### sqlite-zstd

**仓库**: https://github.com/phiresky/sqlite-zstd  
**许可证**: LGPL-2.0-or-later  
**语言**: Rust  

#### 工作原理

```
原始表 → 重命名为 _tablename_zstd → 创建视图 tablename
         ↓
    添加 dict_id 列 → 压缩数据存储为 blob
         ↓
    视图通过 zstd_decompress_col() 透明解压
```

#### 特点

| 特性 | 说明 |
|------|------|
| 压缩级别 | 行级/列级 |
| WAL 支持 | ✅ 完全支持 |
| 透明度 | 视图 + 触发器实现 |
| 维护需求 | 需显式调用 `zstd_incremental_maintenance()` |
| 集成方式 | `sqlite_zstd::load(&conn)` - Rust 原生 |
| 字典缓存 | LRU 缓存，10 秒 TTL |

#### 压缩效果（来自作者基准测试）

| 数据集 | 原始大小 | 压缩后 | 压缩率 |
|--------|---------|--------|--------|
| IMDB 数据库 | 2.0 GB | 528 MB | 74% |
| 时间追踪数据 | 7.6 GB | ~1.9 GB | 75% |
| Android 应用数据 | 800 MB | 72 MB | 91% |

### sqlite_zstd_vfs

**仓库**: https://github.com/mlin/sqlite_zstd_vfs  
**许可证**: MIT  
**语言**: C++  

#### 工作原理

```
SQLite VFS 层拦截 → 压缩整个数据库页 → 存储到外层 SQLite 数据库
                 ↓
            读取时解压页 → 对上层透明
```

#### 特点

| 特性 | 说明 |
|------|------|
| 压缩级别 | 页级（VFS 层） |
| WAL 支持 | ❌ **不支持** |
| 透明度 | 完全透明 VFS |
| 维护需求 | 自动后台压缩 |
| 集成方式 | 加载 .so 扩展 |
| 并发控制 | 强制 EXCLUSIVE 锁定模式 |

#### 性能（作者基准测试）

| 配置 | 原始大小 | 压缩后 | 顺序查询 | 8路连接查询 |
|------|---------|--------|---------|-----------|
| SQLite 默认 | 1182 MB | - | 6.7s | 3.0s |
| zstd_vfs 默认 | - | 647 MB | 8.8s | 35.7s |
| zstd_vfs 调优 | - | 433 MB | 7.8s | 4.5s |

### 关键差异对比

| 维度 | sqlite-zstd | sqlite_zstd_vfs |
|------|-------------|-----------------|
| **WAL 兼容性** | ✅ 支持 | ❌ 不支持 |
| **集成复杂度** | 低（Rust 原生） | 高（需编译 C++） |
| **压缩粒度** | 行级（更细） | 页级 |
| **文本压缩效率** | 更高（可训练字典） | 一般 |
| **对 tidev 适用性** | ✅ 推荐 | ❌ 不适用 |

### 致命兼容性问题

**sqlite_zstd_vfs 无法用于 tidev**：

1. 不支持 WAL 模式，而 tidev 使用 `journal_mode=WAL`
2. 需要 C++ 编译工具链
3. 强制 EXCLUSIVE 锁定模式影响并发

---

## 可行性分析

### 对已有数据的支持

**可以在已有数据上启用压缩**：

1. `zstd_enable_transparent()` 重命名原表，创建视图
2. 已有数据保持未压缩（`dict_id` 为 NULL）
3. 读取时视图自动处理（未压缩数据透传）
4. 调用 `zstd_incremental_maintenance()` 后逐步压缩

**示例流程**：

```sql
-- 1. 启用压缩
SELECT zstd_enable_transparent('{
    "table": "messages",
    "column": "content",
    "compression_level": 3,
    "dict_chooser": "''messages''"
}');

-- 2. 数据仍可正常查询（未压缩部分直接返回）

-- 3. 在空闲时压缩历史数据
SELECT zstd_incremental_maintenance(null, 1);

-- 4. 清理空间
VACUUM;
```

### 关闭/切换压缩的挑战

**不能简单地开关压缩**：

- 压缩后数据格式已变（blob + dict_id 列）
- 需要"迁移"回未压缩格式：

```sql
-- 关闭压缩的迁移流程
CREATE TABLE messages_uncompressed AS SELECT * FROM messages; -- 视图自动解压
DROP VIEW messages;
DROP TABLE _messages_zstd;
ALTER TABLE messages_uncompressed RENAME TO messages;
-- 还需要重建索引
```

### 推荐策略

**分阶段压缩**：通过 `dict_chooser` 控制哪些数据被压缩

```sql
-- 只压缩1小时前的数据，保持近期数据可快速更新
SELECT zstd_enable_transparent('{
    "table": "messages",
    "column": "content",
    "compression_level": 3,
    "dict_chooser": "case 
        when updated_at < datetime(''now'', ''-1 hour'') then ''messages'' 
        else null 
    end"
}');
```

这样：
- 近期数据保持未压缩 → 流式更新性能不受影响
- 历史数据被压缩 → 节省存储空间
- 维护时自动处理

---

## 依赖兼容性

### 当前依赖对比

| 依赖 | sqlite-zstd | tidev | 状态 |
|------|-------------|-------|------|
| rusqlite | 0.35.0 | 0.39.0 | ⚠️ API 可能有变化 |
| zstd | 0.11.2 | ❌ 无 | ✅ 需添加 |
| lru_time_cache | 0.11.11 | ❌ 无 | 可用现有 lru 替代 |
| lazy_static | 1.4.0 | ❌ 无 | 用 `LazyLock` 替代 |
| rand | 0.8.5 | ✅ 已有 | ✅ 兼容 |
| log | ✅ | ✅ | ✅ 兼容 |
| anyhow | ✅ | ✅ | ✅ 兼容 |
| serde / serde_json | ✅ | ✅ | ✅ 兼容 |
| env_logger | 0.9.0 | ❌ 无 | 可移除（简化日志逻辑） |

### 实现方案：复制代码并适配

由于 crates.io 上的 sqlite-zstd 版本依赖较旧，建议直接复制源码并适配：

#### 1. 添加 zstd 依赖

```toml
# Cargo.toml
[dependencies]
zstd = "0.13"
```

#### 2. 需要复制的源文件

| 文件 | 功能 | 修改需求 |
|------|------|---------|
| `lib.rs` | 入口 | 简化，移除 env_logger |
| `add_functions.rs` | 注册 SQL 函数 | 适配 rusqlite API |
| `basic.rs` | 压缩/解压函数 | 小幅修改 |
| `transparent.rs` | 透明压缩逻辑 | 核心逻辑，重点测试 |
| `dict_management.rs` | 字典缓存 | 用 lru 替代 lru_time_cache |
| `dict_training.rs` | 字典训练 | 小幅修改 |
| `util.rs` | 工具函数 | 小幅修改 |

#### 3. LRU 缓存实现

sqlite-zstd 使用 `lru_time_cache`，而 tidev 已有 `lru` 依赖。需要包装 TTL 功能：

```rust
struct TimedLruCache<K, V> {
    cache: LruCache<K, (V, Instant)>,
    ttl: Duration,
}

impl<K, V> TimedLruCache<K, V> {
    fn get(&mut self, key: &K) -> Option<&V> {
        let now = Instant::now();
        if let Some((v, ts)) = self.cache.get(key) {
            if now.duration_since(*ts) < self.ttl {
                return Some(v);
            }
            // 过期，移除
            self.cache.pop(key);
        }
        None
    }
    
    fn put(&mut self, key: K, value: V) {
        self.cache.put(key, (value, Instant::now()));
    }
}
```

**评估**：在 lru 上包装 TTL 是可靠的，因为：
- 字典数量有限（每个压缩配置一个）
- 懒过期对于字典缓存场景足够
- 不访问的字典本来就不需要立即清理

#### 4. rusqlite API 适配

需要验证 rusqlite 0.35 → 0.39 的 API 变化，主要关注：
- `create_scalar_function` 签名
- `create_aggregate_function` 签名
- `get_connection()` unsafe API

#### 5. 许可证合规

sqlite-zstd 使用 LGPL-2.0-or-later：
- 复制代码需保留版权声明和许可证
- 在文件头添加原始许可证信息
- tidev 作为终端应用，LGPL 影响较小

---

## 性能影响评估

### 博客基准测试数据（IMDB 数据集）

| 操作 | 未压缩 | 压缩后 | 变化 |
|------|--------|--------|------|
| 顺序 SELECT | ~300k/s | ~250k/s | -17% |
| 随机 SELECT | ~30k/s | ~50k/s | **+67%** |
| INSERT | ~120k/s | ~100k/s | -17% |
| UPDATE 随机 | ~60k/s | ~20k/s | -67% |

### 关键发现

1. **读取性能**：
   - 压缩后可能更快（数据小 → B树浅 → 缓存命中率高）
   - 字典缓存后解压很快

2. **写入性能**：
   - INSERT：新数据不压缩，只有触发器开销（~17%）
   - UPDATE：有明显开销，原因待查

3. **压缩时机**：
   - 压缩发生在 `zstd_incremental_maintenance()` 调用时
   - 不影响正常写入路径

### tidev 数据特征

| 指标 | 值 |
|------|-----|
| 数据库大小 | 2.9 MB |
| 可压缩文本 | ~1.86 MB (64%) |
| 消息数 | 1,056 条 |
| 平均消息长度 | 651 字节 |
| 最大消息长度 | 7,028 字节 |
| 工具事件数 | 505 条 |
| 平均工具输出 | 1,731 字节 |

### tidev 访问模式分析

| 操作 | 频率 | 数据量 | 性能敏感度 |
|------|------|--------|------------|
| 加载会话列表 | 高 | 元数据 | 低 |
| 加载单个会话消息 | 中 | 批量读取 | 中 |
| 追加消息 | 高 | 单条写入 | 中 |
| 流式更新消息 | 高 | 单条更新 | **高** |
| 加载工具事件 | 低 | 批量读取 | 低 |

### 性能影响结论

**可接受**，理由：

1. **tidev 数据规模小**（<3MB），绝对延迟差异在毫秒级
2. **主要瓶颈在网络 I/O**（LLM API），不是数据库
3. **可通过策略规避**：热数据不压缩（dict_chooser 返回 null）
4. **收益明显**：预计压缩 50-70%，减少磁盘占用

---

## 实现方案

### 集成步骤

#### Phase 1: 基础集成

1. 复制 sqlite-zstd 源码到 `src/sqlite_zstd/`
2. 添加 `zstd` 依赖到 `Cargo.toml`
3. 实现 `TimedLruCache` 替代 `lru_time_cache`
4. 适配 rusqlite 0.39 API
5. 编写单元测试验证基本功能

#### Phase 2: 存储层集成

1. 在 `SessionStore::open()` 后加载扩展：

```rust
impl SessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        // ... 现有代码 ...
        
        // 加载 zstd 扩展（如果配置启用）
        if config.storage.compression {
            sqlite_zstd::load(&connection)?;
        }
        
        Ok(Self { connection, path })
    }
}
```

2. 提供配置选项：

```toml
# ~/.config/tidev/config.toml
[storage]
compression = true
compression_level = 3
compression_dict_chooser = "case when updated_at < datetime('now', '-1 hour') then 'messages' else null end"
```

#### Phase 3: 维护集成

1. 在会话关闭或空闲时触发维护：

```rust
impl SessionStore {
    pub fn run_compression_maintenance(&self) -> Result<()> {
        self.connection.execute(
            "SELECT zstd_incremental_maintenance(5.0, 0.5)",
            params![],
        )?;
        Ok(())
    }
}
```

2. 提供 CLI 命令：

```bash
tidev --compress   # 手动触发压缩维护
tidev --vacuum     # 压缩后清理空间
```

### 数据库 Schema 影响

启用压缩后，表结构变化：

```sql
-- 原始 messages 表
CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    content TEXT,        -- 会被压缩
    ...
);

-- 启用压缩后
ALTER TABLE messages RENAME TO _messages_zstd;
ALTER TABLE _messages_zstd ADD COLUMN _content_dict INTEGER 
    DEFAULT NULL REFERENCES _zstd_dicts(id);
CREATE VIEW messages AS 
    SELECT id, session_id, 
           zstd_decompress_col(content, 1, _content_dict, true) as content,
           ...
    FROM _messages_zstd;
```

---

## 风险与注意事项

### 1. 数据库兼容性

**风险**：压缩后的数据库无法用普通 SQLite 工具查看

```sql
-- 不加载扩展时查询 messages 视图
SELECT * FROM messages;
-- ERROR: no such function: zstd_decompress_col

-- 直接查询底层表
SELECT content FROM _messages_zstd;
-- 返回: 二进制 blob，不可读
```

**缓解措施**：

- 提供导出命令：`tidev --export-uncompressed backup.sqlite3`
- 文档说明压缩后数据库的特殊性

### 2. 许可证合规

**风险**：sqlite-zstd 使用 LGPL-2.0-or-later

**缓解措施**：

- 复制代码时保留原始许可证声明
- 在项目文档中声明使用了 LGPL 代码
- tidev 作为终端应用，LGPL 影响较小

### 3. 数据安全

**风险**：作者声明 "I wouldn't trust it with my data (yet)"

**缓解措施**：

- 作为可选功能，默认关闭
- 建议用户在启用前备份数据
- 充分测试后再在生产环境使用

### 4. 迁移回退

**风险**：无法简单地"关闭"压缩

**缓解措施**：

- 提供迁移工具将压缩数据库转回未压缩格式
- 在配置中明确说明压缩是"单向"操作

### 5. 流式更新性能

**风险**：UPDATE 操作开销可能影响流式消息更新

**缓解措施**：

- 使用 `dict_chooser` 保持近期数据不压缩
- 或在流式阶段暂时禁用触发器（需研究可行性）

---

## 结论与建议

### 技术可行性

✅ **技术上完全可行**

- sqlite-zstd 与 tidev 技术栈兼容良好
- Rust 原生集成，无需额外编译工具
- WAL 模式完全支持
- 行级压缩对文本内容效率高

### 推荐方案

**作为可选功能引入**，具体策略：

1. **默认不启用**：保持最大兼容性和安全性
2. **用户可选**：通过配置启用压缩
3. **智能压缩**：通过 `dict_chooser` 保持热数据不压缩
4. **提供工具**：导出、维护、回退命令

### 配置示例

```toml
# ~/.config/tidev/config.toml

[storage]
# 是否启用透明压缩（默认 false）
compression = false

# 压缩级别 1-19，默认 3（压缩速度和解压速度的平衡）
compression_level = 3

# 压缩目标：哪些表和列（空则使用默认配置）
compression_targets = [
    { table = "messages", column = "content" },
    { table = "messages", column = "reasoning" },
    { table = "tool_events", column = "output_text" },
]

# 热数据保留时间（不压缩最近的数据）
compression_hot_data_hours = 1
```

### 实现优先级

如果决定实现，建议按以下顺序：

1. **P0 - 基础功能**：复制代码、适配依赖、单元测试
2. **P1 - 存储集成**：配置读取、扩展加载、基本压缩
3. **P2 - 维护机制**：自动维护、CLI 命令
4. **P3 - 用户体验**：导出工具、文档、配置向导

### 当前决策

**暂不引入**，原因：

1. 当前数据库规模（<3MB）收益有限
2. 需要更多时间验证 rusqlite API 兼容性
3. 需要设计完善的配置和工具支持
4. 数据安全性需要更多测试验证

**未来触发条件**：

- 数据库增长到 100MB+ 时重新评估
- 用户明确提出存储空间需求时
- 有充足时间进行完整测试和文档编写时

---

## 参考资料

- [sqlite-zstd GitHub](https://github.com/phiresky/sqlite-zstd)
- [sqlite-zstd 博客文章](https://phiresky.github.io/blog/2022/sqlite-zstd)
- [sqlite_zstd_vfs GitHub](https://github.com/mlin/sqlite_zstd_vfs)
- [zstd 官方文档](https://facebook.github.io/zstd/)
- [rusqlite 文档](https://docs.rs/rusqlite/)

---

*本报告由 Claude (Anthropic) 协助生成，基于对 sqlite-zstd 和 sqlite_zstd_vfs 两个开源项目的分析。*
