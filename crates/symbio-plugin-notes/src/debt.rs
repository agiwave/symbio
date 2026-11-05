//! 债表（`debt` 族）：反思那一步在等模型回执时，欠一笔一次性问答（D-338）。
//!
//! 行的身份就是 `plugin_row` 执行体落下的「在等」事实身份（`rows::ask_id`）；提示正文
//! 也只从那条事实的载荷取（`DebtFacts::facts_of`）。端口名写 `model.assist`——端口名
//! 全局解析、执行体按名找，跨插件写端口有 D-336 裁决 1 的先例（compact 写同一成员）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_plugin_api::debt::{debt_row, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::rows::ASK_KIND;
use symbio_plugin_api::CapabilityRow;

use crate::decl::{NOTES_INSIGHT, NOTES_REFLECT};

/// 一次性问答的端口成员（D-336 裁决 4）：指令 / 提问随行载荷进。
const ASSIST_PORT: &str = "model.assist";

pub(crate) struct NotesDebts;

impl DebtSource for NotesDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        // 「在等」事实只有一个种类名（行名在载荷里，D-338）：按 `row` 挑本行那一条，
        // 后写覆盖先写——同一键的再次 Ask 只保留最新的提示。
        let Some((ask_id, ask)) = facts
            .facts_of(ASK_KIND)
            .into_iter()
            .rev()
            .find(|(_, payload)| payload.get("row").and_then(Value::as_str) == Some(NOTES_REFLECT))
        else {
            return Vec::new();
        };
        let Some(prompt) = ask.get("prompt").and_then(Value::as_str) else {
            return Vec::new();
        };
        // 行名不够：同一行会有多条债键（每条便签一条）。排除条件钉死**同一键**的
        // 洞察，不按种类笼统排除——否则一条便签反思过，后面所有便签都不再反思。
        let done = ask
            .get("key")
            .and_then(|key| key_value(key, "tool_call_id"))
            .and_then(|call_id| {
                facts
                    .facts_of(NOTES_INSIGHT)
                    .into_iter()
                    .find(|(_, payload)| {
                        payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
                    })
            })
            .map(|(id, _)| id.to_string());

        let mut spec = DebtSpec::new(
            "notes-reflect-model",
            ASSIST_PORT,
            ask_id.to_string(),
            json!({ "system": "", "user": prompt }),
        )
        .when_id(ASK_KIND, ask_id.to_string());
        if let Some(id) = done {
            spec = spec.unless_id(NOTES_INSIGHT, id);
        }
        vec![spec]
    }
}

/// 债键里某个字段的值（`key` 是 `[(名字, 值)]` 的 JSON 数组）。
fn key_value<'a>(key: &'a Value, name: &str) -> Option<&'a str> {
    key.as_array()?.iter().find_map(|pair| {
        let pair = pair.as_array()?;
        if pair.first()?.as_str()? != name {
            return None;
        }
        pair.get(1)?.as_str()
    })
}

/// 本插件申报的债行（`traverse` 期间交给能力收集）。
pub(crate) fn notes_debt_rows() -> Vec<CapabilityRow> {
    vec![debt_row("notes", Arc::new(NotesDebts))]
}
