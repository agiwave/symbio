---
kind: task
id: T-008-session存量能力迁移
title: S7：session 存量能力逐批迁出
status: doing
track: arch
prio: next
created: 2026-10-29
parent: T-001-能力全面拆分
deps: []
where: `docs/003-design/026-能力拆分与影响类.md` §2.3, `crates/symbio-plugin-session/src`, `crates/symbio-plugin-session/src/projections`
exit: `crates/symbio-plugin-session` 非测试代码里不再有按能力名 / 工具名的读法或策略；每个模块都能指认归属（能力包或五类残留之一）；按名字出富化的读法归零；四条未建能力（#36/48/50/51）随 conation / heartbeat 包落成能力面（最小形态即可：事实 / 段 / 工具之一）；迁移提交不含机制增量（`EX-007`：机制面新增行为 0、删除行不限（`D-283`），`session` 只含删除与绑定切换）；`plugin-surface-audit` 的 HS-002 / HS-006（三臂只许不涨，D-278）对 `session` 无新增，`gen-facts --check` 无漂移；每批落成的能力包按 `EX-001` 复核（首次落地 diff = 0，迁移只在删除侧变化）；补件乙 / 丙（能力开关的事实形状、跨窗三态契约）先立 `### D-NNN` 再动代码；补件次数与迁移批次数并进 `EX-007` 读数，比值 ≤ 1:1
---

# S7：session 存量能力逐批迁出

## 内容

session 比 loop 更接近「会话基底」，迁移的第一步不是搬代码，而是**逐个模块判归属**：
它是某个能力的实现，还是五类残留（环 / 写口 / 全局守门 / 过程体 / 会话基底）之一？
只有「能力实现」才搬；搬的时候按 owner 决定新 crate，不按目录。

名册在 session 的 5 行是：时延分层（#13）、打断与插话（#14）、主动开口（#15）、
目标自生成（#31）、内在动机（#32）。另有四条「拆（未建）」随包落地——主动求知（#36）、
节奏与时机（#48）、好奇与探索（#50）、回避倾向（#51）：它们与 conation / heartbeat
同包，包在本段建出来，这几条就以能力面（事实 / 段 / 工具之一）跟着落地，只落最小形态。
另有一些不在名册但同样按名字存在的形状：

- **按名字出富化**：读侧按工具名 / 能力名挑字段（`changes` 一类），改成读视图或声明表。
- **能力内的运行期状态**：自动唤醒的节拍等，状态应随能力包走
  （能力包持有自己的状态，宿主只给租约与预算）。压缩失败的计数与冷却已随会话内实现
  删除；它在能力包内如何重建（跨轮失败记忆归谁、永久失败格是否进 `PortErrorClass`）
  另记 [T-049-压缩失败跨轮抑制与重试出路](049-压缩失败跨轮抑制与重试出路.md)。
- **与能力相关的折叠**：谁生产谁消费就归谁；两个插件都读的走 S8 的名字读。
- **纯基底**：存储、转写、收件箱读写接口不搬——它们是残留类的判据对象。

**迁移提交不含机制增量**（`EX-007` / D-247）：某一行若发现「还缺一条端口才迁得动」，
停下，把补件单列为带 `### D-NNN` 的独立提交（通用、不含能力名），再恢复迁移；不在迁移
提交里顺手加分支或状态字段，也不把能力实现搬进 `core` / `kernel` 冒充端口。

## 逐模块归属表（处理过程第 1 步的产出）

判据只有一句：**这个模块回答的是「某个能力该怎么判断」，还是「会话本身怎么存取与推进」**。
前者搬，后者留。五类残留的缩写：环 / 写口 / 守门（全局守门）/ 过程体 / 基底（会话基底）。
行数是**非测试**源码行，用来量这一格的分量，不用来量进度。

### 一、能力实现（要搬 / 已拆到只剩基底）

