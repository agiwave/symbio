//! 压缩债：**一行**，身份全部从事实派生（禁计数），永不读载荷。
//!
//! 判据四件：
//! - `when` 本轮有终答这一**种类**；
//! - `when_id` 钉死**本轮**那条终答（上一轮的终答不算）；
//! - `unless_id` 钉死「已压缩」（同一轮只压一次）；
//! - `after` 挂在 model 债的收束键上（模型这笔债还没还完时压缩不抢跑）。
//!
//! 超没超水位不在这里判：`DebtFacts` 只有种类 + 身份，载荷不进来。判定归执行体，
//! 读事实自算（`port.rs`）。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::CapabilityRow;

use crate::port::{COMPACT_PORT, SESSION_CONSOLIDATED};

/// 债行名（环的读数与日志用它，不是契约）。
const ROW: &str = "compact-run";

pub(crate) struct CompactDebts;

impl DebtSource for CompactDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        vec![DebtSpec::new(
            ROW,
            COMPACT_PORT,
            format!("compact:{user_id}"),
            // 载荷只带身份、不带数值：数值由执行体从事实读，环不解释。
            json!({ "root_id": user_id, "final_id": final_id }),
        )
        .when(EVENT_CHAT_ASSISTANT_FINAL)
        .when_id(EVENT_CHAT_ASSISTANT_FINAL, final_id)
        .unless_id(SESSION_CONSOLIDATED, format!("{SESSION_CONSOLIDATED}:{user_id}"))
        .after(format!("model:{user_id}"))]
    }
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn compact_debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("compact", Arc::new(CompactDebts))]
}
