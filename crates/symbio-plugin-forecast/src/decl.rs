//! 本插件交的**唯一扩展面**：一张对外可读的读数视图（`DE-025` D-221 / D-294）。
//!
//! 视图读的是任务插件**公开**的两条事件（`task.opened` / `task.asserted`），不 import
//! 任何兄弟 crate：名字经 `PLUGIN.yml` 的 `shared_events` 投影进
//! `symbio_plugin_api::events`，读方按名字读，owner 换了实现也不需要改这里。
//!
//! 任务族没有 `FactDecl`（这一族不属于义务环），所以这两对按 `D-282` 的第二种读法
//! 登记：名字由 `EventDecl` 保证拼对，字段无从校验——装载期进
//! `Frozen::assertion_reads()` 的名单，不是静默跳过。
//!
//! ## 口径
//!
//! 「还剩多少没完成」= **开过、仍未断言终态**的任务。它不是任务表的第二份状态机：
//! 视图只做集合差，终态判定仍归任务插件写侧（`task.asserted` 才是「验收通过」）。

use serde_json::{json, Value};

use symbio_plugin_api::events::{EVENT_TASK_ASSERTED, EVENT_TASK_OPENED};
use symbio_plugin_api::{Event, PluginDeclarations, ViewDecl};

/// 对外可读的视图：任务的剩余读数。
pub(crate) const FORECAST_REMAINING: &str = "forecast.remaining";

/// owner —— 与目录名同形（`crates/symbio-plugin-forecast`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-forecast";

/// 扫描成本上界（事件条数）。声明的上界必须与全仓**同一个值**（`seal` 的
/// `ViewCostMismatch`：切点只有一个）。
const COST: u32 = 4096;

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: FORECAST_REMAINING,
    ver: 1,
    reads: &[
        (EVENT_TASK_OPENED, "task_id"),
        (EVENT_TASK_ASSERTED, "task_id"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: remaining,
    // 没有跨会话合并的那一半：剩余量是**当前会话**的读数，把两个会话的剩余量并起来
    // 不是任何一个读数的口径（与 `task.readyset` / `memory.live` 同一族判断）。
    agg: None,
}];

/// 纯折叠：`task.opened` 入集合、`task.asserted` 出集合，交出三个数。
///
/// 同一任务重复开只算一次（任务表的幂等键已经挡住重复落格，这里仍按集合算——
/// 视图不能假设上游一定守规矩）。未来的事件不参与（`ts > now`）。
///
/// `opened` / `closed` 是**事件计数**，`remaining` 是**集合大小**：前者回答「发生过
/// 多少次声明 / 断言」，后者回答「现在还剩几件事」，两者不互相推导。
fn remaining(events: &[Event], now: i64) -> Value {
    let mut outstanding: Vec<&str> = Vec::new();
    let mut opened: u64 = 0;
    let mut closed: u64 = 0;

    for e in events {
        if e.ts > now {
            continue;
        }
        let Some(task) = e.payload.get("task_id").and_then(Value::as_str) else {
            continue;
        };
        if e.kind == EVENT_TASK_OPENED {
            opened += 1;
            if !outstanding.contains(&task) {
                outstanding.push(task);
            }
        } else if e.kind == EVENT_TASK_ASSERTED {
            closed += 1;
            outstanding.retain(|t| *t != task);
        }
    }

    json!({
        "opened": opened,
        "closed": closed,
        "remaining": outstanding.len(),
        "outstanding": outstanding,
    })
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
