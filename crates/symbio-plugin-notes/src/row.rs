//! 行执行体：本能力的两条轮内行。
//!
//! - `notes.index`：一条成功的 `notes_add` 产物 ⇒ 一条 `notes.indexed` 草稿。
//! - `notes.reflect`：一条 `notes.indexed` ⇒ **一次模型往返**（提示由本文件交），
//!   回执 ⇒ 一条 `notes.insight` 草稿（D-242）。
//!
//! 两条都是 `Decl.by = plugin_row`（DE-025 D-241）的实现。入口是纯值 [`RowRequest`]：
//! 债键（`tool_call_id`）与本轮的事实切片；出口是 [`RowStep`]——要么直接交
//! [`FactDraft`]，要么交一条提示让宿主代付往返。**不碰事实源**，落格仍由宿主的唯一
//! 写口做；往返用哪个供应商、花多少预算也由宿主管。
//!
//! 判断全部从事实里读：产物格按 `tool_call_id` 认（行已经用 `eq` 收窄到本工具与成功），
//! 便签正文取调用参数里的 `text`，而不是工具回执的措辞。宿主不认识 `notes_add`，
//! 也不认识这一行——它只按行名找实现、按实现声明的写入对账。

use serde_json::{json, Value};
use symbio_core::{Entity, Verb};
use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;
use symbio_plugin_api::rows::{ModelAsk, ModelReply, RowHandler, RowRequest, RowStep};
use symbio_plugin_api::FactDraft;

use super::decl::{NOTES_INDEX, NOTES_INDEXED, NOTES_INSIGHT, NOTES_REFLECT};

pub(crate) struct NoteIndex;

impl RowHandler for NoteIndex {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn row(&self) -> &'static str {
        NOTES_INDEX
    }

    fn writes(&self) -> &'static [&'static str] {
        &[NOTES_INDEXED]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let Some(call_id) = req.key_of("tool_call_id") else {
            return RowStep::Facts(Vec::new());
        };
        // 行开账时已经证明这条产物在本轮切片里；这里按同一身份取回原文。
        let Some(artifact) = req.facts.iter().rev().find(|e| {
            e.kind == EVENT_ARTIFACT_ADDED
                && e.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
        }) else {
            return RowStep::Facts(Vec::new());
        };
        let text = note_text(&artifact.payload);
        RowStep::Facts(vec![FactDraft {
            key: format!("ni-{}-{call_id}", req.anchor()),
            kind: NOTES_INDEXED.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: json!({ "tool_call_id": call_id, "text": text }),
            produced_by: Some(req.user_seq),
        }])
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(NoteIndex);

/// `notes.reflect` 的实现：读 `notes.indexed`，交一条提示，回执落 `notes.insight`。
///
/// 提示由本能力自己造——宿主只做调用；回执是纯值，失败也照样落一条
/// `status = failed` 的洞察（D-242：推进优先于猜测）。
pub(crate) struct NoteReflect;

impl RowHandler for NoteReflect {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn row(&self) -> &'static str {
        NOTES_REFLECT
    }

    fn writes(&self) -> &'static [&'static str] {
        &[NOTES_INSIGHT]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let Some(call_id) = req.key_of("tool_call_id") else {
            return RowStep::Facts(Vec::new());
        };
        let Some(indexed) = req.facts.iter().rev().find(|e| {
            e.kind == NOTES_INDEXED
                && e.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
        }) else {
            return RowStep::Facts(Vec::new());
        };
        let text = indexed
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("");
        RowStep::Ask(ModelAsk {
            prompt: format!(
                "这是一条已记下的便签。请反思它，用一句话提醒我接下来该做什么：\n{text}"
            ),
        })
    }

    fn parse(&self, req: &RowRequest<'_>, reply: ModelReply<'_>) -> Vec<FactDraft> {
        let Some(call_id) = req.key_of("tool_call_id") else {
            return Vec::new();
        };
        let (status, text) = match reply {
            ModelReply::Text(text) => ("ok", text),
            ModelReply::Failed(_) => ("failed", ""),
        };
        vec![FactDraft {
            key: format!("nin-{}-{call_id}", req.anchor()),
            kind: NOTES_INSIGHT.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: json!({ "tool_call_id": call_id, "text": text, "status": status }),
            produced_by: Some(req.user_seq),
        }]
    }
}

// 链接期提交：与索引行同一条入口。
symbio_plugin_api::submit_row!(NoteReflect);

/// `artifact.added.args` 里的 `text`。
///
/// 载荷把定稿参数记成**字符串**（工具调用节点的正文，`DispatchOutcome::args`），
/// 不是 JSON 对象；两种形态都认，是这一格唯一需要解释的地方。解析不出来时返回空串：
/// 行已经用 `eq` 收窄到本工具成功，空串仍会落格并关账（推进优先于猜测）。
fn note_text(payload: &Value) -> String {
    let Some(args) = payload.get("args") else {
        return String::new();
    };
    match args {
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_default(),
        other => other
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}
