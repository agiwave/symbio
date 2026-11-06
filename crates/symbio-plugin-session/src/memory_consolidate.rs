//! 记忆巩固的**控制面写口**（D-268）：收束之后读能力包公开的 `memory.live`，把同标签
//! 最旧两条合并成一条，并给两条源落 `memory.forgotten`。
//!
//! ## 为什么住在这里
//!
//! 巩固的触发条件是「活的同标签记忆 ≥ 4 条 ⇒ 合并最旧两条」，它看的是**一整轮结束后的
//! 活集**，没有天然的事件格位（D-268 记的缺口）；而合并算法（`merge_with_fidelity` /
//! `accept`）住 `symbio_sdk::text`，能力包只准链接 `plugin-api`。所以它留在控制面，
//! 由会话在**驱动空闲、本轮编码已落格之后**执行一次（`project_final_turn`）——
//! 这正是「编码先于巩固」需要的位置（能力包 `EpisodicEncode` 的模块头同款）。
//!
//! **折叠不抄**：活集从能力包的声明视图 `memory.live` 按名字读（一副折叠两个读者，
//! D-268）。本模块不认识「哪些事实算一条记忆」，只认识两组写口形状。
//!
//! ## 写入顺序
//!
//! **先写合并产物、后写两条 `memory.forgotten`**：后者失败 ⇒ 活集里多一条重复（无害）；
//! 前者失败 ⇒ 源记忆还在。反过来会丢记忆。

use serde_json::{json, Value};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_FORGOTTEN};
use symbio_plugin_api::{Entity, Event, FactSource, Verb, ViewRead, PRINCIPAL_USER};
use symbio_sdk::governance::VisScope;
use symbio_sdk::text::merge_kernel::merge_with_fidelity;

/// 能力包公开的活集视图（`memory.live`，owner = `crates/symbio-plugin-memory`）。
const MEMORY_LIVE_VIEW: &str = "memory.live";

/// 合并正文的最大字符数（与能力包的编码上限同一量级）。
const MERGE_MAX_CHARS: usize = 400;

/// 触发一次巩固所需的**活**记忆条数（同标签）。
const CONSOLIDATE_MIN_ENTRIES: usize = 4;

/// 巩固的标签域：两个生产写方的标签都在内（经验 / 判断）。
const CONSOLIDABLE_TAGS: [&str; 2] = ["经验", "判断"];

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

/// 活集里一条记忆（`memory.live` 的读数形状：`{seq, ts, content, tag}`）。
struct LiveLine {
    seq: u64,
    content: String,
    tag: String,
}

/// 一轮收束之后执行一次巩固；返回本轮合并了几组（0 = 条数不够 / 被拒收 / 视图读不到）。
///
/// `user_seq` 是本轮开口格的序号，只用来拼**确定性**事件键：同一轮的重复投影
/// （CLI 的 `session/project` 与后台收轮）写同一组键，第二笔按 `Duplicate` 收敛。
pub(crate) fn consolidate_turn(
    source: &dyn FactSource,
    events: &[Event],
    user_seq: u64,
    now: i64,
) -> Result<usize, String> {
    let value = match ViewRead::named(MEMORY_LIVE_VIEW).and_then(|view| view.read(events, now)) {
        Ok(value) => value,
        Err(why) => {
            // 四种缺席都是**可诊断的缺席**，不是「没有活记忆」。跳过这一轮，
            // 与既有读方（自主触发的意图视图）同一退化方向。
            symbio_plugin_api::plugin_warn!(
                "session",
                "[memory] 活集视图 `{MEMORY_LIVE_VIEW}` 本轮读不到（{why}）：本轮不巩固"
            );
            return Ok(0);
        }
    };
    let Some(rows) = value.as_array() else {
        return Ok(0);
    };
    let live: Vec<LiveLine> = rows
        .iter()
        .filter_map(|row| {
            Some(LiveLine {
                seq: row.get("seq")?.as_u64()?,
                content: row.get("content")?.as_str()?.to_string(),
                tag: row.get("tag")?.as_str()?.to_string(),
            })
        })
        .collect();

    let mut merged = 0usize;
    for tag in CONSOLIDABLE_TAGS {
        // 活集是新近在前 ⇒ 同标签倒序取最旧两条（巩固从最旧开始推进代数）。
        let entries: Vec<&LiveLine> = live.iter().filter(|line| line.tag == tag).collect();
        if entries.len() < CONSOLIDATE_MIN_ENTRIES {
            continue;
        }
        let oldest: Vec<&LiveLine> = entries.iter().rev().take(2).copied().collect();
        if oldest.len() < 2 {
            continue;
        }
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
        let scope_of = |seq: u64| -> VisScope {
            events
                .iter()
                .find(|e| e.seq.map(|s| s.value()) == Some(seq))
                .map(VisScope::of_event)
                .unwrap_or_default()
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
                "session",
                "[memory] 巩固未入格（tag={tag}，gen {generation}/{max}，fidelity {fidelity:.3}/{min:.3}）：源记忆原样保留"
            );
            continue;
        }

        let consolidated = FactDraft {
            key: format!("v2mc-{user_seq}-{}-{}", source_seqs[0], source_seqs[1]),
            kind: EVENT_MEMORY_CONSOLIDATED.to_string(),
            entity: Entity::Memory,
            verb: Verb::Progressed,
            payload: json!({
                "content": content,
                "tag": tag,
                "generation": generation,
                "fidelity": fidelity,
                "sources": source_seqs,
                "vis_scope": scope_name(vis_scope),
            }),
            produced_by: Some(source_seqs[0]),
        };
        let merged_seq = append_or_duplicate(source, consolidated)?;

        // 源记忆 → 排除式遗忘：Log 不删，只是活集不再包含（可撤销、可审计）。
        for target in source_seqs {
            let forgotten = FactDraft {
                key: format!("v2mf-{target}"),
                kind: EVENT_MEMORY_FORGOTTEN.to_string(),
                entity: Entity::Memory,
                verb: Verb::Closed,
                payload: json!({ "why": "consolidated", "into": merged_seq }),
                produced_by: Some(target),
            };
            append_or_duplicate(source, forgotten)?;
        }
        merged += 1;
    }
    Ok(merged)
}

/// 可见域的书写形态（写方与读方共用 `vis_scope` 这个键，见 `VisScope::of_event`）。
fn scope_name(scope: VisScope) -> &'static str {
    match scope {
        VisScope::ThreadPrivate => "thread_private",
        VisScope::Shared => "shared",
        VisScope::Public => "public",
    }
}

/// 落一条巩固事实；**已经落过**（同一轮的第二个投影方）不算错误，返回该键的 seq。
fn append_or_duplicate(source: &dyn FactSource, draft: FactDraft) -> Result<u64, String> {
    let key = draft.key.clone();
    match source.append(PRINCIPAL_USER.to_string(), draft) {
        Ok(seq) => Ok(seq),
        Err(why) if why.contains("Duplicate") => Ok(source
            .facts()
            .into_iter()
            .find(|e| e.event_id == key)
            .and_then(|e| e.seq.map(|s| s.value()))
            .unwrap_or(0)),
        Err(why) => Err(why),
    }
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
