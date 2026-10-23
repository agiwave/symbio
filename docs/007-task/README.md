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

**计数**：☐ todo 6 · 🟡 doing 1 · done 3 · wont 0（共 10）

## 待处理（按依赖顺序）

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-259-节号与裸引用判据` | 节号连续性与裸 §N.M 引用可解析——一条判据钉住 DE-001 缺 §13 与 DE-016 重复编号 | next | impl | T-257-DE015章节重整 |
| `T-260-旁路直调计数棘轮` | 旁路直调 owes() 有 13 处、drive 只驱动 6/16 行——「收口完成」需要一个可判的定义 | next | impl | — |
| `T-261-await行语义裁决` | await-model / await-tools 从未被问过——是 Wait 折叠的声明位，还是行表里的装饰行 | next | plan | — |
| `T-254-轮外压缩重试不入账` | 轮外压缩重试（RetryCompaction）的那一笔摘要往返不入账——无轮可挂的模型调用怎么记账 | later | plan | — |
| `T-262-实测补证五项` | CLI 实测只覆盖了「已接线的 6 行」——压缩真发生、召回命中、拒绝、租约判死、轮外重试五项无证据 | next | verify | T-254-轮外压缩重试不入账 |
| `T-263-场景判据缺口对账` | 18 篇场景的「诚实账」：32 条有证据、34 条缺口，其中已承诺能力占 21 条 | later | plan | — |

## 进行中

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-255-义务环收口批` | 义务环收口批——设计文档整备、旁路收编与实测补证 | now | plan | — |

## 分解树

分解关系只写在 `parent` 字段（子条目指向父条目、同 kind；编号平铺不体现层级——层级是投影不是身份）。
同状态的父子在上面的分组表里相邻；本小节是跨状态的分解全景。

- [`T-255-义务环收口批`](255-义务环收口批.md) 义务环收口批——设计文档整备、旁路收编与实测补证（doing） · 子项 todo 6 / done 3
  - [`T-254-轮外压缩重试不入账`](254-轮外压缩重试不入账.md) 轮外压缩重试（RetryCompaction）的那一笔摘要往返不入账——无轮可挂的模型调用怎么记账（todo）
  - [`T-256-CLI产物路径自证`](256-CLI产物路径自证.md) 构建脚本打印的 CLI 产物路径必须自证存在——不许再说一条本机不存在的路径（done）
  - [`T-257-DE015章节重整`](257-DE015章节重整.md) DE-015 章节重整——两个 §2、两个 §3、缺 §5/§7，5 处裸引用全悬空（done）
  - [`T-258-DE015状态与破句`](258-DE015状态与破句.md) DE-015 §8 状态表 5 项「剩余」里 4 项已完成，另有 6 处断句是批量清 T- 编号的副作用（done）
  - [`T-259-节号与裸引用判据`](259-节号与裸引用判据.md) 节号连续性与裸 §N.M 引用可解析——一条判据钉住 DE-001 缺 §13 与 DE-016 重复编号（todo）
  - [`T-260-旁路直调计数棘轮`](260-旁路直调计数棘轮.md) 旁路直调 owes() 有 13 处、drive 只驱动 6/16 行——「收口完成」需要一个可判的定义（todo）
  - [`T-261-await行语义裁决`](261-await行语义裁决.md) await-model / await-tools 从未被问过——是 Wait 折叠的声明位，还是行表里的装饰行（todo）
  - [`T-262-实测补证五项`](262-实测补证五项.md) CLI 实测只覆盖了「已接线的 6 行」——压缩真发生、召回命中、拒绝、租约判死、轮外重试五项无证据（todo）
  - [`T-263-场景判据缺口对账`](263-场景判据缺口对账.md) 18 篇场景的「诚实账」：32 条有证据、34 条缺口，其中已承诺能力占 21 条（todo）

<!-- registry:summary end -->
