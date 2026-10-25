//! `session/stats` —— v2 事实源的**只读读数口**。
//!
//! 背景：SLO §1.2 的四列（时延 / 兜底 / 成本 / 断点）由 core 的投影算得出，
//! 但**算得出 ≠ 读得到**——没有生产出口，投影只是「能算的函数」，三类收益
//! 无法验收、回归也看不见（[plan/12 §2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
//! 本模块就是那一格的读侧出口：[plan/12 §3 批 0](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")
//! 的出口形态②（只读路由）。
//!
//! ## 两个文件的分工（判据：这一段错了谁会红）
//!
//! - 本文件：**出口形状与取数**——`TierRow` / `SessionStats` / `StatsRequest` 这些
//!   契约字段，以及「读哪些会话、快照怎么来」的取数与路由。它出错，红的是
//!   **读不到 / 字段缺**；
//! - [`stats_reading`]：**读数装配**——把投影值排成每一格（可见域、承诺列、实例级
//!   聚合、面板）。它出错，红的是**某一格的数字与 core 的口径不符**。
//!
//! 分在这里不是为了行数，是因为这两类红的处置动作不同：前者要改的是出口契约与
//! 取数范围，后者要改的是换算，而纪律要求它们都不许自己发明口径。
//!
//! ## 三条纪律
//!
//! - **只调已有投影，不另写一份统计**（[plan/12 §4](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
//!   这两个文件里没有分位数 / 占比 / 累计的算式——口径只活在 core 的投影里，出口
//!   只做「取数 + 排版」。否则同一份事实会长出两套口径，读数与投影静默漂移，
//!   而两边都自称是 SLO。**不变量同理**：清单只调 `check_all`，宽限与预算判据
//!   由 core 定（[04 §3.1 批④](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")——本处不复判）。
//! - **真·只读**：事实由存储层以**值**交回（`SessionStore::snapshot` / 部署级
//!   扫描），本模块不拼路径、不开文件、不取令牌。存储层只读打开：不创建文件、
//!   不截断撕裂尾行——截断是**写方**的恢复语义，读方顺手做会与正在落行的写方
//!   撞车（append 模式下截断后剩下的字节接在新 EOF ⇒ 那一行静默损坏）
//!   （[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）。
//! - **不命名 core 的视图类型**：core 的域模块是私有的（C-002：外部只写
//!   `symbio_core::<符号>`），插件侧因此只出现**根导出的函数名**，类型随返回值
//!   流动、在出口边界一次性序列化。这里没有 `SloLatencyView` 之类的名字，也
//!   不需要为类型加根导出。
//!   ⚠️ **P2 破了一处，理由记在这里**：聚合列要调 `SloLatencyView::merge` /
//!   `FallbackRateView::merge`——**方法是类型的行为**，提成根函数会让 `merge`
//!   变成无主函数（谁的数据谁的行为）。故这两个**视图类型**破例占根出口
//!   （与 `TurnState` / `PromptMessage` 同族：被 core 外消费的类型）。
//!   投影**函数**仍然不在这里出现名字——`slo_report()` 照旧随返回值流动。
//!
//! ## as-of 口径
//!
//! 本模块的读数一律走 `Projection::read_all`（= `now = i64::MAX` + 全量预算）：WAL 是
//! **落盘**事实源，文件里每一条都已发生，没有「未来的事件」可滤——与 `projection/slo.test.rs`
//! 的扫描口同一口径。

use std::sync::Arc;

use serde::Serialize;

use crate::symbio_core::{
    calibration, check_all, checkpoint, cost_ledger, fallback_rate, slo_report, token_ledger,
    transcript, Event, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginPayload, ReadingPanel, EVENT_MEMORY_ENCODED, SESSION_ID, SKILL_TAG,
};

use super::plugin::SessionPlugin;
use super::stats_reading::{commitments_reading, content_scope_of, fleet_reading, panel_of};

