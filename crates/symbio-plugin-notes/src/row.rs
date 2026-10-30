//! 行执行体：本能力的三条轮内行。
//!
//! - `notes.index`：一条成功的 `notes_add` 产物 ⇒ 一条 `notes.indexed` 草稿。
//! - `notes.reflect`：一条 `notes.indexed` ⇒ **一次模型往返**（提示由本文件交），
//!   回执 ⇒ 一条 `notes.insight` 草稿（D-242）。
//! - `notes.summarize`：一条答成了的 `notes.insight` ⇒ 一句摘要 + 一条带区间的
//!   `message.added`——压缩形态的夹具（D-249）。
//!
//! 三条都是 `Decl.by = plugin_row`（DE-025 D-241）的实现。入口是纯值 [`RowRequest`]：
//! 债键（`tool_call_id`）与本轮的事实切片；出口是 [`RowStep`]——要么直接交
//! [`FactDraft`]，要么交一条提示让宿主代付往返。**不碰事实源**，落格仍由宿主的唯一
//! 写口做；往返用哪个供应商、花多少预算也由宿主管。
//!
//! 判断全部从事实里读：产物格按 `tool_call_id` 认（行已经用 `eq` 收窄到本工具与成功），
//! 便签正文取调用参数里的 `text`，而不是工具回执的措辞。宿主不认识 `notes_add`，
//! 也不认识这一行——它只按行名找实现、按实现声明的写入对账。

use serde_json::{json, Value};
use symbio_plugin_api::chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus};
use symbio_plugin_api::events::{EVENT_ARTIFACT_ADDED, EVENT_MESSAGE_ADDED};
use symbio_plugin_api::principal::Principal;
use symbio_plugin_api::rows::{ModelAsk, ModelReply, RowHandler, RowRequest, RowStep};
use symbio_plugin_api::FactDraft;
use symbio_plugin_api::{Entity, Verb};

use super::decl::{
    NOTES_INDEX, NOTES_INDEXED, NOTES_INSIGHT, NOTES_REFLECT, NOTES_SUMMARIZE, NOTES_SUMMARY,
};

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
            key: format!("ni-{}-{call_id}", req.anchor),
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
            key: format!("nin-{}-{call_id}", req.anchor),
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

/// 摘要正文的开场白：**只属于这一行**。端到端用它把「摘要真的进了请求包」与
/// 「记忆召回恰好也带了原文」分开——后者也会出现便签正文，但它不是本用例要验的那件事。
pub(crate) const SUMMARY_HEAD: &str = "便签摘要：";

/// `notes.summarize` 的实现：一条答成了的洞察 ⇒ 一句摘要 + 一条区间省略。
///
/// ## 两笔草稿，一笔债
///
/// - `notes.summary` 是**收口格**（行的 `until`）：它说「这一条便签压过了、压成什么、
///   压掉了哪一段」。没有它，这一行永远开着，环会在同一笔债上反复选中实现。
/// - `message.added` 是**正文**（带 `elision` 的诞生格，D-249）：载荷里的节点就是取代
///   那段区间的摘要，`elision` 指的区间（按事件 seq 闭区间）照进转写、只是不进请求包。
///   两者必须同一条事实出现——分两条落会让「摘要出现了、旧段没被省」成为可达状态。
///
/// ## 压哪一段
///
/// 本轮那条 `notes_add` 的**工具结果**（`artifact.added`）：端到端里它是这段历史中
/// 唯一由这次调用产生的正文。区间取该事件的 seq（`from = to`，单点）——它是一条
/// 事件，不是一段对话。找不到它（切片被裁过）⇒ 只落收口格、不写诞生格：推进优先，
/// 不猜一个不存在的区间。
pub(crate) struct NoteSummarize;

impl RowHandler for NoteSummarize {
    fn owner(&self) -> &'static str {
        "notes"
    }

    fn row(&self) -> &'static str {
        NOTES_SUMMARIZE
    }

    fn writes(&self) -> &'static [&'static str] {
        // 收口格 + 正文格。`message.added` 不属于本能力，但**写它的决定**是这一行做的
        // （与 `policy.compress` 同一口径：写方声明，宿主只落格）。
        &[NOTES_SUMMARY, EVENT_MESSAGE_ADDED]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let Some(call_id) = req.key_of("tool_call_id") else {
            return RowStep::Facts(Vec::new());
        };
        // 行开账时已经证明本轮有一条答成了的洞察；这里取回它的正文。
        let Some(insight) = req.facts.iter().rev().find(|e| {
            e.kind == NOTES_INSIGHT
                && e.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
                && e.payload.get("status").and_then(Value::as_str) == Some("ok")
        }) else {
            return RowStep::Facts(Vec::new());
        };
        let reflected = insight
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("");
        let summary = format!("{SUMMARY_HEAD}{reflected}");

        // 被取代的区间：本轮那条工具结果的 seq。`seq` 只在落库后才有——
        // 读不到就不写诞生格（不猜一个区间）。
        let range = req
            .facts
            .iter()
            .rev()
            .find(|e| {
                e.kind == EVENT_ARTIFACT_ADDED
                    && e.payload.get("tool_call_id").and_then(Value::as_str) == Some(call_id)
            })
            .and_then(|e| e.seq)
            .map(|s| s.value());

        let mut drafts = vec![FactDraft {
            key: format!("ns-{}-{call_id}", req.anchor),
            kind: NOTES_SUMMARY.to_string(),
            entity: Entity::Domain,
            verb: Verb::Asserted,
            payload: json!({
                "tool_call_id": call_id,
                "text": summary,
                "from": range,
                "to": range,
            }),
            produced_by: Some(req.user_seq),
        }];

        if let Some(seq) = range {
            // 摘要节点的身份是**稳定**的：同一条调用只会有一次摘要，重放不会多出一张卡。
            // 身份只写主体那一份（D-147）：`person/local` 折叠成 `role = user`。
            let node = ChatNode {
                id: format!("notes-summary-{call_id}"),
                principal: Some(Principal::from_legacy("user")),
                msg_type: Some(NodeKind::Body),
                status: Some(NodeStatus::Completed),
                content: Some(MessageContent::Text(summary.clone())),
                meta: Some(json!({ "notes_summary_of": call_id })),
                ..Default::default()
            };
            drafts.push(FactDraft {
                key: format!("nsum-{}-{call_id}", req.anchor),
                kind: EVENT_MESSAGE_ADDED.to_string(),
                entity: Entity::Message,
                verb: Verb::Opened,
                payload: json!({
                    "messages": [node],
                    // 这一格让 `message.added` 从「界面卡片」变成「取代一段区间的正文」
                    // （D-249）：转写读它、请求投影按它省掉 `[from, to]` 里的旧事件。
                    "elision": { "from": seq, "to": seq, "reason": NOTES_SUMMARIZE },
                }),
                produced_by: Some(req.user_seq),
            });
        }
        RowStep::Facts(drafts)
    }
}

// 链接期提交：三条行各一条入口，宿主按行名选。
symbio_plugin_api::submit_row!(NoteSummarize);

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
