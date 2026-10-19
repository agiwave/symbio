//! `session/stats` —— v2 事实源的**只读读数口**。
//!
//! 背景：网格**只写不读**——SLO §1.2 的四列（时延 / 兜底 / 成本 / 断点）早就
//! 有投影算得出数，却没有任何生产出口能读到它们，于是这些投影只能挂
//! `#[allow(dead_code)]` 承认「接线未落地」（[04 §3.1 批③](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）。
//! 本模块就是那一格的读侧出口：[plan/12 §3 批 0](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")
//! 的出口形态②（只读路由）。
//!
//! ## 三条纪律
//!
//! - **只调已有投影，不另写一份统计**（[plan/12 §4](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：
//!   本文件里没有分位数 / 占比 / 累计的算式——口径只活在 core 的投影里，出口
//!   只做「取数 + 排版」。否则同一份事实会长出两套口径，读数与投影静默漂移，
//!   而两边都自称是 SLO。**不变量同理**：清单只调 `check_all`，宽限与预算判据
//!   由 core 定（[04 §3.1 批④](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")——本处不复判）。
//! - **真·只读**：WAL 经 [`EventWalStore::open_readonly`] 打开——不创建文件、
//!   不截断撕裂尾行。截断是**写方**的恢复语义，读方顺手做会与正在落行的写方
//!   撞车（append 模式下截断后剩下的字节接在新 EOF ⇒ 那一行静默损坏）。
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
//! 读数取 `now = i64::MAX`：WAL 是**落盘**事实源，文件里每一条都已发生，没有
//! 「未来的事件」可滤——与 `projection/slo.test.rs` 的扫描口同一口径。

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::symbio_core::{
    calibration, check_all, checkpoint, cost_ledger, fallback_rate, slo_report, transcript, Budget,
    EventWalStore, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginPayload, ReadingPanel, VisScope, SESSION_ID,
};

use super::plugin::SessionPlugin;

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

