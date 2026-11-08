---
kind: task
id: T-053-plugin-api分类清单
title: plugin-api 分类清单：契约留下、实现迁 sdk、reports 拆两半（先清单，后搬迁）
status: todo
track: arch
prio: later
created: 2026-11-08
deps: []
where: "`crates/symbio-plugin-api/src`（`effects.rs` 562 行、`port.rs` 589 行、`text.rs` 314 行、`context.rs` 355 行、`reports.rs` 344 行；2026-11-09 实测，生产文件合计 4 921 行）；`crates/symbio-sdk/src`；`scripts/plugin-surface-audit.mjs`（HS-002 / HS-006）"
exit: 先出一张逐块分类清单（每块：符号 / 现址 / 类（契约 / 实现 / 机制）/ 处置（留守 api / 迁 sdk / 只转出）），不改代码；清单经批准后才搬迁；api 只留纯类型、宏、声明面与 re-export；`effects::apply`、端口能力校验、`text` 预算算术、`context` 拼装迁 sdk、api 只转出；`reports` 拆开——`ReportDecl` / `submit_reports!` / `inventory` 是契约，`validate` / `reports()` / `read_view` 是实现；HS-002 / HS-006 各补一条「自有实现」约束（不只数转发条数）；搬迁后 `gate --full` 全绿
---

# plugin-api 分类清单

> 对应架构分析：**P3（裁决类）**。
> 架构批第 3 笔之二。裁决（用户既定，不再开讨论）：
> **`plugin-api` 是契约，不是第三层机制。**
> 依据：P3 核实——api 自有实现 `effects` 562 / `port` 589 / `text` 314 / `context` 355 /
> `reports` 344 行（2026-11-09 实测），而 HS-002 / HS-006 只数门面条数与模块转发行，看不见实现。

## 棘轮初始值（2026-11-09 实测，不改代码）

本块只登记数字，作为搬迁后的「只许降」基线。行数按物理行（含 `//!` 与空行），
`pub fn` 只数顶格与四空格缩进的 `pub fn`（不含 `pub async fn` 之外的形式）。

| 文件 | 物理行 | `pub fn` | 自有实现里的符号（行号 = 实测起点） |
|---|---|---|---|
| `effects.rs` | **562** | 17 | `apply` 517 · `apply_before` 548 |
| `port.rs` | **589** | 19 | `port_family` 422 · `port_row_name` 430 · `port_rows` 460 · `capability_port_of` 514 · `capability_port_check` 567 |
| `text.rs` | **314** | 7 | `truncate_head_chars` 237 · `truncate_tail_chars` 243（及 Tokenizer 实现） |
| `context.rs` | **355** | 7 | `fit_sections` 228 · `sections` 335 |
| `reports.rs` | **344** | 5 | 契约 `ReportDecl` 57 / `submit_reports!` 86；实现 `validate` 170 · `reports()` 234 · `read_view` 256 · `read_view_with_ver` 284 |

上下文（同一 crate 的生产文件，用于判「自有实现占比」）：`debt.rs` 355 · `rows.rs` 311 ·
`drive.rs` 279 · `lib.rs` 237 · `hooks.rs` 233 · `events.rs` 192 · `declarations.rs` 175 ·
`settle.rs` 171 · `drive_entry.rs` 171 · `fact_source.rs` 150 · `retrieval.rs` 134 ·
`routes.rs` 91 · `behaviors.rs` 87 · `termination.rs` 79 · `views.rs` 47 · `vocab.rs` 45。
**生产文件合计 4 921 行；同目录 `*.test.rs` 另 1 921 行不计入基线。**

## 内容

| 类 | 判据 | 处置 |
|---|---|---|
| 契约 | 纯类型、trait 形状、宏、声明面、常量 | 留 api |
| 实现 | 需要状态、I/O、预算算术或拼装过程 | 迁 sdk，api 只 `pub use` 转出 |
| 机制 | 宿主侧的收集 / 校验过程 | 迁 sdk 或既有机制层，api 不持有 |

待拆的具体点：

- `effects::apply`（`crates/symbio-plugin-api/src/effects.rs` 517 行）、
  `apply_before`（同文件 548 行）——文件 562 行；
- 端口能力校验：`capability_port_check`（`crates/symbio-plugin-api/src/port.rs` 567 行）、
  `port_rows`（同文件 460 行）——文件 589 行；
- `text` 预算算术：Tokenizer 实现与 `truncate_*`（`crates/symbio-plugin-api/src/text.rs` 237 / 243 行）——文件 314 行；
- `context` 拼装：`fit_sections`（`crates/symbio-plugin-api/src/context.rs` 228 行）、
  `sections`（同文件 335 行）——文件 355 行；
- `reports` 两半：`ReportDecl`（`crates/symbio-plugin-api/src/reports.rs` 57 行）/
  `submit_reports!`（同文件 86 行）留；
  `validate`（170 行）/ `reports()`（234 行）/ `read_view`（256 行）/ `read_view_with_ver`（284 行）迁
  ——文件 344 行。

## 是否阻塞横向扩展

**是（裁决类）**。api 的自有实现没有登记时，扩展层每加一块都可能继续往 api 里塞实现；
HS-002 / HS-006 只数转发条数，看不见实现。必须补「自有实现」臂，或在代码里把实现迁走。

## 处理过程

1. 逐块填「类 / 处置 / 理由」，**先只出清单，不动代码**；清单交评审后才搬迁。
2. 搬迁后 api 侧只留 `pub use`，实现住在 sdk；`reports` 的契约与实现分居两处。
3. HS-002 / HS-006 今天只判「转发了几条」（`scripts/plugin-surface-audit.mjs` 451-473 行），
   各补一条「自有实现」约束：门面持有实现 = 红，而不是只看条数。
4. 跑 `gate --full`，全绿后提交。
