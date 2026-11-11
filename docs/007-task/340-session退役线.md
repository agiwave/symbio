---
kind: task
id: T-340-session退役线
title: session 退役线
status: doing
track: arch
prio: now
created: 2026-11-11
---

# session 退役线

## 内容

把控制面 `symbio-plugin-session` 的退役从「八条决策里的一句『随 session 退役同批删除』」
变成**有范围、有出口、有进度尺**的一条线。裁决是
[DE-054-判据欠账与到期豁免](../003-design/054-判据欠账与到期豁免.md) 的 `D-451`。

终态不是「删三万行」，而是三条边依次消失，每条只在上一条闭合后才开（表见 D-451 §2）：

1. **边一**：`session → sdk` 的符号面（豁免账 101 条 → 0）——**已校准，可跑**。
2. **边二**：装配（`plugin/turn.rs`）——**未界定**，入场券是「装配的消费方是谁、
   为什么它不能是一张声明表」。
3. **边三**：存储（`store/` 的 WAL 与转写）——**未界定**，入场券是把 `D-081` 的三问
   （停用 / 收敛 / 存储读写照常）在存储侧逐条写成判据。

边二 / 边三 **不登记子任务**：写不出入场券就登记，得到的是「以后再说」的假计划
（D-451 §3）。

where: `scripts/session-retire-audit.mjs`、`scripts/exemptions/plugin-session-sdk.json`、
`crates/symbio-plugin-session/`。

exit: 三条边依次闭合；`scripts/exemptions/plugin-session-sdk.json` 删除且
`session-retire-audit` 随它一起退役（名单同批撤下，不留空转的守卫）；
`crates/symbio-plugin-session/` 或其残余空壳删除。

## 处理过程

- 2026-11-11：**这一轮只做了「让这条线成立」**——定义三条边与各阶出口（D-451）、
  校准进度尺（豁免账 116 → 101，摘掉 15 条零命中余量）、落守卫
  `session-retire-audit`（SR-001 余量即红 / SR-002 只许降 / SR-003 豁免自洽，
  回归 10 条，已接进 `30-docs.mjs` 的 `GUARDS`）。**边一一条都还没收窄**——
  101 是分母不是分子。
- 2026-11-11（同日晚些）：**边一开出第一格**——豁免账 101 → 100，`PromptMessage` 退场
  （D-452：判据的入参面不许比它的比较面宽）。进度尺基线同批下调，三码仍绿。
  这是「分子动了 1」，不是「校准」；边二 / 边三 仍未开。
- 子任务见 `T-341-收窄session的sdk依赖边`。边二 / 边三 待入场券写得出再开。