| 模块 | 它是什么 | 名册行 | 去向 | 今天挡在哪 |
|---|---|---|---|---|
| `conation.rs`（208） | `conation` 投影：从事件折出「表达过哪些欲、各自升没升格」 | #31 目标自生成 | **已搬** → `symbio-plugin-conation`（`ViewDecl` + `agg`，`D-281`） | 无——补件甲落定后搬完：折叠与两半（`eval` / `agg`）都进包，本插件三处读方改按名字读 |
| `config.rs::conation_enabled` + `ConationPolicy` | 「这一轮该不该自发开一个长目标」的开关与判据 | #32 内在动机 | **已搬** → `symbio-plugin-conation`（自申报债 `conation.promote` + 端口执行体，`D-386`） | 无——`D-379` 补上开关事实，`D-386` 把开关读法、目标宽度判据（`MAX_GOAL_LEN` 200 字节）、授权闸与 `ApprovedIntent` 一起交回能力包；`session` 删掉了全部判据与闸门，只写触发 / 健康 / 欲三格 |
| `transcript/inbox.rs::PreemptionDecider`（约 120 行，已删） | 「这一条插话该不该挂起在跑的任务」 | #14 打断与插话 | **已知边界，不迁**（`D-388`）：判定面已删（三态 / 判定者 / 落格函数随死码一并删除），`task.held` 无生产写方——本条不再有触发条件，重启需新 D 号 |
| `active.rs` 的其余部分 + `transcript/inbox.rs`（881） | 收件箱 FIFO、在途中止信号、工作态迁移、`turn_done` 唤醒 | — | **留 基底** | 它们是「收件箱的读写接口」，判据对象本身就是基底（本任务 §内容第 4 条） |
| `projections/slo.rs`（147）+ `tiers.rs`（55） | 时延样本按档位折叠、分位数取法 | #13 时延分层 | **已搬** → `symbio-plugin-slo`（`ViewDecl` + `agg`，`D-281`） | 无——补件甲（`D-281` 两半折叠）+ `D-282`（读数视图可读「有格位、无事实声明」的断言格）落定后搬完：`eval` 交一片、`agg` 交 N 片，`session` 只按名字读 |
| `projections/fallback.rs`（144） | 每档位的兜底率 | 与 #13 同族（SLO §1.2 两列共用 `tiers`） | **已搬** → 随 #13 同包 | 无——与 `slo` 共用 `declared_tier`，同包搬走，那条唯一判据没有跨包 |
| `autonomy.rs`（122） | 无人干预完成率 | 名册无此行（DE-008 §3.1 外部效标） | 随 #31/#32 同包（自主层的读数） | 唯一读方是 `stats`；与 `conation` 同一条迁移面 |
| `projections/span.rs`（110） | 会话级时域跨度 | 名册无此行 | **已搬** → 随 #13 同包 | 无——同族读数，跟着时延列一起进包 |
| `token_ledger.rs`（107）+ `spend_ledger.rs`（133） | 用量 / 金额台账（按主体、按币种） | #18 成本控制 | **不随 #18**：两条折叠只有 `stats` 一个读方，按 D-221「只被 owner 自己在读 ⇒ 住 owner 的 `projections/`」它们已在正确位置；`cost.ledger` 读的是信封字段 `cost_ms`，这两本读的是载荷里的 `usage` / `spend`——搬进成本包要先让那两个字段成为声明过的事实形状，那是另一条机制补件 | 无——归 S8 的那一半（core 的 `cost` 折叠）已落 `crates/symbio-plugin-cost` |
| `projections/changes.rs`（161）+ `changes.rs`（160） | 工作目录变更账 + 它的只读出口 | 名册无此行 | 随工作目录能力（`symbio-plugin-vdfs` / `local`） | 出口是 VDFS 地址面的一段，与 `workdir` 的读写同属一域；搬它要连出口一起搬，那是另一段的事 |
| `transcript/autonomous.rs`（539） | 自主触发的**落格与开轮**（四格事实 + 入队） | #15 主动开口 | **已拆（受限）**：触发时机在 `symbio-plugin-heartbeat`；本文件剩下的「谁有资格在什么时刻写这个会话的事实源」是存储所有者的写窗口纪律，留基底（`D-076` 裁决一 #5/#6）——这一半按裁决**不是能力**，不再有迁移动作 | 无——拆分已完成，判据在 `D-076` |
| `transcript/compression.rs` | 压缩节点的发射器（前端可见的「正在压缩」） | L5 的落点 | **已退役**：压缩由 `symbio-plugin-compact` 承接（两行债 `model.assist` / `compact`，`D-336`），会话内实现已删 | 无——失败抑制与重试出路的缺口另记 [T-049-压缩失败跨轮抑制与重试出路](049-压缩失败跨轮抑制与重试出路.md) |

