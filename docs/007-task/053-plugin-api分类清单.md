---
kind: task
id: T-053-plugin-api分类清单
title: plugin-api 分类清单：契约留下、实现迁 sdk、reports 拆两半（先清单，后搬迁）
status: doing
track: arch
prio: now
created: 2026-11-08
deps: []
where: "`crates/symbio-plugin-api/src`（`port.rs` 478 行、`reports.rs` 344 行；生产文件合计 2 561 行）；`crates/symbio-sdk/src/plugin/`（`effects` / `behaviors` / `context` / `retrieval` / `rows` / `settle` / `fact_source` / `drive_entry`）；`crates/symbio-sdk/src/text/tokenizer.rs`；`scripts/plugin-surface-audit.mjs`（HS-002 / HS-006）；`scripts/facade-loc-audit.mjs`（FA-001…FA-004）"
exit: 先出一张逐块分类清单（每块：符号 / 现址 / 类（契约 / 实现 / 机制）/ 处置（留守 api / 迁 sdk / 只转出）），不改代码；清单经批准后才搬迁；api 只留纯类型、宏、声明面与 re-export；`effects::apply`、端口能力校验、`text` 预算算术、`context` 拼装迁 sdk、api 只转出；`reports` 拆开——`ReportDecl` / `submit_reports!` / `inventory` 是契约，`validate` / `reports()` / `read_view` 是实现；「自有实现只许降」由行数棘轮 `FA-001`…`FA-004`（`scripts/facade-loc-audit.mjs`）判，不另开 HS 臂；搬迁后 `gate --full` 全绿
---

# plugin-api 分类清单

> 对应架构分析：**P3（裁决类）**。
> 架构批第 3 笔之二。裁决（用户既定，不再开讨论）：
> **`plugin-api` 是契约，不是第三层机制。**
> 依据：P3 核实——api 自有实现 `effects` 562 / `port` 478 / `text` 6 / `context` 347 /
> `reports` 344 行（现值以 `scripts/baselines/plugin-api-loc.json` 为准），而 HS-002 / HS-006 只数门面条数与模块转发行，看不见实现。

## 棘轮初始值（2026-11-09 实测，不改代码）

本块只登记数字，作为搬迁后的「只许降」基线。行数按物理行（含 `//!` 与空行），
`pub fn` 只数顶格与四空格缩进的 `pub fn`（不含 `pub async fn` 之外的形式）。

| 文件 | 物理行 | `pub fn` | 自有实现里的符号（搬迁前快照；行号以实测起点记，现值以 `scripts/baselines/plugin-api-loc.json` 为准） |
|---|---|---|---|
| `effects.rs` | **562** | 17 | `apply` 517 · `apply_before` 548 |
| `port.rs` | **478** | 19 | `port_family` 422 · `port_row_name` 430 · `port_rows` 460 · `capability_port_of` 514 · `capability_port_check` 567（搬迁前快照） |
| `text.rs` | **6** | 0 | 转发：实现住 `symbio_sdk::text::tokenizer`（`D-404`） |
| `context.rs` | **347** | 7 | `fit_sections` 228 · `sections` 335（搬迁前快照） |
| `reports.rs` | **344** | 5 | 契约 `ReportDecl` 57 / `submit_reports!` 86；实现 `validate` 170 · `reports()` 234 · `read_view` 256 · `read_view_with_ver` 284 |

上下文（同一 crate 的生产文件，用于判「自有实现占比」）：`debt.rs` 355 · `rows.rs` 311 ·
`drive.rs` 279 · `lib.rs` 237 · `hooks.rs` 233 · `events.rs` 192 · `declarations.rs` 175 ·
`settle.rs` 171 · `drive_entry.rs` 171 · `fact_source.rs` 150 · `retrieval.rs` 134 ·
`routes.rs` 91 · `behaviors.rs` 87 · `termination.rs` 79 · `views.rs` 47 · `vocab.rs` 45。
**生产文件合计 4 494 行；同目录 `*.test.rs` 另 1 921 行不计入基线。**