/// 一个档位的一行读数：时延列与兜底列并排（同一档位的两类收益同处一行）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TierRow {
    /// 档位名（`deep` / `fast` / `reflect` / `autonomous` / `unspecified`）。
    pub tier: String,
    /// 轮数——兜底率的**分母**（开轮的 `user.message` 计数）。
    pub turns: u64,
    /// 其中走了兜底的轮数（分子）。
    pub fallbacks: u64,
    /// 兜底率（口径在 `TierStats::rate`，本处不重算）。
    pub rate: f64,
    /// 成功轮实测时延的分位数（口径在 `TierLatency::percentile`，本处不重算）。
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    /// 时延样本数（兜底轮不计入时延样本——失败的耗时不是成功响应的耗时）。
    pub samples: usize,
}

/// 会话级读数（四列同源，D-044）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionStats {
    /// 会话 id。
    pub session_id: String,
    /// 事实源在不在。不在 = 这个会话还没跑过一轮，全零读数是**有据的零**，
    /// 不是错误（读方不该为了读而创建文件）。
    pub has_wal: bool,
    /// 按档位的时延 / 兜底两列。
    pub tiers: Vec<TierRow>,
    /// 成本台账（`cost_ledger` 视图原样序列化）。
    pub cost: serde_json::Value,
    /// 用量台账（`token_ledger` 视图原样序列化）：`{ by_principal: { <主体>: {
    /// principal, input, output, measured } }, input, output, calls }`，按主体字典序。
    ///
    /// ## 为什么与 [`Self::cost`] 是两列
    ///
    /// `cost` 是**时间**（事件的 `cost_ms`，按事件写出方累计），本列是 **token**
    /// （应答载荷的 `usage`，按内容主体累计）。观察对象与归组键都不同——合成一列
    /// 会让「花了多久」与「用了多少」互相冒充，而两者没有换算关系。
    ///
    /// ## 「没测量」与「测到零」
    ///
    /// `calls: 0` + 三个零 = 这份事实源里没有一次带实测用量的应答（provider 没报
    /// `usage`）；它与「每次都用了 0 个 token」由 `calls` 分辨，不靠补 0 编一个数
    /// ——与 `context_watermark` 缺数整条不出现是同一条纪律。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒ 全零）。
    /// 钱的折算（价格 × 用量）不在这一列：`model` 今天没有价格字段，而按名字猜价格
    /// 是给「换了个端点就全错」埋伏笔（[DE-020](../../../../docs/003-design/020-前端UI-UX.md "DE-020-前端UI-UX") §11 的前件表）。
    pub tokens: serde_json::Value,
    /// 校准（S9 步 22 的**读侧**，[plan/12 批 1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
    /// `calibration` 投影原样序列化（`{ by_skill: { <skill_id>: { skill_id, uses,
    /// fallbacks } } }`，按 `skill_id` 字典序）——`confidence = 1 − 回退率` 的口径活在
    /// core 的投影里，本处不重算。
    ///
    /// 观测面是路由落下的 `memory.recalled{skill_id, fallback}`（数据，不加新格子），
    /// 所以「本会话没编过技能」与「编了但一次没判过」都读成 `{ by_skill: {} }`——
    /// **有据的空**，不是缺列。与四列同一份切片、同一个 as-of，`may_read = false`
    /// 时同样走空切片（与就绪集列同族，不取声誉列的空对象形态）。
    pub calibration: serde_json::Value,
    /// 断点（`checkpoint` 视图原样序列化：`last_seq` / `event_count` /
    /// `kind_counts`——末两项就是「这一格里到底有多少事实」）。
    pub checkpoint: serde_json::Value,
    /// 不变量违规清单（`check_all` 十条：C1 `seq` 单调 / C2 每轮一条 final /
    /// 每轮一条开轮（C2 对偶）/ C3 断言带溯源 / C3 对偶 溯源不悬空 / C4 未收束 /
    /// C5 超预算 / C6 停机原因不在闭集 / `acyclic_deps` 依赖成环或悬空 /
    /// `rework_bounded` 返工超上界）。
    /// **空 = 十条全绿**；每条带 `event_id` 与人话。宽限口径（尾轮在途放行、
    /// 按声明档位取预算、切片首部的跨片溯源边放行、历史收束格缺 `reason` 放行）
    /// 在 core 的 `check_all`，
    /// 任务表两条**无宽限**（环与超界不是时序假象），本处只取数——
    /// [04 §3.1 批④](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")
    /// 的「不变量进 CI/读侧」：e2e 断言的就是这一列。
    pub invariants: serde_json::Value,
    /// 声誉（S6 第 15 步的**读侧**，[04 §3.1 批⑧](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") +
    /// [plan/12 批 2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：`own` = 本会话
    /// 主体自己的条目，`by_principal` = 这份事实源里每个主体的条目（`own` 在其中，
    /// 原样摊开、不另造口径）。
    ///
    /// 无读权限 ⇒ **整列为空对象**，不是零值条目：t28 的三态（没源 / 有源不给 / 有数）
    /// 靠它分辨「这里没有数」与「不给你看」——与四列用「空切片 ⇒ 全零」是同一条判定，
    /// 换个列名不换判据。
    pub reputation: serde_json::Value,
    /// 承诺 / 委托（`agent_run` 那一次立约的**读侧出口**， 段二）：
    /// `{ items: [ { id, from, to, promise, state, why, anchor_seq } ], total, kept,
    /// broken, settled, breach_rate }`，`items` 按承诺号字典序。
    ///
    /// ## 这一列把「自述」变成「查得到的事实」
    ///
    /// `agent_run` 在能力元数据里自声明「我记委托账」（`records_delegation_promise`），
    /// 声明面由 `tool-decls-audit` 守着（名单封闭），但**声明说得不等于事实落了**——
    /// 判据只能是「这一轮委托过 ⇒ 这里读得出那一条」。写侧（工具执行层 → 轮末
    /// `record_derived`）与读侧（core 的 `commitment_records` 翻译）各钉了一段，中间
    /// 只有 `commitment_events` 这一个成格口（I1）。
    ///
    /// ## 违约率的分母是**已了结**数，不是立约数
    ///
    /// 在途的约既没守也没违约，把它放进分母等于把「这一轮还没跑完」读成违约。
    /// 率是纯查询（与 `unattended_rate` / `adoption_rate` 同族）：分子分母落数，
    /// 读时再除，不落第二个数。分母为 0 ⇒ `0.0`（与那两族同一个平凡值）。
    ///
    /// 归**四列**那一族（同一份切片、同一个 as-of）：`may_read = false` ⇒ 切片已空 ⇒
    /// `{ items: [], total: 0, … }`，两种「零条」由 `has_wal` 分辨。
    pub commitments: serde_json::Value,
    /// 就绪任务集（S7 步 16 的**读侧**，[04 §3.1 批⑨](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
    /// `readyset` 投影原样序列化（`{ ready: [ { task_id, seq, depends_on } ] }`），
    /// 按 `task_id` 字典序——同一份事实源永远读出同一个集合。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒ `{ready: []}`），
    /// 不归声誉那一族（那列不给的是「有数但不给你看」的空对象）：就绪集为空在语义上
    /// 与「没有任务 / 依赖未闭合」本来就同值，两种「空」由 `has_wal` 与本列的
    /// 上下游（清单在不在）分辨，不靠列的形状。
    pub readyset: serde_json::Value,
    /// 转写（**对话面读侧**，[plan/12 批 2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
    /// `transcript` 投影原样序列化（`{ entries: [ { role, text } ] }`，按事件顺序 =
    /// append-only 的时间序；`role` 是 `"user"` / `"assistant"` / `"tool"`，
    /// 工具行另带 `tool` = 工具名）。
    ///
    /// 读的是**事实源本身**（`user.message` / `chat.assistant.final` /
    /// `chat.assistant.fallback` / `artifact.added` 四格——最后一格是
    /// plan/10 批 3 纳入的工具结果），
    /// 不是会话存储里的消息副本——同一份事实既喂模型（`Reasoner` 组 prompt 的输入）
    /// 又能从这里读回，D-044 的「实测与判据同源」。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒
    /// `{ entries: [] }`），不取声誉列的空对象形态：对话为空与「不给你看」由 `has_wal`
    /// 分辨——同一份判定，换个列名不换判据。
    ///
    /// 逐条可见域（「这条是谁说的」）属 11 §2-E
    /// 的呈现面过滤（S08 的 `vis_scope`），落地前本列只有**整列**闸——与其余各列同款，
    /// 不在这里先造一个半开的判据。
    pub transcript: serde_json::Value,
    /// 主动性（[02 §3.1 外部效标](../../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") · 主动性）：
    /// `conation` 投影原样序列化（`{ intents: [ { seq, ts, goal, task_id } ] }`，按 seq 升序）——
    /// 「自发发起且被采纳」的分子 / 分母同出一列：已升格（`task_id` 非空）/ 已表达。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ { intents: [] }`）：
    /// 欲是流，截断它等于悄悄丢掉「想要过什么」的事实（conation 投影不设配额）。
    pub conation: serde_json::Value,
    /// 时域跨度（同上 · 时域跨度）：`span` 投影序列化
    /// （`{ events, span_ms }`——本会话事实源的首末事件 `ts` 跨度，Unix 毫秒）。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ { events: 0, span_ms: 0 }`）。
    /// 实例级「任务级时长中位数」在 `fleet` 的 `axes.span` 族（单会话一个跨度没有
    /// 中位数可言，见 `SpanView::merge` 的「为什么」）。
    pub span: serde_json::Value,
    /// 自主程度（同上 · 自主程度）：`autonomy` 投影序列化
    /// （`{ closed, unattended }`——已闭环任务数与其中全程未被 `task.held` 挂起的数量）。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ { closed: 0, unattended: 0 }`）。
    /// 率是**纯查询**（`AutonomyView::unattended_rate`），不落第二个数——与 `tiers` 的
    /// `rate()` 同款：存分子分母，读时再除。
    pub autonomy: serde_json::Value,
    /// **读数面板**（P4：读数的**呈现描述**，前端零业务知识地渲染它）。
    ///
    /// ## 为什么读数也要一份描述
    ///
    /// 「这些数是什么意思、该叫什么、单位与精度怎么取」只有**产者**知道。前端自己
    /// 拼就等于把口径抄进第二处（D-044 的反面），而读数一改，抄本不报错、只静静
    /// 显示旧标签。故本字段由 `panel_of` 从**上面各列已算出的值**生成——
    /// 前端只按 [`ReadingPanel`] 排版，不解析任何会话/统计语义。
    ///
    /// ## 与各列的关系：一份事实、两种用法
    ///
    /// 各列（`tiers` / `cost` / `fleet` …）是**机器可读**的读数，判据与脚本读它们
    /// （`slo-exit-audit` 对的就是这些列）；本字段是**人读**的那一份，同一个数的
    /// 另一种说法。两者由同一个源算出，且判据 `panel_matches_the_columns_it_describes`
    /// 钉住「不脱节」——否则就是两份真相。
    pub panel: crate::symbio_core::ReadingPanel,
    /// 技能编译开关（部署配置 `skill_compile_enabled`，**不是**本会话的事实）。
    ///
    /// 与 [`Self::compiled_skills`] 并读才分辨得出两种「零技能」：`false` = 关着
    /// （本轮轨迹不会被编译成技能）；`true` + `0` = 开着但本会话还没成功收束过
    /// 可编译的轨迹。只有一列时两种零长同一个样子，而处置完全不同（一个是配置
    /// 选择，一个是运行结果）。
    ///
    /// 读侧闸不影响本列：开关是部署事实，不是这份事实源里的内容。
    pub skill_compile_enabled: bool,
    /// 本会话**已编译技能数**：事实源里 `memory.encoded{tag:"skill"}` 的条数。
    ///
    /// 技能是事实、不是特殊类型（S11），故这里只数列里已有的格子，**不新增投影**。
    /// 归四列那一族（同一份切片、同一个 as-of）：`may_read = false` ⇒ 切片已空 ⇒
    /// `0`，与「开着但没编译过」由 [`Self::skill_compile_enabled`] 分辨。
    pub compiled_skills: usize,
    /// **实例级聚合**（P2：跨会话的 SLO 真数）。
    ///
    /// 上面每一列回答「**这一个会话**怎么样」；本列回答「**整个实例**怎么样」——
    /// 口径是把本插件目录下**所有**会话的 WAL 各跑一遍四列里的两个投影
    /// （时延 / 兜底），再交给 core 的 `SloLatencyView::merge` /
    /// `FallbackRateView::merge` 合并。分子分母与分位数因此与单会话列**逐字同一套
    /// 口径**（D-044：实测与判据同源），本处不另写一套统计。
    ///
    /// ## 为什么这不是「多一份真相」
    ///
    /// 它读的是**同一批 WAL、同一批投影**，只是切片范围从「一个会话」扩到
    /// 「整个实例」。真正的第二份真相是「另写一套合并算式」——那正是这里避免的。
    ///
    /// ## 读不到的时候给什么
    ///
    /// `may_read = false` ⇒ `{ wals: 0, sessions: 0 }`（与四列同族的空，不给零值
    /// 条目：不给读的人报数）。实例里一个 WAL 都没有 ⇒ `wals: 0`，同形状。
    ///
    /// ⚠️ 成本：本列要**遍历实例下的每个会话**。会话数上去之后它是本读数口最贵的
    /// 一列（其余各列只读一个文件）。今天没有生产调用方（P4 登记），接进前端时应
    /// 连同这一条成本一起考虑——要么按需取（单独一条路），要么增量缓存。
    pub fleet: serde_json::Value,
}

