---
kind: task
id: T-341-收窄session的sdk依赖边
title: 收窄 session 的 sdk 依赖边
status: todo
track: arch
prio: next
parent: T-340-session退役线
created: 2026-11-11
---

# 收窄 session 的 sdk 依赖边

## 内容

D-451 的**边一**：把 `scripts/exemptions/plugin-session-sdk.json` 的 101 条符号收窄到 0，
然后删掉这份豁免。它是三条边里唯一「读数已经可跑、出口已经写得出」的一条。

**每摘一条符号，只允许三种走法**（都是仓内已有的正路，不新造通道）：

1. **用法本来该消失**：那条读口在被裁决撤掉之后已无人读——删用法（本批摘掉的 15 条
   余量就是这个形态的存量）；
2. **由 owner 提供**：session 只是持有与透传的，改由声明方 / owner 给出口
   （`D-427` 把能力开关真源搬回能力包、`D-428` 把「是不是同一能力的副本」搬回声明面，
   都是这条路）；
3. **经门面转出**：符号本来就该给插件用，走 `symbio-plugin-api` 的 `pub use`
   （`D-443` 之后门面只收生产依赖面，所以这一步要先回答「谁在消费」）。

**不许的走法**：把符号改名 / 换个形状再留在账上；在 session 里再抄一份折法（那是第二条
路径）；为了少一条符号去调宽 `plugin-api`（那只是把依赖搬家，边还在）。

where: `scripts/exemptions/plugin-session-sdk.json`、`crates/symbio-plugin-session/src/`、
`crates/symbio-plugin-api/src/lib.rs`、`crates/symbio-sdk/src/`。

exit: 豁免账 101 → 0；`scripts/exemptions/plugin-session-sdk.json` 删除；
`session-retire-audit` 与它的 10 条回归**同批从 `30-docs.mjs` 撤下**（它的输入没了，
留着会变成一条永远红的守卫）；`plugin-entry-audit` 的 E-009 对 session 只剩「没有这条边」
这一种读法。

## 处理过程

- 2026-11-11：登记。起点 101 条（D-451 校准后的真实分母）。第一批建议从 `reason` 里
  已经点明「只是持有与透传」的那几族入手（`AgentScope` / `agent_perms_of` /
  `stamp_branches` / `META_BRANCH` / `DriveEntry` / `DriveTarget` /
  `declared_drive_binding` / `ViewUnavailable`），它们各自都有 owner 侧的落点。
- 2026-11-11（同轮量账）：101 条里按「实现 / 测试」两分，**7 条只在测试里出现**
  （`PromptMessage` / `VDFS_PARENT_ADDR` / `budget_exceeded` / `commitment_events` /
  `production_matrix` / `vdfs_context` / `verify`）。这是风险最低的第一批：测试里的依赖
  通常是一条**为验证而生的读口**，不是生产路径。
- **首个目标：`PromptMessage`。** 它只剩一处（`store/wal.test.rs` 的 `verify_prompt_fidelity`
  用例），而 `symbio_sdk::projection::prompt_fidelity::verify` **全仓只有这一个消费方**——
  一个公共 sdk 出口只有别家测试在读，正是 `D-443` 拒绝的「为测试面开口」。
  入场券（两条选一，都要先答）：① 把 `verify` 的入参从 `&[PromptMessage]` 换成
  `&[TranscriptEntry]`（折法留在 sdk，调用方不再需要模型消息类型）；② 或把这段校验
  搬进 session 的测试、`verify` 从 sdk 删除（先确认没有第二个消费方、并按
  `arch-dead-code` 的棘轮下调基线）。**不许**为了摘符号在 session 里再抄一份折法。
