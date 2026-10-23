---
kind: task
id: T-244-唤醒事实接入与resume旁路退役
title: PL-017 步 4–5 收口——审批/插话/中止唤醒进 await 行，resume.rs 旁路退役
status: done
created: 2026-10-22
track: plan
parent: T-236-义务环符合性收口批
prio: next
deps: [T-240-唤醒事件时间戳取代墙钟]
where: symbio/src/plugins/loop/resume.rs · symbio/src/plugins/loop/turn_exec.rs · symbio/src/plugins/loop/loop_rows.rs · symbio/src/symbio_core/decl/
exit: 'PL-017 §4 步 4 与步 5 落地：审批 / 用户补充 / 中止各落一条唤醒事实，其**行语义按 [ADR-121](../010-adr/121.md) 定案**（审批与补充重新开账、中止落 `failed{why:"aborted"}`，不改 `await` 行的 `until`）；`resume.rs` 承担的恢复 / 审批 / 问答三条旁路逐条迁进行表后整文件退役（`turn_exec.rs:846,858` 的 `resumed` 旁路与 `loop_rows.rs:138` 的 `resume_anchor` 一并消失）；`close-turn` 收束行进表（PL-017 §3.1：owes `turn.has_final ∧ ¬closed`，until 两条收束格，by `closer`；`has_final` 的日志口径由 ADR-121 给定），收束不再只靠执行体内联（**定稿答话轮除外**：那一轮一次模型往返都不发，`close-turn` 开不出账，缺口归 [T-253](253-定稿答话轮的收束格不进环.md "T-253-定稿答话轮的收束格不进环")）；`resumed` 与续写幕两处「跳过无债守卫」的特例由行表语义接管或有名有姓留档；`authorize-tool` / `dispatch-tools` 接回执行体之后才可以抬（口径 4：那之前放宽只会造出永不派发的 `request`）且 retry 用例使用真实 `PortDecl`；全量门禁与 e2e 绿。'
evidence: 步 4 / 5 共五批提交（git log 的 `refactor(loop)` 五条）；CLI 复验（口径 5）：会话 `verify-t244-f`（审批卡中断 → 重启进程 → 放行 → 续跑）与 `verify-t244-a`（常规两幕）在 `obligation-ring-accept --strict` 下 J1…J8 全项通过；门禁 `node scripts/gate.mjs` 130/130 全绿（含 e2e 78/78）；守卫锚 S-017（`resume.rs` / `resume_anchor` / `resumed` 零出现）；例外：定稿答话轮的收束格缺口归 [T-253](253-定稿答话轮的收束格不进环.md "T-253-定稿答话轮的收束格不进环")
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
   **2026-10-23 步 5e 对账（口径收窄）**：本条覆盖的是**派发过模型往返的轮**（工具轮 /
   普通轮，含中止与适配器失败两条兜底路径——它们的收束格由 `close` 执行体落）。
   **定稿答话轮**（`classify` 判 `Answered` 的短路轮）的收束格**不经** `close-turn` 行：
   那一轮一次模型往返都不发，日志上没有 `response(port=model)` ⇒ `turn.has_final`
   不成立 ⇒ 收束行开不出账；它的收束格由轮首准备 1b 分支就地落
   （`turn_runner/prepare.rs`）。这不是实现取巧，是 [ADR-121](../010-adr/121.md) 的
   `has_final` 口径与 [PL-017](../004-plan/017-端口细粒度化.md "PL-017-端口细粒度化") §3.1 的行形状**合起来
   覆盖不到**这一轮（视图只读日志，而「有一份备好的答话待入格」是内存草稿状态——
   ADR-121 明文否决把内存布尔折进视图）。缺口与候选形状登记为
   [T-253-定稿答话轮的收束格不进环](253-定稿答话轮的收束格不进环.md)（裁决先行）。
