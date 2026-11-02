//! `session/stats` —— v2 事实源的**只读读数口**。
//!
//! 背景：SLO §1.2 的四列（时延 / 兜底 / 成本 / 断点）由本插件的投影与 core 的
//! 协议投影算得出，
//! 但**算得出 ≠ 读得到**——没有生产出口，投影只是「能算的函数」，三类收益
//! 无法验收、回归也看不见。本模块就是那一格的读侧出口：出口形态②（只读路由）。
//! 读数今天还多一族**能力视图**：键来自各能力包公开的 `shared_views` 清单、值由
//! owner 自己的折叠算出，本插件只按名字读、不写任何视图名（DE-025 D-221）——
//! 一项新能力公开一条视图，这一列就多一格，本文件不动一行。
//!
//! ## 两个文件的分工（判据：这一段错了谁会红）
//!
//! - 本文件：**出口形状与取数**——`TierRow` / `SessionStats` / `ViewerRequest` 这些
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
//! - **只调已有投影，不另写一份统计**：
//!   这两个文件里没有分位数 / 占比 / 累计的算式——口径只活在投影里（本插件 `projections/` 与 core 的协议投影），出口
//!   只做「取数 + 排版」。否则同一份事实会长出两套口径，读数与投影静默漂移，
//!   而两边都自称是 SLO。**不变量同理**：清单只调 `check_all`，宽限与预算判据
//!   由 core 定（本处不复判）。
//! - **真·只读**：事实由存储层以**值**交回（`SessionStore::snapshot` / 部署级
//!   扫描），本模块不拼路径、不开文件、不取令牌。存储层只读打开：不创建文件、
//!   不截断撕裂尾行——截断是**写方**的恢复语义，读方顺手做会与正在落行的写方
//!   撞车（append 模式下截断后剩下的字节接在新 EOF ⇒ 那一行静默损坏）
//!   （[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）。
//! - **不重写视图类型**：口径住在 owner 那里——时延 / 兜底 / 跨度三列归能力包 `slo`、
//!   成本列归能力包 `cost`（各自 `ViewDecl::eval`，`slo` 另有 `agg`），本插件按名字读
//!   （`D-221`）并把值原样装进出口（`D-281`）；自主程度等仍是本插件 `projections/` 里的
//!   纯折叠。core 只留协议自身的投影（转写 / 轮状态 / 任务词表等），不再为会话读数占根
//!   出口（DE-025 D-221）。
//!
//! ## as-of 口径
//!
//! 本模块的读数一律走 `Projection::read_all`（= `now = i64::MAX` + 全量预算）：WAL 是
//! **落盘**事实源，文件里每一条都已发生，没有「未来的事件」可滤——按名字读的视图
//! （`read_shared_view_all`）走的是同一个口，两族因此同一个 as-of。

use crate::declarations::{
    CHAT_VOCAB, CONATION_AUTONOMY_VIEW, CONATION_INTENTS_VIEW, COST_LEDGER_VIEW, MESSAGE_VOCAB,
    REPUTATION_LEDGER_VIEW, SKILL_CALIBRATION_VIEW, SLO_FALLBACK_VIEW, SLO_LATENCY_VIEW,
    SLO_SPAN_VIEW, TASK_READYSET_VIEW,
};
use std::sync::Arc;

use serde::Serialize;

use symbio_kernel::event::Event;
use symbio_kernel::plugin::{
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginPayload,
};
use symbio_kernel::present::ReadingPanel;
use symbio_sdk::actors::SKILL_TAG;
use symbio_sdk::invariants::check_all;
use symbio_sdk::keys::SESSION_ID;
use symbio_sdk::projection::transcript::transcript;

use crate::projections::checkpoint::checkpoint;

use super::spend_ledger::spend_ledger;
use super::token_ledger::token_ledger;

use super::plugin::SessionPlugin;
use super::stats_reading::{
    capability_views, commitments_reading, content_scope_of, fleet_reading, panel_of,
    usage_section, ViewerRequest,
};

