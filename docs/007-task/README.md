---
kind: task
id: T-000-任务汇总
title: 任务总览
status: active
projection: true
lists: [task-entries=owner]
created: 2026-10-14
prefix: T
label: 任务
desc: 要做什么、做到哪一步——可勾销的最小工作单元，收口即删（只留 git 历史）

titleEqualsH1: false
required: [track]
optional: [where, exit, signal, evidence, reason, prio]
sections: [内容, 处理过程]
refFields: [exit]
closedStatus: done
requiresDepsDone: [doing, done]
retiredStates: [done, wont]
status.todo:
status.doing:
status.todo.forbid: [evidence, reason]
status.doing.forbid: [evidence, reason]
status.done.require: [evidence]
status.done.forbid: [signal, reason]
status.wont.require: [reason]
status.wont.forbid: [evidence]
summary.order: topo
summary.todo.title: 待处理（按依赖顺序）
summary.todo.columns: [id, title:任务, prio, track, deps]
summary.todo.glyph: ☐
summary.doing.title: 进行中
summary.doing.columns: [id, title:任务, prio, track, deps]
summary.doing.glyph: 🟡
board.columns: [rank, deps]
---

# 任务总览

任务是可勾销的最小工作单元：`todo → doing → done/wont` 一条状态轴，done 必须有证据（evidence）、
wont 必须有理由（reason）。分解（parent）只在任务之间，跨 kind 关联走 deps。

## 收口即删

**`done` / `wont` 是过渡档，不是归档档**：一个任务勾销时，先把结论落进它该去的地方——
落地的形状由设计文档的 `impl` 档位与「关键决策」承载，「不做」的裁决落成一条 `### D-NNN`——
然后**删掉这个文件**。任务只留 git 历史，不占活跃文档的位置。

理由：任务记的是「当时撞上了什么、怎么收的」，那是过程信息；活跃文档只放项目的**当前与最终形态**。
一份两百多行「已完成」的总账，读它的人得不到任何关于「现在是什么样」的信息，却要每次都翻过去。

因此有两条纪律：

1. **非任务文档与源码注释不得引用 `T-` 编号**（[D-014](../README.md "docs/README")）——过程文档一删，
   引用就悬空；要指「为什么这样设计」，指设计文档的 `### D-NNN`。
2. **收口时先落结论再删文件**：结论没落进设计文档就删，等于把做过的判断一起丢掉。
   `done` 要求 `evidence`（落在哪、验过没有）、`wont` 要求 `reason`（为什么不做）——
   这两条字段就是**删除前的门槛**。

**判据（`RG-031`）**：`docs/007-task/` 里 `done` / `wont` 的条目数 **≠ 0 即红**——
「收口即删」不再是靠自觉的纪律，而是 `registry-audit` 判的终态：只认「结论已落 + 文件已删」。
判据的形状不点名 kind：哪个 kind 把 `closedStatus` 也写进退场档，哪个 kind 就受这条约束
（文档类的 `retired` 是在位归档，编号即路径、被其它文档引用着，不在其中）。
删文件时若有条目的 `deps` 指向它，**同一提交里把那条边摘掉**——闭环的编号不再是前置。

## 任务分解的推荐形态（SPRINT 任务）

大目标不直接平铺成一批小任务，而是**先立一个父任务，再往下分解**——`parent` 字段是父子分解树的边
（RG-024 / RG-025：同 kind、不自指、不成环、父子状态档位单调），可多级：

- **SPRINT 任务**：一个迭代批次。一批同目标的任务立一个 SPRINT 父任务（如「T-xxx 批」），
  批内任务挂它名下，批闭则父任务随子任务一起收口删除。
- **何时分解**：一个任务写不下 exit、或要分多次提交推进时，就拆父任务 + 子任务；
  能一个任务说清的不要硬造层级。

<!-- registry:summary begin -->
<!-- list:task-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](../005-spec/001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：☐ todo 6 · 🟡 doing 2 · done 0 · wont 0（共 8）

## 待处理（按依赖顺序）

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-007-loop存量能力迁移` | S6：loop 存量能力逐批迁出 | next | arch | — |
| `T-008-session存量能力迁移` | S7：session 存量能力逐批迁出 | next | arch | — |
| `T-011-共享折叠搬owner` | S8：共享折叠搬 owner——第二读者改名字读 | next | arch | — |
| `T-012-A类能力插件化` | S9：A 类十条能力插件化——不用改架构的正向验证 | later | arch | — |
| `T-009-拆分终态验收` | S11：终态验收——基线清空、账本收口与判据全绿 | later | arch | T-007-loop存量能力迁移、T-008-session存量能力迁移、T-011-共享折叠搬owner、T-012-A类能力插件化 |
| `T-015-求解器表达力` | 求解器表达力：owes 的合取与跨源键绑定 | later | arch | — |

## 进行中

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-001-能力全面拆分` | SPRINT：能力全面拆分与扩展性验证 | now | arch | — |
| `T-014-引擎收口` | 引擎收口：账本、三条收窄与求解器表达力裁决 | next | arch | — |

## 分解树

分解关系只写在 `parent` 字段（子条目指向父条目、同 kind；编号平铺不体现层级——层级是投影不是身份）。
同状态的父子在上面的分组表里相邻；本小节是跨状态的分解全景。

- [`T-001-能力全面拆分`](001-能力全面拆分.md) SPRINT：能力全面拆分与扩展性验证（doing） · 子项 todo 5
  - [`T-007-loop存量能力迁移`](007-loop存量能力迁移.md) S6：loop 存量能力逐批迁出（todo）
  - [`T-008-session存量能力迁移`](008-session存量能力迁移.md) S7：session 存量能力逐批迁出（todo）
  - [`T-009-拆分终态验收`](009-拆分终态验收.md) S11：终态验收——基线清空、账本收口与判据全绿（todo）
  - [`T-011-共享折叠搬owner`](011-共享折叠搬owner.md) S8：共享折叠搬 owner——第二读者改名字读（todo）
  - [`T-012-A类能力插件化`](012-A类能力插件化.md) S9：A 类十条能力插件化——不用改架构的正向验证（todo）
- [`T-014-引擎收口`](014-引擎收口.md) 引擎收口：账本、三条收窄与求解器表达力裁决（doing）
- [`T-015-求解器表达力`](015-求解器表达力.md) 求解器表达力：owes 的合取与跨源键绑定（todo）

<!-- registry:summary end -->
