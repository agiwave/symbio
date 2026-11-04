//! 时延分层能力包的三条视图（名册 #13，DE-016 §8.2 / DE-008 §3.1）。
//!
//! | 视图 | 折什么 | 单片（`eval`） | N 片（`agg`） |
//! |---|---|---|---|
//! | [`SLO_LATENCY`] | 成功轮的实测时延，按档位 | 每档的样本与三个分位数 | 样本并集后重排，再取分位数 |
//! | [`SLO_FALLBACK`] | 兜底率，按档位 | 每档的分子分母与率 | 分子分母相加，率由同一条算式重算 |
//! | [`SLO_SPAN`] | 时域跨度 | 这一片的跨度 | 收 `events ≥ 2` 的会话，取中位数 |
//!
//! ## 为什么三列同一个包
//!
//! 它们共用**同一条档位判据**（[`declared_tier`]）：一列按档折时延、一列按档折兜底率，
//! 分家就会出现两份「这一轮属于哪一档」。档位不是这三条视图的私有词汇，它是派发事实的
//! 读法——判据只有一份，住在 [`declared_tier`]。
//!
//! ## 为什么折叠交两半（`D-281`）
//!
//! 读方要两个基数上的同一个口径：**这一片**的分位数（单会话列）与 **N 片**的分位数
//! （实例级聚合）。`eval` 交前者、`agg` 交后者，两半都由本包交、同经冻结声明表被调用。
//! 少了 `agg`，跨会话合并只能在读方再折一遍——那是同一件事的第二份口径，而分位数的
//! 取法一旦分成两处，「P99 是什么」就有两个答案。
//!
use std::collections::BTreeMap;

use serde_json::{json, Value};

use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::{Entity, Event, LatencyTier, PluginDeclarations, Verb, ViewDecl};

/// 对外可读的三条视图。名字经 `PLUGIN.yml` 的 `shared_views:` 投影进
/// `symbio_plugin_api::view_names`；读方按名字读，不依赖本 crate（`D-221`）。
pub(crate) const SLO_LATENCY: &str = "slo.latency";
pub(crate) const SLO_FALLBACK: &str = "slo.fallback";
pub(crate) const SLO_SPAN: &str = "slo.span";

/// owner —— 与目录名同形，装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-slo";

/// 扫描成本上界（事件条数）。切点全仓只有一个值（`seal` 的 `ViewCostMismatch`）。
const COST: u32 = 4096;

/// 缺失 / 未知档位进这个桶：**可观测，不静默归类**。
const UNSPECIFIED: &str = "unspecified";

pub(crate) const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: SLO_LATENCY,
        ver: 1,
        // 档位只读 `user.message` 的 `tier`。`cost_ms` 是**信封**字段，不是载荷，
        // `reads` 表达不了也不该表达。
        reads: &[(EVENT_USER_MESSAGE, "tier")],
        // 读数不替任何事实关门。
        evidence: &[],
        cost: COST,
        eval: latency,
        agg: Some(latency_merged),
    },
    ViewDecl {
        name: SLO_FALLBACK,
        ver: 1,
        reads: &[(EVENT_USER_MESSAGE, "tier")],
        evidence: &[],
        cost: COST,
        eval: fallback,
        agg: Some(fallback_merged),
    },
    ViewDecl {
        name: SLO_SPAN,
        ver: 1,
        // 只读事件信封的 `ts`（入库时间，不是载荷）。
        reads: &[],
        evidence: &[],
        cost: COST,
        eval: span,
        agg: Some(span_merged),
    },
];

/// 本轮声明的档位：读 `user.message` 载荷的 `tier`；缺失 / 未知 ⇒ [`UNSPECIFIED`]。
///
/// `before_seq`：
/// - `Some(b)` ⇒ 只认 seq `< b` 的声明（兜底 / 收束事件要归到**它之前**那一幕的档位）；
/// - `None` ⇒ 不限 seq，取**最早**一条开轮格——开轮事件的分母按「这一轮以哪一档开局」归位
///   （续写幕可以换档，那是另一幕的事实）。
fn declared_tier(events: &[Event], turn: u64, before_seq: Option<u64>) -> String {
    let before = |e: &Event| before_seq.is_none_or(|b| e.seq.is_some_and(|s| s.value() < b));
    events
        .iter()
        .rev()
        .find(|u| {
            u.entity == Entity::Scope
                && u.verb == Verb::Opened
                && u.scope == turn
                && u.kind.as_str() == EVENT_USER_MESSAGE
                && before(u)
        })
        .and_then(|u| u.payload.get("tier").and_then(Value::as_str))
        .and_then(LatencyTier::from_name)
        .map(|t| t.name().to_string())
        .unwrap_or_else(|| UNSPECIFIED.to_string())
}

