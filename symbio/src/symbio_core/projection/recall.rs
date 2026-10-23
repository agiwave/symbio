//! `recall` 投影（S5 第 12 步，[plan/01 §9](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")、
//! [roadmap/S06 §3](../../../../docs/001-goal/126-长期记忆与语义检索.md "GO-126-长期记忆与语义检索")）。
//!
//! ## 四条口径（每条对应一条验收）
//!
//! - **as-of**：形参 `now` 就是 as-of 锚——`ts > now` 的事件（含巩固与遗忘）
//!   不影响视图（C12：后台巩固不得污染在途决策）；
//! - **排除式遗忘**：`memory.forgotten` 不是删除，是投影不再包含——Log 永不删，
//!   遗忘可撤销、可审计（01 §9.3）；
//! - **可降级**：预算耗尽 ⇒ `degraded: true` + 部分结果，**不允许空且不标降级**
//!   （S06 §6 验收 2；I3 在检索面上的形状）；
//! - **混合打分（语义维）**：`query` 非空 ⇒ 内容重叠分降序（[`recall_score`]），
//!   平手 ts 降序、再平手 seq 降序；`query` 为空 ⇒ 纯 ts 降序——**平凡值**
//! （J2：没有查询就是本维落地前的行为，逐字不变）。 批②。
//!
//! ## 跨主体隔离的边界划分（诚实划界）
//!
//! 本投影按 `actor == viewer` 过滤（记忆默认 `thread_private`，C10）——这是
//! **最小必需**的隔离；`shared` / `public` 的跨主体召回由**读侧**先用
//! [`crate::symbio_core::governance::PermissionMatrix`] 预过滤可见切片，
//! 再调本投影（读侧实现 = `memory_events::recall_view` 的预过滤，
//! ；
//! 调用方传的 viewer 是**认知归属人**，不是跑轮主体）。③ 不 import ⑥——可见性
//! 判定是读路径的职责，投影只管纯函数。

use super::super::event::{
    Entity, Event, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
};
use super::super::view::{RecallEntry, RecallView};
use super::recall_score::{rank, ScoreEntry};
use super::Projection;

/// `recall` 投影：召回 `viewer` 自己的、as-of 可见、未被遗忘的记忆。
///
/// - `tag`：内容标签过滤（`None` = 全部；「记忆类型」是过滤参数，01 §9.1）。这是
///   「**读侧过滤**」这条设计的唯一已落地实例（通用性过滤 / 分层过滤与它同构，见
///   [roadmap/S10 §5](../../../../docs/001-goal/130-个人认知体系注入.md "GO-130-个人认知体系注入")）：过滤只改视图、
///   事实永不删 ⇒ **放宽即召回**（可逆）。它是**承重**的——`memory_events::consolidate`
///   靠它只合并同标签的记忆。三条性质（排除 / 可逆 / 活参数）由 `recall.test.rs` 钉住。
/// - `query`：混合打分的**语义维**——本轮检索意图（`None` / 空 = 无查询 =
///   **平凡值**）。非空时内容重叠分降序（[`recall_score::rank`]，与
///   `verify/recall_eval.rs` 同一份实现）：旧但相关的记忆不再输给新但无关的
/// （ 批②， §3「语义 / 时序 / 因果混合打分」的第一维）。
/// - 排序：见 `query`——无查询时新近度优先（`ts` 降序，确定性——不打浮点分）；
/// - 预算：扫描配额（1 事件 = 1ms）由框架层 [`super::Projection::apply`] 统一执行
/// （[D-105](../../../../docs/003-design/001-核心架构.md#d-105-投影预算语义收敛到框架层闭包只返回-v窗口截断由-apply-统一执行 "DE-001-核心架构")）——超窗只交出切片头部，
///   外壳 `degraded` 由框架置位。本闭包不再自带配额扫描；视图字段
///   `RecallView::degraded`（视图脱离外壳
/// 流动时的自描述，）由**取值点**从外壳誊抄——外壳是唯一的降级事实源。
pub fn recall(
    viewer: impl Into<String>,
    tag: Option<String>,
    query: Option<String>,
) -> Projection<RecallView> {
    let viewer = viewer.into();
    Projection::new(move |events: &[Event], now| {
        let mut entries: Vec<RecallEntry> = Vec::new();
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
                    // 巩固产物（压缩后的新记忆）同样可召回——它也是一条记忆，
                    // 只是 generation > 0（01 §9.2）。
                    if e.actor != viewer {
                        continue; // thread_private：默认只见自己的
                    }
                    let content = e
                        .payload
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if let Some(t) = &tag {
                        let k = e.payload.get("tag").and_then(|v| v.as_str()).unwrap_or("");
                        if k != t.as_str() {
                            continue;
                        }
                    }
                    entries.push(RecallEntry {
                        seq: e.seq.map(|s| s.value()).unwrap_or(u64::MAX),
                        ts: e.ts,
                        content,
                        tag: e
                            .payload
                            .get("tag")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    });
                }
                EVENT_MEMORY_FORGOTTEN => {
                    if let Some(target) = e.produced_by {
                        forgotten.push(target);
                    }
                }
                _ => {}
            }
        }
        // 排除式遗忘：被遗忘的（且遗忘发生在 as-of 之前）不再包含。
        entries.retain(|entry| !forgotten.contains(&entry.seq));
        // 混合打分（ 批②）：排序内核是生产与召回评测集共有的那一份
        // （`recall_score`，verify/recall_eval.rs 用 #[path] 包含同一文件）。
        // 平凡值：无查询 ⇒ 纯 ts 降序，与混合打分落地前逐字相同。
        let scored: Vec<ScoreEntry> = entries
            .iter()
            .map(|e| ScoreEntry {
                content: e.content.clone(),
                tag: e.tag.clone(),
                ts: e.ts,
                seq: e.seq,
            })
            .collect();
        entries = rank(scored, query.as_deref())
            .into_iter()
            .map(|s| RecallEntry {
                seq: s.seq,
                ts: s.ts,
                content: s.content,
                tag: s.tag,
            })
            .collect();
        // 外壳 `degraded` 是框架的（[D-105]）；视图字段恒 `false`，由取值点
        // 誊抄外壳——「空且不标降级」的形状保证挪到取值点（见模块文档）。
        RecallView {
            entries,
            degraded: false,
        }
    })
}

#[cfg(test)]
#[path = "recall.test.rs"]
mod tests;
