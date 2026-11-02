//! 本插件拥有的事实名、格位、归宿行与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221 / D-281）。
//!
//! 一条事实 `conation.expressed`：**「我想要 X」这件事被表达过**。名字、格位、载荷形状与
//! 归宿行同在这里；写方（会话的自主触发）从清单生成物取名字，读方（[`CONATION_INTENTS`]
//! 视图）从本模块取算法——不存在第二份拼写。
//!
//! ## 格位为什么是 `Conation × Opened`
//!
//! `Entity::Conation` 是内核**认识但不解释**的一格：内核不为它保留任何名字常量，写闸只按
//! 本声明校验名字与格位。用 `Opened` 而不是 `Asserted`，因为表达一条欲**开的是**一条待定
//! 的意图（它可能升格成任务，也可能一直停在「想要」），不是对世界下断言。
//!
//! ## 视图为什么也在这里
//!
//! [`CONATION_INTENTS`] 是纯折叠：从 `conation.expressed` 折出「表达过哪些欲、各自升没升格」。
//! 它在 `PLUGIN.yml` 的 `shared_views:` 登记为**对外可读**，读方经
//! `symbio_plugin_api::views` 按名字读——不依赖本 crate，也不在任何控制面里再写一份（D-221）。
//!
//! ## 为什么折叠要交两半（D-281）
//!
//! 读方要的是两个基数上的同一个口径：**这一片**的采纳率（单会话列）与 **N 片**的采纳率
//! （实例级聚合）。`eval` 交前者，`agg` 交后者；两半都由本插件交、同经冻结声明表被调用，
//! 所以「率 = 已升格 / 已表达」这条算式只有一个 owner。少了 `agg`，跨会话合并就只能在读方
//! 再折一遍——那是同一件事的第二份口径。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「欲」的表达。跨插件共享：**写方是会话**（自主触发的落格是会话存储所有者的写窗口），
/// 所以名字经 `PLUGIN.yml` 的 `shared_events:` 投影进 `symbio_plugin_api::events`，
/// 写方按生成物命名，不引本 crate（E-009）。
pub(crate) use symbio_plugin_api::events::EVENT_CONATION_EXPRESSED;

/// 升格溯源的另一半：`task.opened` 的**信封**字段 `produced_by` 指回那条欲事件。
///
/// 它只被当作**溯源边**读，不作为领域事实读——所以它不进 `VIEWS` 的 `reads`：`reads` 列的是
/// 「读了哪个事实的哪个载荷字段」，而 `produced_by` 是信封字段，没有对应的 `FactDecl`
/// （`task.opened` 的 owner 是任务插件，它没声明这条事实的形状）。把一条没声明的名字写进
/// `reads` 会被 `seal` 判 `UnknownFact`——那才是真正的撒谎。
pub(crate) use symbio_plugin_api::events::EVENT_TASK_OPENED;

/// 自主程度折叠读的两格（任务族，`D-282` 第二种读法）：挂起与闭环。
pub(crate) use symbio_plugin_api::events::{EVENT_TASK_ASSERTED, EVENT_TASK_HELD};

/// 对外可读的视图：表达过的欲及其升格状态。
pub(crate) const CONATION_INTENTS: &str = "conation.intents";

/// 对外可读的第二条视图：无人干预完成率（DE-008 §3.1 自主程度）。
pub(crate) const CONATION_AUTONOMY: &str = "conation.autonomy";

