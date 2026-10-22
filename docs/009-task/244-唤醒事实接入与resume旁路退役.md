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
exit: 'PL-017 §4 步 4 与步 5 落地：审批 / 用户补充 / 中止各落一条唤醒事实，其**行语义按 [ADR-121](../010-adr/121.md) 定案**（审批与补充重新开账、中止落 `failed{why:"aborted"}`，不改 `await` 行的 `until`）；`resume.rs` 承担的恢复 / 审批 / 问答三条旁路逐条迁进行表后整文件退役（`turn_exec.rs:846,858` 的 `resumed` 旁路与 `loop_rows.rs:138` 的 `resume_anchor` 一并消失）；`close-turn` 收束行进表（PL-017 §3.1：owes `turn.has_final ∧ ¬closed`，until 两条收束格，by `closer`；`has_final` 的日志口径由 ADR-121 给定），收束不再只靠执行体内联；`resumed` 与续写幕两处「跳过无债守卫」的特例由行表语义接管或有名有姓留档；`authorize-tool` / `dispatch-tools` 接回执行体之后才可以抬（口径 4：那之前放宽只会造出永不派发的 `request`）且 retry 用例使用真实 `PortDecl`；全量门禁与 e2e 绿。'
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
   **前半按 2026-10-23 对账推迟**（见处理过程）：`max_attempts > 1` 的解锁条件不是
   「有活着的等待方」这一条，而是 `authorize-tool` / `dispatch-tools` 两行接回执行体——
   接回前放宽只会造出永不派发的 `request`。本口径拆成两半：retry 用例改用真实
   `PortDecl`（现在做），放宽上限随步 5 的接线一起做。
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
- 2026-10-23 步 4 第一批（场景三：中止落协议格）。施工前对账抓到两处「事实面说假话」：
  ① `tool_dispatch.rs` 的 `ToolPort::close_batch` 原来按
  「`outcomes` 里有这个 id 就算应答」关账，而批尾收口给**没跑到**的调用补的占位结果
  也在 `outcomes` 里 ⇒ 中止的调用落成 `response`，`await-tools` 的 `failed` 出口只有
  resume 一条路走过——判据 6 的第三场景在事实面上等于没接（行级用例
  `loop_rows.test.rs` 早已存在，但生产从不产出那种载荷）。现在词表同源：节点侧
  `meta.failure_kind` 的两个成因（`ABORTED` / `NOT_EXECUTED`）由 `tools` 门面交出，
  端口关账用 `is_unanswered` 判「有没有跑到」。② 同一条判据的另一半：resume 重跑的
  工具只要 `success = false` 就落 `failed{not_executed}`，等于在日志里说「这一笔没跑到」，
  而它执行过——改成「执行层跑过了就是应答」，只有中止才是没跑到。
  用例：`tool_dispatch.test.rs` 六条（三条正向 + 两条反向钉「跑完而失败 / 交回审批卡
  仍是应答」+ 一条边界钉「没放行的调用不补记」）。反向判据实测：把 `close_batch` 退回
  `answered = outcomes.iter().any(...)` ⇒ `an_aborted_call_closes_as_failed_not_as_an_answer`
  与 `a_skipped_call_closes_as_not_executed` 转红。`cargo test --lib` 2072 通过 / 0 失败，
  `clippy --all-targets -D warnings` 干净。
- 2026-10-23 口径 4 对账（前半推迟，理由进验收口径）：`reap` 重试时落的是
  `retry{attempt}` + `request{attempt+1}`（`port.rs:439-459`），而 `dispatch-tools` 的
  `owes` 读 `authz.granted(port=tool)`（`loop_rows.rs:220` 那段注释记录了为什么不是
  `request`）、`authorize-tool` 与 `dispatch-tools` 两行此刻都是 `by: None`（判定住在
  批次内部，不经派发器选债）。⇒ 今天把 `tool` 的 `max_attempts` 抬到 2，租约看护重试
  出来的那笔 `request` 举得起授权债却**没有任何执行体会去判它、派它**，正撞
  [PL-016](../004-plan/016-义务环.md "PL-016-义务环") §2.3 禁止的「永不派发的 request」。
  解锁条件是这两行接回执行体（本任务步 5 退役 `resume.rs` 时必须做——审批续跑要有人派），
  不是「唤醒事实接上了」；原判据把解锁条件写窄了，已按实测订正。
- 2026-10-23 步 4 第二批（判据 6 场景一 / 场景二的**用例落点**）。三场景对完后发现
  口径 1 要区分两件被混在一起的事：场景一的判据是「写方真写出那种载荷」，场景二的判据
  是「`await` 的 `until` 不多出口」。补 `an_authorization_reopens_the_dispatch_debt_and_a_send_moves_to_the_await`
  （`tool_dispatch.test.rs`）：拿生产 `ToolPort::authorize_calls` / `begin_calls` 落
  `authz.granted` 与 `sent`，再问行表 `owes`——行层原有的合成用例只证明「表认得
  granted」，证明不了载荷里 `id` 那一格真在（键对不上时派发行永不开账、日志一片绿）。
  反向判据实测：从 `granted_event` 的载荷删掉 `"id"` ⇒ 该用例转红。
  场景二不补端口用例：补充不产生任何端口事实，判据归行表用例
  `a_supplement_does_not_answer_the_in_flight_tool`（已有的那条），本任务只确认
  **没有**新增 `await` 出口。生产代码零改动，`cargo test --lib` 2073 通过 / 0 失败。
