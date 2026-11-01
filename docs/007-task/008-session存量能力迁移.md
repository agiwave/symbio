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
exit: `crates/symbio-plugin-session` 非测试代码里不再有按能力名 / 工具名的读法或策略；每个模块都能指认归属（能力包或五类残留之一）；按名字出富化的读法归零；四条未建能力（#36/48/50/51）随 conation / heartbeat 包落成能力面（最小形态即可：事实 / 段 / 工具之一）；迁移提交不含机制增量（`EX-007`：`core` / `kernel` diff = 0，`session` 只含删除与绑定切换）；`SC-006` 的基线里不再有 session 条目
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
- **能力内的运行期状态**：压缩熔断计数、自动唤醒的节拍等，状态应随能力包走
  （能力包持有自己的状态，宿主只给租约与预算）。
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
| `conation.rs`（208） | `conation` 投影：从事件折出「表达过哪些欲、各自升没升格」 | #31 目标自生成 | `symbio-plugin-conation`（`ViewDecl`） | 读方是 `stats` / `stats_reading` / `transcript/autonomous`（本插件三处）。三处都要**类型**（`ConationView::adoption_rate` / `merge`），改名字读只剩 JSON ⇒ 见 §四 的那一条共同阻塞 |
| `config.rs::conation_enabled` + `ConationPolicy` | 「这一轮该不该自发开一个长目标」的开关与判据 | #32 内在动机 | 同包（与 #31 同一判定生产者） | 开关今天住在 `SessionConfig`（会话配置形状），而 `D-252` 第 2 条要求能力读 `session.config` 这条**事实**、不读宿主的配置类型。**阻塞：开关要有一条事实形状**，那是另一条补件 |
| `active.rs::PreemptionDecider`（约 120 行） | 「这一条插话该不该挂起在跑的任务」 | #14 打断与插话 | `symbio-plugin-preemption`（I-3 裁决 + I-1 收件箱事实） | 判定与落格被**刻意分开**（`PreemptPending` 三态：判在忙窗、落格在空闲分支），中间那段时间由会话状态背着。搬判定就要把这三态的跨窗契约开成端口——那是补件，不是搬运 |
| `active.rs` 的其余部分 + `transcript/inbox.rs`（881） | 收件箱 FIFO、在途中止信号、工作态迁移、`turn_done` 唤醒 | — | **留 基底** | 它们是「收件箱的读写接口」，判据对象本身就是基底（本任务 §内容第 4 条） |
| `projections/slo.rs`（147）+ `tiers.rs`（55） | 时延样本按档位折叠、分位数取法 | #13 时延分层 | `symbio-plugin-slo`（`ViewDecl`，档位是 `Decl.tier` 参数） | 读方是 `stats` 的 `tiers` 列与 `panel_of` 的呈现段，两处都用**类型**（`TierLatency::percentile` / `SloLatencyView::merge`）。改名字读只剩 JSON ⇒ 类型与取法要么跟着搬（那 `merge` 就成了第二份口径），要么在 `stats` 里再实现一遍。**阻塞：跨会话合并的归属没说清** |
| `projections/fallback.rs`（144） | 每档位的兜底率 | 与 #13 同族（SLO §1.2 两列共用 `tiers`） | 随 #13 同包 | 同上，且它与 `slo` 共用 `declared_tier`——两列必须同包，否则那条唯一判据要跨包 |
| `autonomy.rs`（122） | 无人干预完成率 | 名册无此行（DE-008 §3.1 外部效标） | 随 #31/#32 同包（自主层的读数） | 唯一读方是 `stats`；与 `conation` 同一条迁移面 |
| `projections/span.rs`（110） | 会话级时域跨度 | 名册无此行 | 随 #13 同包（SLO 时延列的同族读数） | 同上 |
| `token_ledger.rs`（107）+ `spend_ledger.rs`（133） | 用量 / 金额台账（按主体、按币种） | #18 成本控制 | S8 的 `symbio-plugin-cost`（与 core 的 `cost` 折叠同包） | 归 S8 不归本段：它要等「熔断与 stats 改名字读」那一批一起动 |
| `projections/changes.rs`（161）+ `changes.rs`（160） | 工作目录变更账 + 它的只读出口 | 名册无此行 | 随工作目录能力（`symbio-plugin-vdfs` / `local`） | 出口是 VDFS 地址面的一段，与 `workdir` 的读写同属一域；搬它要连出口一起搬，那是另一段的事 |
| `transcript/autonomous.rs`（539） | 自主触发的**落格与开轮**（四格事实 + 入队） | #15 主动开口 | 拆（受限）：**触发时机已在 `symbio-plugin-heartbeat`**；本文件剩下的「谁有资格在什么时刻写这个会话的事实源」是存储所有者的写窗口纪律，留基底（`D-076` 裁决一 #5/#6） | 无——这一条的拆分已经完成，剩下的半边按裁决**不是能力** |
| `transcript/compression.rs`（158） | 压缩节点的发射器（前端可见的「正在压缩」） | L5 的落点 | 随 L5 的补件走 | 压缩帧要先进 `plugin-api`，本段不动 |

