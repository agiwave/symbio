//! 本能力包拥有的事件名、事实形状、对外可读视图与行（DE-025 D-241 / D-251 / D-250）。
//!
//! ## 三块职责，各有一个 owner
//!
//! | 能力面 | 本模块交什么 | 怎么写 |
//! |---|---|---|
//! | 编译 | 收束贡献（`memory.encoded{tag:"skill"}`） | `contributor.rs`，`submit_contributor!`（D-250） |
//! | 回退 | `skill.retreat` 视图（低置信技能的 seq 集）+ `skill.recall` 行（观测与命中） | `decl` + `row.rs`，行 `by = plugin_row`（D-241） |
//! | 命中 | `skill.answered`（这一条技能在本轮判成什么） | `skill.recall` 行的出口格 |
//!
//! ## 为什么读的是「事实」而不是召回视图
//!
//! 回退要改的是**本轮提示词**，而提示词由宿主（loop）装配。能力包不能去改宿主的
//! 视图结构——那是跨插件的私有面。于是分工是：宿主把本幕召回的技能以
//! `turn.recalled` 视图**按名字公开**（每条带 `recall` = 技能事件的 seq），本能力
//! 按自己的置信度口径折出 `skill.retreat`（哪些 seq 本轮**不该**进提示词），宿主
//! 读这个名字过滤自己的段。两边都只认名字，不互相链接（DE-025 D-251）。
//!
//! ## 名字
//!
//! `memory.encoded` 的**格位**由 loop 交（它同时是会话统计的读方）；本能力交事实
//! 形状与写入。`memory.calibrated` 自本能力迁入起由本能力**整条拥有**——名字、格位、
//! 写方与读方都在这里（此前写作在 loop）。
//!
//! `tag` 的取值 `"skill"` 是本能力自己的词汇：跨插件读方（会话统计）经 core 的
//! `SKILL_TAG` 读到同一个字面量，两条拼写是否一致由端到端判据钉住，不由本 crate
//! 依赖 core 来保证。

use serde_json::{json, Value};