/// 会话级读数（四列同源，ADR-044）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionStats {
    /// 会话 id。
    pub session_id: String,
    /// 事实源路径——**复算的证据链**：拿这条路径
    /// `open_readonly → range → apply` 必须与本读数逐字相等。
    pub wal: String,
    /// 事实源在不在。不在 = 这个会话还没跑过一轮，全零读数是**有据的零**，
    /// 不是错误（读方不该为了读而创建文件）。
    pub has_wal: bool,
    /// 按档位的时延 / 兜底两列。
    pub tiers: Vec<TierRow>,
    /// 成本台账（`cost_ledger` 视图原样序列化）。
    pub cost: serde_json::Value,
    /// 校准（S9 步 22 的**读侧**，[plan/12 批 1](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：
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
    /// 不变量违规清单（`check_all` 八条：C1 `seq` 单调 / C2 每轮一条 final /
    /// 每轮一条开轮（C2 对偶）/ C3 断言带溯源 / C4 未收束 / C5 超预算 /
    /// `acyclic_deps` 依赖成环或悬空 / `rework_bounded` 返工超上界）。
    /// **空 = 八条全绿**；每条带 `event_id` 与人话。宽限口径（尾轮在途放行、
    /// 按声明档位取预算）在 core 的 `check_all`，任务表两条**无宽限**（环与超界
    /// 不是时序假象），本处只取数——[04 §3.1 批④](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")
    /// 的「不变量进 CI/读侧」：e2e 断言的就是这一列。
    pub invariants: serde_json::Value,
    /// 声誉（S6 第 15 步的**读侧**，[04 §3.1 批⑧](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") +
    /// [plan/12 批 2](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：`own` = 本会话
    /// 主体自己的条目，`by_principal` = 这份事实源里每个主体的条目（`own` 在其中，
    /// 原样摊开、不另造口径）。
    ///
    /// 无读权限 ⇒ **整列为空对象**，不是零值条目：t28 的三态（没源 / 有源不给 / 有数）
    /// 靠它分辨「这里没有数」与「不给你看」——与四列用「空切片 ⇒ 全零」是同一条判定，
    /// 换个列名不换判据。
    pub reputation: serde_json::Value,
    /// 就绪任务集（S7 步 16 的**读侧**，[04 §3.1 批⑨](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：
    /// `readyset` 投影原样序列化（`{ ready: [ { task_id, seq, depends_on } ] }`），
    /// 按 `task_id` 字典序——同一份事实源永远读出同一个集合。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒ `{ready: []}`），
    /// 不归声誉那一族（那列不给的是「有数但不给你看」的空对象）：就绪集为空在语义上
    /// 与「没有任务 / 依赖未闭合」本来就同值，两种「空」由 `has_wal` 与本列的
    /// 上下游（清单在不在）分辨，不靠列的形状。
    pub readyset: serde_json::Value,
    /// 转写（**对话面读侧**，[plan/12 批 2](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：
    /// `transcript` 投影原样序列化（`{ entries: [ { role, text } ] }`，按事件顺序 =
    /// append-only 的时间序；`role` 是 `"user"` / `"assistant"` / `"tool"`，
    /// 工具行另带 `tool` = 工具名）。
    ///
    /// 读的是**事实源本身**（`user.message` / `chat.assistant.final` /
    /// `chat.assistant.fallback` / `artifact.added` 四格——最后一格是
    /// [plan/10 批 3](../../../../docs/004-plan/008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案") 纳入的工具结果），
    /// 不是会话存储里的消息副本——同一份事实既喂模型（`Reasoner` 组 prompt 的输入）
    /// 又能从这里读回，ADR-044 的「实测与判据同源」。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒
    /// `{ entries: [] }`），不取声誉列的空对象形态：对话为空与「不给你看」由 `has_wal`
    /// 分辨——同一份判定，换个列名不换判据。
    ///
    /// 逐条可见域（「这条是谁说的」）属 [11 §2-E](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
    /// 的呈现面过滤（S08 的 `vis_scope`），落地前本列只有**整列**闸——与其余各列同款，
    /// 不在这里先造一个半开的判据。
    pub transcript: serde_json::Value,
    /// 主动性（[02 §3.1 外部效标](../../../../docs/004-plan/002-能力坐标系.md "PL-002-能力坐标系") · 主动性）：
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
    /// 拼就等于把口径抄进第二处（ADR-044 的反面），而读数一改，抄本不报错、只静静
    /// 显示旧标签。故本字段由 [`Self::panel_of`] 从**上面各列已算出的值**生成——
    /// 前端只按 [`ReadingPanel`] 排版，不解析任何会话/统计语义。
    ///
    /// ## 与各列的关系：一份事实、两种用法
    ///
    /// 各列（`tiers` / `cost` / `fleet` …）是**机器可读**的读数，判据与脚本读它们
    /// （`slo-exit-audit` 对的就是这些列）；本字段是**人读**的那一份，同一个数的
    /// 另一种说法。两者由同一个源算出，且判据 `panel_matches_the_columns_it_describes`
    /// 钉住「不脱节」——否则就是两份真相。
    pub panel: crate::symbio_core::ReadingPanel,
    /// **实例级聚合**（P2：跨会话的 SLO 真数）。
    ///
    /// 上面每一列回答「**这一个会话**怎么样」；本列回答「**整个实例**怎么样」——
    /// 口径是把本插件目录下**所有**会话的 WAL 各跑一遍四列里的两个投影
    /// （时延 / 兜底），再交给 core 的 `SloLatencyView::merge` /
    /// `FallbackRateView::merge` 合并。分子分母与分位数因此与单会话列**逐字同一套
    /// 口径**（ADR-044：实测与判据同源），本处不另写一套统计。
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

/// 本会话事实源的**内容侧可见域**（T-094 ②）：记忆载荷 `vis_scope` 的最宽取值。
///
/// 为什么取最宽而不是逐条判：读侧闸是**整列**判据（四列 / 声誉 / 就绪 / 转写
/// 共用同一个答案，见 `read` 的注释），而这份事实源里只要有共享内容，它就不是
/// 纯私有的——「不给你看」必须因为**每一条都私有**，不是因为其中一条私有。
/// 反过来取最窄会让共享内容被私有内容拖着一起锁死，那是把产地的宽度读丢了。
///
/// 缺省（没有任何记忆事件 / 存量事件没有这个字段）= `thread_private`（C10）——
/// 与 T-094 之前的接线逐字一致，存量事实源不因这次接线改变读数。
fn content_scope_of(snapshot: &[crate::symbio_core::Event]) -> VisScope {
    let mut widest = VisScope::ThreadPrivate;
    for e in snapshot {
        if e.entity != crate::symbio_core::Entity::Memory {
            continue;
        }
        let scope = e
            .payload
            .get("vis_scope")
            .and_then(|v| v.as_str())
            .map(VisScope::from_name)
            .unwrap_or_default();
        widest = widest.widen(scope);
    }
    widest
}

