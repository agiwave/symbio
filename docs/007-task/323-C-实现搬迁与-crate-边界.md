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
exit: '可选批次：按 owner 搬投影实现与共享类型、把 schedule / skill / conation / commitment / recall 策略迁回 owner、`fire_hook` 与 `ROUTE_*` 一起搬出 core、拆出 `symbio_kernel`；C-002 的文本审计被 cargo 依赖方向取代。'
---

## 内容

只有 A/B 稳定后才做，不是前置条件：

- C0：按 owner 搬实现与共享类型（纯机械，不改行为 / 声明）；
- C0.5：`fire_hook` 与 `ROUTE_*` 常量一起搬出 core（D-184）——`fire_hook` 是
  session / loop 共用的机制助手，而 `ROUTE_HOOK_FIRE` 是它唯一的 core 依赖；两者
  分开搬会得到「core 依赖 host」或「core 内藏字面量」两者之一，都是漂移；
- C1：actor 策略迁移；跨域的 control / preemption 单独论证；
- C2：拆 `symbio_kernel`，插件直接依赖 kernel；根账本消失或只剩 prelude。

## 处理过程

- 2026-10-26：立任务，置 later。D-183 明确「先模块后 crate」。
- 2026-10-26：接收 B3 的剩余路由常量（D-184）：原计划在 B3 生成 `ROUTE_*`，但
  `ROUTE_HOOK_FIRE` 的唯一 core 消费方是 `fire_hook`，现在搬会形成 core → host
  的反向依赖。改为随 C0.5 一起搬。
