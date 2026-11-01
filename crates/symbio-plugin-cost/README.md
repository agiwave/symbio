---
kind: design
id: crates/symbio-plugin-cost/README
title: 成本能力包（成本台账折叠）
status: active
created: 2026-11-01
impl: done
---

# 成本能力包（cost）

一张视图、一条事实都不新增：`chat.assistant.final` 的 `cost_ms` 早就在写（写方是运行器与
模型边界），本包只交出**把它折成台账**的那条算式。名册 #18「成本控制」的能力包。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 视图 | `cost.ledger` → `{ by_principal: { <主体>: { principal, spent_ms, entries } }, total_ms }` | `loop` 的熔断闸门（`tools/tool_executor/breaker.rs`）与 `session` 的 `stats` 成本列 |
| 事实 | **无**——读的是信封字段 `cost_ms`，不是载荷 | 装载期 `reads` 因此为空 |
| 行为 / 工具 / 路由 / 配置 | **无** | — |

**两个读者、一个 owner**（D-044）：闸门问「这个主体在这个会话累计花了多少」，出口报同一份数。
两处各折一遍必然漂移成「闸门说还有余量、出口说已耗尽」，而错的若是台账，熔断就形同虚设。

## 视图的口径

- **按主体归账**：账记在事件的 `actor`（写出那条收束格的主体）上，不是按请求方摊——
  谁写下这条事实，这笔耗时就算谁的；
- `cost_ms == 0` 的事件**不成行**：桩与零延迟路径的诚实值就是 0，把它记成一笔会让 `entries`
  读出「花过钱而金额为零」的假账；
- `as_of`（`now`）之后的事件不入账：读数只到请求那一刻，重放同一切片得到同一个数（N1）；
- `total_ms` 是**全局**合计（所有主体相加），`by_principal` 才是分主体的——熔断读后者，面板两列都给。

## 缺席是什么形状

跨插件读按名字读，读口没有扫描上界（D-284）：调用方给多大的切片就折多大的。
读本包失败只有四种缺席态（`not_sealed` / `not_shared` / `unknown` / `not_aggregable`），
而两个读者各自的处置写在它们那一侧：闸门按**零账**继续并落一行 warn，出口交**空台账**
（`{ by_principal: {}, total_ms: 0 }`）。静默的 0 与有据的 0 在日志里分得开。

## 关联

- 逐行处置账（#18）：`docs/003-design/026-能力拆分与影响类.md` §2.3
- 迁移段与进度：`docs/007-task/011-共享折叠搬owner.md`
- 能力包的形状来源（夹具）：`../symbio-plugin-notes/README.md`
- 视图的跨插件读口：`crates/symbio-plugin-api/src/views.rs`
