---
kind: task
id: T-266-MemoryStore不变量边界
title: `MemoryStore` 不执行 I1/I2/I3 是边界（D-134）——不变量属于权威事实源的写侧闸门
status: done
created: 2026-10-24
track: design
prio: now
deps: []
parent: T-264-设计实现验证收口批
where: docs/003-design/001-核心架构.md · symbio/src/symbio_core/store/mod.rs
exit: 'DE-001 §7 的 D-134 落格；`store/mod.rs` 的 `MemoryStore` 文档写明它不判跨事件不变量、执行方是持久化实现的唯一写入口；生产路径（`WalStore::gate`）的不变量判据不因该边界而放松。'
evidence: 'DE-001 §7 新增 D-134（`### D-134` 条目）；`store/mod.rs` 的 `MemoryStore` 文档段写明「不执行 I1/I2/I3，执行方是 `WalStore::gate`，本类型只服务测试与 S0 基线」。生产判据仍在 `verify/invariants.rs` 与 `session/store/wal.rs:194-222` 的写侧门禁；`grep` 确认无生产代码依赖 `MemoryStore` 提供不变量。'
---

## 内容

`MemoryStore` 是测试 / S0 基线用的平凡 `Store` 实现：它只做锁内查重与赋号，
**不**检查 `seq` 跨事件单调、`produced_by` 更早、每 turn 至多一条收束。

这不是漏接，而是职责划分：I1/I2/I3 是「一条事实进入**权威**日志」的写侧闸门，
权威日志恒为持久化实现（`WalStore::gate`）。把判据复制进每个 `Store` 实现，
会让「哪些不变量必须成立」散成 N 份，且测试替身与生产实现行为不一致时无从判断。

**风险的封堵方式**：生产装配只认持久化实现（`facts()` 返回 `WalStore`），
`MemoryStore` 不出现在任何生产构造点。这条边界的前提是「测试替身不被误装进生产」，
由装配路径的单一性保证，不需要给替身补判据。

## 处理过程

- 2026-10-24：裁决并落 D-134。同步在 `store/mod.rs` 的类型文档写明边界，
  避免下一位读者把它读成「门禁缺失」。