### 二、五类残留（留）

| 模块 | 残留类 | 为什么不是能力 |
|---|---|---|
| `chat_session.rs` / `read` / `write`（1061） | 基底 | 会话引擎的唯一实现：孤儿清理、轮次窗口、配置读取的共享语义 |
| `store/`（`mod` 464 / `wal` 802 / `layout` 407 / `facts` 273） | 基底 | 一种磁盘布局、一种驻留方式（**无事实源的驻留方式已退役**，D-329）；`Store` 是机制层的端口（名册 #5 时序历史，`留 基底`） |
| `transcript.rs`（621）+ `deliver` / `publish` / `supplements` | 基底 / 过程体 | 转写的唯一写入点与发射器；投递是过程本身，不是状态转移 |
| `plugin.rs`（809）+ `plugin/*`（`nodes` / `run_actions` / `resident` / `words` / `vdfs_*`，约 3200） | 基底 | VDFS 地址面的路由、订阅与各域读写——本任务 §内容第 4 条点名的判据对象 |
| `commands.rs`（613）+ `message_events.rs`（654） | 写口 | 「谁能改会话」的唯一答案；消息变更事实的唯一写方 |
| `grants.rs`（71） | 守门 | 授权闭集的一格（审批永久放行），落盘仍走配置文档唯一路径 |
| `capabilities.rs`（180） | 基底 | 工具与选项的 traverse 收集，收集机制的契约在机制层 |
| `convergence.rs`（262）+ `runtime.rs`（146）+ `working.rs`（73）+ `worker_progress.rs`（217） | 基底 / 过程体 | 运行态的唯一出口、上一轮结局的投影、工作态闭集、worker 进展的读法 |
| `stats.rs`（667）+ `stats_reading.rs`（847） | 基底（只读出口） | 读数的**出口**，不是读数的口径；`capability_views` 已经不认识任何视图名 |
| `paths.rs`（146）+ `types.rs`（290）+ `options/`（255）+ `config.rs`（548 除 `conation_enabled` 外） | 基底 | 路径派生、会话实体、选项字段声明、配置形状 |
| `workdir/`（627） | 基底 | 工作目录的 VDFS 场景实现，不存在第二套树协议 |
| `projections/checkpoint.rs`（52） | 基底 | 断点恢复的数据形态（DE-035），恢复 = 重放 + 纯函数 |
| `temporal.rs` | 基底 | **已删**：`temporal_context` 全仓无生产调用点 |
| `declarations.rs`（119） | 基底 | `system.*` 留这里；`conation.expressed` 随 #31 迁出（同批改名，不留两份） |
| `lib.rs`（46） | 结构 | 模块清单 |

### 三、名册 5 行 → 表里的位置

| # | 能力 | 表里在哪 | 处置 |
|:-:|---|---|---|
| 13 | 时延分层 | 一 · `projections/slo.rs` + `tiers.rs` | **已拆**：能力包 `symbio-plugin-slo`（三视图 + 两半折叠），`session` 只剩读方 |
| 14 | 打断与插话 | 一 · `transcript/inbox.rs::PreemptionDecider`（已删）+ 二 · 收件箱（基底） | **已知边界，面已删**（`D-388`）：今天没有预占判定承担者，重启需新 D 号；收件箱留基底 |
| 15 | 主动开口 | 一 · `transcript/autonomous.rs` | **已拆（受限）**：触发时机在 heartbeat 包；落格与开轮是写窗口纪律，留基底（`D-076` 已裁） |
| 31 | 目标自生成 | 一 · `conation.rs` | **已拆**：能力包 `symbio-plugin-conation`（事实 + 视图 + 两半折叠），`session` 只剩读方 |
| 32 | 内在动机 | 一 · `config.rs::conation_enabled` | **已拆**：能力包 `symbio-plugin-conation`（自申报债 + 端口执行体），`session` 不再读判决、不再构造 `ApprovedIntent`（`D-386`） |

