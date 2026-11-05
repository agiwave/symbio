//! 压缩债：**两行**（D-336），身份全部从事实派生（禁计数），永不读数值载荷。
//!
//! | 行 | 端口 | 执行体在哪 | 做什么 |
//! |---|---|---|---|
//! | `compact-summarize` | `model.assist`（model 插件） | model | 一次问答：把本轮转写压成摘要 |
//! | `compact-write` | `compact`（本插件） | 本插件 | 读回摘要，落 `session.consolidated` |
//!
//! ## 为什么必须两行
//!
//! 执行体归**端口提供方**：`model.assist` 的执行体不认识 `session.consolidated`，
//! 本插件的执行体不调模型。一行做完等于让模型插件写会话世界的事实（D-336 裁决 2）。
//!
//! ## 四谓词
//!
//! 两行各自的身份都由**这一轮**的用户消息派生：
//! - `when`：本轮有终答这一**种类**；
//! - `when_id`：钉死**本轮**那条终答（上一轮的终答不算）；
//! - `unless_id`：钉死「已压缩」（同一轮只压一次）；
//! - `after`：`compact-summarize` 等 `model:<uid>` 收束；`compact-write` 等
//!   `compact-summarize:<uid>` 收束——**按债键钉住**，不靠尝试号猜。
//!
//! 超没超水位在这里判（**欠不欠**），但读的是 `model` 那笔收束的载荷：`context`
//! 的 `request_tokens` / `limit_tokens` 由模型执行体按 provider 实测写入。没超 ⇒
//! 一行都不欠，`model.assist` 不会被派发——阈值住插件内，且不拿每轮一次白问去换。
//! 判定不落到行载荷里：行的载荷仍只带身份（`root_id` / `final_id`）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use symbio_plugin_api::CapabilityRow;

use crate::port::{COMPACT_PORT, SESSION_CONSOLIDATED};

/// 触发阈值（水位占比）。**住插件内**：不下发 `session.config`，不进契约。
const COMPACT_AT: f64 = 0.7;

/// 一行债的名字（环的读数与日志用它，不是契约）。
const SUMMARIZE_ROW: &str = "compact-summarize";
/// 另一行债的名字。
const WRITE_ROW: &str = "compact-write";

/// 一次性问答端口的名字（数据，不是类型）：由 model 插件提供（`assist.rs`）。
pub(crate) const ASSIST_PORT: &str = "model.assist";

/// 摘要指令（**本插件的世界知识**，随行载荷交给 `model.assist` 的执行体）。
///
/// 执行体只按这份 system 发一次问答；它不认识「压缩」——压缩策略归本插件。
pub(crate) const SUMMARIZE_INSTRUCTION: &str =
    "你是上下文压缩器。把下面的对话压缩成一段简洁的中文摘要，\
保留关键事实、决定、未完成事项与用户偏好；不要编造。只输出摘要正文。";

pub(crate) struct CompactDebts;

impl DebtSource for CompactDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        if !over_threshold(facts, user_id) {
            return Vec::new();
        }
        let final_id = format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}");
        let consolidated_id = format!("{SESSION_CONSOLIDATED}:{user_id}");
        let summarize_id = format!("{SUMMARIZE_ROW}:{user_id}");
        vec![
            // 拿摘要：欠 model 插件的端口，载荷只带身份（正文由执行体从事实读）。
            DebtSpec::new(
                SUMMARIZE_ROW,
                ASSIST_PORT,
                summarize_id.clone(),
                json!({
                    "root_id": user_id,
                    "final_id": final_id,
                    "system": SUMMARIZE_INSTRUCTION,
                    "user": "",
                }),
            )
            .when(EVENT_CHAT_ASSISTANT_FINAL)
            .when_id(EVENT_CHAT_ASSISTANT_FINAL, final_id.clone())
            .unless_id(SESSION_CONSOLIDATED, consolidated_id.clone())
            .after(format!("model:{user_id}")),
            // 落压缩事实：必须等**本轮那笔 summarize** 收束；`after` 写的是它的债键，
            // 不是尝试号派生出来的键（失败后重试的尝试号与本行的判据无关）。
            DebtSpec::new(
                WRITE_ROW,
                COMPACT_PORT,
                format!("{WRITE_ROW}:{user_id}"),
                // 载荷只带身份：摘要正文从 summarize 的收束事实读，水位从终答事实读。
                json!({ "root_id": user_id, "final_id": final_id }),
            )
            .when(EVENT_CHAT_ASSISTANT_FINAL)
            .when_id(EVENT_CHAT_ASSISTANT_FINAL, final_id)
            .unless_id(SESSION_CONSOLIDATED, consolidated_id)
            .after(summarize_id),
        ]
    }
}

/// 本插件申报的债行（`debt` 族的登记）。
pub(crate) fn compact_debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("compact", Arc::new(CompactDebts))]
}

/// 本轮水位是否越过阈值。数据来自 `model:<uid>` 的**收束载荷**（provider 实测），
/// 缺任一项 ⇒ 没有可信水位，不欠（宁可少压，不拿猜测去压）。
///
/// **已知项（D-336）**：`context.request_tokens` / `limit_tokens` 由 model 执行体写，
/// 不是 SDK 共享类型；当前唯一消费者是本插件。第二个插件需要同一字段时必须先把它
/// 提升为 SDK 共享类型，不许各自猜载荷形状。
fn over_threshold(facts: &DebtFacts, user_id: &str) -> bool {
    let Some((_, payload)) = facts.closing(&format!("model:{user_id}")) else {
        return false;
    };
    let Some(context) = payload.get("context") else {
        return false;
    };
    let used = context.get("request_tokens").and_then(Value::as_u64);
    let limit = context.get("limit_tokens").and_then(Value::as_u64);
    match (used, limit) {
        (Some(used), Some(limit)) if limit > 0 => used as f64 >= limit as f64 * COMPACT_AT,
        _ => false,
    }
}
