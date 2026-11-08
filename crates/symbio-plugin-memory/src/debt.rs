//! 巩固债：**一行**（D-268 订正 / D-387 Q3），身份从事实派生（禁计数）。
//!
//! | 行 | 端口 | 执行体在哪 | 做什么 |
//! |---|---|---|---|
//! | `memory-consolidate` | `memory`（本插件） | 本插件 | 合并同标签最旧两条，落 `memory.consolidated` + 两条 `memory.forgotten` |
//!
//! ## 活集怎么算
//!
//! 活集 = `memory.encoded` ∪ `memory.consolidated` − 被 `memory.forgotten` 排除的源
//! （与 [`crate::consolidate`] / 检索同一副折叠）。**不是「本轮编码」**：合并对象可能
//! 跨轮，只按本轮算会让上一轮遗留永远合不掉。
//!
//! 申报方读不到事件（`DebtFacts` 只有身份与载荷），因此按同一笔账算条数：一次巩固
//! 恰好写 2 条 `memory.forgotten` + 1 条 `memory.consolidated`，于是
//!
//! ```text
//! 活集条数 = encoded 条数 − consolidated 条数
//! ```
//!
//! 低于阈值 ⇒ **返回空**（不申报）。申报一笔注定 NoDebt 的债会让每轮都留痕，那是新噪音。
//!
//! ## 幂等
//!
//! 债 id 由**最新一条编码事实**与**最新一条巩固事实**的 id 拼成：同一份活集重复申报
//! 得到同一个 id，环按 `(port, id, attempt)` 吸住；活集一变（新编码 / 新巩固），两个
//! 分量之一随之变化，才会产生新的一行。「最新」是位置不是计数。
//!
//! 执行体的产物键同样由源 `seq` 派生（见 [`crate::consolidate`]），所以同一份快照
//! 重放不会写出第二笔 `memory.consolidated`。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::CapabilityRow;

use crate::consolidate::{CONSOLIDABLE_TAGS, CONSOLIDATE_MIN_ENTRIES};
use crate::port::MEMORY_PORT;

/// 行名（环的读数与日志用它，不是契约）。
const ROW: &str = "memory-consolidate";

pub(crate) struct MemoryDebts;

impl DebtSource for MemoryDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let encoded = facts.facts_of(EVENT_MEMORY_ENCODED);
        let consolidated = facts.facts_of(EVENT_MEMORY_CONSOLIDATED);
        let mut rows = Vec::new();
        for tag in CONSOLIDABLE_TAGS {
            let encoded_n = count_tag(&encoded, tag);
            let consolidated_n = count_tag(&consolidated, tag);
            // 活集条数 = encoded − consolidated（每次巩固写 2 条遗忘 + 1 条产物）。
            let live_n = encoded_n.saturating_sub(consolidated_n);
            if live_n < CONSOLIDATE_MIN_ENTRIES {
                continue;
            }
            let Some(last_encoded) = last_tag_id(&encoded, tag) else {
                continue;
            };
            // 巩固后最新事实变了 ⇒ id 变 ⇒ 还有可合并的就再欠一笔。
            let last_consolidated = last_tag_id(&consolidated, tag).unwrap_or("-");
            let id = format!("{ROW}:{tag}:{last_encoded}:{last_consolidated}");
            rows.push(DebtSpec::new(ROW, MEMORY_PORT, id, json!({ "tag": tag })));
        }
        rows
    }
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn memory_debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("memory", Arc::new(MemoryDebts))]
}

fn count_tag(facts: &[(&str, &Value)], tag: &str) -> usize {
    facts
        .iter()
        .filter(|(_, payload)| tag_of(payload) == Some(tag))
        .count()
}

fn last_tag_id<'a>(facts: &[(&'a str, &Value)], tag: &str) -> Option<&'a str> {
    facts
        .iter()
        .rev()
        .find(|(_, payload)| tag_of(payload) == Some(tag))
        .map(|(id, _)| *id)
}

fn tag_of(payload: &Value) -> Option<&str> {
    payload.get("tag").and_then(Value::as_str)
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