/// 一个档位的一行读数：时延列与兜底列并排（同一档位的两类收益同处一行）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TierRow {
    /// 档位名（`deep` / `fast` / `reflect` / `autonomous` / `unspecified`）。
    pub tier: String,
    /// 轮数——兜底率的**分母**（开轮的 `user.message` 计数）。
    pub turns: u64,
    /// 其中走了兜底的轮数（分子）。
    pub fallbacks: u64,
    /// 兜底率（口径在能力包 `slo` 的 `rate`，本处只读它交的值）。
    pub rate: f64,
    /// 成功轮实测时延的分位数（口径在能力包 `slo` 的 `percentile`，本处只读它交的值）。
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    /// 时延样本数（兜底轮不计入时延样本——失败的耗时不是成功响应的耗时）。
    ///
    /// 取自 owner 交的 `sample_ms` 的**长度**：owner 交的是样本本身（跨会话合并要从
    /// 样本重算分位数），出口只要个数——取长度是排版，不是重算。
    pub samples: usize,
}

/// 按名字读一条能力包交的视图；读不到 ⇒ **调用方给的形状**，并出声一次。
///
/// 空形状不是「这一档没有数」，是「这一列今天读不出来」——两者的区别由
/// `capability_views` 那一列的可诊断缺席承担，本处只保证出口形状可读（读方不必分诊）。
/// 空形状由调用方交而不是本函数写死：`{ by_tier: [] }` 是逐档视图的形状，跨度视图是
/// `{ events, span_ms }`——读方替 owner 猜形状，就是第二份口径。
fn read_view_or(
    name: &str,
    snapshot: &[Event],
    empty: impl FnOnce() -> serde_json::Value,
) -> serde_json::Value {
    match symbio_plugin_api::read_shared_view_all(name, snapshot) {
        Ok(value) => value,
        Err(why) => {
            symbio_plugin_api::plugin_warn!(
                symbio_plugin_api::PLUGIN_ID_SESSION,
                "[stats] `{name}` 读不到（{why}）：按空读数交"
            );
            empty()
        }
    }
}

/// 逐档视图的**空形状**：`{ by_tier: [] }`（与 owner 交的形状同键，读方不必分诊）。
///
/// 单会话列与实例级聚合列共用它——两处写两份，迟早一处改了另一处没改，
/// 于是「读不到」在抽屉里与在会话头上长得不一样。
pub(super) fn empty_by_tier() -> serde_json::Value {
    serde_json::json!({ "by_tier": [] })
}

/// 成本台账的**空形状**：`{ by_principal: {}, total_ms: 0 }`（与 owner 交的形状同键）。
///
/// 与 [`empty_by_tier`] 同一条纪律：读方不替 owner 猜形状——猜错一次，出口就长出
/// 一份 nobody 认识的读数。
fn empty_ledger() -> serde_json::Value {
    serde_json::json!({ "by_principal": {}, "total_ms": 0 })
}

/// 校准账的**空形状**：`{ by_skill: {} }`（与 owner 交的形状同键）。
///
/// 「本会话没判过技能」与「这条视图今天读不出来」在出口上都读成这一份空表；两者的区别
/// 由 `capability_views` 那一列的可诊断缺席承担（与 [`empty_ledger`] 同一条分工）。
fn empty_calibration() -> serde_json::Value {
    serde_json::json!({ "by_skill": {} })
}

/// 声誉台账的**空形状**：`{ by_principal: {} }`（与 owner 交的形状同键）。
///
/// 与 [`empty_ledger`] 同一条纪律：读方不替 owner 猜形状。
fn empty_reputation() -> serde_json::Value {
    serde_json::json!({ "by_principal": {} })
}

/// 就绪集列的**排版**：owner 交的是「谁就绪、按什么序」（`task.readyset`，`D-289`），
/// 本函数只套上本轮的容量——`cap` 是读者的预算，不是折叠的一格。
///
/// `cap = 0` 按 1 处理，与 owner 那侧的调度器同一条口径（容量为零的调度器不是「更保守」，
/// 是「永不动」）。视图读不出来时 [`empty_readyset`] 先给一份空表，这里因此仍产出
/// `{ ready: [], cap }` 这一份**文档写的形状**，与迁移前的列逐字相同。
fn readyset_column(value: serde_json::Value, cap: usize) -> serde_json::Value {
    let cap = cap.max(1);
    let mut ready = value
        .get("ready")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    ready.truncate(cap);
    serde_json::json!({ "ready": ready, "cap": cap })
}

/// 就绪集的**空形状**：`{ ready: [] }`（owner 交的那一份去掉容量之后的键）。
fn empty_readyset() -> serde_json::Value {
    serde_json::json!({ "ready": [] })
}

