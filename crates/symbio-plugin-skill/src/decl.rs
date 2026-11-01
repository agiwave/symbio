//! 本能力包拥有的事件名、事实形状、对外可读视图与行（DE-025 D-241 / D-251 / D-250）。
//!
//! ## 三块职责，各有一个 owner
//!
//! | 能力面 | 本模块交什么 | 怎么写 |
//! |---|---|---|
//! | 编译 | 收束贡献（`memory.encoded{tag:"skill"}`，名字与事实形状归情景记忆家族，D-263） | `contributor.rs`，`submit_contributor!`（D-250） |
//! | 校准 | `memory.calibrated` 的账（`skill.calibration` 视图：每条技能的 `uses` / `fallbacks`） | 本模块的 `calibration_ledger`，算式只有一份 |
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
//! `memory.encoded` 的**名字、格位与事实形状**都随记忆家族走：格位由 loop 交（它
//! 同时是会话统计的读方），事实形状由情景记忆能力包交（DE-026 D-263）。本能力只是
//! 这条事实的一个**写方**（`tag:"skill"` 那半），不申报它的形状与归宿——一个事实
//! 只有一个形状 owner，`seal` 的 `DuplicateFact` 也按这条判。
//! `memory.calibrated` 由本能力**整条拥有**——名字、格位、写方、折法与读方都在这
//! 一个模块里：一条观测怎么折成账，与谁按这个名字读它，是同一件事的两半。
//!
//! `tag` 的取值 `"skill"` 是本能力自己的词汇：跨插件读方（会话统计）经 core 的
//! `SKILL_TAG` 读到同一个字面量，两条拼写是否一致由端到端判据钉住，不由本 crate
//! 依赖 core 来保证。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use symbio_plugin_api::events::EVENT_MEMORY_ENCODED;
use symbio_plugin_api::ring::{Decl, Lit, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// owner —— 与目录名同形（`crates/symbio-plugin-skill`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-skill";

/// 校准观测的名字：**本能力独占这条事实**，所以常量住在这里，不住共享表。
///
/// `CS-007` 判的就是「除 owner 之外没有插件在生产代码里引用它」——折叠搬进本包之后
/// 唯一的另一个读者改按视图名读 `skill.calibration`（`D-286`），于是这条名字退出
/// `PLUGIN.yml` 的 `shared_events`，清单生成物里不再有它。
pub(crate) const EVENT_MEMORY_CALIBRATED: &str = "memory.calibrated";

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

/// 对外可读视图：校准账本身（每条技能用过几次、其中回退几次）。
///
/// 公开的理由只有一个读方在别的 crate：会话统计的校准列（`crates/symbio-plugin-session`
/// 的 `stats`）。本能力自己要的那一半（置信度）经 [`calibration_ledger`] 直接折，
/// 不绕名字读自己的观测。
pub(crate) const SKILL_CALIBRATION: &str = "skill.calibration";

/// 行名：本幕召回的每条技能按置信度过闸，命中即零往返作答。`by = plugin_row`。
pub(crate) const SKILL_RECALL: &str = "skill.recall";

/// 技能正文与触发串的截断上限（字符）。
///
/// 与记忆编码同一口径（情景记忆能力包的 `ENCODE_MAX_CHARS`）——编译写下的触发串必须
/// 与命中判定截断的**同一对函数、同一个上限**，否则长发言永远命不中。
pub(crate) const COMPILE_LIMIT: usize = 400;

/// 反自动化回退阈值（DE-041 §5）：低于它 ⇒ 本技能不进本轮提示词。
///
/// `confidence = 1 − 回退率`，0.8 读作「最近 5 次里至多 1 次回退」。零使用的新技能
/// 按 1.0 算（不给新技能判死刑）；一次回退就掉到 0.0 ⇒ 下一轮即被摘掉。
pub(crate) const CONFIDENCE_THRESHOLD: f64 = 0.8;

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算：超界时 `Frozen::eval` 回 `None`，
/// 环把这一格折成空集。跨插件读口不查它——读侧按调用方给的切片全量折（`D-284`）。
const COST: u32 = 4096;

pub(crate) const EVENTS: &[EventDecl] = &[
    // 校准观测（本能力自有格子）：`memory.calibrated{skill_id, fallback}`。
    EventDecl::new(EVENT_MEMORY_CALIBRATED, (Entity::Memory, Verb::Asserted)),
    // 本能力对自己的判定（这一条技能本轮命中没有）的断言。
    EventDecl::new(SKILL_ANSWERED, (Entity::Domain, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    // 校准观测：两个字段就是 [`calibration_ledger`] 的契约。
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

pub(crate) const VIEWS: &[ViewDecl] = &[
    // 校准账：一条技能在这份事实源里被判过几次、其中回退几次。
    //
    // `reads` 就是事实声明的那两个字段——按名字读它的人（会话统计）拿到的形状由
    // 本能力定，`seal` 也按这张表校验字段有没有人申报过形状。
    ViewDecl {
        name: SKILL_CALIBRATION,
        ver: 1,
        reads: &[
            (EVENT_MEMORY_CALIBRATED, "skill_id"),
            (EVENT_MEMORY_CALIBRATED, "fallback"),
        ],
        // 账不替任何事实关门：它是读数，不是证据。
        evidence: &[],
        cost: COST,
        eval: calibration_ledger_view,
        agg: None,
    },
    ViewDecl {
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
        agg: None,
    },
];

pub(crate) const ROWS: &[Decl] = &[
    // `memory.encoded` 的归宿行不在这里：一条事实只有一张归宿行，它随事实形状的 owner
    // 走（情景记忆能力包，DE-026 D-263）。`tag:"skill"` 那半同样是「有即闭合」，
    // 被那张 eq 为空的行覆盖。
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

/// 一条技能在校准账里的那两列。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SkillStat {
    /// 被判过的次数（命中与回退各算一次）。
    pub(crate) uses: u64,
    /// 其中回退到完整推理的次数。
    pub(crate) fallbacks: u64,
}

impl SkillStat {
    /// 置信度 = 1 − 回退率；零使用 ⇒ 1.0（没有数据不等于有反证）。
    pub(crate) fn confidence(&self) -> f64 {
        if self.uses == 0 {
            1.0
        } else {
            1.0 - (self.fallbacks as f64 / self.uses as f64)
        }
    }
}

/// 纯折叠：校准观测 → 按 `skill_id` 字典序的账。
///
/// 认输入只认**这条名字**，不认格子、也不载荷字段猜：`memory × asserted` 是共享的格子，
/// 将来任何落这格的断言都会凭「恰好带这两个字段」混进置信度。格子对了字段仍逐个校——
/// 缺字段的事件**不参与统计**，而不是记成「未回退」：后者让 `uses` 涨而 `fallbacks` 不涨，
/// 凭空造出高置信度，那正是回退机制失效的形态。
///
/// 同一份切片、同一个 as-of：账与被它过滤的事实用同一个 `now`，读数不会一半在过去一半在未来。
pub(crate) fn calibration_ledger(events: &[Event], now: i64) -> BTreeMap<String, SkillStat> {
    let mut by_skill: BTreeMap<String, SkillStat> = BTreeMap::new();
    for e in events {
        if e.ts > now || e.kind != EVENT_MEMORY_CALIBRATED {
            continue;
        }
        let (Some(skill_id), Some(fallback)) = (
            e.payload.get("skill_id").and_then(Value::as_str),
            e.payload.get("fallback").and_then(Value::as_bool),
        ) else {
            continue;
        };
        let row = by_skill.entry(skill_id.to_string()).or_default();
        row.uses += 1;
        if fallback {
            row.fallbacks += 1;
        }
    }
    by_skill
}

/// 某条技能的置信度：账里没有它 ⇒ [`SkillStat::default`]（零使用 ⇒ 1.0）。
pub(crate) fn confidence_of(events: &[Event], now: i64, skill_id: &str) -> f64 {
    calibration_ledger(events, now)
        .get(skill_id)
        .copied()
        .unwrap_or_default()
        .confidence()
}

/// 校准账的读出口：`{ by_skill: { <skill_id>: { skill_id, uses, fallbacks } } }`。
///
/// 键序由 `BTreeMap` 给（字典序），因此同一份事实源永远折出同一串字节（N1）。
/// 没判过任何技能的会话读成 `{ by_skill: {} }`——**有据的空**，与「读不出来」是两件事，
/// 后者由读口的缺席态承担。
fn calibration_ledger_view(events: &[Event], now: i64) -> Value {
    let by_skill: serde_json::Map<String, Value> = calibration_ledger(events, now)
        .into_iter()
        .map(|(skill_id, stat)| {
            (
                skill_id.clone(),
                json!({
                    "skill_id": skill_id,
                    "uses": stat.uses,
                    "fallbacks": stat.fallbacks,
                }),
            )
        })
        .collect();
    json!({ "by_skill": Value::Object(by_skill) })
}

/// 纯折叠：技能事实 × 校准账 → 置信度低于阈值的那些 seq。
///
/// 输出是数组（义务环与宿主的读口都按元素读），每项 `{ recall, skill_id }`：
/// `recall` 是技能事件的 seq，宿主拿它对齐自己召回视图里的条目。
///
/// 账只有一份（[`calibration_ledger`]）：本视图不另写统计，只把「低于阈值」那一半挑出来。
fn retreating_skills(events: &[Event], now: i64) -> Value {
    let cal = calibration_ledger(events, now);
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
        if cal.get(skill_id).copied().unwrap_or_default().confidence() < CONFIDENCE_THRESHOLD {
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