use symbio_plugin_api::events::{EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::ring::{Decl, Lit, Pattern};
use symbio_plugin_api::{
    calibration_for, Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb,
    ViewDecl,
};

/// owner —— 与目录名同形（`crates/symbio-plugin-skill`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-skill";

/// 技能事实的标签：`memory.encoded{tag:"skill"}` 是**技能**而不是普通记忆的唯一标记。
///
/// 与 core 的 `SKILL_TAG` 同一个字面量，但本 crate 不依赖 core（DE-025 D-258）：
/// 这个字面量是本能力写下的词汇，读它的会话统计另有自己的一份。
pub(crate) const SKILL_TAG: &str = "skill";

/// 观测格：这一条技能在本轮**判成什么**（命中 / 未命中）。
///
/// 它是 `skill.recall` 行的收口格，也是「这条技能本轮被拿去过闸」的事实形态。
pub(crate) const SKILL_ANSWERED: &str = "skill.answered";

/// 对外可读视图：本轮**不该进提示词**的技能（置信度低于阈值）的 seq 集。
pub(crate) const SKILL_RETREAT: &str = "skill.retreat";

/// 行名：本幕召回的每条技能按置信度过闸，命中即零往返作答。`by = plugin_row`。
pub(crate) const SKILL_RECALL: &str = "skill.recall";

/// 技能正文与触发串的截断上限（字符）。
///
/// 与记忆编码同一口径（`memory_events::ENCODE_MAX_CHARS`）——编译写下的触发串必须
/// 与命中判定截断的**同一对函数、同一个上限**，否则长发言永远命不中。
pub(crate) const COMPILE_LIMIT: usize = 400;

/// 反自动化回退阈值（DE-041 §5）：低于它 ⇒ 本技能不进本轮提示词。
///
/// `confidence = 1 − 回退率`，0.8 读作「最近 5 次里至多 1 次回退」。零使用的新技能
/// 按 1.0 算（不给新技能判死刑）；一次回退就掉到 0.0 ⇒ 下一轮即被摘掉。
pub(crate) const CONFIDENCE_THRESHOLD: f64 = 0.8;

/// 扫描成本上界（事件条数）。与义务环的视图同一口径：超界不静默截断，读方拿到
/// `OverBudget` 后按自己的策略降级（宿主的读口是 fail-open，见 loop 的说明）。
const COST: u32 = 4096;

pub(crate) const EVENTS: &[EventDecl] = &[
    // 校准观测（本能力自有格子）：`memory.calibrated{skill_id, fallback}`。
    EventDecl::new(EVENT_MEMORY_CALIBRATED, (Entity::Memory, Verb::Asserted)),
    // 本能力对自己的判定（这一条技能本轮命中没有）的断言。
    EventDecl::new(SKILL_ANSWERED, (Entity::Domain, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    // 技能事实：与记忆同格子（`memory.encoded`），格位由 loop 交，形状由本能力交。
    FactDecl {
        name: EVENT_MEMORY_ENCODED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["content", "tag", "skill_id", "trigger", "vis_scope"],
    },
    // 校准观测：两个字段就是 `calibration_for` 的契约。
    FactDecl {
        name: EVENT_MEMORY_CALIBRATED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["skill_id", "fallback"],
    },
    // 判定事实：键 `(recall, stage)` 与行的债键同形——`skill.recall` 的 `until`
    // 按它关账，一个幕一条判定。
    FactDecl {
        name: SKILL_ANSWERED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["recall", "stage", "skill_id", "hit"],
    },
];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: SKILL_RETREAT,
    ver: 1,
    reads: &[
        (EVENT_MEMORY_ENCODED, "tag"),
        (EVENT_MEMORY_ENCODED, "skill_id"),
        (EVENT_MEMORY_CALIBRATED, "skill_id"),
        (EVENT_MEMORY_CALIBRATED, "fallback"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: retreating_skills,
}];

pub(crate) const ROWS: &[Decl] = &[
    // 技能事实的**归宿行**：`owes` 与 `until` 同源——有技能即闭合，没有则不欠。
    //
    // 没有它，`seal` 会把 `memory.encoded` 判成 `OrphanFact`（每条事实都要有归宿行；
    // 本能力不把它做成环上的债——编译没有「必须发生」的时机，它是一条收束贡献）。
    Decl {
        name: EVENT_MEMORY_ENCODED,
        prio: 200,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[("tag", Lit::Str(SKILL_TAG))],
        },
        until: &[Pattern {
            source: EVENT_MEMORY_ENCODED,
            key: &[],
            eq: &[("tag", Lit::Str(SKILL_TAG))],
        }],
        unless: &[],
    },
    // 校准观测的归宿行（同上）。
    Decl {
        name: EVENT_MEMORY_CALIBRATED,
        prio: 200,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_MEMORY_CALIBRATED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_MEMORY_CALIBRATED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    // 本幕召回里**属于技能**的那些条目，逐条过置信度闸：
    //
    // - 前件是宿主公开的 `turn.recalled`（每条带 `recall` = 技能事件的 seq 与 `stage`
    //   = 召回阶段格的 seq）。`eq tag = "skill"` 是行的前件，不是实现里的 if：
    //   普通记忆条目的债根本不开。
    // - `prio 50`：召回（40）之后、接缝（52）之前——命中要在装配请求之前定下来，
    //   观测也要在收束之前落格。
    // - 出口是本能力自己的 `skill.answered`（同键），一个幕一条；摘行（回退）与
    //   命中都关同一笔账，不存在「判了但不落格」的第三态。
    Decl {
        name: SKILL_RECALL,
        prio: 50,
        by: Some(symbio_plugin_api::rows::PLUGIN_ROW),
        // 这条行不调模型（命中即零往返作答）：反射档。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn.recalled",
            key: &["recall", "stage"],
            eq: &[("tag", Lit::Str(SKILL_TAG))],
        },
        until: &[Pattern {
            source: SKILL_ANSWERED,
            key: &["recall", "stage"],
            eq: &[],
        }],
        unless: &[],
    },
];

/// 纯折叠：技能事实 × 校准账 → 置信度低于阈值的那些 seq。
///
/// 输出是数组（义务环与宿主的读口都按元素读），每项 `{ recall, skill_id }`：
/// `recall` 是技能事件的 seq，宿主拿它对齐自己召回视图里的条目。
///
/// 口径与 `symbio_plugin_api::calibration_for` 逐字同源（`confidence = 1 − 回退率`）：
/// 本视图不另写一份统计，只把「低于阈值」那一半挑出来。
fn retreating_skills(events: &[Event], now: i64) -> Value {
    let cal = calibration_for(EVENT_MEMORY_CALIBRATED)
        .read_all(events)
        .value;
    let mut out: Vec<Value> = Vec::new();
    for e in events {
        if e.ts > now || e.kind != EVENT_MEMORY_ENCODED {
            continue;
        }
        if e.payload.get("tag").and_then(Value::as_str) != Some(SKILL_TAG) {
            continue;
        }
        let (Some(seq), Some(skill_id)) = (
            e.seq.map(|s| s.value()),
            e.payload.get("skill_id").and_then(Value::as_str),
        ) else {
            continue;
        };
        if cal.of(skill_id).confidence() < CONFIDENCE_THRESHOLD {
            out.push(json!({ "recall": seq, "skill_id": skill_id }));
        }
    }
    Value::Array(out)
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
