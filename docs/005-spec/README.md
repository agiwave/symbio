---
kind: spec
id: SP-000-规范汇总
title: 规范总览
status: active
projection: true
lists: [spec-entries=owner]
created: 2026-10-14
prefix: SP
label: 规范
desc: 必须成立的条文——机制与判据的规范面

titleEqualsH1: false
required: [scope]
optional: [reason, supersedes, clears]
sections: [规范, 判据]
refSections: [判据]
clearsTarget: task
closedStatus: active
requiresDepsDone: [active]
retiredStates: [retired]
status.draft:
status.active:
status.draft.forbid: [reason]
status.active.forbid: [reason]
status.retired.require: [reason]
summary.order: [active, draft, retired]
summary.active.title: 现行规范（active）
summary.active.columns: [id, title:规范, scope, guard, clears, supersedes]
summary.draft.title: 草案（draft）
summary.draft.columns: [id, title:规范, scope, supersedes]
summary.retired.title: 废止（retired）
summary.retired.columns: [id, title:规范, reason, supersedes]
board.columns: [scope]
---

# 规范总览

规范是「必须成立的条文」：每条有规范小节（条文）与判据小节（怎么验证）。取代链（supersedes）显式记录冲突如何消解，豁免挂在清除任务上（clears）。

<!-- registry:summary begin -->
<!-- list:spec-entries owner -->

