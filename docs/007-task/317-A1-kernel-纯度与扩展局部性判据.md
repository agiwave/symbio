---
kind: task
id: T-317-A1-kernel-纯度与扩展局部性判据
title: Phase A1：kernel 纯度与扩展局部性判据（带反向用例）
status: todo
created: 2026-10-26
track: impl
prio: now
deps: []
parent: T-316-内核边界与扩展局部性收口
where: scripts/core-shape-audit.mjs · symbio/src/plugins/ · docs/004-ref/
exit: 'kernel 纯度判据（领域名词 0 / `crate::plugins` 0）进 `core-shape-audit` 且有反向用例；五类扩展夹具（事实 / 视图 / 策略 / 插件 / 共享类型）各一，机械核验 kernel 文件集不变；RE-006 报 churn 比例（只报告，不设硬门禁）。'
---

## 内容

只加判据，不改行为。

1. **kernel 纯度**：领域名词 denylist + 代码中的 `crate::plugins` 引用（注释不算）。
   进现有 `core-shape-audit`，不新增独立脚本。
2. **扩展局部性**：五类扩展各一个插件内夹具；夹具必须只用公开扩展口（`PluginDeclarations`
   / `CapabilityRow` / 装配位），并断言 kernel 文件集未被改动。
3. **churn 报告**：RE-006 增一列「功能 / 重构提交触达 core 的比例」，只报告趋势。

## 处理过程

- 2026-10-26：立任务。纯度判据的扫描范围依赖 T-318 的 kernel 子集；先做夹具与扫描器骨架。
