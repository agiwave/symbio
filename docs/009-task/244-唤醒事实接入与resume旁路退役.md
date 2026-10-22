---
kind: task
id: T-244-唤醒事实接入与resume旁路退役
title: PL-017 步 4–5 收口——审批/插话/中止唤醒进 await 行，resume.rs 旁路退役
status: doing
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-240-唤醒事件时间戳取代墙钟]
where: symbio/src/plugins/loop/resume.rs · symbio/src/plugins/loop/turn_exec.rs · symbio/src/plugins/loop/loop_rows.rs · symbio/src/symbio_core/decl/
exit: 'PL-017 §4 步 4 与步 5 落地：审批 / 用户补充 / 中止各落一条唤醒事实，其**行语义按 [ADR-121](../010-adr/121.md) 定案**（审批与补充重新开账、中止落 `failed{why:"aborted"}`，不改 `await` 行的 `until`）；`resume.rs` 承担的恢复 / 审批 / 问答三条旁路逐条迁进行表后整文件退役（`turn_exec.rs:846,858` 的 `resumed` 旁路与 `loop_rows.rs:138` 的 `resume_anchor` 一并消失）；`close-turn` 收束行进表（PL-017 §3.1：owes `turn.has_final ∧ ¬closed`，until 两条收束格，by `closer`；`has_final` 的日志口径由 ADR-121 给定），收束不再只靠执行体内联；`resumed` 与续写幕两处「跳过无债守卫」的特例由行表语义接管或有名有姓留档；唤醒接好后 `tool` 端口 `max_attempts > 1`（PL-016 §2.3 行 C：重试由「有没有活着的等待方」决定，本任务即解锁条件）且 retry 用例使用真实 `PortDecl`；全量门禁与 e2e 绿。'
---

## 内容

父任务：[T-236-义务环符合性收口批](236-义务环符合性收口批.md)；父设计：[PL-017-端口细粒度化](../004-plan/017-端口细粒度化.md "PL-017-端口细粒度化") §4 步 4 / 步 5。

2026-10-22 对账：三端口 `max_attempts` 全为 1、retry 仅合成用例；`resume.rs`
三路旁路仍在且 `resumed` 在派发器里走特殊分支——「外部事件唯一进环通道」仍是
内存旁路而非事实。步 4/5 是 PL-017 已声明未收口的两步，本任务销掉。

## 验收口径

1. PL-017 §5 判据 6 三场景各一条用例（用例名带场景）：审批落 `authz.granted(port=tool)`
   ⇒ `dispatch-tools` 重新开账；补充落 `user.message` / `turn.supplemented` ⇒ 走既有出口，
   **不新增 `await` 出口**（原判据「插话唤醒 await-turn」不是方案要求，§3.2 只写
   `await-tools`，已按 ADR-121 后果节订正）；中止落 `failed{why:"aborted"}` + 收束格，
   `await-tools` 由既有 `failed` 出口收清。
2. `resume.rs` 的三条私有路径全部消失（文件删除或 0 旁路），`turn_exec.rs` 无 `resumed`
   分支、`loop_rows::resume_anchor` 整格删除——grep 断言进门禁或测试。
3. `close-turn` 行在表且 `chat.assistant.final` / `chat.assistant.fallback` 经它收口；
   `turn.has_final` 进视图声明（口径：本幕最近一条 `response(port=model)` 的 `calls` 为空
   ∧ 未收束），视图 `ver` 相应升号。
4. `tool` 端口 `max_attempts > 1` 且 retry 用例使用真实 `PortDecl`（非 `port.test.rs`
   里合成的 `RETRYING`）——销「重试路径只有合成覆盖」；`turn` / `model` 保持 1
   （PL-016 §2.3：等待方随进程消失 ⇒ 到顶即判死，不许顺手放宽）。
5. CLI 复验：一条**审批卡中断 → 唤醒 → 续跑**的实测会话（重启进程后再放行），
   加上常规两幕；`obligation-ring-accept --strict` 全项通过 + 门禁 + e2e 全绿。
6. 熔断线：本任务按步 4 / 步 5 分提交，每批非注释生产代码 ≤ 300 行（PL-016 §5）。

## 处理过程

- 2026-10-22 登记（PL-016 三面对账：方案 → 代码 → CLI 实证，见 T-236 内容节）。
- 2026-10-23 裁决先行：施工前对账发现 §3.2 那句「`await-tools` 的 `until` 增加对应的
  事实名」与 §2.3 的债键纪律（`await` 行键 = `(port, id)`）冲突——`user.message` 与
  两条收束格只带 `turn`，照字面加出口会得到一条永不命中的 `until`；同时 §3.1 的
  `turn.has_final` 仓内无任何实现，视图只读日志、折不进执行体的内存草稿。
  两条都在 [ADR-121](../010-adr/121.md "ADR-121") 定案（唤醒按来源分「重新开账 / 协议格收清」两种作用；
  `has_final` = 本幕最近一条 `response(port=model)` 的 `calls` 为空 ∧ 未收束），
  本任务的验收口径与 `exit` 按裁决订正三处，未动模块。