### 四、这张表给出的结论：真缺口只有一处（补件甲），#14 不是补件

逐行看下去，五条里没有一条卡在「代码难搬」，也没有一条卡在「找不到归属」。#15 已经拆完
（触发时机在 heartbeat 包，剩下的半边按 `D-076` 不是能力）。剩下四条里，#13 / #31 / #32
共用补件甲那条缺口；补件乙已落（`D-379`）；#14 经查**不是补件**，判定面已删
（`D-388`）——能力不存在，不是阻塞：

**补件甲：读数能力的读方要类型，而跨插件只能按名字读 JSON。** 这是 §一 里六条读数折叠
（`slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`）的共同阻塞，也是本段
唯一真正的架构缺口。今天的形状是：owner 交 `ViewDecl`（`eval: fn(&[Event], i64) -> Value`），
读方经 `read_view` 拿到 `Value`。**只读一格**没问题（`capability_views` 那一列就是原样装一格，
它不认识任何视图名）；**要参与算**就不行——`stats` 的 `tiers` 列要用 `TierLatency::percentile`
（最近邻取法，`ceil(p/100 × n) - 1`）与 `SloLatencyView::merge`（同档位样本并集后重排），
`stats_reading` 要用 `ConationView::merge` / `adoption_rate`。这三处都是**算式**，不是读数：

- 把算式留在 `session` ⇒ 名字读回来的是 JSON，`session` 得自己再从 JSON 折一遍 —— 两份口径，
  正是 `D-251`「两处各判一遍必然漂移」要消灭的形状；
- 把算式搬进能力包 ⇒ `session` 要调它 —— 跨插件调用，`E-009` 直接红；
- 让 `session` 反序列化成同名类型 ⇒ 形状有了两个 owner（`DuplicateFact` 的同一条理由）。

三条都堵死 ⇒ 这一格在裁决之前**没有合法形状**，而这与「搬代码」无关。它已按 `D-269` §5.2
记进 [DE-025-能力包与扩展面](../003-design/025-能力包与扩展面.md "DE-025-能力包与扩展面")
的缺口账（「读侧折叠 + 多切片」一行，结论 `开格`）：两条准入都满足（不含能力名、六条读数
能力要用）。**已裁并已落：`D-281`**——`ViewDecl` 末尾加 `agg: Option<fn(&[Value]) -> Value>`，
owner 把「一片怎么折」（`eval`）与「N 片怎么合」（`agg`）一起交，两半同经冻结声明表；读方
直接从 `ViewDecl.agg` 取用，`None` ⇒ 那一格可诊断地缺席并出声（不是恒等）。
补件提交只动 `kernel` / `plugin-api`，`session` 一字未改，
符合 `EX-007` 的补件口径。

**补件乙：能力的开关要有一条事实形状。** `D-252` 第 2 条已经把口径裁死了（能力读
`session.config` 这条事实，不读 `SessionConfig` 活值），`memory` 的活集去重已经照它落地。
**已裁并已落：`D-379`**——快照是配置定义的平铺投影，`conation_enabled` 补进载荷，
闸门改读最近一条 `session.config`（缺格取出厂值 `false`）；写方唯一是轮首装配，
读方是 conation 自申报的债（`D-386` 已迁）只按共享名读**本轮** config。它与 #14 无关：
#14 与开关形状无关；它的判定面已删（`D-388`）。