/// 声誉列的**排版**：把台账里本会话主体那一行单独提出来，另附整本账。
///
/// 本函数不加总、不打分——「守约 − 违约」的算式住在声誉能力包那一侧（`D-221`），
/// 这里只做两件事：按名字取一行，以及没立过约时补一行零。那一行零不是重算，是让
/// 出口恒有一格 `own` 可读（t75 判的正是它的 `principal` 必须是**目标会话**的主体，
/// 不是请求方的——跨会话查数时两者不同）。
fn reputation_reading(ledger: serde_json::Value, principal: &str) -> serde_json::Value {
    let rows = ledger
        .get("by_principal")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let own = rows.get(principal).cloned().unwrap_or_else(|| {
        serde_json::json!({
            "principal": principal,
            "offered": 0,
            "kept": 0,
            "broken": 0,
            "score": 0,
        })
    });
    serde_json::json!({ "own": own, "by_principal": rows })
}

/// 一条视图交的档位名。形状归 owner（`D-281`），本处只做**查表**。
fn tier_names(view: &serde_json::Value) -> Vec<String> {
    view.get("by_tier")
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|r| r.get("tier").and_then(serde_json::Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 某档位在一条视图里的那一行；查不到 ⇒ `Null`（`get` 对 `Null` 一律 `None`，读方按缺省取）。
fn tier_of<'a>(view: &'a serde_json::Value, tier: &str) -> &'a serde_json::Value {
    static NO_ROW: serde_json::Value = serde_json::Value::Null;
    view.get("by_tier")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|r| r.get("tier").and_then(serde_json::Value::as_str) == Some(tier))
        })
        .unwrap_or(&NO_ROW)
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
    /// 成本台账（能力包交的 `cost.ledger` 视图读数原样交出）：`{ by_principal: { <主体>: {
    /// principal, spent_ms, entries } }, total_ms }`，按主体字典序。
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
    /// 钱的折算（价格 × 用量）不在这一列，它是下一列 [`Self::spend`]——本列只报
    /// 「用了多少」，「花了多少钱」由价格配置决定（不按模型名猜）。
    pub tokens: serde_json::Value,
    /// 金额台账（`spend_ledger` 视图原样序列化）：
    /// `{ by_currency: { <币种>: { currency, amount_nano, charges,
    /// by_principal: { <主体>: { principal, amount_nano, charges } } } } }`，
    /// 币种与主体均字典序。
    ///
    /// ## 与 [`Self::tokens`] / [`Self::cost`] 是三列
    ///
    /// `tokens` 是**用量**（provider 实测），`cost` 是**时间**（`cost_ms`），
    /// 本列是**钱**（价格 × 用量）。钱由价格折算，而价格是 Provider 配置
    /// （`ModelProviderConfig.pricing`，每个端点自己填）——所以本列的存在性取决于
    /// 配置，不是取决于 token 有没有测到：没配价格时它整列是
    /// `{ by_currency: {} }`（有据的空），token 列照旧有数。
    ///
    /// ## 「没定价」与「明确免费」
    ///
    /// `by_currency` 为空 = 这份事实源里没有一条带金额的往返（没配价格 / 没测到用量）；
    /// 某个币种的 `charges > 0` 且 `amount_nano == 0` = 那个 Provider 的单价被显式
    /// 配成 0。两者不合并，也不靠补零冒充（与 `tokens` 的 `calls` 同一条判据）。
    ///
    /// 不同币种**不相加**：金额只在同一币种内可加，汇率不是事实源里的东西。
    pub spend: serde_json::Value,
    /// 校准（的**读侧**）：能力包 `crates/symbio-plugin-skill` 交的 `skill.calibration`
    /// 视图读数原样交出（`{ by_skill: { <skill_id>: { skill_id, uses,
    /// fallbacks } } }`，按 `skill_id` 字典序）——`confidence = 1 − 回退率` 的口径活在
    /// owner 的折叠里，本处不重算。
    ///
    /// 观测面是技能召回行落下的 `memory.calibrated{skill_id, fallback}`，所以「本会话
    /// 没编过技能」与「编了但一次没判过」都读成 `{ by_skill: {} }`——**有据的空**，不是
    /// 缺列。与四列同一份切片、同一个 as-of，`may_read = false` 时同样走空切片
    /// （与就绪集列同族，不取声誉列的空对象形态）。
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
    /// 「不变量进 CI/读侧」：e2e 断言的就是这一列。
    pub invariants: serde_json::Value,
    /// 声誉（的**读侧**）：`own` = 本会话
    /// 主体自己的条目，`by_principal` = 这份事实源里每个主体的条目（`own` 在其中，
    /// 原样摊开、不另造口径）。
    ///
    /// 无读权限 ⇒ **整列为空对象**，不是零值条目：t28 的三态（没源 / 有源不给 / 有数）
    /// 靠它分辨「这里没有数」与「不给你看」——与四列用「空切片 ⇒ 全零」是同一条判定，
    /// 换个列名不换判据。
    pub reputation: serde_json::Value,
    /// 承诺 / 委托（`agent_run` 那一次立约的**读侧出口**）：
    /// `{ items: [ { id, from, to, promise, state, why, anchor_seq } ], total, kept,
    /// broken, settled, breach_rate }`，`items` 按承诺号字典序。
    ///
    /// ## 这一列把「自述」变成「查得到的事实」
    ///
    /// `agent_run` 在能力元数据里自声明「我记委托账」（`delegation_promise` 行为名），
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
    /// 就绪任务集（的**读侧**）：
    /// 共享视图 `task.readyset` 原样序列化（`{ ready: [ { task_id, seq, depends_on } ] }`），
    /// 按 `task_id` 字典序——同一份事实源永远读出同一个集合。
    ///
    /// ## 折叠不在本插件，本插件只交本轮的容量
    ///
    /// 「谁就绪」由任务能力包算（owner：`crates/symbio-plugin-task`，`D-289`），本插件按
    /// 名字读——与 `cost.ledger` / `reputation.ledger` 同一条通路。容量是本会话配置里的
    /// `task_cap`：它是**读者的预算**，视图交的是无界的候选集，`cap` 在这里套上
    /// （`cap.max(1)` 后截断），所以这一列的读数与折叠搬出 core 之前逐字节相同。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒ `{ready: []}`），
    /// 不归声誉那一族（那列不给的是「有数但不给你看」的空对象）：就绪集为空在语义上
    /// 与「没有任务 / 依赖未闭合」本来就同值，两种「空」由 `has_wal` 与本列的
    /// 上下游（清单在不在）分辨，不靠列的形状。
    pub readyset: serde_json::Value,
    /// 转写（**对话面读侧**）：
    /// `transcript` 投影原样序列化（`{ entries: [ { role, text } ] }`，按事件顺序 =
    /// append-only 的时间序；`role` 是 `"user"` / `"assistant"` / `"tool"`，
    /// 工具行另带 `tool` = 工具名）。
    ///
    /// 读的是**事实源本身**（`user.message` / `chat.assistant.final` /
    /// `chat.assistant.fallback` / `artifact.added` 四格——最后一格是工具结果），
    /// 不是会话存储里的消息副本——同一份事实既喂模型（`Reasoner` 组 prompt 的输入）
    /// 又能从这里读回，D-044 的「实测与判据同源」。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒
    /// `{ entries: [] }`），不取声誉列的空对象形态：对话为空与「不给你看」由 `has_wal`
    /// 分辨——同一份判定，换个列名不换判据。
    ///
    /// 逐条可见域（「这条是谁说的」）不在这里过滤：本列只有**整列**闸（`may_read`），
    /// 与其余各列同款。
    pub transcript: serde_json::Value,
    /// 主动性（[DE-008 §3.1 外部效标](../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") · 主动性）：
    /// `conation` 投影原样序列化（`{ intents: [ { seq, ts, goal, task_id } ] }`，按 seq 升序）——
    /// 「自发发起且被采纳」的分子 / 分母同出一列：已升格（`task_id` 非空）/ 已表达。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ { intents: [] }`）：
    /// 欲是流，截断它等于悄悄丢掉「想要过什么」的事实（conation 投影不设配额）。
    pub conation: serde_json::Value,
    /// 时域跨度（同上 · 时域跨度）：能力包 `slo` 的 `slo.span` 视图原样交出
    /// （`{ events, span_ms }`——本会话事实源的首末事件 `ts` 跨度，Unix 毫秒）。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false` ⇒ `{ events: 0, span_ms: 0 }`）。
    /// 实例级「任务级时长中位数」在 `fleet` 的 `axes.span` 族（单会话一个跨度没有
    /// 中位数可言，见 `crates/symbio-plugin-slo/src/decl.rs::span_merged` 的「为什么」）。
    pub span: serde_json::Value,
    /// 自主程度（DE-008 §3.1 外部效标）：**读数由能力包交**（`conation.autonomy`，
    /// `D-221` 按名字读、`D-281` 折叠两半），本插件原样装一格，不算率、不认识形状。
    ///
    /// 归**四列**那一族（同切片、同 as-of）。读不到 ⇒ `null`（不是三个零：零是
    /// 「没有闭环任务」，`null` 是「这一格今天读不出来」）。
    pub autonomy: serde_json::Value,
    /// **能力视图**（DE-025 D-221）：别的插件公开的视图，按名字读出来的原样值。
    ///
    /// ## 为什么这一列没有写死的键名
    ///
    /// 它是**通用读口**，不是某一项能力的专用列：键 = owner 在 `PLUGIN.yml` 的
    /// `shared_views` 里公开的名字（`symbio_plugin_api::shared_views()`），值 =
    /// `{ owner, status: "ok", ver, value }`，或
    /// `{ owner, status: "unavailable", reason, detail }`。加一条「最近几条便签」
    /// 只需要能力包自己声明视图并公开名字——本插件不改一行，下一次读数就多一格；
    /// 读的是 owner 的折叠（`ViewDecl::eval`），本插件不抄它的口径。
    ///
    /// ## 缺席态各有名字
    ///
    /// 未冻结 / 未公开 / 已公开但没实现分别报 `not_sealed` / `not_shared` / `unknown`
    /// （词表在 `ViewUnavailable::reason`），不折叠成空值——「没算出来」与「算出来是空」
    /// 必须分得开（与 `tokens.calls` 同一条纪律）。读侧没有「超预算」这一态：切片给多长
    /// 就折多长，`cost` 只约束环内窗口求值（`D-284`）。
    ///
    /// 归**四列**那一族：与其余各列同一份切片、同一个 as-of；`may_read = false`
    /// ⇒ 空切片 ⇒ 各视图读到「有据的空」，由 `has_wal` 与缺席态分辨。
    pub capability_views: serde_json::Value,
    /// **读数面板**（读数的**呈现描述**，前端零业务知识地渲染它）。
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
    pub panel: symbio_kernel::present::ReadingPanel,
    /// 用量那**一格**的读数（`ReadingSection`）：给「地方只有一条、但必须在眼前」的
    /// 位置用（会话头），与 [`Self::panel`] 里那一段**同一次装配**
    /// （`stats_reading::usage_section` 的唯一实现，两处调同一个函数，不是两份口径）。
    ///
    /// ## 为什么不与 `panel` 合并成一个字段
    ///
    /// 两个消费方要的量不同：抽屉要**全部**分区（时延 / 断点 / 不变量……），会话头
    /// 只要**一行**。让前端从 `panel.sections` 里按标题文本挑「用量」那一节，等于把
    /// 中文标签变成跨栈的定位键——改一个字，前端就静默少一格（方言里
    /// [`ReadingSection`] 没有身份字段，只有标题）。故产者直接把要的那一格交出来，
    /// 前端一个字符串都不用认。
    ///
    /// `None` = 一次实测都没有（`calls == 0`）⇒ 前端整格不渲染，不画一条空读数
    /// （与 `context_watermark` 缺数整条不出现同一条纪律）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<symbio_kernel::present::ReadingSection>,
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
    /// 技能是事实、不是特殊类型，故这里只数列里已有的格子，**不新增投影**。
    /// 归四列那一族（同一份切片、同一个 as-of）：`may_read = false` ⇒ 切片已空 ⇒
    /// `0`，与「开着但没编译过」由 [`Self::skill_compile_enabled`] 分辨。
    pub compiled_skills: usize,
    /// **实例级聚合**（跨会话的 SLO 真数）。
    ///
    /// 上面每一列回答「**这一个会话**怎么样」；本列回答「**整个实例**怎么样」——
    /// 口径是把本插件目录下**所有**会话的 WAL 各读一遍，再交给能力包 `slo` 的
    /// `ViewDecl::agg` 合并（`D-281`）。分子分母与分位数因此与单会话列**逐字同一套
    /// 口径**（D-044：实测与判据同源），本处不另写一套统计，也不认识交回来的形状。
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
    /// 一列（其余各列只读一个文件）。今天没有生产调用方，接进前端时应
    /// 连同这一条成本一起考虑——要么按需取（单独一条路），要么增量缓存。
    pub fleet: serde_json::Value,
}