/// 实例级聚合列（P2）：把 `storage_root` 下**所有**会话的 WAL 合并成一份读数。
///
/// ## 为什么在插件侧遍历、在 core 侧合并
///
/// 「会话目录长什么样」是插件自己的布局知识（[`super::paths::session_dir`]），
/// 而「多个视图怎么合成一个」是统计口径——后者必须单点，故进 core 的
/// `SloLatencyView::merge` / `FallbackRateView::merge`。本函数只做遍历 + 开档 +
/// 套投影 + 交出去合并。
///
/// ## 遍历的形状
///
/// **全会话**（`paths::session_wals_deep`）：顶层目录各一份 WAL，外加嵌套子会话
/// （`<根>/<safe(父)>/sessions/<safe(子)>/`，可继续嵌套）。子会话不是「别的实例」，
/// 文档上一行说着「**所有**会话」——只扫一层会让那句话说假话，且分形委派
/// （agent_run 派生的子会话再派生子会话）整棵子树都从实例读数里消失。
/// 判据是**存在性**（有 `v2-events.wal` 就是事实源）：`tool_archives` /
/// `transcripts` 等子树里没有它，天然被过滤，不需要维护排除名单。
///
/// 单个 WAL 打不开 ⇒ 跳过并出声（撕裂尾行正在被写方修复是正常状态），不因此让
/// 整列失败：一列读数是**诊断**，为一个坏会话让整个实例的读数消失是本末倒置。
fn fleet_reading(storage_root: &std::path::Path) -> serde_json::Value {
    let wals = super::paths::session_wals_deep(storage_root);
    fleet_from_wals(&wals)
}

