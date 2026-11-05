//! model 的债表（数据）：模型接缝欠哪一轮、什么时候欠，全由本插件自报。
//!
//! 两行是**两个输入身份**：
//! - `model-reply`：用户输入事实的 id = 这一轮的身份（`when` 有输入、`unless_id` 该轮尚无终答）；
//! - `model-after-tools`：最近一条工具产物事实的 id = 工具轮之后那一轮的身份
//!   （`when` 有工具请求、`when_no_open` 工具端口无未还债）。
//!
//! ## 输入依赖写进 `when`（不是屏障）
//!
//! 模型执行体要读 `session.config` 的**工具表与系统提示词**；请求缺了它只能发半成品
//! （空工具、空提示词）。所以两行的可派发条件里都带 `when_id(session.config, …)`：
//! 必须是**这一轮** `user.message` 名下的那条 config，上一轮留下的不算。
//!
//! 这不是「等配置就绪」的时序补丁，而是**依赖声明**：行说清楚自己读哪条输入，
//! 环按输入的存在性选债。写方（session 开轮单元）保证 config 一定会落——
//! 空表也落，见 `consume_inbox_turn` 的兜底。
//!
//! `unless_id` 认**种类 + 身份**：终答是独立事实种类（`chat.assistant.final`），
//! 环的 `request` / `sent` / `retry` 因种类不同不误命中——可重试失败后的重试轮
//! 不会被提前跳过。债表永不读载荷。

use std::sync::Arc;

use serde_json::json;
use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG, EVENT_TOOL_REQUESTED,
    EVENT_USER_MESSAGE,
};

use crate::port::MODEL_PORT;

/// 工具端口名（数据，不是类型）：本行只问它「还有没有未还债」，不解释它的语义。
const TOOL_PORT: &str = "tool";

/// 终答事实的身份前缀：与执行体落的那条事实**逐字一致**（`chat.assistant.final:<root_id>`）。
const FINAL_FACT_PREFIX: &str = EVENT_CHAT_ASSISTANT_FINAL;

/// model 的债表（无状态：每轮从请求与事实视图现算）。
pub(crate) struct ModelDebts;

impl DebtSource for ModelDebts {
    fn rows(&self, request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        // 这一轮的 config 身份 = 由这条 user.message 的 id 派生（写方同源）。
        let config_id = session_config_id(user_id);
        let payload = json!({
            "provider": request.provider,
            "root_id": user_id,
            "principal": "user",
        });
        let mut rows = vec![
            // 有用户输入、尚无终答：这一轮欠一次模型往返。
            // 输入依赖：本轮 `session.config` 必须已落（工具表 / 提示词的唯一来源）。
            DebtSpec::new(
                "model-reply",
                MODEL_PORT,
                format!("model:{user_id}"),
                payload.clone(),
            )
            .when(EVENT_USER_MESSAGE)
            .when_id(EVENT_SESSION_CONFIG, config_id.clone())
            .unless_id(FINAL_FACT_PREFIX, format!("{FINAL_FACT_PREFIX}:{user_id}")),
        ];
        // 工具轮结束后的续接：身份来自最近一条工具产物，没有产物就不欠。
        // `when` 认 `tool.requested`：没有这一轮的工具请求，`when_no_open` 会在
        // 「根本没有工具债」时恒成立，每条响应都想再派一次。
        // 续接请求同样读本轮 config，因此依赖同一条身份。
        if let Some(tool_id) = facts.latest_fact_id(EVENT_ARTIFACT_ADDED) {
            rows.push(
                DebtSpec::new(
                    "model-after-tools",
                    MODEL_PORT,
                    format!("model:after:{tool_id}"),
                    payload,
                )
                .when(EVENT_TOOL_REQUESTED)
                .when_id(EVENT_SESSION_CONFIG, config_id)
                .when_no_open(TOOL_PORT),
            );
        }
        rows
    }
}

/// 把两行债挂进 `debt` 族（构建 / 能力收集机制，不是新注册面）。
pub(crate) fn model_debt_rows() -> Vec<symbio_plugin_api::CapabilityRow> {
    vec![symbio_plugin_api::debt::debt_row(
        "model",
        Arc::new(ModelDebts) as Arc<dyn DebtSource>,
    )]
}

#[cfg(test)]
#[path = "debt.test.rs"]
mod tests;
