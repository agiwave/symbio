//! 分类能力的债表：**每轮开账 + 两行接力**（W2c-1b）。
//!
//! ## 形状
//!
//! `classify_enabled`（本轮 `session.config` 的**有效值**）为真、本轮尚无判决时，
//! 每轮开一笔 `classify.decide`：
//!
//! ```text
//! classify.decide（plugin_row，债 id 恒为 `classify.decide:id=<user.message id>`）
//!   ├─ 规则可判 → 债键带 reason → 执行体直接回判决事实（零 LLM）
//!   └─ 规则不可判 → 债键只有 id → 执行体 Ask（plugin.row.asked）
//!        └─ model.assist（申报方自报，D-338）→ 回执 → 同一行重触发 → parse 落判决
//! ```
//!
//! 模型路径的 `reason` 在**解析时**才出现，所以不能进债键：键一变，`ask_id` 与
//! 「等哪一笔债」的身份就跟着变。债 id 只由「本轮哪句话」决定——两种路径同一个 id。
//!
//! ## 为什么模型行等的是「判决行收束」，而不是让执行体再查一次事实
//!
//! 环的计划是**一次算全、按批派发**（`DriveRequest` → `rows()` → `plan()`）：同一计划
//! 里分类行与模型行都会被选中，而计划用的是**派发前**的事实。若让模型执行体在发送前
//! 再查一次「本轮判过了吗」，请求发不发就由执行体临时决定，而环早已按「已派发」记账
//! ——两层判据一旦分叉，就是最难查的一类错（账上派了、线上没发 / 反之亦然）。
//! **时间差只能由派发顺序解决**：`model-reply` 用 `after` 指向本行收束的债 id，
//! 前置债未收束时它**根本不进计划**（`SupportSource::rows` 的 `after` 解析），
//! 收束后的下一次计划才轮得到它。执行体保持纯执行——不读判据，不改账。
//!
//! `after` 管时序，`model/debt.rs` 那道门（本轮已判 `Answered` ⇒ 不欠）管语义，
//! 两者并存，都不许下沉到 `port.rs` 执行体。

use std::sync::Arc;

use serde_json::{json, Value};

use symbio_plugin_api::debt::{
    debt_row, session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest,
};
use symbio_plugin_api::events::{EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE};
use symbio_plugin_api::rows::{ASK_KIND, PLUGIN_ROW};
use symbio_plugin_api::{CapabilityRow, EVENT_RESPONSE};

use super::config::ClassifyConfig;
use super::decide::SYSTEM_PROMPT;
use super::decl::{CLASSIFY_ANSWERED, CLASSIFY_DECIDE, CLASSIFY_ESCALATE};
use super::rules::classify_by_rule;

/// 一次性问答的端口成员（数据；由 `model` 插件提供，D-336 裁决 4）。
const ASSIST_PORT: &str = "model.assist";

/// 分类的债表（无状态：每轮从事实视图现算）。
pub(crate) struct ClassifyDebts {
    rule_shortcut: bool,
    system_prompt: String,
}

impl DebtSource for ClassifyDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        // 本轮这句用户原话：没有它就没有可判的东西。
        let Some(utterance) = facts
            .fact_payload(EVENT_USER_MESSAGE, user_id)
            .and_then(|p| p.get("text"))
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        // 有效开关：这一轮 config 说的才算（上一轮的不算）。
        let enabled = facts
            .fact_payload(EVENT_SESSION_CONFIG, &session_config_id(user_id))
            .and_then(|c| c.get("classify_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !enabled {
            return Vec::new();
        }
        if verdict_exists(facts, user_id) {
            return Vec::new();
        }

        // 规则命中：reason 是**规则表的确定性结论**（同一输入恒定），可以进债键。
        // 规则未命中：债键只留 id，等 `parse` 把结论落成事实。
        let key = match self.rule_shortcut.then(|| classify_by_rule(utterance)).flatten() {
            Some(reason) => vec![
                ("id".to_string(), user_id.to_string()),
                ("reason".to_string(), reason.to_string()),
            ],
            None => vec![("id".to_string(), user_id.to_string())],
        };
        // 债 id 与路径无关：模型行按这个 id 等本行收束（见模块文档）。
        let debt_id = format!("{CLASSIFY_DECIDE}:id={user_id}");
        let mut spec = DebtSpec::new(
            CLASSIFY_DECIDE,
            PLUGIN_ROW,
            debt_id.clone(),
            json!({ "row": CLASSIFY_DECIDE, "key": key }),
        )
        .lease(60_000, 1);

        // 已经 Ask 过：回执一落格就重触发同一行（`when` 只在等待期挂上——
        // 首轮没有 ask 时挂 `EVENT_RESPONSE` 会让本行永远欠一笔不存在的应答）。
        let ask = ask_for(facts, user_id);
        if ask.is_some() {
            spec = spec.when(EVENT_RESPONSE);
        }
        let mut rows = vec![spec];

        // 申报方自报往返（D-338）：行只落「在等」，真正的请求由这里欠给端口。
        // 正文留空 ⇒ 执行体从事实读面拼本轮转写（`model/assist.rs::transcript`）。
        if let Some((ask_id, _)) = ask {
            rows.push(
                DebtSpec::new(
                    "classify-decide-model",
                    ASSIST_PORT,
                    ask_id.clone(),
                    json!({ "system": self.system_prompt, "user": "" }),
                )
                .when_id(ASK_KIND, ask_id),
            );
        }
        rows
    }
}

/// 本行那一笔「在等」事实（`ask_id` 是它的身份；`prompt` 由执行体交）。
fn ask_for<'a>(facts: &'a DebtFacts, user_id: &str) -> Option<(String, &'a Value)> {
    facts
        .facts_of(ASK_KIND)
        .into_iter()
        .rev()
        .find(|(_, payload)| {
            payload.get("row").and_then(Value::as_str) == Some(CLASSIFY_DECIDE)
                && key_value(payload.get("key"), "id") == Some(user_id)
        })
        .map(|(id, payload)| (id.to_string(), payload))
}

/// 债键里某个字段的值（`key` 是 `[(名字, 值)]` 的 JSON 数组）。
fn key_value<'a>(key: Option<&'a Value>, name: &str) -> Option<&'a str> {
    key?.as_array()?.iter().find_map(|pair| {
        let pair = pair.as_array()?;
        if pair.first()?.as_str()? != name {
            return None;
        }
        pair.get(1)?.as_str()
    })
}

/// 本轮是否已经判过（`answered` 或 `escalate` 任一）。
fn verdict_exists(facts: &DebtFacts, user_id: &str) -> bool {
    [CLASSIFY_ANSWERED, CLASSIFY_ESCALATE].iter().any(|kind| {
        facts
            .facts_of(kind)
            .iter()
            .any(|(_, payload)| payload.get("id").and_then(Value::as_str) == Some(user_id))
    })
}

/// 把分类债表挂进 `debt` 族（构建 / 能力收集机制，不是新注册面）。
pub(crate) fn classify_debt_rows(config: &ClassifyConfig) -> Vec<CapabilityRow> {
    vec![debt_row(
        "classify",
        Arc::new(ClassifyDebts {
            rule_shortcut: config.rule_shortcut,
            system_prompt: config
                .prompt_override()
                .unwrap_or(SYSTEM_PROMPT)
                .to_string(),
        }) as Arc<dyn DebtSource>,
    )]
}
