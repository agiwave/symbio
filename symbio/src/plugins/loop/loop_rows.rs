//! 义务环的行表（T-222）：五条策略行 + 一条**临时**执行行。
//!
//! ## 一行只写「什么时候欠、什么算还」
//!
//! 判据全部是数据：`owes` / `until` / `unless` 三个模式，`prio` 是**全局**裁决序
//! （`(prio, name, key)` 升序）。执行体只拿到一笔 [`Debt`](crate::symbio_core::ring::Debt)，
//! 不再读任何请求作用域状态——旧 `LoopState` 的五个变体在这里对应五个视图读到的事实。
//!
//! ## 五条策略行的前后关系
//!
//! ```text
//! 10 classify ── 轮首判决（本幕 round=0 且有发言）
//! 20 report   ── 中途汇报（一段静默到期，未用完配额）
//! 30 compress ── 自动压缩（开关开着且本幕还没压过）
//! 40 recall   ── 记忆 / 技能 / 任务段（有持久事实源且本幕还没召回）
//! 50 reflex   ── 技能快路（非续写幕且本幕召回命中）
//! 60 execute  ── 临时执行行：包住现有 `execute_turn`（T-223 再给它 sent / await）
//! ```
//!
//! 顺序进表而不是进调用点：同一点上多行竞争时，`drive` 取最小的一笔。旧表的
//! `prio` 只是格内序，这里第一次跨行可比（T-222 判据 10）。
//!
//! ## 一行**不在表里**的条件（写在这里，不写在事后）
//!
//! **汇报的轮次前提**：`轮数 > 0 ∧ 轮数 ≥ progress_min_rounds`。日志里没有
//! 「当前工具轮数」这一格（`chat.assistant.reported.tool_rounds` 只在说出口之后
//! 才有），所以它归触发点——`round_report_hook` 自己数批次边界，表判「静默够久
//! ∧ 配额未满」。`report_due` 视图把 `min_rounds` 一并交出，触发点因此也不用读配置。
//!
//! ## 查询口：事实是输入，`now` 也是输入
//!
//! [`owes`] / [`view`] 只收事实切片、轮号与 `now`——不读配置、不读时钟、不碰内存
//! 缓存（ADR-115 决策 1）。切片由 [`turn_slice`] 收窄到本轮：视图声明的扫描成本
//! 上界针对**一轮**，不是整份 WAL。
//!
//! ## 装载期检查
//!
//! 首次查询时冻结声明表并跑 [`check_rows`]：`OrphanFact` 是**已知**的后续任务
//! （协议四名 / `turn.dispatch` / `authz.denied`），记 info；其余缺口（无出口、
//! 键不一致、未声明源）记 error。装载失败 ⇒ 表空 ⇒ 永不前进，但系统其余部分
//! 照常运行（T-222 判据 6）。

use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::symbio_core::ring::{check_rows, owed, Decl, Gap, Lit, Pattern};
use crate::symbio_core::{builtin, Event, Frozen, LatencyTier};