/// 最近邻取法（`ceil(p/100 × n) − 1`）：P50 = 中位、P100 = max，与校准彩排同一取法。
///
/// 空样本 ⇒ 0（**有据的零**：这一档没有成功轮，不是「分位数未知」）。
fn percentile(samples: &[u64], p: u64) -> u64 {
    if samples.is_empty() || p == 0 {
        return 0;
    }
    let n = samples.len();
    let idx = ((p * n as u64).div_ceil(100) as usize).min(n) - 1;
    samples[idx]
}

/// 每档一行：`{ tier, sample_ms, p50, p95, p99 }`（按档名字典序，逐字节确定）。
///
/// `sample_ms` 是**升序样本本身**，不是计数：分位数只能从样本重算，跨会话合并（[`latency_merged`]）
/// 要的正是它。读方要「几个样本」就取它的长度——视图不替读方决定呈现口径。
fn latency_rows(by_tier: BTreeMap<String, Vec<u64>>) -> Vec<Value> {
    by_tier
        .into_iter()
        .map(|(tier, mut sample_ms)| {
            sample_ms.sort_unstable();
            json!({
                "tier": tier,
                "sample_ms": sample_ms,
                "p50": percentile(&sample_ms, 50),
                "p95": percentile(&sample_ms, 95),
                "p99": percentile(&sample_ms, 99),
            })
        })
        .collect()
}

/// 时延（单片）：成功收束轮（`chat.assistant.final`）的 `cost_ms`，按档位归。
///
/// **兜底轮不计入时延样本**：兜底是失败路径，把它的耗时混进成功响应的分位数会同时污染
/// 两列（时延被拉高、兜底率看不见）。失败轮的耗时记在 fallback 事件的 `cost_ms` 里，
/// 由需要它的读方按需取。
fn latency(events: &[Event], now: i64) -> Value {
    let mut by_tier: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for e in events {
        if e.ts > now {
            continue;
        }
        if e.entity != Entity::Scope
            || e.verb != Verb::Closed
            || e.kind.as_str() != EVENT_CHAT_ASSISTANT_FINAL
        {
            continue;
        }
        let tier = declared_tier(events, e.scope, e.seq.map(|s| s.value()));
        by_tier.entry(tier).or_default().push(e.cost_ms);
    }
    json!({ "by_tier": latency_rows(by_tier) })
}

/// 时延（N 片）：同档位样本**并集后重排**再取分位数。
///
/// 升序是 [`percentile`] 的前提，不是巧合；输入里没有的档位不出现在结果里（不造空账）。
/// 分位数只能从**样本**重算——把各片的 p50/p95/p99 再平均一次得到的不是分位数，
/// 那正是「同一件事两份口径」在跨会话这一侧的形态。
fn latency_merged(parts: &[Value]) -> Value {
    let mut by_tier: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for part in parts {
        let Some(rows) = part.get("by_tier").and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            let Some(tier) = row.get("tier").and_then(Value::as_str) else {
                continue;
            };
            let entry = by_tier.entry(tier.to_string()).or_default();
            if let Some(list) = row.get("sample_ms").and_then(Value::as_array) {
                entry.extend(list.iter().filter_map(Value::as_u64));
            }
        }
    }
    json!({ "by_tier": latency_rows(by_tier) })
}

/// 每档一行：`{ tier, turns, fallbacks, rate }`。
fn fallback_rows(by_tier: BTreeMap<String, (u64, u64)>) -> Vec<Value> {
    by_tier
        .into_iter()
        .map(|(tier, (turns, fallbacks))| {
            json!({
                "tier": tier,
                "turns": turns,
                "fallbacks": fallbacks,
                "rate": rate(turns, fallbacks),
            })
        })
        .collect()
}

