//! 义务环的行表（T-222a）：五条策略行 + 一条**临时**执行行。
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
//! ## 两处**不在表里**的条件（写在这里，不写在事后）
//!
//! 1. **汇报的触发前提**：`round > 0 ∧ 工具轮数 ≥ progress_min_rounds`。日志里
//!    没有「当前工具轮数」这一格（`chat.assistant.reported.tool_rounds` 只在说出口
//!    之后才有），所以它仍是触发点的前提，直到 T-223 把 `sent` / 工具轮落成协议
//!    事实。表里这一行判的是「静默够久 ∧ 配额未满」——静默锚是用户格时，轮首的
//!    `quiet_ms ≈ 0`，实际不会在轮首误开账。
//! 2. **执行行的动作**：本批只是把 `execute_turn` 包成 actor；`sent` / await /
//!    在途收清是 T-223。故它此刻用 `turn_closed` 当出口，而不是协议事实。
//!
//! ## 为什么模块暂时 `cfg(test)`
//!
//! T-222a 的交付物是「表与求解 + 对拍」，控制面（`chat_loop` / `Registry`）一个字
//! 不动。没有生产消费方时编进非测试构建会被 `dead_code` 逐个点名；与 `decl` 同一
//! 形态如实标 `cfg(test)`，T-222b 接线时删掉 `mod.rs` 里的 `#[cfg(test)]`。

use crate::symbio_core::ring::{Decl, Lit, Pattern};
use crate::symbio_core::LatencyTier;

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

#[cfg(test)]
#[path = "loop_rows.test.rs"]
mod tests;
