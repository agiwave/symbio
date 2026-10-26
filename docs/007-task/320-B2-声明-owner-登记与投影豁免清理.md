---
kind: task
id: T-320-B2-声明-owner-登记与投影豁免清理
title: Phase B2：非协议声明全 owner-local，登记 owner，清投影类 C-003 豁免
status: todo
created: 2026-10-26
track: impl
prio: next
deps: [T-319-B1-单消费方投影下沉试点]
parent: T-316-内核边界与扩展局部性收口
where: symbio/src/symbio_core/decl/ · symbio/src/plugins/loop/ · scripts/core-export-audit.mjs · docs/
exit: '核验并补齐：`BUILTIN_*` 的非协议部分为零；7 个控制面视图均为 owner 声明；manifest 逐条记录视图 / 事实 / 类型的 owner；C-003 的投影类豁免清零，存量只降不升。'
---

## 内容

1. **核验**：`decl` 内核只预置协议五格（`kernel_declares_only_the_protocol_cells` 已是硬证据）；
   控制面 7 视图已由 `plugins::loop::control_decl` 声明——本任务先证明，再补缺口。
2. **owner 登记**：每个视图 / 事实 / 共享类型在 manifest 里写「声明方 → 消费方」；
   owner 由「谁交进 `Builder`」确定，不给 `ViewDecl` 加字段。
3. **豁免清理**：C-003 的投影类豁免随 B1 / A2 的作用域收窄逐条退役；
   剩下的「接缝」类（`ActorSpec`、协议键）保留并写明理由。

## 处理过程

- 2026-10-26：立任务。声明侧迁移的先行业已完成（控制面视图），本任务以核验与登记为主。
