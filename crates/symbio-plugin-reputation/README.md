---
kind: design
id: crates/symbio-plugin-reputation/README
title: 声誉能力包（声誉台账折叠）
status: active
created: 2026-11-02
impl: done
---

# 声誉能力包（reputation）

一张视图、一条事实都不新增：`commitment.*` 四格早就在写（写方是效果通路与 `agent` 的立约
效果，格位的 owner 是 `loop`），本包只交出**把其中三条折成声誉表**的那条算式。名册 `#22`
「信任账本」与 `#28`「声誉与互信」是同一条折叠的两个名字，共用这一个口径（`D-288`）。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 视图 | `reputation.ledger` → `{ by_principal: { <主体>: { principal, offered, kept, broken, score } } }` | `loop` 的淘汰回路（`derived_facts::record_elimination`）与 `session` 的 `stats` 声誉列 |
| 事实 | **无**——读的是 `loop` 那一族的载荷字段，本包不产格 | 装载期这四对读法落在断言格通道（`D-282`） |
| 行为 / 工具 / 路由 / 配置 | **无** | — |

**两个读者、一个 owner**（D-044）：回路问「这个主体的分低到该收回派活权没有」，出口报
同一份数。两处各折一遍必然漂移成「回路说这一行负分、面板说它守约」，而收回权威的动作
建立在一份没人复核的数上。

## 视图的口径

- **账记在立约方头上**：取 `commitment.opened` 的载荷 `from`，不是事件写出方（`actor`）——
  委托由父主体发起、由子主体执行，声誉要记在「谁许的诺」那一个身上；
- **配对靠载荷 `id`**：收束格经 `id` 找回立约方，`from` 是立约时的事实、收束时不重抄；
  认不出的了结**跳过**而不猜「大概履行了吧」（与 `commitment_records` 同一条 fail-closed）；
- **自报不进账**：`commitment.asserted`（主体宣告自己守约）不是收束格，永远改变不了数——
  声誉的定义就是「可被别人观测的那一份」（DE-038 §2）；
- `as_of`（`now`）之后的立约与收束都不入账：未来的了结不影响当前声誉，重放同一切片得到
  同一个数（N1）；
- `score = 守约 − 违约` 是**平凡值**不是架构：换算法改那一处，不加机制。阈值与判决不在
  本包——那是读方（淘汰回路）的词汇（DE-043）。

## 缺席是什么形状

跨插件读按名字读，读口没有扫描上界（D-284）：调用方给多大的切片就折多大的。
读本包失败只有四种缺席态（`not_sealed` / `not_shared` / `unknown` / `not_aggregable`），
而两个读者各自的处置写在它们那一侧：**回路按不淘汰继续**并落一行 warn（读不出一份账
不该成为收回权威的理由，fail-safe 与 fail-closed 在这里同向——没数就不动 grants），
出口交**文档写的空形状**并出声。静默的 0 与有据的 0 在日志里分得开。

## 关联

- 逐行处置账（#22 / #28）：`docs/003-design/026-能力拆分与影响类.md` §2.3
- 折叠搬 owner 的裁决：`docs/003-design/045-能力包迁出与落地裁决.md`（`D-287` 判定先收纯分数、`D-288` 本包落地）
- 只读折叠包的合法性（先例）：`../symbio-plugin-cost/README.md`
- 承诺域的形状来源：`docs/003-design/038-多主体与对等承诺.md`
- 视图的跨插件读口：`crates/symbio-plugin-api/src/views.rs`
