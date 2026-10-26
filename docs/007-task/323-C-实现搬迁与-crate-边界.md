---
kind: task
id: T-323-C-实现搬迁与-crate-边界
title: Phase C（后置、可选）：实现搬迁、actor 策略迁移与 crate 边界
status: todo
created: 2026-10-26
track: impl
prio: later
deps: [T-322-B4-根账本与共享类型-owner]
parent: T-316-内核边界与扩展局部性收口
where: symbio/src/symbio_core/ · symbio/src/plugins/ · Cargo.toml
exit: '可选批次：按 owner 搬投影实现与共享类型、把 schedule / skill / conation / commitment / recall 策略迁回 owner、拆出 `symbio_kernel`；C-002 的文本审计被 cargo 依赖方向取代。'
---

## 内容

只有 A/B 稳定后才做，不是前置条件：

- C0：按 owner 搬实现与共享类型（纯机械，不改行为 / 声明）；
- C1：actor 策略迁移；跨域的 control / preemption 单独论证；
- C2：拆 `symbio_kernel`，插件直接依赖 kernel；根账本消失或只剩 prelude。

## 处理过程

- 2026-10-26：立任务，置 later。D-183 明确「先模块后 crate」。