/// `session/stats` 的请求体：**全部可选**——不传 = 本机默认（今天的行为）。
///
/// `principal` 是读方身份（[plan/01 §7](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 读侧）：
/// 声明了才判可见域，没声明就不判定。**这里的选择权只在读方，可见域与属主
/// 不在载荷里**——请求方自己声明「我能看到什么」等于给自己授权（governance 模块
/// 文档点名的那条捷径），本字段只回答「我是谁」。
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub(crate) struct StatsRequest {
    /// 读方身份；缺席 / `null` ⇒ 不判定。
    #[serde(default)]
    pub principal: Option<String>,
}

/// 读一个会话的事实源，出四列读数 + 校准 / 声誉 / 就绪集 / 转写四列 + 不变量清单 + 承诺列。
///
/// 纯读：不写文件、不改网格、不碰会话存储（消息 / 转写）。事实由存储层以**值**
/// 交回（[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：`snapshot` 是本会话
/// 完整史（含归档前缀），`deep_facts` 是部署级快照的**惰性取数**——只有读方有权
/// 看时才调用（`may_read` 由本函数**单点**判定，见下）。
///
/// `viewer = Some(身份)` 时先按可见域取**可读切片**再出各列（**读什么由能看什么
/// 决定**，不是先算完再裁结果）；`None` = 本机默认，与判定引入之前逐字一致。
///
/// `session_principal` 是**本会话主体**（声誉列的 `own` 取谁），与读方身份是两回事：
/// 读方是「谁在看」，会话主体是「这份承诺是谁立的」——把两者合成一个参数会让
/// 「别人看我的声誉」退化成「别人的声誉」。
///
/// `skill_compile_enabled` 是部署开关（本函数不读配置，由调用方交值）：它与
/// 「已编译技能数」并读才能分辨「关着」与「开着但没编译」两种零。
pub(crate) fn read(
    session_id: &str,
    snapshot: Option<Vec<Event>>,
    deep_facts: impl FnOnce() -> Vec<Vec<Event>>,
    viewer: Option<&str>,
    session_principal: &str,
    skill_compile_enabled: bool,
    // 调度容量（本会话配置 `task_cap`）：就绪集列按**同一个** `cap` 取，
    // 与调度段 / 调度者同源——三处各取一个数，就会读出「调度派了 1 笔，
    // 统计说候选有 5 笔」这种看不出谁错的差。
    task_cap: usize,
) -> Result<SessionStats, PluginError> {
    let has_wal = snapshot.is_some();
    // 完整史（归档段 + 未取代的那段）：`snapshot` 由存储层按 `range(0)` 口径交值。
    let snapshot = snapshot.unwrap_or_default();

    // 读侧闸（[04 §3.1 批⑥](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：**读什么由能看
    // 什么决定**——各列与不变量列都只从「你看得见的事实」算，不先算完再裁结果。
    //
    // 属主 = 本机会话的属主（`SESSION_OWNER`，部署事实；`[会话] ≈ [线程]`，S4 的
    // thread 实体落地前用会话属主）。于是：
    // 属主本人 = 全量读数（与不声明时逐字一致）；矩阵外主体 = **空切片**
    // ⇒ 四列全零、就绪集为空、不变量清单为空，与 `has_wal: true` 并排即可分辨「有源但不给你
    // 看」，而不是被读成「这里没有数」。
    // 可见域只判**一次**：四列、声誉列、就绪集列与转写列共用同一个答案——两个判定点 = 两套判据，
    // 必然漂移成「四列能看、声誉不能看」这种没有理由的半开半掩。
    //
    // **scope 取内容侧的事实**（②）：
    // 本会话记忆载荷里记的 `vis_scope`（`memory_events::encode` 的产地， ①），
    // 取**最宽**的那个——这份事实源里只要有共享内容，它就不是纯私有的。缺省
    // （没有任何记忆 / 存量事件没这个字段）= `thread_private`（C10），与接线前
    // 逐字一致。判据不再是恒传缺省值：「人的认知可供他人借用」（SC-010 判据 3）
    // 由此有生产读方——矩阵内主体读得到他人共享内容，矩阵外主体照旧空切片。
    //
    // **属主恒放行 + 矩阵谓词**那一条判据住在 core（`authz::content_visible`，
    // 与召回的逐条预过滤同一条， ⑧）；本处只加自己的那一层作用域：
    // **未声明读方 = 全量**（CLI / 前端不署名的那条路径，沿用接线前口径）。
    let content_scope = content_scope_of(&snapshot);
    let may_read = match viewer {
        None => true,
        Some(principal) => crate::symbio_core::content_visible(content_scope, principal),
    };
    let snapshot = if may_read { snapshot } else { Vec::new() };

    // 四列全部来自同一份事件切片（D-044：实测与判据同源）。
    let latency = slo_report().read_all(&snapshot).value;
    let fallback = fallback_rate().read_all(&snapshot).value;
    let cost = cost_ledger().read_all(&snapshot).value;
    // 用量列：与四列同一份切片、同一个 as-of。它读的是应答载荷的 `usage`
    // （`token_ledger` 的模块文档说明为什么不能与 `cost` 合成一列）。
    let tokens = token_ledger().read_all(&snapshot).value;
    // 校准列（[plan/12 批 1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：与四列同一份
    // 切片、同一个 as-of。观测面 = 路由收束时落的 `memory.recalled{skill_id, fallback}`。
    let cal = calibration().read_all(&snapshot).value;
    let ck = checkpoint().read_all(&snapshot).value;

    // 档位取两列的并集：只开轮没收束的档位（时延列无样本）与只有兜底样本的
    // 档位都要在表里出现——漏掉一档等于把「这一档没有数」报成「这一档不存在」。
    let mut names: Vec<String> = latency
        .by_tier
        .keys()
        .chain(fallback.by_tier.keys())
        .cloned()
        .collect();
    names.sort_unstable();
    names.dedup();

    let tiers = names
        .into_iter()
        .map(|t| {
            let l = latency.of(&t);
            let f = fallback.of(&t);
            TierRow {
                tier: l.tier.clone(),
                turns: f.turns,
                fallbacks: f.fallbacks,
                rate: f.rate(),
                p50: l.p50(),
                p95: l.p95(),
                p99: l.p99(),
                samples: l.count(),
            }
        })
        .collect();

    let cost = serde_json::to_value(&cost)
        .map_err(|e| PluginError::InternalError(format!("成本台账序列化失败：{e}")))?;
    let tokens = serde_json::to_value(&tokens)
        .map_err(|e| PluginError::InternalError(format!("用量台账序列化失败：{e}")))?;
    let calibration = serde_json::to_value(&cal)
        .map_err(|e| PluginError::InternalError(format!("校准序列化失败：{e}")))?;
    let checkpoint = serde_json::to_value(&ck)
        .map_err(|e| PluginError::InternalError(format!("断点序列化失败：{e}")))?;
    let invariants = serde_json::to_value(check_all(&snapshot))
        .map_err(|e| PluginError::InternalError(format!("不变量清单序列化失败：{e}")))?;

    // 声誉列：与四列同一份切片、同一个 as-of（`i64::MAX` = 落盘即已发生）。
    // 没有权限时不给零值条目——给零值等于替一个你本来看不到的主体报数。
    let rep = crate::symbio_core::reputation().read_all(&snapshot).value;
    let reputation = if may_read {
        let own = rep.of(session_principal);
        serde_json::json!({ "own": own, "by_principal": rep.by_principal })
    } else {
        serde_json::json!({})
    };

    // 承诺列（段二）：与声誉列读**同一批格子**，但问的是另一件事——声誉问
    // 「这个主体几分」，本列问「这一约是什么、违约为什么」。切片已按 `may_read` 取过。
    let commitments = commitments_reading(&snapshot);

    // 就绪集列（批⑨）：与四列同一份切片、同一个 as-of。`may_read = false` 时切片
    // 已被换成空的 ⇒ 本列是 `{ready: []}`（四列的形态），不是声誉那列的空对象。
    let readyset = serde_json::to_value(
        crate::symbio_core::readyset(task_cap)
            .read_all(&snapshot)
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("就绪集序列化失败：{e}")))?;

    // 转写列（对话面读侧，[plan/12 批 2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
    // doc-link-allow D-004: 「四格 → 三角色」是事件形状到角色的静态映射，非版本改造
    // 与四列同一份切片、同一个 as-of。口径（四格 → 三角色，含
    // plan/10 批 3 的工具结果）只活在 core
    // 的 `transcript` 投影里，本处只取数 + 排版；`may_read = false` 时切片已空 ⇒
    // `{ entries: [] }`（四列形态，与就绪集 / 校准同族）。
    let transcript = serde_json::to_value(transcript().read_all(&snapshot).value)
        .map_err(|e| PluginError::InternalError(format!("转写序列化失败：{e}")))?;

    // 主动性列（02 §3.1 外部效标）：与四列同一份切片、同一个 as-of。观测面是
    // `conation.expressed`（欲）与 `task.opened`（升格，溯源认）——「自发发起」的
    // 分子分母都是事实源里已有的事件，不加新格子。`may_read = false` 时切片已空
    // ⇒ `{ intents: [], adoption_rate: 0.0 }`（四列形态）。
    //
    // 率与流同列给出：流是**事实**（不截断，见列定义），率是**读数**（纯查询，
    // 与 `tiers` 的 `rate()`、跨会话 `ConationSummary::adoption_rate` 同一套算式）。
    let conation_view = crate::symbio_core::conation().read_all(&snapshot).value;
    let conation = serde_json::json!({
        "intents": conation_view.intents,
        "adoption_rate": conation_view.adoption_rate(),
    });

    // 时域跨度列（同上）：`span` 投影只读事件信封的 `ts`（入库时间，与 `cost_ms`
    // 同源的实测面），as-of 之后的事件一概不见。
    let span = serde_json::to_value(crate::symbio_core::span().read_all(&snapshot).value)
        .map_err(|e| PluginError::InternalError(format!("时域跨度序列化失败：{e}")))?;

    // 自主程度列（同上）：`autonomy` 投影认 `task.held`（插话抢占 = 外部介入在事实源
    // 里唯一的痕迹）与 `task.asserted`（闭环）。率是纯查询，此处只落分子分母。
    let autonomy = serde_json::to_value(crate::symbio_core::autonomy().read_all(&snapshot).value)
        .map_err(|e| PluginError::InternalError(format!("自主程度序列化失败：{e}")))?;

    // 实例级聚合列（P2）。`may_read = false` 时**不扫**：扫遍实例再报「不给你看」
    // 是白花代价，而结论与空形状相同。空形状与满读数**同键**（axes 两族给有据的
    // 零 / 空，不给「缺列」——读方不必为每个键分诊它在不在）。
    // 技能编译列：技能就是 `memory.encoded{tag:"skill"}` 这条事实（S11），故只数
    // 切片里的格子，不另造投影。切片已按 `may_read` 取过 ⇒ 无权读的人得 0，
    // 与「开着但没编译过」由部署开关 `skill_compile_enabled` 分辨。
    let compiled_skills = snapshot
        .iter()
        .filter(|e| {
            e.kind == EVENT_MEMORY_ENCODED
                && e.payload.get("tag").and_then(|v| v.as_str()) == Some(SKILL_TAG)
        })
        .count();

    let fleet = if may_read {
        fleet_reading(&deep_facts())
    } else {
        serde_json::json!({
            "wals": 0, "sessions": 0, "turns": 0, "fallbacks": 0,
            "by_tier": { "latency": [], "fallback": [] },
            "axes": {
                "conation": { "expressed": 0, "escalated": 0, "adoption_rate": 0.0 },
                "span": { "sessions": 0, "median_ms": null },
                "autonomy": { "closed": 0, "unattended": 0, "unattended_rate": 0.0 },
            },
        })
    };

    let reading = SessionStats {
        session_id: session_id.to_string(),
        has_wal,
        tiers,
        cost,
        tokens,
        calibration,
        checkpoint,
        invariants,
        reputation,
        commitments,
        readyset,
        transcript,
        conation,
        span,
        autonomy,
        fleet,
        skill_compile_enabled,
        compiled_skills,
        panel: ReadingPanel::default(),
    };
    Ok(SessionStats {
        panel: panel_of(&reading),
        ..reading
    })
}