**补件丙不成立（`D-380`）。** #14 原先写的「判定只能在忙窗、落格只能在空闲分支，中间由
`PreemptPending` 三态背着」是**设计意图**，不是代码现状。没有两个 owner，就没有「跨窗
开面」可开。因此 #14 记为**已知边界，面已删**（`D-388`）：判定器、三态与落格函数全部
删除，今天没有预占判定承担者；重启需新 D 号，`S7` 的补件计数不 +1。

**所以 S7 的下一步是拿补件甲搬剩下的读数能力**：它一条解锁两条读数能力（`#13` / `#31`），
且是六条读数折叠（`slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`）的
共同入口——本段真正的门。这条门**已经开了**（`D-281`），`#31` 与 `#13` 都已搬完；剩下的
读数折叠按同一形状各落一包。

### 五、分量读数（为什么 S7 与 S6 形状不同）

本插件约 21k 行非测试代码里，**能力实现约 1.4k 行**（`conation` 208 + `autonomy` 122 +
`slo` 147 + `tiers` 55 + `span` 110 + `fallback` 144 +
`conation_enabled` 相关约 40 + `changes` 两件 321 + 台账 240），其余约 19k 行是会话基底与
过程体。这正是「session 比 loop 更接近会话基底」这句话的量化形式：

- S6 搬的是**控制面里的策略**（一段一段各自成块，搬完就少一块）；
- S7 搬的是**长在基底上的读数与判定**（每一条都嵌在基底的读出口里，搬一条要动一片读方）。

两条段的目标一样（`loop` / `session` 只留五类），但**判据的密度不同**：S6 的读数看「删掉了
什么」，S7 的读数看「还剩几条读方在按类型读别人折出来的数」。后者今天有六条，那就是补件甲
的规模。


### 六、读方测试的边界：数字归 owner，契约归读方

#31 搬完之后，第一版做法是**给会话的测试夹具补一份「算法一样」的替身**（把能力包的折叠
抄进 `declarations.test.rs`，让原来那三条按数字断言的用例继续绿）。这是错的，而且仓库自己
的判据当场就红了：`stats.test.rs::capability_views_column_follows_the_shared_manifest`
断言「本测试进程没链接 owner ⇒ 每一条共享视图都必须是 `unavailable` / `unknown`」——
替身把那一条从红改成了绿。三条理由，任何一条都足够：

1. **替身是第二份实现**。折叠的口径抄到读方的测试里，两处各自演化时，能力包的用例不会红
   （它测的是自己那份），读方的用例也不会红（它测的是抄的那份）。这正是 `D-251`
   「两处各判一遍必然漂移」的形状，只是这次发生在测试里。
2. **它让 T3 失去判据**。「卸载这个能力包，系统仍然完整、且能力确实消失」这句话要靠
   「没有 owner ⇒ 读不到 ⇒ 按退化方向走」来判；替身一交，卸载与否都是绿的，
   于是这句话变成不可判——而不可判的判据比没有判据更坏，它看起来是绿的。
3. **它把「测试通过」偷换成「复制品通过」**。读者看到「主动性列有用例」会以为这条链路
   被验证过，实际被验证的是那份抄件。

所以分工改成三处，各管一段，互不替代：

| 谁 | 钉什么 | 为什么只能在它那里 |
|---|---|---|
| 能力包（`symbio-plugin-conation`） | **数字**：升格靠溯源、as-of、分母含未升格、合并可加、`declared` 只含升格过的那几条 | 算法住在这里；它不链接会话 |
| 会话（`stats` / `stats_reading` / 自主触发） | **契约**：按名字读、原样透传两格、**缺席怎么退化**（空列 / `null` 轴 / 不升格） | 这三条不依赖 owner 的算法，在「只有一半」的二进制里测得了——而「只有一半」正是它们的被测条件 |
| e2e（`t36`） | **组合**：真实装配里跑两个 tick，就绪集始终只有一条心跳任务；`fleet.axes.conation` 由 `agg` 汇总 | 组合要求两半同时在场，只有真实装配满足 |