/// fleet 列的**读取半程**：拿一份 WAL 路径清单，逐个开档（不可开者跳过并出声）、
/// 套投影、交 core 合并。与枚举半程（[`fn@fleet_reading`]，存在性判据）拆开的原因：
/// 存在性枚举会把「路径不是文件」的假 WAL 提前滤掉，跨平台构造不出「是文件却
/// 读不开」的夹具（权限位在 Windows 管理员下不生效）——跳过韧性只能直接喂路径钉。
fn fleet_from_wals(wals: &[std::path::PathBuf]) -> serde_json::Value {
    let sessions = wals.len();

    let mut lat_parts = Vec::new();
    let mut fb_parts = Vec::new();
    // 外部效标两族（02 §3.1 · 主动性 / 时域跨度）：与 lat/fb 同批扫描、同一份
    // snapshot——「多扫一遍」不是更彻底，是多一次读错范围的机会。
    let mut con_parts = Vec::new();
    let mut span_parts = Vec::new();
    let mut aut_parts = Vec::new();
    for wal in wals {
        let Ok(store) = EventWalStore::open_readonly(wal) else {
            crate::plugin_warn!(
                "session",
                "[stats] 实例聚合跳过打不开的 WAL：{}",
                wal.display()
            );
            continue;
        };
        // `history()` 而非 `range()`：SLO 四列是**累计口径**，压缩归档把被取代的
        // 前缀移出 live 段后，只读 live 会让实例聚合**静默**少算（见 `history` 文档）。
        let snapshot = store.history();
        lat_parts.push(
            slo_report()
                .apply(&snapshot, i64::MAX, Budget::generous())
                .value,
        );
        fb_parts.push(
            fallback_rate()
                .apply(&snapshot, i64::MAX, Budget::generous())
                .value,
        );
        con_parts.push(
            crate::symbio_core::conation()
                .apply(&snapshot, i64::MAX, Budget::generous())
                .value,
        );
        span_parts.push(
            crate::symbio_core::span()
                .apply(&snapshot, i64::MAX, Budget::generous())
                .value,
        );
        aut_parts.push(
            crate::symbio_core::autonomy()
                .apply(&snapshot, i64::MAX, Budget::generous())
                .value,
        );
    }

    let latency = crate::symbio_core::SloLatencyView::merge(&lat_parts);
    let fallback = crate::symbio_core::FallbackRateView::merge(&fb_parts);
    let turns: u64 = fallback.by_tier.values().map(|s| s.turns).sum();
    let fallbacks: u64 = fallback.by_tier.values().map(|s| s.fallbacks).sum();
    // 两族的跨会话合并都在 core（统计口径单点）：分子分母可加、率是同一套算式的
    // 再应用；span 的「一个事件的会话不进分母」也在 core 判，不在这边挑数据。
    let conation = crate::symbio_core::ConationView::merge(&con_parts);
    let span = crate::symbio_core::SpanView::merge(&span_parts);
    let autonomy = crate::symbio_core::AutonomyView::merge(&aut_parts);

    serde_json::json!({
        // 实际读进来的 WAL 数（打不开的不计——见上面出声处）
        "wals": lat_parts.len(),
        // 清单里的 WAL 路径数（含打不开的），与 `wals` 的差就是被跳过的；
        // 枚举判据是「是文件」（存在性），「路径存在但是目录」连清单都进不了
        "sessions": sessions,
        "turns": turns,
        "fallbacks": fallbacks,
        // 逐档位并集：合并后由 `of()` / 分位数纯查询取，与单会话列同一套口径
        "by_tier": {
            "latency": latency.by_tier.values().map(|t| serde_json::json!({
                "tier": t.tier, "samples": t.samples.len(),
                "p50": t.p50(), "p95": t.p95(), "p99": t.p99(),
            })).collect::<Vec<_>>(),
            "fallback": fallback.by_tier.values().map(|s| serde_json::json!({
                "tier": s.tier, "turns": s.turns, "fallbacks": s.fallbacks,
                "rate": s.rate(),
            })).collect::<Vec<_>>(),
        },
        // 外部效标两族（02 §3.1）：与 by_tier 同地位——同一个实例、同一批 WAL、
        // 同一批投影，只是问的问题不同（「哪条轴被抬高了」而不是「哪档跑得好」）。
        // 键名稳定（不随文档改字号）：conation ↔ 主动性、span ↔ 时域跨度。
        "axes": {
            "conation": {
                "expressed": conation.expressed,
                "escalated": conation.escalated,
                "adoption_rate": conation.adoption_rate(),
            },
            "span": {
                "sessions": span.sessions,
                "median_ms": span.median_ms(),
            },
            "autonomy": {
                "closed": autonomy.closed,
                "unattended": autonomy.unattended,
                "unattended_rate": autonomy.unattended_rate(),
            },
        },
    })
}