> ⚠️ **本区块由 `node scripts/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：active 27 · draft 0 · retired 5（共 32）

## 现行规范（active）

| 编号 | 规范 | 适用范围 | 判据 | 清除 | 取代 |
|---|---|---|---|---|---|
| `SP-001` | 登记条目 front-matter 与状态规范 | 仓内全部活跃 `.md` 文档（kind 闭集由各目录 README meta 自注册；豁免名单见 plan/14 §0） | 人工 | — | — |
| `SP-003` | 提交纪律 1——实现 → 单测 → e2e → 全部门禁，绿才提交 | 全部代码与文档改动 | `scripts/gate.mjs` | — | — |
| `SP-004` | 提交纪律 1b——跨批行为变更的中间态提交 | 跨多批才能完成的行为变更（中间态必然有红点） | `scripts/check-commit-msg.mjs` | — | — |
| `SP-005` | 提交纪律 1c——原子批次：多开关必须同翻 | 任一真子集观测值恒零的开关组 | 人工 | — | — |
| `SP-006` | 提交纪律 2——每一批新增一条 e2e，且跑出厂默认路径 | 全部行为批次的 e2e 用例 | `scripts/gate.d/58-e2e.mjs` | — | — |
| `SP-007` | 提交纪律 3——架构改动必须逐条论证「必需」 | 全部架构改动（新字段 / 新 kind / 新机制元素 / 放宽既有形状） | `scripts/core-shape-audit.mjs` | — | — |
| `SP-008` | 提交纪律 4——不做历史数据兼容 | 全部数据形状与协议形状的改动 | `scripts/schema-audit.mjs` | — | — |
| `SP-009` | 提交纪律 5——完成判据必须含一条端到端判据 | 全部批次的完成判据 | `scripts/gate.d/58-e2e.mjs` | — | — |
| `SP-010` | 纪律 1（13 §0）——只写目标态：注释只写不变量与陷阱，不写过程 | 全部活跃文档与源码注释（条文原属已归档的 plan/13 §0，随归档迁入本条目） | `scripts/grep-audit.mjs` | — | — |
| `SP-011` | 纪律 2（13 §0）——机制优先：每条问题落成守卫 / 真源 / 类型约束 | 全部改进批次与问题登记（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-012` | 纪律 3（13 §0）——不新增真相：改文档前先确认那条事实的 owner | 全部文档改动（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-013` | 纪律 4（13 §0）——出口可核对：判据要么是守卫，要么是脚本可核对的锚 | 全部任务的 exit 字段与批次出口（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-014` | 纪律 5（13 §0）——豁免必有清除任务：无清除任务的豁免不许登记 | 守卫豁免注释 / 豁免账本条目（条文原属已归档的 plan/13 §0，随归档迁入本条目） | 人工 | — | — |
| `SP-015` | 死码承认台账——承认挂计划步 + 棘轮只许降 + 终局基线 8 | 全部 #[allow(dead_code)] 标记；owner = scripts/dead-code-audit.mjs 的 R-002 段 | `scripts/dead-code-audit.mjs` | — | — |
| `SP-016` | 逐阶出口的口径——用户可见出口必须是 session/stats 真存在的一列 | SLO 出口表与任何「这一阶长出来了」的声称 | `scripts/slo-exit-audit.mjs` | — | — |
| `SP-017` | CI 断言的口径——每条断言必须落成一条可执行判据 | CI 断言清单与守卫脚本 | `scripts/no-direct-call-audit.mjs` | — | — |
| `SP-022` | 验证纪律 1——每条声称对应一条可执行断言 | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/55-verify.mjs` | — | — |
| `SP-023` | 验证纪律 2——断言的输入不得包含结论 | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/_shared.mjs` | — | — |
| `SP-024` | 验证纪律 3——每条断言配反向用例（改输入，结论必须变） | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/55-verify.mjs` | — | — |
| `SP-025` | 验证纪律 4——声称「编译期强制」必须附一段已知编译不过的代码 | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/55-verify.mjs` | — | — |
| `SP-026` | 验证纪律 5——编译期反例必须写在外部模块里 | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/55-verify.mjs` | — | — |
| `SP-027` | C-003 根出口单消费方豁免账本（5 条） | scripts/core-export-audit.mjs 的 WAIVERS 账本（ActorSpec / turnstate / SloLatencyView / FallbackRateView / mutations_latest_snapshot） | `scripts/core-export-audit.mjs` | — | — |
| `SP-028` | doc-link D-004 全文豁免（定义变更史禁区的两处反例措辞） | docs/archive/13-质量改进总纲.md 首行与 docs/README.md 首行的 doc-link-allow D-004 | `scripts/doc-link-audit.mjs` | — | — |
| `SP-029` | dead-code R-002 契约登记存量（8 处） | scripts/dead-code-audit.mjs 的 R-002 死码承认账本（终局基线 8，见 docs/003-design/016-工程落地与验证.md §3.1） | `scripts/dead-code-audit.mjs` | — | — |
| `SP-030` | panic-allow 不变量登记（C7，56 个登记项 / 71 处 panic 面） | crates/crates/** 的 panic-allow 注释、scripts/panic-audit.mjs 账本与 scripts/_shared.mjs 的 BASELINE.panicSites | `scripts/panic-audit.mjs` | — | — |
| `SP-031` | color 守卫自指豁免（测试与实现提到禁用模式） | scripts/color.test.mjs（守卫回归与实现里的禁用模式字样） | `scripts/color.test.mjs` | — | — |
| `SP-032` | 设计文档的编号分区——0xx 系统级、1xx 组件级 | docs/003-design/ 的系统级设计，与就地存放的组件级设计（crates/crates/**、tauri/docs、cli/docs） | `scripts/registry-audit.mjs` | — | — |

## 草案（draft）

（无）

## 废止（retired）

| 编号 | 规范 | 理由 | 取代 |
|---|---|---|---|
| `SP-002` | 决策的域分配（已废止）——adr kind 已废除，决策折入设计文档 | adr kind 已废除：决策折进所属设计文档的「关键决策」节，domain 字段不再存在 | — |
| `SP-018` | 验证纪律（旧编号）——已被 SP-022…SP-026 取代 | 同一套纪律的浓缩版；按「一个事实一个 owner」保留全表，本套退场 | — |
| `SP-019` | 验证纪律（旧编号）——已被 SP-022…SP-026 取代 | 同一套纪律的浓缩版；按「一个事实一个 owner」保留全表，本套退场 | — |
| `SP-020` | 验证纪律（旧编号）——已被 SP-022…SP-026 取代 | 同一套纪律的浓缩版；按「一个事实一个 owner」保留全表，本套退场 | — |
| `SP-021` | 验证纪律（旧编号）——已被 SP-022…SP-026 取代 | 同一套纪律的浓缩版；按「一个事实一个 owner」保留全表，本套退场 | — |

<!-- registry:summary end -->