/// 内建行表。注册顺序**不参与**裁决——同 `prio` 由 `(name, key)` 决出。
pub(crate) static ROWS: &[Decl] = &[
    // ── 轮首判决 ─────────────────────────────────────────────────────────
    Decl {
        name: "policy.classify",
        prio: 10,
        by: Some("decide"),
        // 判决可能落到模型，但不生成：快速档（与旧行 `effects.tier` 同值）。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("has_utterance", Lit::Bool(true)),
                ("classify_enabled", Lit::Bool(true)),
                ("classify_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("decide"))],
        }],
        unless: &[],
    },
    // ── 中途汇报 ─────────────────────────────────────────────────────────
    //
    // 一段静默 = 一个 `(turn, anchor)`。汇报事实落格后静默锚前移，下一段静默
    // 是另一笔账——配额之内可以汇报多次，逐段关账而不是「一说就全关」。
    Decl {
        name: "policy.report",
        prio: 20,
        by: Some("report"),
        // 模板产线，零模型往返。
        tier: LatencyTier::Reflex,
        owes: Pattern {
            source: "report_due",
            key: &["turn", "anchor"],
            eq: &[("due", Lit::Bool(true))],
        },
        until: &[Pattern {
            source: "reported_anchors",
            key: &["turn", "anchor"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 自动语义压缩 ─────────────────────────────────────────────────────
    Decl {
        name: "policy.compress",
        prio: 30,
        by: Some("compress"),
        // 语义压缩调模型：深度档。
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("auto_compress", Lit::Bool(true)),
                ("compress_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("compress"))],
        }],
        unless: &[],
    },
    // ── 轮首准备（记忆 / 技能 / 任务段）─────────────────────────────────
    Decl {
        name: "policy.recall",
        prio: 40,
        by: Some("recall"),
        // 事实投影的纯读 + 技能路由，不调模型。
        tier: LatencyTier::Reflex,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("is_persistent", Lit::Bool(true)),
                ("recall_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("recall"))],
        }],
        unless: &[],
    },
    // ── 反射档（技能快路）────────────────────────────────────────────────
    //
    // 续写幕不走快路（落格形状会撞幂等键）；`skill_hit` 来自本幕召回阶段的
    // `result`，所以「命中」这件事第一次有了事实形态。
    Decl {
        name: "policy.reflex",
        prio: 50,
        by: Some("reflex"),
        // 快路就是「不调模型」：反射档是它的定义。
        tier: LatencyTier::Reflex,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[
                ("resumed", Lit::Bool(false)),
                ("skill_hit", Lit::Bool(true)),
            ],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 执行（临时行）────────────────────────────────────────────────────
    //
    // T-222 让控制面第一次成为真环：只要本幕还开着，就欠一次执行。T-223 把它
    // 拆成 `sent` / await 两笔，`by` 换成协议适配器；那一批**不新增调度器**。
    Decl {
        name: "execute-turn",
        prio: 60,
        by: Some("execute"),
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
];

/// 冻结的声明表（进程内唯一）。装载失败 ⇒ `None`：所有 `owes` 恒假、环停在原处。
static FROZEN: LazyLock<Option<Arc<Frozen>>> = LazyLock::new(|| {
    let frozen = match builtin() {
        Ok(frozen) => frozen,
        Err(errors) => {
            crate::plugin_error!(
                "session",
                "[Loop] 事实 / 视图声明装载失败，义务环不再前进：{:?}",
                errors
            );
            return None;
        }
    };
    for gap in check_rows(ROWS, &frozen) {
        match gap {
            // 已知缺口：协议四名 / `turn.dispatch` / `authz.denied` 的消费行
            // 归属后续任务，装载期继续点名而不是豁免。
            Gap::OrphanFact { fact } => {
                crate::plugin_info!("session", "[Loop] 声明的事实暂无行消费：{}", fact);
            }
            other => crate::plugin_error!("session", "[Loop] 行表装载期缺口：{:?}", other),
        }
    }
    Some(frozen)
});

fn frozen() -> Option<Arc<Frozen>> {
    FROZEN.clone()
}

/// 本轮的事实切片：视图只扫**一轮**（声明的成本上界针对轮，不针对整份 WAL）。
///
/// 轮号单调，因此「本轮第一条事件」就是本轮窗口的起点；续写幕复用轮号时，
/// 首幕的事件也在切片里，而视图的 `drive_of` 会按**最新幕锚**收窄。
fn turn_slice(events: &[Event], turn: u64) -> &[Event] {
    match events.iter().position(|e| e.turn == turn) {
        Some(start) => &events[start..],
        None => &[],
    }
}

/// 这一行此刻是否欠着且可还（`owes` 命中、未被 `until` / `unless` 关掉）。
///
/// 返回布尔而不是 [`Debt`](crate::symbio_core::ring::Debt)：调用点只决定「这一步
/// 做不做」。要裁决顺序（同一点多行竞争）时用 `ring::drive`，本函数不代劳。
pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    let Some(frozen) = frozen() else {
        return false;
    };
    let debt = owed(ROWS, &frozen, turn_slice(events, turn), now)
        .into_iter()
        .find(|d| d.decl.name == name && !d.blocked);
    if let Some(debt) = &debt {
        trace(&frozen, debt);
    }
    debt.is_some()
}

/// 按声明求一个视图值（本轮切片）。未声明 / 扫描超界 ⇒ `None`。
pub(crate) fn view(name: &str, events: &[Event], turn: u64, now: i64) -> Option<Value> {
    frozen()?.eval(name, turn_slice(events, turn), now)
}

/// 这一行携带的档位。未登记 ⇒ 深度档（fail-safe：不静默提速）。
pub(crate) fn tier(name: &str) -> LatencyTier {
    ROWS.iter()
        .find(|d| d.name == name)
        .map(|d| d.tier)
        .unwrap_or(LatencyTier::Deep)
}

/// 一笔可还的**可观测面**：行 / 债标识 / 档位 / 视图口径。与旧 `Hit::trace` 同一
/// 目的——「某一步怎么发生了」在日志里要能指认到行与那一笔债，而不只是行名本身。
///
/// 视图版本一并记下（`ViewDecl.ver`）：投影读法一变，同一份事实折出的结论就变，
/// 回放时「这一轮用了哪一版」必须在日志里有答案。
fn trace(frozen: &Frozen, debt: &crate::symbio_core::ring::Debt<'_>) {
    let decl = debt.decl;
    let key = debt.key.trace();
    let view = frozen.view(decl.owes.source);
    crate::plugin_debug!(
        "session",
        "[Loop] 行 {} 可还（{} / {}ms；债 {}；视图 {} v{}）",
        decl.name,
        decl.tier.name(),
        decl.tier.budget_ms(),
        if key.is_empty() { "-" } else { key.as_str() },
        decl.owes.source,
        view.map(|v| v.ver).unwrap_or(0)
    );
}

#[cfg(test)]
#[path = "loop_rows.test.rs"]
mod tests;
