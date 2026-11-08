//! 「升格 / 重开」那笔债由 conation 自己申报（DE-042；`D-386` 之后的归属）。
//!
//! 本包只申报**一笔**债：把一条 `conation.expressed`（「欲」）升格成 `task.opened`。
//! session 只写触发、健康两格与「欲」本身，不再按视图名字读判决；三个 early return
//! 全在这里——
//!
//! | 闸 | 判据 | 读什么 |
//! |---|---|---|
//! | 未 enabled | 本轮 `session.config` 的 `conation_enabled` | 最近一份 config（本轮开轮时写） |
//! | 未授权 | `production_matrix().can_write_name(PRINCIPAL_AUTONOMOUS, CAP_DEFINE_WORK)` | 授权表 |
//! | 已声明 | 已有 `task.opened` 的 `payload.goal` 与本条相同 | 事实切片 |
//!
//! 「已声明」按 goal 相等判，而不是按溯源判：同一目标由另一条能力升起时，本包也不该
//! 再开一次（用户可见的是目标，不是谁的 seq）。目标宽度上界也在本包：过宽的「欲」
//! 无法在一轮内落成可执行行，升格它只会制造空转。
//!
//! 判据住这里、动作住 [`crate::port`]：`rows()` 决定欠不欠，执行体自判授权并交出
//! `task.opened` 草稿（`D-385` 的「环不判、能力自判」）。

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{json, Value};

use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CONATION_EXPRESSED, EVENT_SESSION_CONFIG, EVENT_TASK_OPENED,
};
use symbio_plugin_api::{production_matrix, CapabilityRow, CAP_DEFINE_WORK, PRINCIPAL_AUTONOMOUS};

/// 本包申报的端口名：执行体也住本包（能力自带的 `PortExecutor`，D-385）。
pub(crate) const PROMOTE_PORT: &str = "conation";

/// 债行名（只在本包内用）。
const PROMOTE_DEBT: &str = "conation.promote";

/// 目标长度上界（字节）：过宽的目标 = 不可评估的意图。
///
/// 它是**策略参数**，住 owner：迁移前它以 `ConationPolicy::default()` 的字面量躺在
/// 会话里，读方替能力包记着一条它自己的规则。
pub(crate) const MAX_GOAL_LEN: usize = 200;

/// 本包申报的债表：升格 / 重开。
pub(crate) struct PromoteDebts;

impl DebtSource for PromoteDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        // early return 1：本轮开关（`D-386` 的口径：本轮 config，不是上一轮）。
        if !enabled(facts) {
            return Vec::new();
        }
        // early return 2：授权闸。升格就是「定义工作」，判据与写侧闸同一把尺。
        if !authorized() {
            return Vec::new();
        }
        // early return 3：已声明。按 goal 相等判（见模块头）。
        let declared: BTreeSet<&str> = facts
            .facts_of(EVENT_TASK_OPENED)
            .iter()
            .filter_map(|(_, payload)| payload.get("goal").and_then(Value::as_str))
            .collect();
        facts
            .facts_of(EVENT_CONATION_EXPRESSED)
            .into_iter()
            .filter_map(|(id, payload)| {
                let goal = payload.get("goal").and_then(Value::as_str)?;
                if goal.len() > MAX_GOAL_LEN || declared.contains(goal) {
                    return None;
                }
                let source_seq = payload.get("source_seq").and_then(Value::as_u64)?;
                Some(
                    DebtSpec::new(
                        PROMOTE_DEBT,
                        PROMOTE_PORT,
                        id,
                        json!({ "intent_id": id, "goal": goal, "source_seq": source_seq }),
                    )
                    // 只在这一条「欲」还活着时重算：同一条目标由下一次触发表达时是新的
                    // id，本行随之重新申报；旧 id 的行不因别的事实变化而重派。
                    .when_id(EVENT_CONATION_EXPRESSED, id),
                )
            })
            .collect()
    }
}

/// 升格闸的授权判据：自主主体只持 `define.work`（`authz::ROWS`）。
///
/// 债源与执行体调的是同一个函数——两处各写一遍判据必然漂移（`D-251`）。
pub(crate) fn authorized() -> bool {
    production_matrix().can_write_name(PRINCIPAL_AUTONOMOUS, CAP_DEFINE_WORK)
}

/// 本轮 `session.config` 的 `conation_enabled`；没有 config 或缺键 ⇒ 关。
///
/// 取**最近一份** config：它由本轮开轮时写，因此读到的是本轮开关。`D-384` 的会话
/// 读点发生在开轮前（读上一轮 config），口径差异见 `D-386`。
fn enabled(facts: &DebtFacts) -> bool {
    facts
        .facts_of(EVENT_SESSION_CONFIG)
        .last()
        .and_then(|(_, payload)| payload.get("conation_enabled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// 装配口的登记行。
pub(crate) fn row() -> CapabilityRow {
    debt_row("conation", Arc::new(PromoteDebts))
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
