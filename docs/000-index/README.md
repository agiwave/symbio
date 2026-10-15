---
kind: index
id: KIND-000-登记总账
title: 登记总账
status: active
projection: true
lists: [index-entries=owner]
created: 2026-10-14
prefix: KIND
label: 总账
desc: 跨 kind 的登记总账——各 kind 条目数 / 状态分布 / 分解边数
---

# 登记总账

这里是全仓登记机制的总出口：每个 kind 一行，条目数与状态分布一眼可见。各 kind 的形状由其目录 README.md 的 meta 自描述，本页只投影、不裁决。

<!-- registry:summary begin -->
<!-- list:index-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 条目各是一个单文档：条目类**在位存放**（文件即条目，kind 只来自 front-matter，目录不参与判定）；
> 各 kind 的形状由其 README meta 自描述；机制见 [PL-011 · 登记与汇总机制](../004-plan/011-登记与汇总机制.md "PL-011-登记与汇总机制")，字段规范见 [SP-001](../008-spec/001.md)。

| 类别 | 身份 | 条目数 | 状态分布 | 分解 | 汇总页 |
|---|---|---|---|---|---|
| [index · 总账](README.md "KIND-000-登记总账") | 编号前缀 `KIND-` | 0 | draft 0 · active 0 · retired 0 | — | [KIND-000-登记总账](README.md) |
| [goal · 目标](../001-goal/README.md "GO-000-目标汇总") | 编号前缀 `GO-` | 6 | draft 0 · active 6 · retired 0 | 3 条分解边 | [GO-000-目标汇总](../001-goal/README.md) |
| [scene · 场景](../002-scene/README.md "SC-000-场景汇总") | 编号前缀 `SC-` | 14 | draft 0 · active 14 · retired 0 | — | [SC-000-场景汇总](../002-scene/README.md) |
| [stage · 阶段](../003-stage/README.md "ST-000-阶段汇总") | 编号前缀 `ST-` | 14 | draft 0 · active 14 · retired 0 | — | [ST-000-阶段汇总](../003-stage/README.md) |
| [plan · 计划](../004-plan/README.md "PL-000-计划汇总") | 编号前缀 `PL-` | 12 | draft 0 · active 12 · retired 0 | — | [PL-000-计划汇总](../004-plan/README.md) |
| [arch · 架构](../005-arch/README.md "AR-000-架构汇总") | 编号前缀 `AR-` | 47 | draft 0 · active 47 · retired 0 | — | [AR-000-架构汇总](../005-arch/README.md) |
| [ref · 参考](../006-ref/README.md "RE-000-参考汇总") | 编号前缀 `RE-` | 7 | draft 0 · active 7 · retired 0 | — | [RE-000-参考汇总](../006-ref/README.md) |
| [guide · 指南](../007-guide/README.md "GU-000-指南汇总") | 编号前缀 `GU-` | 15 | draft 0 · active 15 · retired 0 | — | [GU-000-指南汇总](../007-guide/README.md) |
| [spec · 规范](../008-spec/README.md "SP-000-规范汇总") | 编号前缀 `SP-` | 31 | draft 0 · active 31 · retired 0 | — | [SP-000-规范汇总](../008-spec/README.md) |
| [task · 任务](../009-task/README.md "T-000-任务汇总") | 编号前缀 `T-` | 113 | todo 5 · doing 0 · done 108 · wont 0 | 3 条分解边 | [T-000-任务汇总](../009-task/README.md) |
| [adr · 架构决策](../010-adr/README.md "ADR-000-架构决策汇总") | 编号前缀 `ADR-` | 68 | active 58 · superseded 7 · withdrawn 3 | — | [ADR-000-架构决策汇总](../010-adr/README.md) |

<!-- registry:summary end -->

