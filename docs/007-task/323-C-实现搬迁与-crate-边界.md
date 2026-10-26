---
kind: task
id: T-323-C-实现搬迁与-crate-边界
title: Phase C（后置、可选）：实现搬迁、actor 策略迁移与 crate 边界
status: todo
created: 2026-10-26
track: impl
prio: later
deps: []
where: symbio/src/symbio_core/ · symbio/src/plugins/ · Cargo.toml
exit: '可选批次：按 owner 搬投影实现与共享类型、把 schedule / skill / conation / commitment / recall 策略迁回 owner、`fire_hook` 与 `ROUTE_*` 一起搬出 core、拆出 `symbio_kernel`；C-002 的文本审计被 cargo 依赖方向取代。'
---

## 内容

只有 A/B 稳定后才做，不是前置条件：

- C0：**已判定无可搬条目**（D-187）：`node scripts/core-export-audit.mjs --verbose`
  的 `[L1]` 段是**候选报告**——13 条逐条核过，全部命中配对契约 / 单一统计口径或机制
  状态 / 跨 crate API / 闭集词汇·签名·安全面之一，搬走要先把私有面提成根出口或切出
  第二份口径，与「纯机械」的前提相反。判据面同批修正：`#[cfg(test)] mod <stem>;`
  声明的整文件测试模块算测试（`EventStore` 因此不在清单里）。
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
- 2026-10-26：接收 B4 的两条 exit（B 阶段结构上不可达，D-186 / T-316 的收口记录）：
  「`core/mod.rs` 手改 = 0」归 C2（随 crate 边界消失），「删除只为可达而存的根导出」归 C0
  （C-002 下根出口是唯一通道，删一条 = 搬实现）。本任务因此与 A/B 父任务解耦，独立成立。
- 2026-10-26：C0 收口为 **0 条可搬**（D-187）。`[L1]` 的 13 条逐条核过：配对契约
  （`capability_resolve`）、单一统计口径 / 机制状态（`fallback_rate` / `slo_report` /
  `check_all` / `creator_ids` / `logger_init`）、跨 crate API（两个 `logger_*`）、
  闭集词汇·签名·安全面（两个 embedding 项、`EVENT_BUS_KIND_SYSTEM`、
  `text_truncate_bytes`、`branch_hide_unselected`）。同批修正消费方口径：
  `#[cfg(test)] mod <stem>;` 声明的整文件测试模块不算生产消费方（`EventStore` 退出清单）。
