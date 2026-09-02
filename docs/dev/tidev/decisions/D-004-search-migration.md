# D-004: tidev-search 独立迁移

**日期**: 2026-07-02  
**状态**: 已完成

## 背景

`FileSearchIndex`（后台文件索引 + notify 文件系统监听）在旧代码中位于 `tidev-engine/src/shared/file_search.rs`（866 行），是独立的叶子模块。

## 决策

**整体迁移至 `tidev-search` crate，不做架构修改。**

## 模块组织

```
tidev-search/src/lib.rs
  └── FileSearchIndex        — 后台索引 + notify 监听
  └── FileEntryKind          — File / Directory / Image
  └── FileSuggestion         — 搜索建议结果
  └── current_at_fragment()  — @ 片段提取（TUI 补全用）
```

## 理由

1. 零内部 tidev 依赖，纯外部 crate（ignore、notify、rayon、serde、log）
2. 逻辑独立、稳定，不需要改动即可使用
