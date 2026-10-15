---
kind: task
id: T-105-Scope-Child首产点批
title: `Scope::Child` 全仓第一个生产构造点：派生子会话挂父作用域 + 读方
status: done
created: 2026-10-15
track: plan
parent: T-128-多主体与对等承诺
deps: []
prio: later
evidence: ① **语义先划清**：意图 = 「权威的上限」落 [T-128 §3.2](../009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")（`Root` = 独立主体无上限；`Child(parent)` = 子主体的收束还须父主体也持同一能力；读方唯一 = 写侧收束闸）。② **生产构造点**：`chat_loop::state::request_scope`（新增）+ `PersistentChatSession::delegating_principal`（新增，`chat_session.rs`）——子会话经 `metadata.parent_session_id` → 父会话 `metadata.agent_id` → `principal_of` 拿父主体，顶层会话恒 `Root`；两处轮次入口（`v2_exec::execute_turn` / `execute_final_reply_turn`）同批把 `scope` 传入 `request_actor_spec`（新增第三形参）。③ **真读方**：`turn_runner::closure_granted` 加第二道 `within_delegation_ceiling`——`scope = child <parent>` 时父主体也须持同一项能力（谓词复用 `can_reply` / `can_produce_artifact`，与许可同源）。④ **判据**：`turn_runner.test.rs::scope_ceiling_tests` 三条（**反向用例**——父主体不持该能力 ⇒ 收束被拒、`scope` 改回 `Root` ⇒ 放行；窄父派生宽子不得发言而产物照常；等强父无影响）+ `chat_session.test.rs::scope_follows_the_delegation_chain_and_defaults_to_root`（委派链取值：子会话 ⇒ `Child(父主体)`、顶层/无归属 ⇒ `Root`）。⑤ **形状**：`Scope` / `ActorSpec` 成员未增减（只加读方与构造点），`gate.mjs` 全绿

exit: ① `Scope::Child` 出现生产构造点（委托 / 派生子会话链上，规格从 `Scope::Root` 收窄为 `Scope::Child(parent_principal)`）；② `scope` 字段有真读方（子会话轮次的某个判定问它——可见域 / 溯源 / 收束三者之一，语义先在 [T-128](../009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺") §3 划清再落）；③ 至少一条判据（单测或 e2e 反向用例：把 `Child` 改回 `Root` 必红）；④ `node scripts/core-shape-audit.mjs` 零违规。**推不开才按 [ADR-055](../010-adr/055.md) 立决策**
---

## 内容

[T-082](082-角色行生产构造批.md "T-082-角色行生产构造批") 批① 调研拆出的前置：`Scope::Child` 类型在、全仓零构造
（`grep "Scope::"` 的生产命中只有 `chat_loop/state.rs:72` 的 `Scope::Root` 一个）。
T-082 exit 要求「`Scope::Child` 出现全仓第一个生产构造点」，但它牵连两件本批不该捎带的事：

1. **多主体委托链的形状**：派生子会话（e2e t30 的 grandReqs 那条链）是 `session` 插件的
   委托路径，规格在子会话建立时构造——`Child` 的值要挂**父主体**还是父会话，语义在
   T-128 §3（「子智能体」的定义）里尚未划到「可当读方用」的精度；
2. **读方的三选一**：`scope` 造出来谁问它——可见域判定（`governance`）、溯源
   （`produced_by` 方向）、还是收束闸（`closure_granted`）——三个候选的改动面差一个量级，
   未裁决就落 = 造一个没有读方的字段（正是 T-082 要消灭的形态）。

两条都需先裁决，故独立成批。批内顺序建议：先在 T-128 §3 划清「子智能体」的读方语义
（纯文档裁决），再落构造点与读方，最后补判据。

## 不是本任务范围

- S03 规划者 / 验证者角色行本身（T-082 批⑤，dep 本任务）；
- 组织形态（市场 / 仲裁 / 守门）= T-073。

## 处理过程

- 2026-10-15 T-082 开批调研时登记（横切缺口拆解：scope 首产点牵连委托链与可见域语义，
  按用户指示「任务有困难时创建前置依赖任务」拆出，避免把未裁决的语义塞进角色行批次）
- 2026-10-15 落地（提交见 `git log`）：读方取「收束闸」支（三选一里改动面最小且与
  [ADR-065](../010-adr/065.md "ADR-065") 的按种类收束同址）；语义落 [T-128 §3.2](../009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")。
  [T-123 §3.1](../009-task/123-多步任务与返工.md "T-123-多步任务与返工") 的「机制键 `scope`」行同批由「仅类型，零生产构造」订正为「已在生产」。
  **不动** `authz::ROWS`（不给子会话建窄行——约束的正确位置是上游父主体，见 T-128 §3.2）、
  不动事件 schema（`scope` 是规格侧上限，不是逐事件数据）。