### 二、五类残留（留）

| 模块 | 残留类 | 为什么不是能力 |
|---|---|---|
| `chat_session.rs` / `read` / `write`（1061） | 基底 | 会话引擎的唯一实现：孤儿清理、轮次窗口、配置读取的共享语义 |
| `store/`（`mod` 464 / `wal` 802 / `layout` 407 / `facts` 273） | 基底 | 一种磁盘布局两种驻留方式；`Store` 是机制层的端口（名册 #5 时序历史，`留 基底`） |
| `transcript.rs`（621）+ `deliver` / `publish` / `supplements` | 基底 / 过程体 | 转写的唯一写入点与发射器；投递是过程本身，不是状态转移 |
| `plugin.rs`（809）+ `plugin/*`（`nodes` / `run_actions` / `resident` / `words` / `vdfs_*`，约 3200） | 基底 | VDFS 地址面的路由、订阅与各域读写——本任务 §内容第 4 条点名的判据对象 |
| `orchestrator.rs` + `orchestrator/*`（约 2550） | 环 / 过程体 | 一轮的生命周期：请求装配、消费循环、失败降级、限流、Stop 钩子。`consolidate.rs` 是压缩的**落点**（写口），不是压缩的策略 |
| `commands.rs`（613）+ `message_events.rs`（654） | 写口 | 「谁能改会话」的唯一答案；消息变更事实的唯一写方 |
| `grants.rs`（71） | 守门 | 授权闭集的一格（审批永久放行），落盘仍走配置文档唯一路径 |
| `capabilities.rs`（180） | 基底 | 工具与选项的 traverse 收集，收集机制的契约在机制层 |
| `convergence.rs`（262）+ `runtime.rs`（146）+ `working.rs`（73）+ `worker_progress.rs`（217） | 基底 / 过程体 | 运行态的唯一出口、上一轮结局的投影、工作态闭集、worker 进展的读法 |
| `stats.rs`（667）+ `stats_reading.rs`（847） | 基底（只读出口） | 读数的**出口**，不是读数的口径；`capability_views` 已经不认识任何视图名 |
| `paths.rs`（146）+ `types.rs`（290）+ `options/`（255）+ `config.rs`（548 除 `conation_enabled` 外） | 基底 | 路径派生、会话实体、选项字段声明、配置形状 |
| `workdir/`（627） | 基底 | 工作目录的 VDFS 场景实现，不存在第二套树协议 |
| `projections/checkpoint.rs`（52） | 基底 | 断点恢复的数据形态（DE-035），恢复 = 重放 + 纯函数 |
| `temporal.rs`（46） | 基底 | 用户消息落库前填的那一段 `<context>`；唯一调用点在入口装配 |
| `declarations.rs`（119） | 基底 | `system.*` 留这里；`conation.expressed` 随 #31 迁出（同批改名，不留两份） |
| `lib.rs`（46） | 结构 | 模块清单 |

### 三、名册 5 行 → 表里的位置

| # | 能力 | 表里在哪 | 处置 |
|:-:|---|---|---|
| 13 | 时延分层 | 一 · `projections/slo.rs` + `tiers.rs` | 拆（阻塞：见 §四 共同阻塞） |
| 14 | 打断与插话 | 一 · `active.rs::PreemptionDecider`（判定）+ 二 · 收件箱（基底） | 拆（阻塞：跨窗三态契约，另一条补件） |
| 15 | 主动开口 | 一 · `transcript/autonomous.rs` | 拆（受限）：触发时机已在 heartbeat 包，落格与开轮留基底（`D-076` 已裁） |
| 31 | 目标自生成 | 一 · `conation.rs` | 拆（阻塞：见 §四 共同阻塞） |
| 32 | 内在动机 | 一 · `config.rs::conation_enabled` | 拆（阻塞：开关要有事实形状，另一条补件） |

### 四、这张表给出的结论：四条卡在**三条**补件上，不是四条

逐行看下去，五条里没有一条卡在「代码难搬」，也没有一条卡在「找不到归属」。#15 已经拆完
（触发时机在 heartbeat 包，剩下的半边按 `D-076` 不是能力）；另外四条各缺一样**通用**的东西，
而每一样都不是为某一条能力定制的通道，因此按 `D-247` 第 1 类各自够格开一次补件：

**补件甲：读数能力的读方要类型，而跨插件只能按名字读 JSON。** 这是 §一 里六条读数折叠
（`slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`）的共同阻塞，也是本段
唯一真正的架构缺口。今天的形状是：owner 交 `ViewDecl`（`eval: fn(&[Event], i64) -> Value`），
读方经 `ViewRead` 拿到 `Value`。**只读一格**没问题（`capability_views` 那一列就是原样装一格，
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
经 `ViewRead::aggregate` / `aggregable` 取用，`None` 报 `ViewUnavailable::NotAggregable`
（第五种可辨的缺席，不是恒等）。补件提交只动 `kernel` / `plugin-api`，`session` 一字未改，
符合 `EX-007` 的补件口径。