落地动作（都是**减**）：删掉 `conation_column_recomputes_and_moves_with_the_wal`（120 行，
数字断言）；`gate_opens_the_long_goal_exactly_once_per_goal` 改写成
`without_the_capability_pack_nothing_promotes`（断言的是退化方向，不是去重结果）；
`fleet_axes_aggregate_across_sessions` 把主动性轴从「有据的零」改成 `null`（缺席不伪装成
读数）；去重与汇总的**数字**在 `t36` 里补两条（第二个 tick 只开一格 + `fleet.axes.conation`
由汇总的分子分母重算）。会话侧的测试代码净减约 110 行，且不含任何折叠逻辑。

`#13` 是这条分工的**第二遍**，也是第一次在「读方原本自己算」的列上走完：`stats.test.rs` 里
原来有五条按数字断言的用例（时延样本 / 分位数 / 兜底率 / 跨度 / 跨会话合并），搬完之后的
处置是——

| 原来那条 | 现在归谁 |
|---|---|
| `stats_recompute_equals_direct_apply` 里的 `row.p95 / samples / rate` | owner（`slo/src/decl.test.rs`）+ e2e `t28`（`tiers[0]` 逐格对账） |
| `reverse_case_each_column_moves_when_the_wal_changes` 的档位列 | owner（删 `final` ⇒ 样本 1 → 0）；本处改成断点列与成本列的反向用例 |
| `span_column_recomputes_and_moves_with_the_wal` | 删；改成 `unavailable_columns_take_their_documented_empty_shape`（缺席交什么形状） |
| `fleet_axes_aggregate_across_sessions` 的跨度中位数 | 删；改成 `fleet_axes_are_shaped_and_the_autonomy_family_still_aggregates_here`（自主程度仍是本插件的投影，跨会话合并在本处判） |
| `panel_matches_the_columns_it_describes` 的档位行 | 保留，但输入改成**合成的 `TierRow`**：面板排版是本插件的代码，验的是「面板里的数 = 列里的数」，与 owner 在不在无关 |

净效果：会话侧删掉 4 个旧投影文件（`slo` / `fallback` / `span` / `tiers`，含三个 `.test.rs`），
`stats.test.rs` 的断言条数下降，而**判据强度不降**——每一条都搬到了能真正判它的那一侧。

**这条经验对后面的每一批都成立**：能力搬走时，读方的用例只能留下「我怎么读、读不到怎么
办」，凡是「算出来是多少」的断言都跟着实现走——否则搬完只是把实现复制了一份，而规模没降。

### 七、迁移批次里最容易踩的一条：新增行别带字面量

`EX-007` 判「`loop` / `session` 没有新增带能力名的源码行」，`S7a` 上抓到的第一件事不是别人
的能力名，而是**自己的插件 id**（`plugin_warn!("session", …)`，5 处）与**出口键名**
（`axes` 里那一格）。两处都改成一个名字只定义一次的写法——前者取清单生成物
`symbio_plugin_api::PLUGIN_ID_SESSION`，后者从 owner 给这一格起的名字派生（`D-387` 之后
取报表登记的 `fleet`：`axes.conation` ⇒ `conation`，`stats_reading::fleet_key_of`）。

不是为了讨好判据：读方在出口处再抄一遍能力名，等于给自己开第二份叫法，漂移了没有东西
会变红；`"session"` 那几处本来就是「同一个 id 抄了十几遍」的存量，改完少一处定义。
本批只改**新增的**那 5 处——`transcript/autonomous.rs` 里还剩 12 处存量，随下一次触碰那个
文件时一并收口：迁移批次不该顺带重排无关行，否则「只搬运」这句话就查不清了。


## 处理过程

1. ~~先出一张逐模块归属表（能力 / 五类残留），名册 5 行逐行指到表里。~~ **已落**（见上一节
   「逐模块归属表」）：55 个非测试模块逐格判完，5 行指到表里；结论是真缺口只有补件甲一处，
   一条解锁六条读数能力——它是本段的入口。
