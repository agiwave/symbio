---
kind: task
id: T-018-session协议面的Turn收编
title: 裁决：session 对外协议面的 `Turn`——留、退，还是只留在读法里
status: todo
created: 2026-10-29
track: protocol
prio: later
deps: []
where: docs/003-design/012-会话输入与地址.md、docs/003-design/015-义务环.md、crates/symbio-plugin-session/src/declarations.rs、crates/symbio-plugin-session/src/commands.rs、crates/symbio-plugin-session/src/projections
exit: 一条 `D-nnn` 落在 [DE-012-会话输入与地址](../003-design/012-会话输入与地址.md) 或 [DE-015-义务环](../003-design/015-义务环.md) 的关键决策节，回答三问：`turn.branch_selected`（`declarations.rs:37`）与 `Entity::Turn` 在会话事件族里**留还是退**；以 `Entity::Turn` 为轴的投影（`projections/fallback.rs:102` 等）与 transcript / stats 的轮次分组**跟谁**；`D-129`「`Turn` 已从协议退役」与代码现状（`commands.rs:485` 仍落 `Entity::Turn`）的冲突**以哪一侧为准**。裁决为「退」⇒ 把施工触点与判据拆成新任务并登记；裁决为「留」⇒ 改写 `D-129` 的表述，写明「退役」指的是终态配对还是整个事件族，使条文与代码不再互相否定。本条自己不动代码
---

## 内容

### 为什么它悬着

`D-129`（`DE-015`）的原话是「`Turn` 概念已从协议里退役、会话面统一按 `ChatNode` 父子树呈现」，
并明说「在退役概念上立判据等于给它续命，代价是两套终态并存」。

代码是另一侧的形状：session 插件仍以 `Entity::Turn` 声明 `turn.branch_selected`
（`declarations.rs:37`），`commands.rs:485` 写它，`projections/fallback.rs` 按它过滤，
stats / slo / tiers / span 与 transcript 分组也各自以轮次编号为轴。两者都是事实，
没有一条裁决说这一族留还是退——`D-129` 说的是「终态不按 Turn 配对补」，不是「事件族退场」。

这不是 `T-013` 的 `R8`：那一项管的是 loop 插件**内部**的 `TurnState`×2 / `TurnOutcome`×3；
这里是**跨插件的协议面**，改它要动事件声明、投影与前端读法，量级不同。

### 为什么不是直接登记一个实现任务

`SP-003` §3 纪律 0：架构改动只认方案（裁决），「实现困难」不是改架构的理由，反过来
「实现不难」也不是跳过裁决的理由。两侧都能自洽：

- **留**：会话面仍需要「这一轮」的粗身份——切换器选支、统计、分组、SLO 窗口都以它为轴；
- **退**：与 `D-129` 的目标一致，会话面统一到骨架节点，代价是上述读法各要改一遍。

选哪一侧是裁决，不是施工顺序。裁决之前开工，做出来的必然是「断言当前形状」的改动——
正是 `SP-033` 的 `J0` 要剪掉的那一类。

### 不做什么

- **不动 loop 插件内部的 `Turn*` 名字**：`T-001` 的边界写明「名字只是症状」，且 `R8` 只收同名不同义。
- **不动 session 的任务壳**（`TurnLauncher` / watchdog / `TurnOutcome` 三值）：那是产品词汇，
  不在这条射程。

## 处理过程

- 待开工。前置：无（裁决可以独立成立），但读它之前先看 `T-013` 的 `R8` 结论——
  先把 loop 内部的同名口径写清，两处才不会混谈。
- 裁决要回答的输入（全部可静态取）：`grep Entity::Turn crates/symbio-plugin-session` 的全部写方与读方；
  `turn.branch_selected` 的消费面（`vdfs_messages.rs` 的读法改写）；transcript / stats 的轮次轴
  能否由骨架节点直接替代（父节点 + 批序）。
- 产出：一条 `D-nnn` + 一句「施工在哪」（拆任务）或「不做的理由」（登记 `wont`，收口即删）。
