//! 记忆巩固的**写口**（D-268 / D-387 Q3）：读活集，把同标签最旧两条合并成一条，
//! 并给两条源落 `memory.forgotten`。
//!
//! ## 它的格位是债，不是事件（D-268 订正）
//!
//! 巩固的触发条件是「活的同标签记忆 ≥ 4 条 ⇒ 合并最旧两条」，它看的是**一整轮结束后
//! 的活集**，没有天然的事件格位。**它的格位因此不是某一条事实，而是一笔债**：
//! [`crate::debt`] 的申报方从活集条数判「有东西可合并」，把 `memory-consolidate` 行
//! 交给 `memory` 端口；[`crate::port`] 的执行体在债被 `sent` 之后跑，读计划期快照、
//! 自己合并、自己落格。**不要再照「没有位置」去翻事件格，也不要再给它找一个「收轮后」
//! 的登记面。**
//!
//! ## 活集怎么算
//!
//! 活集 = `memory.encoded` ∪ `memory.consolidated` − 被 `memory.forgotten` 排除的源
//! （[`crate::recall::live::live`]，与检索同一副折叠）。**不是「本轮编码」**：合并对象
//! 可能跨轮，且永远取活集最旧两条；只按本轮算，上一轮遗留的条目永远合不掉。
//!
//! 债的申报方不读事件（`DebtFacts` 只有身份与载荷），它按同一笔账算条数：
//! 每次巩固恰好写 2 条 `memory.forgotten` + 1 条 `memory.consolidated`，因此
//! `活集条数 = encoded 条数 − consolidated 条数`。低于阈值 ⇒ **返回空行**（不申报），
//! 不申报一笔注定 NoDebt 的债——后者每轮都留痕，那是新噪音。
//!
//! ## 折叠不抄
//!
//! 活集从本包的折叠函数按值读（一副折叠两个读者：检索 / 巩固），本模块不认识「哪些
//! 事实算一条记忆」，只认识两组写口形状。
//!
//! ## 写入顺序
//!
//! **先写合并产物、后写两条 `memory.forgotten`**：后者失败 ⇒ 活集里多一条重复（无害）；
//! 前者失败 ⇒ 源记忆还在。反过来会丢记忆。

use serde_json::{json, Value};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::EVENT_MEMORY_CONSOLIDATED;
use symbio_plugin_api::{Entity, Event, Verb};

use crate::decl::EVENT_MEMORY_FORGOTTEN;
use crate::recall::live::{live, LiveEntry};

mod merge;
use merge::merge_with_fidelity;

/// 合并正文的最大字符数（与能力包的编码上限同一量级）。
const MERGE_MAX_CHARS: usize = 400;

/// 可见域的载荷键（线格式）。能力包不取 sdk 的 `VisScope` 类型（HS-001），
/// 只继承源载荷里那个字符串；三个合法值与读侧 `VisScope::from_name` 同一套拼写。
const VIS_SCOPE_KEY: &str = "vis_scope";

/// 触发一次巩固所需的**活**记忆条数（同标签）。
///
/// 来源是一次巩固的收支，不是拍出来的值：它吃掉 2 条源、写出 1 条合并产物
///（共涉及 3 条事实，见 [`merge_next`]），因此阈值取 **3 + 1 条余量**——多出的 1 条是
/// 「记忆还会长回来」的余量，也让刚合完的那一轮不至于手里一条不剩。低于 4 不申报：
/// 那笔债注定 `NoDebt`，每轮留痕只是噪音。
pub(crate) const CONSOLIDATE_MIN_ENTRIES: usize = 4;

/// 巩固的标签域：两个生产写方的标签都在内（经验 / 判断）。
pub(crate) const CONSOLIDABLE_TAGS: [&str; 2] = ["经验", "判断"];

/// 巩固的接受边界（代数上界 + 保真度下限）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ConsolidateParams {
    pub(crate) max_gen: u32,
    pub(crate) min_fidelity: f64,
}

impl Default for ConsolidateParams {
    fn default() -> Self {
        Self {
            max_gen: 3,
            min_fidelity: 0.7,
        }
    }
}

/// 拒收原因：**不入格**（不是降标入库，也不是写条日志照写）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Rejection {
    RejectedMaxGen { generation: u32, max: u32 },
    RejectedLowFidelity { fidelity: f64, min: f64 },
}

/// 判定方只有一个：代数上界是**预期的终点**，保真度不足是**这条策略装不下它**。
pub(crate) fn accept(
    params: &ConsolidateParams,
    generation: u32,
    fidelity: f64,
) -> Result<(), Rejection> {
    if generation > params.max_gen {
        return Err(Rejection::RejectedMaxGen {
            generation,
            max: params.max_gen,
        });
    }
    if fidelity < params.min_fidelity {
        return Err(Rejection::RejectedLowFidelity {
            fidelity,
            min: params.min_fidelity,
        });
    }
    Ok(())
}