2. **先裁补件甲，再搬读数**。它不改 `session`，只回答「多切片合并是能力的一部分还是读方的
   一部分」；裁出来之后 `slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`
   六条按同一个形状各落一包（可同包，账里写明）。补件乙（开关的事实形状）已落 `D-379`，
   解锁 #32 的开关面；#14 经查不是补件（`D-380`），面已删（`D-388`）。
   **补件甲已裁并已落（`D-281`）**：`ViewDecl::agg`（`None` ⇒ 该格可诊断地缺席，不是恒等）。`#31` 与 `#13` 都已用它搬完（见 §六）；`#13` 还额外
   用上 `D-282`（读数视图可以读「有 `EventDecl` 格位、无 `FactDecl` 声明」的断言格，
   进报告名单而不是判 `UnknownFact`）——`turn.dispatch` 的 `tier` 正是那一类格子。
3. 按表分批：能独立提交的，一个能力一个提交；共用状态的先拆状态再搬。
   - 时延分层：档位是 `Decl.tier` 参数，读数折叠独立成包；
   - 打断与插话：裁决走 I-3，收件箱事实走 I-1；E2 强制留核；
   - 主动开口：与 heartbeat 归并，session 的 initiator / gate 只留强制点；
   - 目标自生成 / 内在动机：判定生产者进 conation 能力包，IntentGate 留核；
   - 四条未建（#36/48/50/51）：随 conation / heartbeat 包交最小能力面——它们是账里
     唯一「包先建、面后补」的一档，不单独建段。
4. 每批跑端到端与 `EX-003` 反向用例，`plugin-surface-audit` 的 HS-002 / HS-006 读数只降不涨；核对 `EX-007`（机制面新增行为 0，删除行不限——`D-283`；`session` 只含删除与绑定切换）。
5. 全部搬完后核对：session 的 `lib.rs` 导出面只剩基底接口，能力名的读写为零。
6. 与 S6 的终态合并检查，再进 S8。

### 进度

| 项 | 状态 | 读数 |
|---|---|---|
| #31 目标自生成 | **已落**（`a675118e`） | `crates/symbio-plugin-conation` 交出投影 + 两半折叠（`eval` / `agg`）；`session` 三处读方（`stats` 列 / 实例级 `aggregate` / 自主触发）全部按名字读；`conation.rs` 已删，`conation.expressed` 的格位随包走。验证：`cargo test -p symbio-plugin-conation` 9 条通过，`t36-autonomous-initiator` e2e 通过（30.4s）。**本片剩余 diff = 0**（工作区两次核对为空），无新增机制行 |
| 自主程度轴 `conation.autonomy` | **已落**（`d05fb921` + `b0d1a286`） | 折叠迁入 `symbio-plugin-conation`（`eval` / `agg` 两半，`D-281`；读 `task.held` / `task.asserted` 按 `D-282` 第二种读法，与 `task.readyset` 同形）；`session` 两处读方（`stats` 列 / `fleet.axes.autonomy`）改按名字读，读不到 ⇒ `null`；`autonomy.rs` / `autonomy.test.rs` 与唯一读它的 `TASK_VOCAB` 在切换提交里同批删除（`D-245`）。数字 6 条用例随包走，读方测试改判契约。验证：`cargo test -p symbio-plugin-conation` 16/16、`cargo test -p symbio-plugin-session --lib` 469 passed / 1 ignored、e2e `t28` / `t36` 通过；`capability-migration-audit --last` 绿（1 迁移批次 / 0 补件） |
| 四条未建能力（#36/48/50/51） | **声明面已落** | #36 / #50 / #51 进 `symbio-plugin-conation`（`conation.inquiry` / `conation.curiosity` 落 `Conation × Opened`，`conation.avoidance` 落 `Conation × Asserted`）；#48 进 `symbio-plugin-heartbeat`（`heartbeat.cadence` 落 `Control × Asserted`）。四条各交名字 / 格位 / 一格载荷 / `by = None` 归宿行，**写方待接**，故都不进 `shared_events`（单消费方，`CS-007`）。`session` / `loop` / 机制面零 diff |
