---
kind: task
id: T-241-ActorDecl执行体注册表
title: ActorDecl 执行体注册表——by 名字到执行体一个动作，派发器业务分支退役
status: todo
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-238-短路轮进债环]
where: symbio/src/symbio_core/decl/ · symbio/src/plugins/loop/turn_exec.rs · symbio/src/plugins/loop/port.rs · symbio/src/plugins/loop/loop_rows.rs
exit: 'PL-016 §2.3 / §3.2 的执行体契约落地：`ActorDecl { name, mode, reads, writes, run }` 存在且 `run` 返回 `Result<Vec<Fact>, ActorError>`（**账本语义**：见 PL-016 §2.3 补文——声明内的领域事实允许直写同一 store，协议四格只由引擎写）；`by: Some(name)` 必须能在注册表解析到执行体，**`decl::ring::drive` 的 `accept` 谓词退役、签名退回 `(rows, frozen, events, now)`**（ADR-118 订正 1），未接线的执行体行显式改 `by = None` 占位、接线时与 `ActorDecl` 注册同批改回 `Some`；`turn_exec` 的 `Executor::for_by` 名字 match 与「模型 vs 工具批次」if/else 退役——引擎对 by 只有一个动作；`Err` ⇒ 引擎写 `failed` 保留；守卫用例 `accept_skips_rows_without_an_executor` 作废，代之以「`by=Some` 无注册 ⇒ 装载红」用例；单次提交结构性改动 ≤ 300 行；全量门禁与 e2e 绿。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)。

2026-10-22 对账：`ActorDecl` / `ActorError` 全仓不存在，执行体是裸函数 +
名字串匹配（`turn_exec.rs:892-910` 仅 `"execute"` 一档）；执行体返回
`Result<TurnOutcome>` 且**自己 append 事实**，与 §2.3「执行体交回若干事实、
引擎追加」的契约相反。§3.2 表明的三样静态检查（mode 闸门 / reads 存在 /
writes 封闭）因此全部失效。本任务是 [T-242-装载检查余四道](242-装载检查余四道.md) 与
[T-243-Builder六口与按会话seal](243-Builder六口与按会话seal.md) 的前件（检查 ⑤⑥ 需要 `ActorDecl.writes`）。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。
