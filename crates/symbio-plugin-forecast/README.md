---
kind: design
id: crates/symbio-plugin-forecast/README
title: 未来预测能力包（forecast）
status: active
created: 2026-11-13
impl: done
---

# 未来预测能力包（forecast）

名册 #34「未来预测」的第一条**读数型**能力：把任务事实折成「还剩多少没完成」。
它是 `D-294` 首条准入能力的落地——只用读侧折叠这一格，`kernel` / `sdk` / `loop` /
`session` 一行都不动。

## 它交什么

| 面 | 交出物 | 谁用 |
|---|---|---|
| 视图 | `forecast.remaining` → 开过、仍未断言终态的任务集合 | **包内私有**：没有第二读者，名字不进报表登记 |
| 事实 / 工具 / 端口 / 路由 / 配置 | **无** | — |

视图读的是任务插件**公开**的两条事件（`task.opened` / `task.asserted`），不 import 任何
兄弟 crate：名字经 `PLUGIN.yml` 的 `shared_events` 投影进 `symbio_plugin_api::events`，
ownership 换了实现也不需要改这里。

## 口径

「还剩多少没完成」= **开过、仍未断言终态**。它不是任务表的第二份状态机：视图只做集合差，
终态判定仍归任务插件写侧——`task.asserted` 才是「验收通过」，本包不替任何事实关门
（`evidence` 为空）。

扫描成本上界是事件条数，必须与全仓同一个值（`seal` 的 `ViewCostMismatch`：切点只有一个）。

## 它为什么带着撤除路径

这一格只由一条能力撑着，所以账行里写了期限、触发条件与承接任务（`DE-026 §2.3` #34）：
能力退场时，视图与整个 crate 一起下线，不留「没人读的视图」。这是 `D-294` 准入协议的另一半
——**准入时就想好怎么退**。

## 关联

- 准入协议与移除计划：`docs/003-design/026-能力拆分与影响类.md` §2.3
- 读侧折叠的公开动作：`crates/symbio-plugin-api/src/views.rs`（`D-221`）
- 同形状的读数包（比本包多一个「有第二读者」的面）：`crates/symbio-plugin-slo/README.md`