impl SessionPlugin {
    /// 路由 `session/stats` 的处理：向存储层取**值**（本会话快照 + 部署级提示的
    /// 惰性取数），交给 [`read`]。
    ///
    /// 与 `chat/send` / `chat/abort` 并列在路由表里，但性质不同——那两条是
    /// **编排 / 控制**，本条是**读数**：既不是数据 CRUD（CRUD 全走 VDFS，见
    /// `plugin.rs::route` 的分支注释），也不产生任何副作用。
    pub(crate) async fn handle_stats(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/stats 需要 metadata.session_id".into(),
            ));
        }
        let store = self.get_store().await?;
        // 本会话快照立即取（读侧闸要它算可见域）；部署级扫描推迟到闸判之后——
        // `read` 只在 `may_read` 时调用 `deep_facts`（不给看就不扫）。
        let snapshot = store.snapshot(&sid);
        // 载荷 = **读方身份声明**（可选）：不带 `principal` = 本机默认，与读侧闸
        // 引入之前逐字一致；带了却读不出身份（类型不对）⇒ 声明了却判不了 ⇒ 拒绝
        // 读数，不悄悄降级成「未声明」（fail-closed）。载荷整体缺席同理拒绝。
        let viewer: Option<String> = match ctx.payload::<serde_json::Value>()? {
            serde_json::Value::Null => None,
            v => {
                serde_json::from_value::<StatsRequest>(v)
                    .map_err(|e| {
                        PluginError::ValidationError(format!("session/stats 载荷不合法：{e}"))
                    })?
                    .principal
            }
        };
        // 会话主体 = **目标会话自己的**（`metadata.agent_id` → `principal_of`，
        // 与 `delegating_principal` / 心跳回退链同一份词汇），不是请求方的：
        // 本端点支持跨会话查数（委派监督、e2e 都传别人的 `session_id`），而
        // [`read`] 文档写明 `own` 列的语义是「这份事实源里的承诺是谁立的」——
        // 接到请求方就会把「别人看我的声誉」读成「别人的声誉」（t75 抓到的错标）。
        // 元数据没记 `agent_id`（未选智能体的会话 / 旧会话）⇒ 回落请求方取值点：
        // 自查场景两者恒同，行为与接线前逐字一致。
        let session_principal = match self.get_or_create_session(&sid).await.ok().and_then(|s| {
            s.metadata
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        }) {
            Some(agent) => crate::symbio_core::principal_of(Some(&agent)),
            None => crate::symbio_core::request_principal(ctx.as_ref()),
        };
        // 技能编译开关是部署配置（不在这份事实源里）：与已编译数并读，才分得清
        // 「关着」与「开着但没编译过」两种零。
        let (skill_compile_enabled, task_cap) = {
            let cfg = self.config.read().await;
            (cfg.skill_compile_enabled, cfg.task_cap)
        };
        let stats = read(
            &sid,
            snapshot,
            || store.deep_snapshots(),
            viewer.as_deref(),
            &session_principal,
            skill_compile_enabled,
            task_cap,
        )?;
        Ok(PluginPayload::new(&stats))
    }
}

#[cfg(test)]
#[path = "stats.test.rs"]
mod tests;
