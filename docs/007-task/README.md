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

**计数**：☐ todo 18 · 🟡 doing 2 · done 0 · wont 0（共 20）

## 待处理（按依赖顺序）

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-273-S04调度器与租约落地` | S04 调度器与租约落地——`task.ready` / `task.assigned` / `readyset:cap` / TTL 同批（D-064 重开） | now | impl | — |
| `T-274-记忆召回参数族与预算读方` | 记忆召回参数族——`layer` / `spread` / `decay` / `density` 各要先有值写方；巩固者预算接真读方 | next | impl | — |
| `T-275-语义记忆接线` | 语义记忆接线——`memory.encoded` 的 `vec` 写方与 `recall` 的语义维（D-110 前置兑现） | next | impl | — |
| `T-278-Agent权限四字段执行` | Agent 权限四字段执行面——network / fs.read / fs.write / limits 同批接入（D-102 重开） | next | impl | — |
| `T-280-骨架惰性重取` | 骨架惰性重取——`expand_skeleton` 动作按 DE-019 §6 落地（长会话折叠态的按需取子） | next | impl | — |
| `T-281-前端信任四件套` | 前端信任四件套——变更面板 / 工具授权分级 / 用量成本 / 分支切换（DE-020 §3 P2） | next | impl | — |
| `T-283-三条读数补建` | 三条读数补建——规范执行方覆盖率 / 异常注入矩阵 / 前端类型分支口径 | next | verify | — |
| `T-284-设计文档盘面对账` | 设计文档盘面对账——过期机制名、高估的端口面与漂移的注释（两份评估的 F/P2 项归口） | next | doc | — |
| `T-285-场景账本收口` | 场景账本收口——SC-014…018 五项能力逐项裁决「立项（带 impl 载体）或移除」 | next | plan | — |
| `T-298-步六插件可卸载判据` | 步 6——session / loop 的「可独立卸载」从一句话变成可复跑判据（DE-002 §4） | next | impl | — |
| `T-303-契约边界显形` | 义务环契约边界显形——租约收租窗口的上界用例与「事后写封闭」的契约条文 | next | verify | — |
| `T-282-旁路直调归零` | 旁路基线拆两类——5 处「第二套调度」归零，3 处「幂等点问」显式承认；策略行接线各带生产者证据 | later | impl | — |
| `T-299-冗余清偿批` | 冗余清偿批（sprint）：过渡双写、退场矛盾与生成物口径 | later | impl | — |
| `T-301-DE014决策锚点归位` | DE-014 决策锚点归位——13 条 D-064…D-128 各有归属，「retired 却承载生效裁决」矛盾消除 | later | doc | — |
| `T-300-role双写清偿` | `ChatNode.role` 双写清偿——先写末档与分档，再降基线（146 → 目标值） | later | impl | T-301-DE014决策锚点归位 |
| `T-302-生成物口径` | 生成物口径——`routes.gen.ts` 与 `messageFacts.ts` 回到「有消费方」或「显式承认」二态 | later | impl | — |
| `T-304-过渡豁免与兼容分支裁决` | 过渡豁免与前端兼容分支的显式裁决——`ActorEntry.transitional` 与四处前端兼容层逐条定去留 | later | plan | — |
| `T-306-可见挂载裁剪接线` | 可见挂载裁剪接线——`capability_mounts` 的读侧消费方，或删掉这个读口 | later | impl | — |

## 进行中

| 编号 | 任务 | 优先 | 轨 | 依赖 |
|---|---|---|---|---|
| `T-272-设计实现总收口批` | 设计实现总收口批（sprint）：判据面归位、能力缺口逐阶收口与读数补建 | now | plan | — |
| `T-295-架构迁移收尾批` | 架构迁移收尾批（sprint）：DE-090 §1 六步的账实对齐与最后两步 | next | plan | — |

## 分解树

分解关系只写在 `parent` 字段（子条目指向父条目、同 kind；编号平铺不体现层级——层级是投影不是身份）。
同状态的父子在上面的分组表里相邻；本小节是跨状态的分解全景。

- [`T-272-设计实现总收口批`](272-设计实现总收口批.md) 设计实现总收口批（sprint）：判据面归位、能力缺口逐阶收口与读数补建（doing） · 子项 todo 11
  - [`T-273-S04调度器与租约落地`](273-S04调度器与租约落地.md) S04 调度器与租约落地——`task.ready` / `task.assigned` / `readyset:cap` / TTL 同批（D-064 重开）（todo）
  - [`T-274-记忆召回参数族与预算读方`](274-记忆召回参数族与预算读方.md) 记忆召回参数族——`layer` / `spread` / `decay` / `density` 各要先有值写方；巩固者预算接真读方（todo）
  - [`T-275-语义记忆接线`](275-语义记忆接线.md) 语义记忆接线——`memory.encoded` 的 `vec` 写方与 `recall` 的语义维（D-110 前置兑现）（todo）
  - [`T-278-Agent权限四字段执行`](278-Agent权限四字段执行.md) Agent 权限四字段执行面——network / fs.read / fs.write / limits 同批接入（D-102 重开）（todo）
  - [`T-280-骨架惰性重取`](280-骨架惰性重取.md) 骨架惰性重取——`expand_skeleton` 动作按 DE-019 §6 落地（长会话折叠态的按需取子）（todo）
  - [`T-281-前端信任四件套`](281-前端信任四件套.md) 前端信任四件套——变更面板 / 工具授权分级 / 用量成本 / 分支切换（DE-020 §3 P2）（todo）
  - [`T-282-旁路直调归零`](282-旁路直调归零.md) 旁路基线拆两类——5 处「第二套调度」归零，3 处「幂等点问」显式承认；策略行接线各带生产者证据（todo）
  - [`T-283-三条读数补建`](283-三条读数补建.md) 三条读数补建——规范执行方覆盖率 / 异常注入矩阵 / 前端类型分支口径（todo）
  - [`T-284-设计文档盘面对账`](284-设计文档盘面对账.md) 设计文档盘面对账——过期机制名、高估的端口面与漂移的注释（两份评估的 F/P2 项归口）（todo）
  - [`T-285-场景账本收口`](285-场景账本收口.md) 场景账本收口——SC-014…018 五项能力逐项裁决「立项（带 impl 载体）或移除」（todo）
  - [`T-306-可见挂载裁剪接线`](306-可见挂载裁剪接线.md) 可见挂载裁剪接线——`capability_mounts` 的读侧消费方，或删掉这个读口（todo）
- [`T-295-架构迁移收尾批`](295-架构迁移收尾批.md) 架构迁移收尾批（sprint）：DE-090 §1 六步的账实对齐与最后两步（doing） · 子项 todo 2
  - [`T-298-步六插件可卸载判据`](298-步六插件可卸载判据.md) 步 6——session / loop 的「可独立卸载」从一句话变成可复跑判据（DE-002 §4）（todo）
  - [`T-303-契约边界显形`](303-契约边界显形.md) 义务环契约边界显形——租约收租窗口的上界用例与「事后写封闭」的契约条文（todo）
- [`T-299-冗余清偿批`](299-冗余清偿批.md) 冗余清偿批（sprint）：过渡双写、退场矛盾与生成物口径（todo） · 子项 todo 4
  - [`T-300-role双写清偿`](300-role双写清偿.md) `ChatNode.role` 双写清偿——先写末档与分档，再降基线（146 → 目标值）（todo）
  - [`T-301-DE014决策锚点归位`](301-DE014决策锚点归位.md) DE-014 决策锚点归位——13 条 D-064…D-128 各有归属，「retired 却承载生效裁决」矛盾消除（todo）
  - [`T-302-生成物口径`](302-生成物口径.md) 生成物口径——`routes.gen.ts` 与 `messageFacts.ts` 回到「有消费方」或「显式承认」二态（todo）
  - [`T-304-过渡豁免与兼容分支裁决`](304-过渡豁免与兼容分支裁决.md) 过渡豁免与前端兼容分支的显式裁决——`ActorEntry.transitional` 与四处前端兼容层逐条定去留（todo）

<!-- registry:summary end -->