4. `tool` 端口 `max_attempts > 1` 且 retry 用例使用真实 `PortDecl`（非 `port.test.rs`
   里合成的 `RETRYING`）——销「重试路径只有合成覆盖」；`turn` / `model` 保持 1
   （PL-016 §2.3：等待方随进程消失 ⇒ 到顶即判死，不许顺手放宽）。
   **前半按 2026-10-23 对账推迟**（见处理过程）：`max_attempts > 1` 的解锁条件不是
   「有活着的等待方」这一条。**2026-10-23 步 5b 施工前再对账，这条后半本身判错了**：
   解锁条件也不是「两行接回执行体」——三行工具债的键都是 `(port, id)`（不含 `attempt`），
   重试出来的那笔 `request{attempt+1}` 被**上一 attempt** 的 `granted` 提前关门，
   接线之后也没有任何行会去派它。放宽上限的前置是**债键的 attempt 口径**，
   与本任务的接线无关，另立 [T-252-租约重试的债键不含尝试号](252-租约重试的债键不含尝试号.md) 裁决。
   本口径到此收成一半：retry 用例改用真实 `PortDecl`（已做），
   「抬上限」从本任务移出。
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
  `Failed(InternalError("step_limit"))`。删除条件写在 `TurnSession::has_pending_close`<!-- doc-symbol-allow: 记名的是恢复旁路时期的旧符号（`has_pending_close`），随 T-244 步 5d 的旁路退役而删除——「不存在」正是退役已落地的状态 -->
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
- 2026-10-23 步 5b 施工前对账（**两条互斥的装载检查**，裁决见
  [ADR-124](../010-adr/124.md)）。三行工具债要接回执行体，实测装不上表：`dispatch-tools` 的出口是
  `sent | failed`，两格都在 `ENGINE_ONLY_CELLS`（第 5 条不许执行体声明），而第 6 条要求
  `by=Some(a)` 的行的执行体 `writes ∩ 出口 ≠ ∅`——两条对**同一行**互斥，这一行永远只能
  `by: None`；`dispatch-turn` / `dispatch-model` 同形。这不是「实现困难」，是方案
  （[PL-017](../004-plan/017-端口细粒度化.md "PL-017-端口细粒度化") §3.1 给这三行写了 `by: courier`）与机制条文打架，且互斥此前从未暴露：
  第 6 条写成 ∩ 那天，接线只有 `execute-turn` / `close-turn` 两行，它们的出口恰是自己写的
  领域格。裁决=第 6 条按出口可声明性分两档（出口有领域格 ⇒ 原判据不变；出口全为引擎格 ⇒
  改判「名单非空」，关门由引擎按 `before` 代行），「这次真没写」那一半归
  [ADR-123](../010-adr/123.md) 的空账本守卫。实现只动 `check_rows`（`drive` / `owed` 字节未变，PTS-003
  比对通过），PL-016 §3.4 第 6 条同批改文，用例
  `an_actor_serving_an_engine_exit_row_must_still_declare_writes` 钉两面。
  **同批否掉本任务口径 4 的后半**（原文已就地订正）：`authorize-tool` / `dispatch-tools`
  的债键是 `(port, id)`，不含 `attempt`，而 `reap` 重试落的是同一 `(port, id)` 的
  `request{attempt+1}` ⇒ 上一 attempt 的 `granted` 已经把这一行的门关上，接线接不出
  「重试出来的那笔有人派」。抬 `max_attempts` 的前置是键口径，另立
  [T-252-租约重试的债键不含尝试号](252-租约重试的债键不含尝试号.md)。
- 2026-10-23 步 5b（工具批三笔债接回执行体：创建 / 授权 / 派发 / 拒绝收尾）。
  `ToolDispatch` 从一次调用改成三个动作（`open` / `authorize` / `dispatch`，各收一个
  `store`：行表按事实判欠不欠，测试替身不落协议事实就会在同一笔债上空转到步数上界）；
  `Prepared` 加 `pending_calls` 作三步之间的寄存位；`TurnSession` 对五类动作
  （`model` / `open` / `authorize` / `dispatch` / `close`）各给一个方法，派发器仍只有
  「按 `by` 查表、跑一个动作」一条通路（ADR-118 决策 2）。
  表侧三处：
  ① `open-tools`（67）/ `dispatch-tools`（68）接上 `opener` / `courier` 执行体；
  ② 新增 `deny-tools`（69，owes `authz.denied(port=tool)`，until `failed(port=tool)`）——
  `dispatch-tools` 只欠 `granted`，一批全被拒（或全是非法调用）时它不欠，没有这一行
  执行层永远跑不到、`failed{why}` 永不落；混合批次里那一笔由 68 顺带关掉，本行随后
  自然不欠。不带 `port` 的收束格拒绝按 `eq` 取不到实例，不误开工具债；
  ③ `turn_head` 升 v5 加 `tools_open`（本幕最近一条模型响应的 `calls` 里还有没落
  `response` / `failed` 的）——`execute-turn` 的 `owes` 加 `tools_open = false`，
  它因此给工具批让位；否则 60 永远比 67/68/69 先生效，新一批交换会带着没有结果的
  调用再跑一轮模型。
  非法调用（缺 id / 缺工具名 / 参数解析失败 / 超长回执）也照样开账再拒绝
  （`invalid` + `deny_why`），不再在开账时静默跳过——跳过会让 `open-tools` 的展开
  永远关不掉那一笔。`close_batch` 对拒绝项落 `failed{why}`（`fail` 不再对未放行的
  调用提前返回）。
  **反向判据实测**：`deny-tools` 改回 `by: None` ⇒
  `a_fully_denied_batch_is_closed_by_deny_tools` 转 `no_progress`；删掉
  `execute-turn.owes` 的 `tools_open` 条件 ⇒ `tool_round_lands_artifact_and_feeds_next_prompt`
  的分发计数读到 0。用例：`turn_runner.test.rs` 新增全批拒绝一条（协议序列 + 无
  `sent(tool)` + 拒绝结果带角色 / id 进下一轮），旧三条工具轮用例的事实序列按新增的
  工具协议格更新；`loop_rows.test.rs` 新增 `deny-tools` 两正一反；`decl/mod.test.rs`
  新增 `tools_open` 两条（逐笔关账 / 幕锚之前的应答不算）。门禁：`cargo test --lib`
  2095 通过 / 0 失败，`node scripts/gate.mjs` 130/130 全绿（含 e2e 78/78）。
  非注释生产代码 +206 行（熔断线 300）。