/// 兜底率：**一条算式**，单片与 N 片都调它（无 turn ⇒ 0.0，不给空档位报警）。
fn rate(turns: u64, fallbacks: u64) -> f64 {
    if turns == 0 {
        return 0.0;
    }
    fallbacks as f64 / turns as f64
}

/// 兜底率（单片）。
///
/// - **turn** = `user.message`（`turn × opened`）——用户消息开启一轮；
/// - **档位** = `user.message` 载荷的 `tier`；
/// - **兜底** = `chat.assistant.fallback`（`turn × closed`）——生成失败也必须有输出，
///   fallback 落在哪档就是哪档的兜底。
fn fallback(events: &[Event], now: i64) -> Value {
    let mut by_tier: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for e in events {
        if e.ts > now || e.entity != Entity::Scope {
            continue;
        }
        match e.verb {
            Verb::Opened => {
                if e.kind.as_str() != EVENT_USER_MESSAGE {
                    continue;
                }
                // 分母按整条史取**最早**一幕的派发档。
                let tier = declared_tier(events, e.scope, None);
                by_tier.entry(tier).or_default().0 += 1;
            }
            Verb::Closed => {
                if e.kind.as_str() != EVENT_CHAT_ASSISTANT_FALLBACK {
                    continue;
                }
                let tier = declared_tier(events, e.scope, e.seq.map(|s| s.value()));
                by_tier.entry(tier).or_default().1 += 1;
            }
            _ => {}
        }
    }
    json!({ "by_tier": fallback_rows(by_tier) })
}

/// 兜底率（N 片）：分子分母都是**计数**，逐档相加即可；率由 [`rate`] 从汇总值重算。
///
/// 同档位在两份输入里都有时**相加**，不取较新的一份：WAL 是 append-only 的，读数没有
/// 「哪一份更新」可谈。
fn fallback_merged(parts: &[Value]) -> Value {
    let mut by_tier: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for part in parts {
        let Some(rows) = part.get("by_tier").and_then(Value::as_array) else {
            continue;
        };
        for row in rows {
            let Some(tier) = row.get("tier").and_then(Value::as_str) else {
                continue;
            };
            let entry = by_tier.entry(tier.to_string()).or_default();
            entry.0 += row.get("turns").and_then(Value::as_u64).unwrap_or(0);
            entry.1 += row.get("fallbacks").and_then(Value::as_u64).unwrap_or(0);
        }
    }
    json!({ "by_tier": fallback_rows(by_tier) })
}

/// 时域跨度（单片）= 末事件 `ts` − 首事件 `ts`（Unix 毫秒）。
///
/// `ts` 是**入库时间**（事件信封的元数据，不是载荷自填）——所以跨度天然含系统自身开销，
/// 与 `cost_ms` 同源（实测与判据同源）。as-of：`ts > now` 一概不见。
fn span(events: &[Event], now: i64) -> Value {
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = None;
    let mut n = 0u64;
    for e in events {
        if e.ts > now {
            continue;
        }
        n += 1;
        first = Some(first.map_or(e.ts, |f| f.min(e.ts)));
        last = Some(last.map_or(e.ts, |l| l.max(e.ts)));
    }
    json!({
        "events": n,
        // last ≥ first 恒成立（同一次遍历里赋值），max(0) 只是不赌
        "span_ms": (last.unwrap_or(0) - first.unwrap_or(0)).max(0) as u64,
    })
}

/// 时域跨度（N 片）：各会话跨度取中位数。
///
/// **一个事件的会话不进中位数**：0 / 1 事件的会话跨度恒为 0，进分母会把中位数系统性
/// 往下拉——那不是「任务更快」，是「还没有任务」被读成了「零时长任务」。
///
/// 无样本 ⇒ `median_ms: null`（**有据的空**，不是 0——「实例里没有会话」与「会话时长
/// 0ms」是两件事，给 0 就读混了）。
fn span_merged(parts: &[Value]) -> Value {
    let mut spans: Vec<u64> = parts
        .iter()
        .filter(|p| p.get("events").and_then(Value::as_u64).unwrap_or(0) >= 2)
        .filter_map(|p| p.get("span_ms").and_then(Value::as_u64))
        .collect();
    spans.sort_unstable();
    let median = if spans.is_empty() {
        Value::Null
    } else {
        json!(percentile(&spans, 50))
    };
    json!({ "sessions": spans.len(), "median_ms": median })
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
