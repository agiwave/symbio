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

> ⚠️ **本区块由 `node scripts/audits/registry-audit.mjs --write` 维护（标记之间），请勿手改标记区内内容。**
> 状态的唯一登记处是各条目文件本身，本区块是投影。字段规范见 [SP-001](001.md)，机制见 [DE-017 · 登记与汇总机制](../003-design/017-登记与汇总机制.md "DE-017-登记与汇总机制")。

**计数**：active 9 · draft 0 · retired 0（共 9）

## 现行规范（active）

| 编号 | 规范 | 适用范围 | 判据 | 清除 | 取代 |
|---|---|---|---|---|---|
| `SP-001` | 登记条目 front-matter 与状态规范 | 仓内全部活跃 `.md` 文档（kind 闭集由各目录 README meta 自注册；豁免名单见 `scripts/registry-lib.mjs`） | 人工 | — | — |
| `SP-003` | 提交纪律总册——顺序、中间态、原子批、e2e、架构必需、不做兼容、完成判据 | 全部代码与文档改动、批次划分与提交信息 | `scripts/audits/slo-exit-audit.mjs` | — | — |
| `SP-010` | 施工纪律总册——只写目标态、机制优先、不新增真相、出口可核对、豁免必有清除任务 | 全部活跃文档与源码注释、改进批次、批次出口与守卫豁免 | `scripts/audits/grep-audit.mjs` | — | — |
| `SP-015` | 死码承认台账——理由指名消费方 + 棘轮只许降 + 终局基线 8 | 全部 #[allow(dead_code)] 标记；owner = scripts/audits/dead-code-audit.mjs 的 R-002 段 | `scripts/audits/dead-code-audit.mjs` | — | — |
| `SP-016` | 出口口径总册——逐阶出口必须是用户可见的一列，CI 断言必须是可执行判据 | SLO 出口表、任何「这一阶长出来了」的声称、CI 断言清单与守卫脚本 | `scripts/audits/slo-exit-audit.mjs` | — | — |
| `SP-022` | 验证纪律总册——声称、断言、反向用例与编译期反例 | docs/003-design/verify/ 下的验证程序与任何「已证明」的声称 | `scripts/gate.d/55-verify.mjs` | — | — |
| `SP-028` | 判据豁免与存量登记——四处「该豁免存在」的登记与它们的清除任务 | doc-link D-004 全文豁免 / dead-code R-002 账本 / panic-allow 登记 / color 守卫自指豁免 | `scripts/audits/doc-link-audit.mjs` | — | — |
| `SP-032` | 设计文档的编号分区——0xx 系统级、1xx 组件级 | docs/003-design/ 的系统级设计，与就地存放的组件级设计（crates/**、tauri/docs、crates/symbio-cli/docs） | `scripts/audits/registry-audit.mjs` | — | — |
| `SP-033` | 控制面终态判据：六条不变量与判据的退场规则 | 控制面（义务环 / 派发面 / 驱动循环）的验收判据；不覆盖文档登记、插件入口、前端与 SLO 等其它判据命名空间 | `scripts/audits/control-plane-audit.test.mjs` | — | — |

## 草案（draft）

（无）

## 废止（retired）

（无）

<!-- registry:summary end -->