/// 从本函数的真实数据生成面板描述（`session/stats` 的呈现，concern 在**产者**）。
///
/// ## 只吃 `SessionStats` 的**其它字段**，一目了然、无法漂移
///
/// 签名故意收 `&SessionStats`：生成过程读到的每个数都是从**同一份数据**上算出来的，
/// 不存在「另算一遍」的可能。反过来若在这里重新扫 WAL 或重算分位数，两份口径就
/// 可以各自漂移——而判据 `panel_matches_the_columns_it_describes` 只看结果，
/// 看不住过程。写在这里的转换只有三类：起标签、定单位、挑语气。
fn panel_of(stats: &SessionStats) -> ReadingPanel {
    use crate::symbio_core::{ReadingRow, ReadingSection, ReadingTone};

    // 档位列 → 一段「时延与兜底」。tiers 已按档位排序，此处原序输出（顺序是
    // 数据给的事实，不是排版选择）。
    let perf = ReadingSection {
        title: Some("时延与兜底".into()),
        rows: stats
            .tiers
            .iter()
            .map(|t| ReadingRow {
                label: format!("{} 档", t.tier),
                value: format!("{} 轮 · 兜底率 {:.0}%", t.turns, t.rate * 100.0),
                hint: Some(format!(
                    "P50 {}ms / P95 {}ms / P99 {}ms（{} 个成功样本）",
                    t.p50, t.p95, t.p99, t.samples
                )),
                tone: if t.rate > 0.05 {
                    ReadingTone::Warn
                } else {
                    ReadingTone::Normal
                },
            })
            .collect(),
    };

    // 断点列：事实条数 / 事实源在不在，再加一行种类分布。
    let ck = &stats.checkpoint;
    let events = ck.get("event_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let kinds = ck.get("kind_counts").and_then(|v| v.as_object());
    let last = ck.get("last_seq").and_then(|v| v.as_u64());
    let mut continuity_rows = vec![
        ReadingRow {
            label: "已记录事实".into(),
            value: format!("{events} 条"),
            hint: Some(format!(
                "末条 seq = {}",
                last.map(|s| s.to_string()).unwrap_or_else(|| "—".into())
            )),
            tone: ReadingTone::Normal,
        },
        ReadingRow {
            label: "事实源在不在".into(),
            value: if stats.has_wal {
                "在".into()
            } else {
                "不在".into()
            },
            hint: Some("不在 = 这个会话还没跑过一轮，其余各列是有据的零".into()),
            tone: if stats.has_wal {
                ReadingTone::Normal
            } else {
                ReadingTone::Warn
            },
        },
    ];
    // 事实种类的分布（BTreeMap 序列化 ⇒ 有序）。只列前 6 种：全部铺开是把
    // 「概览」变成清单，而清单归各列自己的读法。
    if let Some(kinds) = kinds {
        continuity_rows.push(ReadingRow {
            label: "事实种类".into(),
            value: kinds
                .iter()
                .take(6)
                .map(|(k, v)| format!("{k} × {}", v.as_u64().unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(" · "),
            hint: Some("按事件 kind 计数（本面板只列前 6 种）".into()),
            tone: ReadingTone::Normal,
        });
    }
    let continuity = ReadingSection {
        title: Some("事实源".into()),
        rows: continuity_rows,
    };

    // 不变量：违规清单为空 ⇒ 一句「全绿」；非空 ⇒ 一条一行、语气需留意。
    let violations = stats.invariants.as_array().map(|a| a.len()).unwrap_or(0);
    let invariants = ReadingSection {
        title: Some("不变量".into()),
        rows: vec![ReadingRow {
            label: "违规".into(),
            value: if violations == 0 {
                "0 条".into()
            } else {
                format!("{violations} 条")
            },
            hint: Some(
                "seq 单调 / 每轮一条开轮与收束 / 断言带溯源 / 未收束 / 超预算 / 依赖环 / 返工界"
                    .into(),
            ),
            tone: if violations == 0 {
                ReadingTone::Normal
            } else {
                ReadingTone::Warn
            },
        }],
    };

    ReadingPanel {
        title: "会话读数".into(),
        subtitle: Some(stats.session_id.clone()),
        sections: vec![perf, continuity, invariants],
    }
}

/// `session/stats` 的请求体：**全部可选**——不传 = 本机默认（今天的行为）。
///
/// `principal` 是读方身份（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 读侧）：
/// 声明了才判可见域，没声明就不判定。**这里的选择权只在读方，可见域与属主
/// 不在载荷里**——请求方自己声明「我能看到什么」等于给自己授权（governance 模块
/// 文档点名的那条捷径），本字段只回答「我是谁」。
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub(crate) struct StatsRequest {
    /// 读方身份；缺席 / `null` ⇒ 不判定。
    #[serde(default)]
    pub principal: Option<String>,
}

/// 读一个会话的事实源，出四列读数 + 校准 / 声誉 / 就绪集 / 转写四列 + 不变量清单。
///
/// 纯读：不写文件、不改网格、不碰会话存储（消息 / 转写）。转写列读的是**事实源**
/// （WAL）里的格，不是会话存储那份消息副本——本函数只经 `open_readonly` 打开
/// 一个文件，别的盘面一概不碰。
///
/// `viewer = Some(身份)` 时先按可见域取**可读切片**再出各列（**读什么由能看什么
/// 决定**，不是先算完再裁结果）；`None` = 本机默认，与判定引入之前逐字一致。
///
/// `session_principal` 是**本会话主体**（声誉列的 `own` 取谁），与读方身份是两回事：
/// 读方是「谁在看」，会话主体是「这份承诺是谁立的」——把两者合成一个参数会让
/// 「别人看我的声誉」退化成「别人的声誉」。
pub(crate) fn read(
    session_id: &str,
    wal: &Path,
    viewer: Option<&str>,
    session_principal: &str,
    storage_root: &Path,
) -> Result<SessionStats, PluginError> {
    let has_wal = wal.exists();
    let store = EventWalStore::open_readonly(wal)
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 读取失败：{e}")))?;
    // `history()` 而非 `range()`（S2.7-b2）：本出口的四列 / 不变量 / 声誉全是
    // **累计口径**——压缩归档把被取代的前缀移出 live 段后，只读 live 会同时
    // ① 让 C1「无跳号」把窗口首号读成违规（假红）、② 让四列与声誉少算被归档的历史。
    // 归档段是「完整事件史」的一半（见 `WalStore::history`）。
    let snapshot = store.history();

    // 读侧闸（[04 §3.1 批⑥](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：**读什么由能看
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
    // **scope 取内容侧的事实**（[T-094](../../../../docs/009-task/094-个人认知可见域产地批.md "T-094-个人认知可见域产地批") ②）：
    // 本会话记忆载荷里记的 `vis_scope`（`memory_events::encode` 的产地，T-094 ①），
    // 取**最宽**的那个——这份事实源里只要有共享内容，它就不是纯私有的。缺省
    // （没有任何记忆 / 存量事件没这个字段）= `thread_private`（C10），与接线前
    // 逐字一致。判据不再是恒传缺省值：「人的认知可供他人借用」（SC-010 判据 3）
    // 由此有生产读方——矩阵内主体读得到他人共享内容，矩阵外主体照旧空切片。
    //
    // **属主恒放行**（owner 相等，C10 的读侧口径）：`user` 在矩阵里没有行
    // （能力表按「智能体的权威动作种类」划分，两张表里的人不靠成员身份），
    // 于是 `shared` 的「矩阵内主体可见」若不叠这一条，属主本人会被自己的
    // 共享内容挡在门外——那是把「本人写、智能体读」读成了「本人也不能读」。
    // 两条是同一次判定的两个分支，不是两个判定点：私有内容属主与矩阵成员的
    // 答案本来就由 `can_see` 的一个 match 给出。
    let content_scope = content_scope_of(&snapshot);
    let may_read = match viewer {
        None => true,
        Some(principal) => {
            principal == crate::symbio_core::authz::SESSION_OWNER
                || crate::symbio_core::authz::production_matrix().can_see(
                    principal,
                    crate::symbio_core::authz::SESSION_OWNER,
                    content_scope,
                )
        }
    };
    let snapshot = if may_read { snapshot } else { Vec::new() };

    // 四列全部来自同一份事件切片（ADR-044：实测与判据同源）。
    let latency = slo_report()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let fallback = fallback_rate()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let cost = cost_ledger()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    // 校准列（[plan/12 批 1](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：与四列同一份
    // 切片、同一个 as-of。观测面 = 路由收束时落的 `memory.recalled{skill_id, fallback}`。
    let cal = calibration()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let ck = checkpoint()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

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
    let calibration = serde_json::to_value(&cal)
        .map_err(|e| PluginError::InternalError(format!("校准序列化失败：{e}")))?;
    let checkpoint = serde_json::to_value(&ck)
        .map_err(|e| PluginError::InternalError(format!("断点序列化失败：{e}")))?;
    let invariants = serde_json::to_value(check_all(&snapshot))
        .map_err(|e| PluginError::InternalError(format!("不变量清单序列化失败：{e}")))?;

    // 声誉列：与四列同一份切片、同一个 as-of（`i64::MAX` = 落盘即已发生）。
    // 没有权限时不给零值条目——给零值等于替一个你本来看不到的主体报数。
    let rep = crate::symbio_core::reputation()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let reputation = if may_read {
        let own = rep.of(session_principal);
        serde_json::json!({ "own": own, "by_principal": rep.by_principal })
    } else {
        serde_json::json!({})
    };

    // 就绪集列（批⑨）：与四列同一份切片、同一个 as-of。`may_read = false` 时切片
    // 已被换成空的 ⇒ 本列是 `{ready: []}`（四列的形态），不是声誉那列的空对象。
    let readyset = serde_json::to_value(
        crate::symbio_core::readyset()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("就绪集序列化失败：{e}")))?;

    // 转写列（对话面读侧，[plan/12 批 2](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）：
    // doc-link-allow D-004: 「四格 → 三角色」是事件形状到角色的静态映射，非版本改造
    // 与四列同一份切片、同一个 as-of。口径（四格 → 三角色，含
    // [plan/10 批 3](../../../../docs/004-plan/008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案") 的工具结果）只活在 core
    // 的 `transcript` 投影里，本处只取数 + 排版；`may_read = false` 时切片已空 ⇒
    // `{ entries: [] }`（四列形态，与就绪集 / 校准同族）。
    let transcript = serde_json::to_value(
        transcript()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("转写序列化失败：{e}")))?;

    // 主动性列（02 §3.1 外部效标）：与四列同一份切片、同一个 as-of。观测面是
    // `conation.expressed`（欲）与 `task.opened`（升格，溯源认）——「自发发起」的
    // 分子分母都是事实源里已有的事件，不加新格子。`may_read = false` 时切片已空
    // ⇒ `{ intents: [], adoption_rate: 0.0 }`（四列形态）。
    //
    // 率与流同列给出：流是**事实**（不截断，见列定义），率是**读数**（纯查询，
    // 与 `tiers` 的 `rate()`、跨会话 `ConationSummary::adoption_rate` 同一套算式）。
    let conation_view = crate::symbio_core::conation()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let conation = serde_json::json!({
        "intents": conation_view.intents,
        "adoption_rate": conation_view.adoption_rate(),
    });

    // 时域跨度列（同上）：`span` 投影只读事件信封的 `ts`（入库时间，与 `cost_ms`
    // 同源的实测面），as-of 之后的事件一概不见。
    let span = serde_json::to_value(
        crate::symbio_core::span()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("时域跨度序列化失败：{e}")))?;

    // 自主程度列（同上）：`autonomy` 投影认 `task.held`（插话抢占 = 外部介入在事实源
    // 里唯一的痕迹）与 `task.asserted`（闭环）。率是纯查询，此处只落分子分母。
    let autonomy = serde_json::to_value(
        crate::symbio_core::autonomy()
            .apply(&snapshot, i64::MAX, Budget::generous())
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("自主程度序列化失败：{e}")))?;

    // 实例级聚合列（P2）。`may_read = false` 时**不扫**：扫遍实例再报「不给你看」
    // 是白花代价，而结论与空形状相同。空形状与满读数**同键**（ axes 两族给有据的
    // 零 / 空，不给「缺列」——读方不必为每个键分诊它在不在）。
    let fleet = if may_read {
        fleet_reading(storage_root)
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
        wal: wal.display().to_string(),
        has_wal,
        tiers,
        cost,
        calibration,
        checkpoint,
        invariants,
        reputation,
        readyset,
        transcript,
        conation,
        span,
        autonomy,
        fleet,
        panel: ReadingPanel::default(),
    };
    Ok(SessionStats {
        panel: panel_of(&reading),
        ..reading
    })
}

impl SessionPlugin {
    /// 路由 `session/stats` 的处理：由会话 id 派生事实源路径，交给 [`read`]。
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
        let wal =
            super::paths::session_dir(&self.storage_dir(), &sid).join(super::paths::V2_WAL_FILE);
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
            Some(agent) => crate::symbio_core::authz::principal_of(Some(&agent)),
            None => crate::symbio_core::authz::request_principal(ctx.as_ref()),
        };
        let stats = read(
            &sid,
            &wal,
            viewer.as_deref(),
            &session_principal,
            // 实例根（聚合列 P2 的扫描范围）：本插件自己的目录，即 `session_dir` 的
            // 上一层。与 WAL 路径同源，不另立一个根。
            self.storage_dir().as_path(),
        )?;
        Ok(PluginPayload::new(&stats))
    }
}

#[cfg(test)]
#[path = "stats.test.rs"]
mod tests;
