//! **活集折叠**：`memory.encoded` ∪ `memory.consolidated` − 被 `memory.forgotten` 排除的源
//! （DE-026 D-264 / D-268）。
//!
//! ## 一副折叠，两个读者
//!
//! | 读者 | 怎么读 | 为什么 |
//! |---|---|---|
//! | 检索（本包内） | 直接调 [`live`] | 检索是包内的判据，不必绕一圈声明 |
//! | 巩固（本包内） | 直接调 [`live`] | 格位是债（D-268 订正），执行体与折叠同在一个 crate |
//! | `memory` 报表（跨插件） | 按名字读声明视图 `memory.live`（`eval` = [`memory_live`]） | 报表按名字取，不抄口径（D-387 Q3） |
//!
//! 几条路走同一个函数：`memory_live` 只是把 [`live`] 的输出序列化成 JSON，没有第二份
//! 折叠逻辑。这正是 `D-267`「记忆一个所有者」的兑现——若记忆真是四个 crate，这副折叠
//! 就得跨 crate，那时只能靠声明视图；现在它是同一个 crate 里的一个函数 + 一张声明。
//!
//! ## 四条口径（每条都对应一条既有验收）
//!
//! - **as-of**：`ts > now` 的事件一概不见（C12：后台巩固不得污染在途决策）；
//! - **排除式遗忘**：`memory.forgotten` 不是删除，是折叠不再包含它 `produced_by` 指的
//!   那条源——Log 永不删，遗忘可撤销、可审计（DE-001 §9.3）；
//! - **巩固产物同样可召回**：`memory.consolidated` 也是一条记忆，只是 `generation > 0`；
//! - **认知归属人**：只折 `actor == PRINCIPAL_USER` 的那些——记忆事件的 actor 是**认知
//!   归属人**，不是跑轮主体（跨主体收放是读侧闸的事，见 `loop` 的可见域预过滤）。
//!
//! ## 排序
//!
//! 新近在前（`ts` 降序，平手 `seq` 降序）：检索随后按查询重排（[`super::score`]），
//! 巩固按「倒序取最旧两条」推进代数。两个读者都要的次序在这里定一次，调用方不再排第二遍。

use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::{Entity, Event, PRINCIPAL_USER};

use crate::decl::EVENT_MEMORY_FORGOTTEN;

/// 活集里的一条记忆。
///
/// `seq` 是**来源事实的存储位置**——它是这条记忆的身份（检索的债键格、巩固的
/// `sources` 溯源、遗忘的排除目标都按它点名）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LiveEntry {
    pub(crate) seq: u64,
    /// 编码时刻（Unix 毫秒）。跨会话比较新近只能靠它（Turn 类事件的 `ts` 是 0）。
    pub(crate) ts: i64,
    pub(crate) content: String,
    pub(crate) tag: String,
}

/// 活集：`events` 里 as-of 可见、未被遗忘、按新近度排好的记忆条目。
pub(crate) fn live(events: &[Event], now: i64) -> Vec<LiveEntry> {
    let mut entries: Vec<LiveEntry> = Vec::new();
    let mut forgotten: Vec<u64> = Vec::new();
    for e in events {
        // as-of：未来的事件（巩固 / 遗忘 / 编码）一概不见。
        if e.ts > now {
            continue;
        }
        if e.entity != Entity::Memory {
            continue;
        }
        match e.kind.as_str() {
            EVENT_MEMORY_ENCODED | EVENT_MEMORY_CONSOLIDATED => {
                if e.actor != PRINCIPAL_USER {
                    continue;
                }
                entries.push(LiveEntry {
                    seq: e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
                    ts: e.ts,
                    content: payload_str(e, "content"),
                    tag: payload_str(e, "tag"),
                });
            }
            EVENT_MEMORY_FORGOTTEN => {
                // 排除边读的是**信封**字段 `produced_by`（不是载荷）：`ViewDecl.reads`
                // 只收 `(事实名, 载荷字段)`，所以依赖图少一条「视图依赖遗忘事实」的边。
                // 缺口登记在 DE-026 D-268，不开 `reads` 的口子（那是架构改动）。
                if let Some(target) = e.produced_by {
                    forgotten.push(target);
                }
            }
            _ => {}
        }
    }
    entries.retain(|entry| !forgotten.contains(&entry.seq));
    entries.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.seq.cmp(&a.seq)));
    entries
}

fn payload_str(e: &Event, key: &str) -> String {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// 声明视图 `memory.live` 的求值函数（`ViewDecl.eval`）。
///
/// 输出形状是一条**有次序的数组**，每项 `{seq, ts, content, tag}`——巩固按 `tag` 分组、
/// 按 `seq` 点查（代数 / 可见域 / 最初一代仍走全史），因此这里只交检索与巩固都要的
/// 那几格，不替调用方折第二遍。
///
/// 空活集 ⇒ 空数组（不是 `null`）：读方按「没有活记忆」处理，与「视图读不出来」
/// （`ViewUnavailable`）必须可区分。
pub(crate) fn memory_live(events: &[Event], now: i64) -> serde_json::Value {
    serde_json::Value::Array(
        live(events, now)
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "seq": e.seq,
                    "ts": e.ts,
                    "content": e.content,
                    "tag": e.tag,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "live.test.rs"]
mod tests;