/// 读一个会话的事实源，出四列读数 + 校准 / 声誉 / 就绪集 / 转写四列 + 不变量清单 + 承诺列，
/// 再加一列**能力视图**：按各能力包公开的清单逐条经 `symbio_plugin_api::views` 读，
/// 本插件不写任何视图名（DE-025 D-221）。
///
/// 纯读：不写文件、不改网格、不碰会话存储（消息 / 转写）。事实由存储层以**值**
/// 交回（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：`snapshot` 是本会话
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

    // 读侧闸：**读什么由能看什么决定**——各列与不变量列都只从「你看得见的事实」算，不先算完再裁结果。
    //
    // 属主 = 本机会话的属主（`SESSION_OWNER`，部署事实；`[会话] ≈ [线程]`，
    // thread 实体落地前用会话属主）。于是：
    // 属主本人 = 全量读数（与不声明时逐字一致）；矩阵外主体 = **空切片**
    // ⇒ 四列全零、就绪集为空、不变量清单为空，与 `has_wal: true` 并排即可分辨「有源但不给你
    // 看」，而不是被读成「这里没有数」。
    // 可见域只判**一次**：四列、声誉列、就绪集列与转写列共用同一个答案——两个判定点 = 两套判据，
    // 必然漂移成「四列能看、声誉不能看」这种没有理由的半开半掩。
    //
    // **scope 取内容侧的事实**（②）：
    // 本会话记忆载荷里记的 `vis_scope`（情景记忆能力包收束贡献的产地，①），
    // 取**最宽**的那个——这份事实源里只要有共享内容，它就不是纯私有的。缺省
    // （没有任何记忆 / 存量事件没这个字段）= `thread_private`（C10），与接线前
    // 逐字一致。判据不再是恒传缺省值：「人的认知可供他人借用」（SC-010 判据 3）
    // 由此有生产读方——矩阵内主体读得到他人共享内容，矩阵外主体照旧空切片。
    //
    // **属主恒放行 + 矩阵谓词**那一条判据住在 core（`authz::content_visible`，
    // 与召回的逐条预过滤同一条，⑧）；本处只加自己的那一层作用域：
    // **未声明读方 = 全量**（CLI / 前端不署名的那条路径，沿用接线前口径）。
    let content_scope = content_scope_of(&snapshot);
    let may_read = match viewer {
        None => true,
        Some(principal) => symbio_sdk::authz::content_visible(content_scope, principal),
    };
    let snapshot = if may_read { snapshot } else { Vec::new() };

    // 四列全部来自同一份事件切片（D-044：实测与判据同源）。
    //
    // 时延、兜底与成本三列由能力包交（`D-221` 按名字读、`D-281` 折叠两半）：本插件不认识
    // 这些列的形状，也不在这里算分位数、率或累计——那是 owner 的算式。读不到 ⇒ 空表（可诊断的
    // 缺席，不是「这一档没有数」）。
    let latency = read_view_or(SLO_LATENCY_VIEW, &snapshot, empty_by_tier);
    let fallback = read_view_or(SLO_FALLBACK_VIEW, &snapshot, empty_by_tier);
    // 成本列与 `loop` 的熔断闸门读的是**同一个视图名**：两处各折一遍必然漂移成
    // 「闸门说还有余量、出口说已耗尽」（D-044）。
    let cost = read_view_or(COST_LEDGER_VIEW, &snapshot, empty_ledger);
    // 用量列：与四列同一份切片、同一个 as-of。它读的是应答载荷的 `usage`
    // （`token_ledger` 的模块文档说明为什么不能与 `cost` 合成一列）。
    let tokens = token_ledger().read_all(&snapshot).value;
    // 金额列：同一个切片、同一个 as-of，读应答载荷的 `spend`（价格 × 用量，
    // 写方在 adapter 边界折好）。它不重算价格——配置改了也只影响之后的事实。
    let spend = spend_ledger().read_all(&snapshot).value;
    // 校准列：折叠与阈值口径归技能能力包（名册 #19/#20，`D-221` 按名字读），本插件
    // 不重算回退率。读不到 ⇒ 有据的空形状并出声，与成本列同一条纪律。
    let calibration = read_view_or(SKILL_CALIBRATION_VIEW, &snapshot, empty_calibration);
    let ck = checkpoint().read_all(&snapshot).value;

    // 档位取两列的并集：只开轮没收束的档位（时延列无样本）与只有兜底样本的
    // 档位都要在表里出现——漏掉一档等于把「这一档没有数」报成「这一档不存在」。
    let mut names: Vec<String> = tier_names(&latency)
        .into_iter()
        .chain(tier_names(&fallback))
        .collect();
    names.sort_unstable();
    names.dedup();

    let tiers = names
        .into_iter()
        .map(|t| {
            let l = tier_of(&latency, &t);
            let f = tier_of(&fallback, &t);
            TierRow {
                tier: t,
                turns: f
                    .get("turns")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                fallbacks: f
                    .get("fallbacks")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                // 率是 owner 的算式，本处只**读**它交的值（`D-281`：形状归 owner）。
                rate: f
                    .get("rate")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0),
                p50: l
                    .get("p50")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                p95: l
                    .get("p95")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                p99: l
                    .get("p99")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                samples: l
                    .get("sample_ms")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len),
            }
        })
        .collect();

    let tokens = serde_json::to_value(&tokens)
        .map_err(|e| PluginError::InternalError(format!("用量台账序列化失败：{e}")))?;
    let spend = serde_json::to_value(&spend)
        .map_err(|e| PluginError::InternalError(format!("金额台账序列化失败：{e}")))?;
    let checkpoint = serde_json::to_value(&ck)
        .map_err(|e| PluginError::InternalError(format!("断点序列化失败：{e}")))?;
    // 不变量清单的词汇面（DE-024 D-203）：判据问「这个名字扮演什么角色」，答案取自
    // owner 交来的装配表；本二进制没装出表时用清单生成物（`symbio_plugin_api::vocab`）。
    let vocab = symbio_plugin_api::vocab();
    let invariants = serde_json::to_value(check_all(vocab, &snapshot))
        .map_err(|e| PluginError::InternalError(format!("不变量清单序列化失败：{e}")))?;

    // 声誉列：折叠归声誉能力包（名册 #22/#28，`D-221` 按名字读），本插件只取数 + 排版。
    // 与四列同一份切片、同一个 as-of（`i64::MAX` = 落盘即已发生），也与 `loop` 的淘汰
    // 回路读的是**同一条算式**（D-044：两处各折一遍，迟早一个是负的、一个是守约）。
    // 没有权限时不给零值条目——给零值等于替一个你本来看不到的主体报数。
    let reputation = if may_read {
        reputation_reading(
            read_view_or(REPUTATION_LEDGER_VIEW, &snapshot, empty_reputation),
            session_principal,
        )
    } else {
        serde_json::json!({})
    };

    // 承诺列（段二）：与声誉列读**同一批格子**，但问的是另一件事——声誉问
    // 「这个主体几分」，本列问「这一约是什么、违约为什么」。切片已按 `may_read` 取过。
    let commitments = commitments_reading(&snapshot);

    // 就绪集列：与四列同一份切片、同一个 as-of。`may_read = false` 时切片
    // 已被换成空的 ⇒ 本列是 `{ready: []}`（四列的形态），不是声誉那列的空对象。
    // **折叠归任务能力包**（名册 #23，`D-289`）：本插件按名字读 owner 交的候选集，
    // 只把自己的容量套上去——不引用兄弟插件（E-009），也不再链 core 的投影目录，
    // 更不由本插件重算「谁就绪、什么序」。
    let readyset = readyset_column(
        read_view_or(TASK_READYSET_VIEW, &snapshot, empty_readyset),
        task_cap,
    );

    // 转写列（对话面读侧）：
    // doc-link-allow D-004: 「四格 → 三角色」是事件形状到角色的静态映射，非版本改造
    // 与四列同一份切片、同一个 as-of。口径（四格 → 三角色，含工具结果）只活在 core
    // 的 `transcript` 投影里，本处只取数 + 排版；`may_read = false` 时切片已空 ⇒
    // `{ entries: [] }`（四列形态，与就绪集 / 校准同族）。
    let transcript = serde_json::to_value(
        transcript(CHAT_VOCAB, MESSAGE_VOCAB)
            .read_all(&snapshot)
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("转写序列化失败：{e}")))?;

    // 主动性列（DE-008 §3.1.1 外部效标）：与四列同一份切片、同一个 as-of。观测面是
    // `conation.expressed`（欲）与 `task.opened`（升格，溯源认）——「自发发起」的
    // 分子分母都是事实源里已有的事件，不加新格子。`may_read = false` 时切片已空
    // ⇒ `{ intents: [], adoption_rate: 0.0 }`（四列形态）。
    //
    // **读数由能力包交**（`D-221` 按名字读、`D-281` 折叠两半）：本插件按名字读 `conation.intents`，不认识
    // 意图流的形状、也不算采纳率——率是 owner 的算式（单片那一半就在视图里，跨会话那一半
    // 由 `ViewDecl::agg` 交）。本处只挑两格排版，与 `capability_views` 同一纪律。
    let conation = conation_reading(&snapshot);

    // 时域跨度列（同上）：`slo.span` 只读事件信封的 `ts`（入库时间，与 `cost_ms`
    // 同源的实测面），as-of 之后的事件一概不见。空形状是 `{ events: 0, span_ms: 0 }`
    // （四列那一族的有据的零），不是逐档视图的 `{ by_tier: [] }`。
    let span = read_view_or(
        SLO_SPAN_VIEW,
        &snapshot,
        || serde_json::json!({ "events": 0, "span_ms": 0 }),
    );

    // 自主程度列（同上）：**折叠归能力包**（`conation.autonomy`）——本插件按名字读，
    // 不认识分子分母是怎么数的，也不再自己算率。读不到 ⇒ `null`，与 `may_read = false`
    // 的零形状分开：后者是「不给你看」，前者是「装配里没有 owner」。
    let autonomy = read_view_or(CONATION_AUTONOMY_VIEW, &snapshot, || {
        serde_json::Value::Null
    });

    // 实例级聚合列。`may_read = false` 时**不扫**：扫遍实例再报「不给你看」
    // 是白花代价，而结论与空形状相同。空形状与满读数**同键**（axes 两族给有据的
    // 零 / 空，不给「缺列」——读方不必为每个键分诊它在不在）。
    // 技能编译列：技能就是 `memory.encoded{tag:"skill"}` 这条事实，故只数
    // 切片里的格子，不另造投影。切片已按 `may_read` 取过 ⇒ 无权读的人得 0，
    // 与「开着但没编译过」由部署开关 `skill_compile_enabled` 分辨。
    let compiled_skills = snapshot
        .iter()
        .filter(|e| {
            e.kind == symbio_plugin_api::events::EVENT_MEMORY_ENCODED
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

    // 能力视图列（DE-025 D-221）：遍历**公开清单**，逐条经 plugin-api 的读口求值。
    // 本插件不认识任何具体视图名——名字、版本、成本与求值函数都在 owner 的声明里；
    // 新能力包公开一条视图，这里就多一格，不动本文件。
    let capability_views = capability_views(&snapshot);

    let reading = SessionStats {
        session_id: session_id.to_string(),
        has_wal,
        tiers,
        cost,
        tokens,
        spend,
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
        capability_views,
        skill_compile_enabled,
        compiled_skills,
        panel: ReadingPanel::default(),
        usage: None,
    };
    // 会话头那一格与面板里的「用量」段同源：两处都调 `usage_section` 这**一个**函数，
    // 入参是同一份 `reading`。分别算两次是纯函数，不产生第二份口径。
    let usage = usage_section(&reading);
    Ok(SessionStats {
        panel: panel_of(&reading),
        usage,
        ..reading
    })
}

/// 主动性列：**按名字读**能力包交的视图，挑两格排版（`D-221`）。
///
/// ## 为什么只挑两格
///
/// 视图交的是 `{ intents, declared, expressed, escalated, adoption_rate }`；本列的历史形状
/// 是 `{ intents, adoption_rate }`（e2e 与前端都按它读）。多出来的三格是**给别的读方**的：
/// `declared` 供自主触发去重、两个计数供实例级聚合复算。把整份值原样下发会让「这一列的
/// 形状」变成「owner 今天交了什么」——那是把出口契约交给 owner 的口径去决定。
///
/// ## 读不到 ⇒ 空形状 + 出声
///
/// 四种缺席都可诊断（`reason` 在 `capability_views` 那一列里另有完整报告），这里折成
/// 与 `may_read = false` 同一个空形状，并**出声**一次：静默空形状会让「装配坏了」与
/// 「没主动发起过」看起来一样，而这两件事一个要修、一个不用。
fn conation_reading(snapshot: &[Event]) -> serde_json::Value {
    let empty = || serde_json::json!({ "intents": [], "adoption_rate": 0.0 });
    match symbio_plugin_api::read_shared_view_all(CONATION_INTENTS_VIEW, snapshot) {
        Ok(value) => serde_json::json!({
            "intents": value.get("intents").cloned().unwrap_or_else(|| serde_json::json!([])),
            "adoption_rate": value
                .get("adoption_rate")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(0.0)),
        }),
        Err(why) => {
            symbio_plugin_api::plugin_warn!(
                symbio_plugin_api::PLUGIN_ID_SESSION,
                "[stats] 主动性列读不到 `{CONATION_INTENTS_VIEW}`（{why}）：按空读数交"
            );
            empty()
        }
    }
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
                serde_json::from_value::<ViewerRequest>(v)
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
            Some(agent) => symbio_sdk::authz::principal_of(Some(&agent)),
            None => symbio_sdk::authz::request_principal(ctx.as_ref()),
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