/// owner —— 与目录名同形（`crates/symbio-plugin-conation`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-conation";

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。
///
/// 声明的上界必须与全仓**同一个值**（`seal` 的 `ViewCostMismatch`：切点只有一个），
/// 所以这里不按「欲通常很少」调小——调小会改整张表的切点。
const COST: u32 = 4096;

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    EVENT_CONATION_EXPRESSED,
    (Entity::Conation, Verb::Opened),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: EVENT_CONATION_EXPRESSED,
    port: None,
    key: &[],
    expand: &[],
    // 只声明视图真正读的那一格。`ts` / `seq` / `produced_by` 是信封字段，不是载荷。
    fields: &["goal"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有表达即闭合，没有则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal` 会把
/// `conation.expressed` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: EVENT_CONATION_EXPRESSED,
    phase: 8,
    anchor: 220,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: EVENT_CONATION_EXPRESSED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: EVENT_CONATION_EXPRESSED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: CONATION_INTENTS,
        ver: 1,
        // 只声明载荷字段。升格那一半读的是 `task.opened` 的**信封**（`produced_by`），
        // 见 [`EVENT_TASK_OPENED`] 的注释：信封不是事实面，`reads` 表达不了也不该表达。
        reads: &[(EVENT_CONATION_EXPRESSED, "goal")],
        // 读出口径不是收束判据：本视图不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: intents,
        agg: Some(merge),
    },
    ViewDecl {
        name: CONATION_AUTONOMY,
        ver: 1,
        // 任务族只有 `EventDecl`、没有 `FactDecl`：这两对按 `D-282` 的第二种读法登记
        // （名字由 `EventDecl` 保证拼对，字段无从校验），与 `task.readyset` 同形。
        reads: &[
            (EVENT_TASK_HELD, "task_id"),
            (EVENT_TASK_ASSERTED, "task_id"),
        ],
        // 读数不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: crate::autonomy::autonomy,
        agg: Some(crate::autonomy::merge),
    },
];

/// 采纳率：**一条算式**，单片与 N 片都调它。
///
/// 无表达 ⇒ `0.0`（**有据的零**：没主动发起过，不是「采纳率未知」）。分母含被否决 / 被忽略
/// 的欲——表达过但没升格的欲照常入视图，它是**事实**，藏起来才是撒谎。
fn rate(expressed: u64, escalated: u64) -> f64 {
    if expressed == 0 {
        return 0.0;
    }
    escalated as f64 / expressed as f64
}

/// 纯折叠：`conation.expressed` + `task.opened` 的溯源边 → 意图流 + 两个计数 + 这一片的率。
///
/// 两条口径（与迁移前的投影逐字同源）：
///
/// - **升格靠溯源认**：一条欲「已升格」当且仅当存在一条 `task.opened` 的 `produced_by` 指向
///   它。不靠 goal 字符串相等——字符串相等会把**别的**来源开的同目标任务也算成这条欲的产物。
/// - **as-of**：形参 `now` 就是锚，`ts > now` 的欲与升格一概不见。
///
/// 欲是**流**：按 seq 升序交回（顺序即发生顺序），不截断、不打分——截断它等于悄悄丢掉
/// 「想要过什么」的事实。
fn intents(events: &[Event], now: i64) -> Value {
    // 欲事件按到达序收；升格事件可能先于 / 晚于欲事件到达，故两趟合并。
    let mut items: Vec<(u64, i64, String)> = Vec::new();
    let mut promoted: std::collections::BTreeMap<u64, String> = std::collections::BTreeMap::new();
    for e in events {
        if e.ts > now {
            continue;
        }
        match e.entity {
            Entity::Conation => {
                if e.kind.as_str() == EVENT_CONATION_EXPRESSED {
                    let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
                    let goal = e
                        .payload
                        .get("goal")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    items.push((seq, e.ts, goal));
                }
            }
            Entity::Task => {
                if e.kind.as_str() == EVENT_TASK_OPENED {
                    if let (Some(src), Some(tid)) = (
                        e.produced_by,
                        e.payload.get("task_id").and_then(Value::as_str),
                    ) {
                        promoted.insert(src, tid.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    // 欲是流：按发生顺序（seq 升序）排，不打分、不按时间倒序。
    items.sort_by_key(|(seq, _, _)| *seq);
    let expressed = items.len() as u64;
    let mut escalated = 0u64;
    let mut declared: Vec<String> = Vec::new();
    let stream: Vec<Value> = items
        .into_iter()
        .map(|(seq, ts, goal)| {
            let task_id = promoted.get(&seq).cloned();
            if task_id.is_some() {
                escalated += 1;
                declared.push(goal.clone());
            }
            json!({ "seq": seq, "ts": ts, "goal": goal, "task_id": task_id })
        })
        .collect();
    json!({
        "intents": stream,
        // 「哪些目标已经声明过」由 owner 判（= 升格过的那几条），读方只做**查表**。
        // 让读方自己扫 `intents` 找 `task_id.is_some()`，等于把「已声明」的定义搬到读方——
        // 那是第二份口径，而它今天恰好只有一行，最容易在两处各改一半。
        "declared": declared,
        "expressed": expressed,
        "escalated": escalated,
        "adoption_rate": rate(expressed, escalated),
    })
}

/// **跨会话合并**：N 份 [`intents`] 的产物 → 一份汇总（`D-281`）。
///
/// 只收分子 / 分母，率由 [`rate`] 从汇总值重算——合并的是**可加的量**，率是同一套算式在
/// 汇总值上的再应用，不是另一份口径。不合并 `intents`：跨会话的意图流没有统一顺序，
/// 拼起来既不是「这一片」也不是「全部」，读方要流就逐会话读。
///
/// 认不出的载荷按 0 计：形状是本插件自己上一版交的，读不出说明版本对不上——那会让率变小，
/// 而不会让率凭空出现（缺项不许伪装成读数，`D-281` 理由 3）。
fn merge(parts: &[Value]) -> Value {
    let mut expressed = 0u64;
    let mut escalated = 0u64;
    for p in parts {
        expressed += p.get("expressed").and_then(Value::as_u64).unwrap_or(0);
        escalated += p.get("escalated").and_then(Value::as_u64).unwrap_or(0);
    }
    json!({
        "expressed": expressed,
        "escalated": escalated,
        "adoption_rate": rate(expressed, escalated),
    })
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