/// 在事件快照上执行一次「同标签最旧两条」合并；没有可合并的条目 ⇒ `None`。
///
/// 返回的草稿**顺序写死**：合并产物在前、两条遗忘在后（见本模块头注的写入顺序）。
/// 事件键由源 `seq` 派生，**同一份快照重放得到同一组键**——事实源按键吸收重复，
/// 因此重放不会产生第二笔 `memory.consolidated`。
pub(crate) fn merge_next(events: &[Event], tag: &str, now: i64) -> Option<Vec<FactDraft>> {
    let live = live(events, now);
    let entries: Vec<&LiveEntry> = live.iter().filter(|entry| entry.tag == tag).collect();
    if entries.len() < CONSOLIDATE_MIN_ENTRIES {
        return None;
    }
    // 活集是新近在前 ⇒ 同标签倒序取最旧两条（巩固从最旧开始推进代数）。
    let oldest: Vec<&LiveEntry> = entries.iter().rev().take(2).copied().collect();
    let source_seqs = [oldest[0].seq, oldest[1].seq];

    // 代数与可见域从源事件载荷读（活集是读视图，不带写账本）。
    let generation_of = |seq: u64| -> u32 {
        events
            .iter()
            .find(|e| e.seq.map(|s| s.value()) == Some(seq))
            .and_then(|e| e.payload.get("generation"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32
    };
    let scope_of = |seq: u64| -> &'static str {
        let raw = events
            .iter()
            .find(|e| e.seq.map(|s| s.value()) == Some(seq))
            .and_then(|e| e.payload.get(VIS_SCOPE_KEY))
            .and_then(Value::as_str);
        // fail-closed 的方向是更窄：认不出（含缺字段）一律 thread_private，
        // 与读侧 `VisScope::from_name` 同一条口径。
        match raw {
            Some("shared") => "shared",
            Some("public") => "public",
            _ => "thread_private",
        }
    };
    let generation = generation_of(source_seqs[0]).max(generation_of(source_seqs[1])) + 1;
    let vis_scope = scope_of(source_seqs[0]);

    // 保真度与**最初一代**比（跨代累积），不是与当次两条源比。
    let basis = origin_texts(
        events,
        &[
            (source_seqs[0], oldest[0].content.as_str()),
            (source_seqs[1], oldest[1].content.as_str()),
        ],
    );
    let basis_refs: Vec<&str> = basis.iter().map(String::as_str).collect();
    let (content, fidelity) = merge_with_fidelity(
        &[oldest[0].content.as_str(), oldest[1].content.as_str()],
        &basis_refs,
        MERGE_MAX_CHARS,
    );

    if let Err(rejection) = accept(&ConsolidateParams::default(), generation, fidelity) {
        let (generation, max, fidelity, min) = match rejection {
            Rejection::RejectedMaxGen { generation, max } => (generation, max, fidelity, 0.0),
            Rejection::RejectedLowFidelity { fidelity, min } => (generation, 0, fidelity, min),
        };
        symbio_plugin_api::plugin_debug!(
            "memory",
            "[memory] 巩固未入格（tag={tag}，gen {generation}/{max}，fidelity {fidelity:.3}/{min:.3}）：源记忆原样保留"
        );
        return None;
    }

    let merged_key = format!("v2mc-{}-{}", source_seqs[0], source_seqs[1]);
    let mut drafts = Vec::with_capacity(3);
    drafts.push(FactDraft {
        key: merged_key.clone(),
        kind: EVENT_MEMORY_CONSOLIDATED.to_string(),
        entity: Entity::Memory,
        verb: Verb::Progressed,
        payload: json!({
            "content": content,
            "tag": tag,
            "generation": generation,
            "fidelity": fidelity,
            "sources": source_seqs,
            "vis_scope": vis_scope,
        }),
        produced_by: Some(source_seqs[0]),
    });

    // 源记忆 → 排除式遗忘：Log 不删，只是活集不再包含（可撤销、可审计）。
    for target in source_seqs {
        drafts.push(FactDraft {
            key: format!("v2mf-{target}"),
            kind: EVENT_MEMORY_FORGOTTEN.to_string(),
            entity: Entity::Memory,
            verb: Verb::Closed,
            // `into` 是合并产物的**事件 id**：执行体在落格前拿不到产物 seq，
            // 而键由源 seq 派生、同一份活集恒定（「被谁取代」照样可审计）。
            payload: json!({ "why": "consolidated", "into": merged_key.clone() }),
            produced_by: Some(target),
        });
    }
    Some(drafts)
}

/// 溯源到**最初一代**的正文：源是合并产物 ⇒ 沿 `sources` 链走到叶子。
fn origin_texts(events: &[Event], sources: &[(u64, &str)]) -> Vec<String> {
    fn walk(
        events: &[Event],
        seq: u64,
        seen: &mut std::collections::HashSet<u64>,
        out: &mut Vec<String>,
    ) {
        if !seen.insert(seq) {
            return;
        }
        let Some(event) = events
            .iter()
            .find(|e| e.seq.map(|s| s.value()) == Some(seq))
        else {
            return;
        };
        if let Some(sources) = event.payload.get("sources").and_then(Value::as_array) {
            for s in sources.iter().filter_map(Value::as_u64) {
                walk(events, s, seen, out);
            }
            return;
        }
        if let Some(content) = event.payload.get("content").and_then(Value::as_str) {
            out.push(content.to_string());
        }
    }

    let mut out = Vec::new();
    for &(seq, fallback) in sources {
        let before = out.len();
        walk(events, seq, &mut std::collections::HashSet::new(), &mut out);
        if out.len() == before {
            out.push(fallback.to_string());
        }
    }
    out
}

#[cfg(test)]
#[path = "consolidate.test.rs"]
mod tests;
