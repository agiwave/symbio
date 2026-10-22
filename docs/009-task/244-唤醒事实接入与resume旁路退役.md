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
- 2026-10-23 步 5 第一批（`close-turn` 进表 + `turn.has_final` 进视图声明）。
  [PL-017](../004-plan/017-端口细粒度化.md "PL-017-端口细粒度化") §3.1 收束那一行落地：
  `close-turn`（prio 55，赶在 `execute-turn` 的 60 前）+ 独立执行体 `close`，前件按
  [ADR-121](../010-adr/121.md "ADR-121") 决策 2 的口径折进视图 `turn_head`——`ver` 3 → 4，
  `reads` 加 `response.port` / `response.calls` 与两条收束格的 `text`。写入侧
  `halt` 仍是收束格唯一写方，只是多了一个调用点（`ModelRound::Close` 不再内联收束，
  应答寄存于 `Prepared::closing`）。
  **两处与 §3.1 的字面偏差，都是过渡形状**：
  ① 出口收窄成只有 `chat.assistant.final`，`chat.assistant.fallback` 暂留 `execute-turn`。
  原因是装载期检查的边定义（「b 的执行体写得出 a 的出口格 ⇒ 边 a→b」）：粗端口期两行
  若共享 `{final, fallback}` 出口集，`execute` 与 `close` 两个执行体互为对方的出口写方，
  表直接以 `DependencyCycle ["close-turn","execute-turn","close-turn"]` 拒装。
  收窄后两个执行体各写一格、两条边指向不同行，环消失、表装得上。复原条件与时间：
  步 5b 把 `authorize-tool` / `dispatch-tools` 接回执行体、`execute-turn` 退成薄壳之后，
  按 §3.1 恢复 `final | fallback`。
  ② 口径 3 的「`chat.assistant.final` / `chat.assistant.fallback` **经它收口**」本批只算
  部分兑现：走 `close-turn` 的只有「答完了」这一条路径。中止 / 工具失败 / `step_limit`
  三条兜底格仍在执行体内部直接落——那些幕里没有一次 `calls` 为空的模型应答，
  `has_final` 的前件门关着，`close-turn` 判不出欠账。要接管它们得先给「没收束且不会再
  有应答」立一个视图字段，那是步 5d 之后的事，不在本批范围。
  **本批新增一条有名有姓的旁路**（口径 1「续写幕特例由行表语义接管**或**有名有姓留档」，
  本批走后半句）：`turn_exec.rs::run_execute` 的首分支读 `has_pending_close()`。续写幕
  带着「已落应答、未收束」的草稿进来，而派发器的选债口只看**本轮事件切片**——那条
  `response` 在归档段里，切片读不到 ⇒ `close-turn` 判不出欠账；没有这一分支就反复跑
  模型直到 `step_limit`。反向判据实测：删掉该分支 ⇒
  `an_open_turn_archived_by_the_next_turn_is_still_the_resume_anchor` 转
  `Failed(InternalError("step_limit"))`。删除条件写在 `TurnSession::has_pending_close`
  的文档里：步 5d 退役 `resume.rs` 的 `resumed` 旁路时，连同它一起删。
  用例：`loop_rows.test.rs` 三条（前件真值 / 反向钉「有调用时不欠」/ `next` 在答完后
  先派 `close-turn`）+ `decl/mod.test.rs` 三条（`has_final` 正向、四种「还没答完」的形状、
  幕锚之前的响应不算）。门禁：`cargo test --lib` 2078 通过 / 0 失败，
  `cargo clippy --all-targets -p symbio -- -D warnings` 干净，CS-001 ✓（core 形状未变宽），
  PTS-001…004 ✓（`decl/ring.rs` 字节未变）。非注释生产代码 +112 行（熔断线 300）。
  CLI 实证（会话 `cli1a165d53fcc0`，Provider `LMStudio`，两幕）：第二幕日志里
  `[Loop] 派发 execute-turn（执行体 execute…）` 之后模型应答落 `calls: []`，紧接
  `[Loop] 行 close-turn 可还（reflex / 80ms；债 turn=1）` 与
  `[Loop] 派发 close-turn（执行体 close；债 turn=1）`，收束格由它落出（WAL seq 20→21）——
  收束第一次是**行表派发的**，不是执行体内联的。`obligation-ring-accept --strict` 的
  J1…J7 全项通过；J8 仍报 `HTTP 2 ≠ sent(model) 2 + 豁免未入账 1`，差额是 classify 短路轮
  在插件内部打的那次模型往返，与本批无关，归 [T-247](247-插件内部模型往返进环.md "T-247-插件内部模型往返进环")。
- 2026-10-23 步 5 第二批（口径 4 前半：retry 用例改吃真实 `PortDecl`）。
  `the_lease_retries_while_the_port_has_attempts_left` 原来带一条独立的合成声明
  （`port` / `before` / `lease_ms` / `max_attempts` 四格自填），真表漂了用例照样绿——
  「重试路径只有合成覆盖」就是这个形状。现在由 `loop_rows::PORTS` map 出来，只覆盖
  `max_attempts` 一格，`stranded` 写的租约与判定用的 `now` 也从同一份真表推。
  补一条钉住真表此刻取值的用例 `no_port_in_the_real_table_retries_today`（三端口全 1
  ⇒ 生产里重试分支不可达），反向判据实测：把 `turn` 的上限抬到 2 ⇒ 它转红并指名
  `[("turn", 2), ("model", 1), ("tool", 1)]` 与「先确认重试出来的 `request` 有执行体会派它」。
  抬上限本身仍随步 5 的接线一起做（口径 4 后半未动）。