- 2026-10-23 步 5b 的边界（留给步 5c/5d）：`dispatch-turn` / `dispatch-model` 两行仍是
  `by: None`（它们的派发点还在执行体内，随细端口接回）；`close-turn` 的出口仍只有
  `chat.assistant.final`、`execute-turn` 仍带 `chat.assistant.final` 的过渡豁免，两者
  一起等 `run_execute` 的 `has_pending_close` 旁路退役后按 §3.1 复原。
- 2026-10-23 步 5d（唤醒请求进环 + `resume.rs` 旁路整文件退役）。恢复路径今天只落
  **一笔事实**：`ToolPort::land_wake` 写 `request(port=tool)`（载荷带 `resume` 标记，
  键 `tool-resume-{turn}-{target}-{attempt}`）；授权 / 派发 / 拒绝由行表自己的
  `authorize-tool`（0）/ `dispatch-tools`（68）/ `deny-tools`（69）承担——它们从那笔
  `request` 的载荷复原工具批（`ToolPort::wake_batch`），再按各自的 `owes` 判欠不欠。
  **环外不再有第二条恢复事实**：`resume.rs`（632 行）+ `resume.test.rs`（255 行）整份
  删除，`TurnInput.resume` / `TurnResume` / `turn_exec::ResumedTool` 三处入参形状退场，<!-- doc-symbol-allow: 记名的是本批随 `resume.rs` 退役而删除的旧入参形状（`ResumedTool`）——「不存在」正是退役已落地的状态 -->
  `run_execute` 的 `has_pending_close()` 首分支、`loop_rows::resume_anchor` 整格、
  续写幕的 `resumed` 特例一并消失；`tools::apply_not_executed` 随之删除。<!-- doc-symbol-allow: 记名的是本批随 `resume.rs` 退役而删除的旧函数（`apply_not_executed`）——「不存在」正是退役已落地的状态 -->
  它的跨域取用方只有 `resume.rs`，随它一并退场。
  形状改动三处：① `WakeSpec` 把七个位置参数收成一个结构体（位置错序编译器不报——
  `wire_id` 与 `turn_id` 互换过一次）；② `WakeIntent::{Execute, Result}` 区分「继续跑
  工具」与「答复内容」两种唤醒；③ 唤醒 id 带 `attempt`：同一笔被重复唤醒时旧的那笔
  `request` 由 `failed{superseded}` 关掉（`wake_terminal` 按事实 seq 判，不看墙钟）。
  **反向判据实测**：把 `resume.rs` 放回来 ⇒ 新守卫 S-017 转红（文件存在即违例）；
  `resume_anchor` / `resumed` 任一名字回来 ⇒ 转红；`land_wake` 载荷的 `id` 那一格删掉
  ⇒ `tool_dispatch.test.rs` 的唤醒用例转红（键对不上时派发行永不开账、日志一片绿）。
  门禁侧两处：① **新增 S-017**（恢复路径不许再有旁路：`resume.rs` 文件不存在 +
  `resume_anchor` / `resumed` 零出现），配 4 条回归用例；② **S-015 重指**——布尔壳
  `closure_granted` 随它唯一的生产调用点（续写幕的产物臂）退场而删除后，旧判据
  （`!closure_granted(` 零出现）已不可能变红 ⇒ 空转守卫。重指为两条可证伪的形状：
  布尔壳零出现、闸的每次调用当场处理 `Err`（不许 `.is_ok()` / `.is_err()` 取布尔，
  也不许把结果存起来另判——拆行写法骗得过行级正则）。回归用例 4 条 → 5 条。
  文档侧：21 处失效符号指认随本批清零——`turn_runner::closure_granted` 16 处随改名跟进，<!-- doc-symbol-allow: 记名的是收束闸的旧布尔形态（`closure_granted`），它随收束闸收敛到单一 owner 而退役——「不存在」正是单一 owner 生效的状态 -->
  `turn_exec::ResumedTool` / `ToolPort::open_resume` / `tool_dispatch::open_resume` 5 处随旁路退役走行内承认通道——`doc-symbol-audit` 由 21 处红转绿。<!-- doc-symbol-allow: 记名的是恢复旁路时期的旧符号（`ResumedTool` / `open_resume`），随 T-244 步 5d 的旁路退役而删除——「不存在」正是退役已落地的状态 -->
  门禁：`cargo test --lib` 2094 通过 / 0 失败，`node scripts/gate.mjs` 130/130 全绿
  （含 e2e 78/78）。基线随本批下调三格，各带归属说明：`rustTests` 2097→2094、
  `ciRustTestsTotal` 1436→1433、`LEGACY_ROLE_WRITE_BASELINE` 151→146。
