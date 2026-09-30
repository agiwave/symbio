//! `recall` 投影（S5 第 12 步，[plan/01 §9](../../../../docs/plan/01-核心架构.md)、
//! [roadmap/S06 §3](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md)）。
//!
//! ## 三条口径（每条对应一条验收）
//!
//! - **as-of**：形参 `now` 就是 as-of 锚——`ts > now` 的事件（含巩固与遗忘）
//!   不影响视图（C12：后台巩固不得污染在途决策）；
//! - **排除式遗忘**：`memory.forgotten` 不是删除，是投影不再包含——Log 永不删，
//!   遗忘可撤销、可审计（01 §9.3）；
//! - **可降级**：预算耗尽 ⇒ `degraded: true` + 部分结果，**不允许空且不标降级**
//!   （S06 §6 验收 2；I3 在检索面上的形状）。
//!
//! ## 跨主体隔离的边界划分（诚实划界）
//!
//! 本投影按 `actor == viewer` 过滤（记忆默认 `thread_private`，C10）——这是
//! **最小必需**的隔离；`shared` / `public` 的跨主体召回由**读侧**先用
//! [`crate::symbio_core::governance::PermissionMatrix`] 预过滤可见切片，
//! 再调本投影。③ 不 import ⑥——可见性判定是读路径的职责，投影只管纯函数。

use super::super::event::{
    Entity, Event, EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
};
use super::super::view::{Budget, RecallEntry, RecallView, View};
use super::Projection;

/// `recall` 投影：召回 `viewer` 自己的、as-of 可见、未被遗忘的记忆。
///
/// - `tag`：内容标签过滤（`None` = 全部；「记忆类型」是过滤参数，01 §9.1）；
/// - 排序：新近度优先（`ts` 降序，确定性——不打浮点分）；
/// - 预算：`budget.ms` 为扫描配额（1 事件 = 1ms），超配额 ⇒ 降级 + 部分结果。
pub fn recall(viewer: impl Into<String>, tag: Option<String>) -> Projection<RecallView> {
    let viewer = viewer.into();
    Projection::new(move |events: &[Event], now, budget: Budget| {
        let quota = (budget.ms as usize).max(1);
        let mut entries: Vec<RecallEntry> = Vec::new();
        let mut forgotten: Vec<u64> = Vec::new();
        let mut scanned = 0usize;
        let mut degraded = false;
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
                    scanned += 1;
                    if scanned > quota {
                        degraded = true;
                        break; // 预算耗尽：带部分结果降级，不装作扫完了
                    }
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
        entries.sort_by(|a, b| b.ts.cmp(&a.ts).then_with(|| b.seq.cmp(&a.seq)));
        View {
            value: RecallView { entries },
            degraded,
            used: Budget::new(0, scanned as u64),
        }
    })
}
