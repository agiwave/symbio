//! 措辞生成的**行债**：谁欠一次模型往返（D-341 裁决三、D-344）。
//!
//! `wording` 不再直连 provider：只有「本轮可作答」（`session.config.compose_enabled`）
//! 且分类判出「能凭上下文直接答」时，本插件才申报一行 `model.assist` 的口债。
//! 派发、五格与执行体全归环与 model 的端口；本插件**不写转写**（D-020）。
//!
//! 回执由 `session` 读走并落 `chat.assistant.final`（宿主侧的唯一写入者），
//! 匹配键是 `(port = model.assist, root_id = 本轮 user.message 的事实 id)`——
//! 「同一次往返」由这两个字段双认，不靠次序。

use std::sync::Arc;

use serde_json::{json, Value};

use symbio_plugin_api::debt::{
    debt_row, session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest,
};
use symbio_plugin_api::events::{EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE};
use symbio_plugin_api::{CapabilityRow, Verdict};

use super::config::ComposeConfig;
use super::wording;

/// 债行名（行表数据）：`plugin_rows` 按它选执行体，环按它派发。
pub(crate) const COMPOSE_WORDING_MODEL: &str = "compose-wording-model";

/// 一次性问答的端口（D-336 裁决 4）。与 model 插件 `AssistPort` 的声明同名。
pub(crate) const ASSIST_PORT: &str = "model.assist";

/// 上游判决事实的种类与 id 前缀（归 classify；本插件只读，不改写）。
const CLASSIFY_ANSWERED: &str = "classify.answered";

/// 措辞的运行时债表（挂载时由 `traverse` 报到，每轮 `rows()` 现算）。
pub(crate) struct ComposeDebts {
    config: ComposeConfig,
}

impl ComposeDebts {
    pub(crate) fn new(config: ComposeConfig) -> Self {
        Self { config }
    }
}

impl DebtSource for ComposeDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let Some(config) = facts.fact_payload(EVENT_SESSION_CONFIG, &session_config_id(user_id))
        else {
            return Vec::new();
        };
        if !config
            .get("compose_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Vec::new();
        }
        let Some(reason) = facts
            .fact_payload(CLASSIFY_ANSWERED, &format!("{CLASSIFY_ANSWERED}:{user_id}"))
            .and_then(|payload| payload.get("reason"))
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        // 模板产线不欠模型往返：只有「能凭上下文答」这一档需要组织文本。
        let verdict = Verdict::Answered {
            reason: reason.to_string(),
        };
        if !verdict.needs_wording_generation() {
            return Vec::new();
        }
        let debt_id = format!("{COMPOSE_WORDING_MODEL}:{user_id}");
        // 回执（或判死）已落 ⇒ 这笔已经还过。身份由本轮用户消息派生（D-323），
        // 同一轮重放不会造出第二笔债。
        if facts.closing(&debt_id).is_some() {
            return Vec::new();
        }
        let session_prompt = config
            .get("system_prompt")
            .and_then(Value::as_str)
            .unwrap_or("");
        vec![DebtSpec::new(
            COMPOSE_WORDING_MODEL,
            ASSIST_PORT,
            debt_id,
            json!({
                "system": wording::system_prompt(&self.config, session_prompt),
                // 对话线由执行体从事实投影进 `user`（与 classify 的请求同一形态）：
                // 这里不再拼第二份「以下是历史」。
                "user": "",
                // 回执的匹配键：宿主按 `(port, root_id)` 双认这一次往返。
                "root_id": user_id,
                // 环账的用途名（WAL 与 e2e 按它认领这次内部请求）。
                "via": "compose/wording",
            }),
        )
        .when_id(CLASSIFY_ANSWERED, format!("{CLASSIFY_ANSWERED}:{user_id}"))
        .lease(60_000, 1)]
    }
}

/// 本插件申报的债行（与 `classify_debt_rows` 同一形状：一个族项，源随插件生命周期）。
pub(crate) fn compose_debt_rows(config: &ComposeConfig) -> Vec<CapabilityRow> {
    vec![debt_row(
        "compose",
        Arc::new(ComposeDebts::new(config.clone())) as Arc<dyn DebtSource>,
    )]
}