- 2026-10-23 步 5c（收束格全部归 `close` 执行体）。断环的方式是**让两个执行体写的格
  不相交**，不是改判据：`chat.assistant.final` 与 `chat.assistant.fallback` 两条收束格
  全归 `close`，`execute` 那一步只跑模型往返。代价是收束行的前件得能看出「模型没答成」
  ——`has_final`（本幕最近一条 `response(port=model)` 的 `calls` 为空）在中止轮与适配器
  失败轮**不成立**（答都没答成，哪来应答），于是新增视图字段 `turn.closeable`
  （`turn_head` v6 → v7）：`has_final` ∨ `model_outcome_failed`。
  `model_outcome_failed` 的口径：幕锚之后最新一条 `response` / `failed`（`port=model`）
  是 `failed`，**且**同一 `(port, id)` 的 `request(port=model)` 也在幕锚之后。后半句不是
  装饰——崩溃恢复时租约看护会在幕锚之后落一条 `failed(port=model)`，它收的是上一世那笔
  卡住的调用（`request` 在锚之前）：少了它，恢复幕一进门就把「重跑一次模型往返」读成
  「这一轮已经答不上来了」。
  运行侧：模型往返的三种收法（答完了 / 模型没答成 / 中止）不再就地落格，而是把**结局**
  寄存在 `Prepared::closing`（新枚举 `PendingClose` 三变体），由 `close-turn` 行的执行体
  落——中止格与兜底格因此从执行体内部挪到了收束动作里。`close-turn` 的出口按
  [PL-017](../004-plan/017-端口细粒度化.md "PL-017-端口细粒度化") §3.1 复原成
  `final | fallback`（步 5 第一批为绕环临时收窄成只有 `final`，本批复原）。
  **两条控制面失败仍留在 `execute` 就地落**：模型往返压根没派发（行不欠 / 端口没声明）
  与工具批开不出来（模型要了工具而本轮没有分发通道）。它们在日志上**没有**模型的终局
  事实（`failed(port=model)` 落不下来）⇒ 收束行开不出账 ⇒ 那两格只能由 `execute` 写，
  走它的**过渡豁免**（`loop_rows::ACTORS` 里 `execute` 那条的 `transitional` 现在只有
  `chat.assistant.fallback` 一条，步 5 第一批时是 `chat.assistant.final`）。生产里这两条
  都不可达（模型调用点恒带分发通道：工具轮带、定稿答话轮走 `final_reply` 一步不发请求）。
  **反向判据实测**：① 把 `closeable` 退回 `has_final`（去掉 `model_outcome_failed` 一支）
  ⇒ `turn_runner.test.rs::aborted_run_closes_the_cell_with_aborted_status` 转红
  （「中止必须在结果里可见」——中止轮收不成，结局根本交不回来）；② 把
  `chat.assistant.final` 放回 `execute` 的 `writes` ⇒ 装载期判红
  `DependencyCycle { cycle: ["close-turn", "execute-turn", "close-turn"] }`
  （`loop_rows.test.rs::the_production_table_loads_with_no_gap`）。
  用例：`decl/mod.test.rs` 三条（`closeable` 正向 / 上一世的失败不算本幕的 / 收束过就不欠）
  + `loop_rows.test.rs` 一条（收束行在「本幕模型往返以失败收场」时开账）。
  门禁：`cargo test --lib` 2098 通过 / 0 失败，`node scripts/gate.mjs` 130/130 全绿
  （含 e2e 78/78）。基线随本批上调两格：`rustTests` 2094→2098（门禁自动回填）、
  `ciRustTestsTotal` 1433→1437（同批新增的 4 条无 `cfg` 门，CI 求和同样多这 4 条）。
  **与 PL-017 §3.1 的一处字面偏差**：那一行写的是 `owes: turn.has_final ∧ ¬closed`，
  今天是 `turn.closeable`（`¬closed` 折在 `closeable` 里）。偏差的来源是方案自身的
  缺口——§3.1 的前件只认「答完了」，而 §3.1 的**出口**却写着两条收束格（兜底格与中止格
  都要落），前件认不全时出口的那一格永远开不了账。步 5 第一批已把这个缺口记在案
  （「要接管它们得先给『没收束且不会再有应答』立一个视图字段」），本批就是那个字段。
