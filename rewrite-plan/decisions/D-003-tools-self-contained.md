# D-003: tidev-tools 自包含原则

**日期**: 2026-07-02  
**状态**: 暂缓（先做 tidev-agent）

## 决策

tidev-tools 应自包含，不依赖其他 tidev crate（除 tidev-types 外）。对于需要的外部能力（存储、配置、指令解析），通过 traits 或简单内部实现解决。

## 理由

tidev-storage、tidev-config、tidev-instructions 等 crate 尚未成熟，tidev-tools 不应被其阻塞。