**收尾时的现值（2026-11-09，以 `scripts/baselines/plugin-api-loc.json` 与 `scripts/arch-dead-code.json` 为准）**：
`port` **478**（`D-404` 已搬）、`reports` **344**；门面合计 **2 561**（原 4 494）；
`effects` / `text` / `context` 三格按退役形态记 `lines: 0` + `retired`（文件已删，格子钉零）。
`effects` / `behaviors`（`D-408`）、`text`（`D-404`）、`context`（`D-412`）与五个链接期注册表
`retrieval` / `rows` / `settle` / `fact_source` / `drive_entry`（`D-413`）整块住在
`crates/symbio-sdk/src/plugin/`；门面不再留纯转发文件，模块名在 `lib.rs` 根上一行转出，
提交宏同在根上。`drive_entry` 只转出 `publish_deployment_root`（唯一扩展层消费方）。
另按 `D-409` 撤掉了行侧效果面（`EffectFace` / `DialogDraft` / `LiveDraft` / `EffectDraft` 与
`RowStep` 的 `FactsWithEffects` 变体），零调用清单由 73 条降到 71 条。

## 内容

| 类 | 判据 | 处置 |
|---|---|---|
| 契约 | 纯类型、trait 形状、宏、声明面、常量 | 留 api |
| 实现 | 需要状态、I/O、预算算术或拼装过程 | 迁 sdk，api 只 `pub use` 转出 |
| 机制 | 宿主侧的收集 / 校验过程 | 迁 sdk 或既有机制层，api 不持有 |

待拆的具体点（**搬迁前快照**：符号行号是当时实测，现值以 `scripts/baselines/plugin-api-loc.json` 为准）：

- `effects::apply`（`crates/symbio-sdk/src/plugin/effects.rs` 517 行）、
  `apply_before`（同文件 548 行）——文件 562 行（搬迁前快照）；
- 端口能力校验：`capability_port_check`（`crates/symbio-plugin-api/src/port.rs` 567 行）、
  `port_rows`（同文件 460 行）——文件 478 行；
- `text` 预算算术：实现住 `symbio_sdk::text::tokenizer`（`D-404`）；
  下列 Tokenizer 实现与 `truncate_*`（`crates/symbio-sdk/src/text/tokenizer.rs` 237 / 243 行）为搬迁前快照；
- `context` 拼装：`fit_sections`（`crates/symbio-sdk/src/plugin/context.rs` 228 行）、
  `sections`（同文件 335 行）——文件 347 行（搬迁前快照）；
- `reports` 两半：`ReportDecl`（`crates/symbio-plugin-api/src/reports.rs` 57 行）/
  `submit_reports!`（同文件 86 行）留；
  `validate`（170 行）/ `reports()`（234 行）/ `read_view`（256 行）/ `read_view_with_ver`（284 行）迁
  ——文件 344 行。

## 是否阻塞横向扩展

**是（裁决类）**。api 的自有实现没有登记时，扩展层每加一块都可能继续往 api 里塞实现；
HS-002 / HS-006 只数转发条数，看不见实现。**这一条已由棘轮堵上**（下一步①），
搬迁（下一步②）仍待做。

## 处理过程

1. ~~把「自有实现只许降」写成门禁~~ **已完成**：`scripts/facade-loc-audit.mjs`
   （`FA-001`…`FA-004`）、基线 `scripts/baselines/plugin-api-loc.json`，进门禁 `GUARDS`；
   回归用例 `scripts/facade-loc-audit.test.mjs` 钉住「实现加 5 行 ⇒ 红」。判据与初始读数
   已登记在 `018-模块架构.md` 的 `D-399`。