**补件乙：能力的开关要有一条事实形状。** `D-252` 第 2 条已经把口径裁死了（能力读
`session.config` 这条事实，不读 `SessionConfig` 活值），`memory` 的活集去重已经照它落地。
但 `conation_enabled` 这一格今天仍只有「配置字段」一个形状，没有一条按名字的事实可读 ——
#32 因此搬不动。它与 #14 的跨窗三态契约是**两件事**，不能合并成一条补件（一条讲「开关怎么
读」，一条讲「判定与落格之间的那段时间谁背」）。

**补件丙：判定与落格之间的那段时间谁背。** #14 的 `PreemptPending` 三态是**刻意**的：
判定只能在忙窗做（事实源里只有上一轮的收束入格），落格只能在空闲分支做（`task.held` 要在
插话轮开跑之前就位、`task.progress` 要等它结束），中间那段时间由会话状态背着跨过去。
把判定搬进能力包，就要把这三态的跨窗契约开成一条面——它服务的是「判在 A 时刻、落在 B 时刻」
这一族，不是打断这一条能力专有的形状。

**所以 S7 的下一步不是搬代码，是先裁补件甲**：它一条解锁两条读数能力（`#13` / `#31`），
且是六条读数折叠（`slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`）的
共同入口——本段真正的门。这条门**已经开了**（`D-281`），剩下的活是拿它搬第一个能力：
`#31` 目标自生成 → `symbio-plugin-conation`（`conation` 投影 + `ConationView::merge` 进包，
`session` 侧三处读方改成按名字读 + `aggregate`），同批删 `conation.rs` / `autonomy.rs` /
`declarations.rs` 里的 `conation.expressed`。

### 五、分量读数（为什么 S7 与 S6 形状不同）

本插件约 21k 行非测试代码里，**能力实现约 1.5k 行**（`conation` 208 + `autonomy` 122 +
`slo` 147 + `tiers` 55 + `span` 110 + `fallback` 144 + `PreemptionDecider` 约 120 +
`conation_enabled` 相关约 40 + `changes` 两件 321 + 台账 240），其余约 19k 行是会话基底与
过程体。这正是「session 比 loop 更接近会话基底」这句话的量化形式：

- S6 搬的是**控制面里的策略**（一段一段各自成块，搬完就少一块）；
- S7 搬的是**长在基底上的读数与判定**（每一条都嵌在基底的读出口里，搬一条要动一片读方）。

两条段的目标一样（`loop` / `session` 只留五类），但**判据的密度不同**：S6 的读数看「删掉了
什么」，S7 的读数看「还剩几条读方在按类型读别人折出来的数」。后者今天有六条，那就是补件甲
的规模。


## 处理过程

1. ~~先出一张逐模块归属表（能力 / 五类残留），名册 5 行逐行指到表里。~~ **已落**（见上一节
   「逐模块归属表」）：55 个非测试模块逐格判完，5 行指到表里；结论是四条卡在**三条**补件上，
   其中补件甲（读数能力的跨会话合并归谁）一条解锁六条读数能力——它是本段的入口。
2. **先裁补件甲，再搬读数**。它不改 `session`，只回答「多切片合并是能力的一部分还是读方的
   一部分」；裁出来之后 `slo` / `fallback` / `span` / `autonomy` / `conation` / `changes`
   六条按同一个形状各落一包（可同包，账里写明）。补件乙（能力的开关要有一条事实形状）独立
   成一次，解锁 #32；补件丙（判定与落格之间的那段时间谁背）再独立成一次，解锁 #14。三次
   补件都**不含能力名**、各服务 ≥2 条能力。
   **补件甲已裁并已落（`D-281`）**：`ViewDecl::agg` + `ViewRead::aggregate` / `aggregable`
   + `ViewUnavailable::NotAggregable`。下一步是拿它搬 `#31`。
3. 按表分批：能独立提交的，一个能力一个提交；共用状态的先拆状态再搬。
   - 时延分层：档位是 `Decl.tier` 参数，读数折叠独立成包；
   - 打断与插话：裁决走 I-3，收件箱事实走 I-1；E2 强制留核；
   - 主动开口：与 heartbeat 归并，session 的 initiator / gate 只留强制点；
   - 目标自生成 / 内在动机：判定生产者进 conation 能力包，IntentGate 留核；
   - 四条未建（#36/48/50/51）：随 conation / heartbeat 包交最小能力面——它们是账里
     唯一「包先建、面后补」的一档，不单独建段。
4. 每批跑端到端与 `EX-003` 反向用例，`SC-006` 基线只减不增；核对 `EX-007`（`core` / `kernel` diff = 0，`session` 只含删除与绑定切换）。
5. 全部搬完后核对：session 的 `lib.rs` 导出面只剩基底接口，能力名的读写为零。
6. 与 S6 的终态合并检查，再进 S8。