- 2026-10-23 步 5e（口径 5：审批卡中断 → 唤醒 → 续跑 的 CLI 实证 + 判据脚本一处口径订正）。
  实测会话 `verify-t244-f`（Provider `LMStudio`，网关 `127.0.0.1:9241`）：第一程里模型要
  `http_request` 打 `https://example.com`，该工具风险等级 `high` ⇒ 要审批 ⇒ CLI 没有应答
  入口 ⇒ 本轮停在审批卡上。日志里的派发序列是 `execute-turn`（执行体 execute）→
  `open-tools`（执行体 opener）→ `authorize-tool`（执行体 authorizer）→
  `deny-tools`（执行体 courier），CLI 印的是「⏸ 等待用户动作：需要确认：发送 HTTP 请求…」
  与「CLI 没有回答入口：本轮停在这里，请在桌面端作答后继续」。事实面那一笔走的是
  `request`（seq 12）→ `authz.denied{detail:"needs_approval"}`（13）→
  `failed{why:"needs_approval"}`（14）—— **没有 `sent`**：没放行就没派发。
  第二程**重启进程**后放行（经与桌面端同构的 `/api/v1/invoke` 边界送
  `vdfs/action(…, "approve")`，HTTP 200 + `ok:true`）：唤醒落成 `request(port=tool)`
  id `tool-resume-0-32134f43-1`（seq 20），派发序列变成 `authorize-tool`（authorizer）→
  `dispatch-tools`（courier）→ `execute-turn`（execute）→ **`close-turn`（close）**；
  事实面 `authz.granted`（27）→ `sent`（28）→ `response`（29），模型再跑一轮后
  `response(port=model)` 带 `calls: []`（33）⇒ `has_final` 成立 ⇒ 收束格
  `chat.assistant.final`（34）由 `close-turn` 行的执行体落。
  **判据脚本一处口径订正**：`obligation-ring-accept.mjs` 的 J1 原本要求每一组端口往返都有
  `sent`，而这一笔被拒的调用**不该**有 `sent`（e2e T78 判的正是「拒 ⇒ 不得派发」）
  ⇒ J1 改为「缺 `sent` 只在没被拒时判红；被拒却派发了判红」。改前 `verify-t244-f` 红在
  「J1 tool-0-d0-model-0-d0-s0-32134f43: 缺 sent」；改后常规两幕 `verify-t244-a`
  （纯问答 + 工具轮，58 格 / 11 组往返，`HTTP 8 / sent(model) 6 / 豁免未入账 2`）与
  审批续跑 `verify-t244-f`（39 格 / 6 组往返，`HTTP 3 / sent(model) 2 / 豁免未入账 1`）
  在 `--strict` 下全项通过。豁免的那 1 与 2 是 `classify` 插件内部的判决往返，归
  [T-247](247-插件内部模型往返进环.md "T-247-插件内部模型往返进环")。
  **一处与口径 3 的偏差**（当场登记，不顺手改）：定稿答话轮（`classify` 判 `Answered` 的
  短路轮）的收束格**不经** `close-turn` 行——那一轮一次模型往返都不发，日志上没有
  `response(port=model)` ⇒ `turn.has_final` 不成立 ⇒ 收束行开不出账；它的收束格由轮首
  准备 1b 分支就地落（`turn_runner/prepare.rs`）。这不是实现取巧，是 ADR-121 的 `has_final`
  口径与 PL-017 §3.1 的行形状合起来覆盖不到这一轮。缺口与三个候选形状登记为
  [T-253-定稿答话轮的收束格不进环](253-定稿答话轮的收束格不进环.md "T-253-定稿答话轮的收束格不进环")
  （裁决先行，未裁决不动模块）；本任务的 `exit` 与口径 3 同批订正（例外条款 + 缺口归属）。
