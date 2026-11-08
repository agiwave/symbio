---
kind: task
id: T-053-plugin-api分类清单
title: plugin-api 分类清单：契约留下、实现迁 sdk、reports 拆两半（先清单，后搬迁）
status: todo
track: arch
prio: later
created: 2026-11-08
deps: [T-050-架构文档与残留对齐]
where: "`crates/symbio-plugin-api/src`（`effects.rs` 519 行、`port.rs` 540 行、`text.rs` 286 行、`context.rs` 327 行、`reports.rs` 327 行）；`crates/symbio-sdk/src`；`scripts/plugin-surface-audit.mjs`（HS-002 / HS-006）"
exit: 先出一张逐块分类清单（每块：符号 / 现址 / 类（契约 / 实现 / 机制）/ 处置（留守 api / 迁 sdk / 只转出）），不改代码；清单经批准后才搬迁；api 只留纯类型、宏、声明面与 re-export；`effects::apply`、端口能力校验、`text` 预算算术、`context` 拼装迁 sdk、api 只转出；`reports` 拆开——`ReportDecl` / `submit_reports!` / `inventory` 是契约，`validate` / `reports()` / `read_view` 是实现；HS-002 / HS-006 各补一条「自有实现」约束（不只数转发条数）；搬迁后 `gate --full` 全绿
---

# plugin-api 分类清单

> 架构批第 3 笔之二。裁决（用户既定，不再开讨论）：
> **`plugin-api` 是契约，不是第三层机制。**
> 依据：P3 核实——api 自有实现 `effects` 519 / `port` 540 / `text` 286 / `context` 327 /
> `reports` 327 行，而 HS-002 / HS-006 只数门面条数与模块转发行，看不见实现。

## 内容

| 类 | 判据 | 处置 |
|---|---|---|
| 契约 | 纯类型、trait 形状、宏、声明面、常量 | 留 api |
| 实现 | 需要状态、I/O、预算算术或拼装过程 | 迁 sdk，api 只 `pub use` 转出 |
| 机制 | 宿主侧的收集 / 校验过程 | 迁 sdk 或既有机制层，api 不持有 |

待拆的具体点：

- `effects::apply`（`crates/symbio-plugin-api/src/effects.rs` 517 行）、
  `apply_before`（同文件 548 行）；
- 端口能力校验：`capability_port_check`（`crates/symbio-plugin-api/src/port.rs` 567 行）、
  `port_rows`（同文件 460 行）；
- `text` 预算算术：Tokenizer 实现与 `truncate_*`（`crates/symbio-plugin-api/src/text.rs` 237 / 243 行）；
- `context` 拼装：`fit_sections`（`crates/symbio-plugin-api/src/context.rs` 228 行）、
  `sections`（同文件 335 行）；
- `reports` 两半：`ReportDecl`（`crates/symbio-plugin-api/src/reports.rs` 57 行）/
  `submit_reports!`（同文件 86 行）留；
  `validate`（170 行）/ `reports()`（234 行）/ `read_view`（256 行）迁。

## 处理过程

1. 逐块填「类 / 处置 / 理由」，**先只出清单，不动代码**；清单交评审后才搬迁。
2. 搬迁后 api 侧只留 `pub use`，实现住在 sdk；`reports` 的契约与实现分居两处。
3. HS-002 / HS-006 今天只判「转发了几条」（`scripts/plugin-surface-audit.mjs` 451-473 行），
   各补一条「自有实现」约束：门面持有实现 = 红，而不是只看条数。
4. 跑 `gate --full`，全绿后提交。
