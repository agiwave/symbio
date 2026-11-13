---
kind: design
id: crates/symbio-plugin-slo/README
title: 时延分层能力包（slo）
status: active
created: 2026-11-13
impl: done
---

# 时延分层能力包（slo）

按**派发档**折三列读数：时延、兜底率、时域跨度。名册 #13，口径见 `DE-016 §8.2` /
`DE-008 §3.1`。

## 它交什么

| 面 | 交出物 | 谁读 |
|---|---|---|
| 视图 | `slo.latency` / `slo.fallback` / `slo.span` | 任何按名字读的插件（`D-221`）；对外经 `submit_reports!` 登记 |
| 多片合并 | `ViewDecl::agg`：N 份单片读数 → 实例级分位数 / 率 / 中位数 | 实例级聚合列的读方 |
| 事实 / 工具 / 路由 / 配置 | **无** | — |

三列读的都是别人的格：`chat.assistant.final` 的 `cost_ms`（信封字段）、`user.message` 的
`tier`、`turn.dispatch` 的档位。所以 `PLUGIN.yml` 里没有 `shared_events`。

## 为什么三列同一个包

它们共用**同一条档位判据**（`declared_tier`）：一列按档折时延、一列按档折兜底率，分家就会
出现两份「这一轮属于哪一档」。档位不是这三条视图的私有词汇，它是派发事实的读法。

## 为什么折叠交两半

读方要两个基数上的同一个口径：**这一片**的分位数（单会话列）与 **N 片**的（实例级聚合）。
`eval` 交前者、`agg` 交后者，两半都由本包交、同经冻结声明表被调用。少了 `agg`，跨会话合并
只能在读方再折一遍——那是同一件事的第二份口径，而分位数的取法一旦分成两处，
「P99 是什么」就有两个答案。

未知档位进一个具名桶：**可观测，不静默归类**。

## 为什么它有 `Plugin` 实现却没有工具与路由

扩展面都走链接期收集，实例不参与任何判断；实例存在只为**挂载**——装配清单按插件目录挂载，
停用插件就该让三列视图从插件树里消失。`route` 恒 `NotFound`、`traverse` 恒空数组，
两者都不是占位符而是答案。

## 关联

- 时延分层的口径：`docs/003-design/016-工程落地与验证.md` §8.2
- 报表登记就是公开（无第二份视图名清单）：`crates/symbio-plugin-api/src/reports.rs`（`D-387`）
- 多片合并的裁决：`D-281`；同形状无第二读者的读数包：`crates/symbio-plugin-forecast/README.md`