2. 逐块搬迁（清单已评审）。第一块 `text` 已落地：实现住 `symbio-sdk/src/text/tokenizer.rs`，
   api 侧在根上转出模块名（`D-413` 收掉了过渡期的转发文件）。
   搬迁后 api 侧只留转出行，实现住在 sdk；`reports` 的契约与实现分居两处。
   每搬一块 **同批下调基线**（`FA-002` 会拦住「删了不同批下调」）。口径见
   054 的 `D-404`：净零搬迁不记欠账，`from` / `to` 与两侧行数写进提交正文。

   **第二块 `effects` 已落地（`D-408`），并顺带撤了一条零使用通道（`D-409`）**：
   `effects` + `behaviors` 整块搬进 `crates/symbio-sdk/src/plugin/`（门面只留 7 行转出），
   同时删掉 D-293 的**行侧效果面**（`EffectFace` / `DialogDraft` / `LiveDraft` / `EffectDraft`
   与 `RowStep` 的 `FactsWithEffects` 变体）。这一笔因此是**净减**，按迁移记 1 笔（`EX-007` 7 → 6）。

   > **剩下两块为什么不能「只搬函数」**：`context` 的 `fit_sections` / `sections` 吃的是门面的
   > `Section`，`reports` 的 `validate` / `reports()` 吃的是 `ReportDecl`——而 `ReportDecl` 正是
   > `submit_reports!` 的采集类型。`inventory::collect!` 的采集类型必须与宏同 crate，`sdk` 又不能
   > 依赖门面（`D-258`），所以它们的搬法和 `effects` 一模一样：**类型闭包整块搬、门面转出**。
   > 只搬函数体那一半在这里是做不到的（`D-408` 的三条硬事实）。

   **第三块 `context` 已落地（`D-412`）**：`context.rs`（347 行）+ `context.test.rs`（266 行）
   整块搬进 `crates/symbio-sdk/src/plugin/`；`submit_context_provider!` 随注册表
   搬走并经根上转出，四个插件的提交写法一字未改。同批下调 `FA-002`（347 → 8、合计
   3833 → 3496），`CS-009` 钉到搬后实测 **552 : 249**——其中 23 格字段 / 5 格变体是换地址，
   另清掉搬迁前就存在的 10 格 `structFields` 存量余量（实测 529 / 基线 539）。

   **第四批五个注册表已落地（`D-413`）**：`retrieval`（134）/ `rows`（282）/ `settle`（171）/
   `fact_source`（150）/ `drive_entry`（171）连同三个测试文件整块搬进
   `crates/symbio-sdk/src/plugin/`；门面九份纯转发文件一并收掉，模块名与提交宏在根上转出，
   插件侧 `symbio_plugin_api::submit_*!` 写法不变。同批下调 `FA-002`（门面合计 3496 → 2561），
   `CS-009` 钉到搬后实测 **596 : 255**（+44 字段 / +6 变体，全是换地址）。

   > **`reports` 与 `context` 不同形，搬前须先裁**：`context` 的闭包只碰 kernel 与 sdk 自己的
   > 类型，整块搬得动；`reports` 的读侧三函数（`read_view` / `read_view_with_ver` / `read_all`）
   > 还要调门面的 `sealed_declarations()`（`declarations.rs`）与 `ViewUnavailable`（`views.rs`），
   > 而 `sealed_declarations` 又依赖门面的 `events::SHARED_EVENTS`（清单生成物）与
   > `symbio_sdk::plugin::rows::row_writes()`。把它整块搬进 sdk 会逼出 `sdk → plugin-api` 的反向边（`D-258` 直接红）。
   > 因此 `reports` 不是「同一个闭包换地址」，而是**两个闭包**：注册表半（`ReportDecl` /
   > `submit_reports!` / `validate` / `reports()`，只碰 kernel，搬得动）与读侧半（依赖冻结声明表，
   > 搬不动）。怎么切要先裁，不在本批顺手造形状。
3. HS-002 / HS-006（`scripts/plugin-surface-audit.mjs`）仍只判「转发了几条」；
   「门面持有实现」这一面由 `FA-001` 判，不重复造第二臂。
4. 跑 `gate --full`，全绿后提交。
