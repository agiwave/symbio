//! 收束行（D-250 / D-252）：终答落格之后，把能力交出的收束贡献落成事实。
//!
//! 贡献表由编译期 inventory（`settle::contributors()`）给出，本行只做三件事：
//! 构造收束这一刻的 [`Settling`]（值）、逐条取出草稿、最后落一条 `session.settled`
//! 关自己的账。收束因此是环上的一笔债，而不是宿主里的一个旁路。
//!
//! **顺序**：贡献表已按 `(prio, owner)` 排好；行只调用一次、无法在贡献之间重取史
//! （`run` 返回后才落格），因此只承诺「本条依赖的输入是终答之前已落的事实」。

use serde_json::{json, Value};
use symbio_plugin_api::context::{RecallFacts, RecallLine};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_TOOL_REQUESTED, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep};
use symbio_plugin_api::settle::{contributors, SettleFace, Settling};
use symbio_plugin_api::{Entity, Verb};

use crate::declarations::{EVENT_SESSION_RECALL, EVENT_SESSION_SETTLED, OWNER, SESSION_CLOSE_TURN};

pub(crate) struct CloseTurn;

impl RowHandler for CloseTurn {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn row(&self) -> &'static str {
        SESSION_CLOSE_TURN
    }

    fn writes(&self) -> &'static [&'static str] {
        &[EVENT_SESSION_SETTLED]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let root_id = req.key_of("root_id").unwrap_or_default();
        // 本轮开口格：最后一条用户发言。收束的溯源锚与「哪一面」都从它往后数。
        let opening = req
            .history
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_USER_MESSAGE);
        let user_seq = opening
            .and_then(|e| e.seq.as_ref().map(|s| s.value()))
            .unwrap_or(req.user_seq);
        let user_text = opening
            .and_then(|e| e.payload.get("text").and_then(Value::as_str))
            .unwrap_or_default();
        let final_event = req.history.iter().rev().find(|e| {
            e.kind == EVENT_CHAT_ASSISTANT_FINAL
                && e.payload.get("root_id").and_then(Value::as_str) == Some(root_id)
        });
        let response = final_event
            .and_then(|e| e.payload.get("text").and_then(Value::as_str))
            .filter(|text| !text.trim().is_empty());
        // 生成面 = 本轮有工具交换；没有就是定稿答话面（贡献者据此判该不该落笔）。
        let generated = req.history.iter().any(|e| {
            e.seq
                .as_ref()
                .map(|s| s.value())
                .is_some_and(|seq| seq >= user_seq)
                && matches!(e.kind.as_str(), EVENT_TOOL_REQUESTED | EVENT_ARTIFACT_ADDED)
        });
        // 本轮开口时的召回读数（`session.recall`，写方是同轮的 `append_recall`）。
        // 收束贡献里有两类消费者：语义记忆记「本轮用过什么」，技能记账按真实命中
        // 写 `memory.calibrated`。**读的是本轮那一格**（`produced_by = 开口格 seq`），
        // 不是「最近一格」——收束与召回必须同属一轮，否则账目会记到别轮头上。
        let recall_reading = req
            .history
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_SESSION_RECALL && e.produced_by == Some(user_seq));
        let recall_lines: Vec<RecallLine<'_>> = recall_reading
            .and_then(|e| e.payload.get("entries").and_then(Value::as_array))
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        Some(RecallLine {
                            seq: entry.get("seq")?.as_u64()?,
                            content: entry.get("content")?.as_str()?,
                            tag: entry.get("tag")?.as_str()?,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let recall = recall_reading.map(|e| RecallFacts {
            degraded: e
                .payload
                .get("degraded")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            lines: &recall_lines,
        });
        let at = Settling {
            face: if generated {
                SettleFace::Generated
            } else {
                SettleFace::Answered
            },
            now_ms: req.now_ms,
            principal: req.principal,
            user_seq,
            anchor: req.anchor,
            user_text,
            response,
            recall: recall.as_ref(),
            history: req.history,
        };

        // 贡献者只交事实草稿：归属不在草稿里，也不在贡献者手里。`plugin_row` 执行体
        // 按**声明表**取 `Frozen::event_writer(kind)` 后走 `emit_owned`——贡献方无权
        // 指定归属（DE-026 D-252 / D-339）。
        let mut facts: Vec<FactDraft> = contributors()
            .iter()
            .flat_map(|contributor| contributor.contribute(&at))
            .map(|contribution| contribution.fact)
            .collect();
        facts.push(FactDraft {
            key: format!("{EVENT_SESSION_SETTLED}:{root_id}"),
            kind: EVENT_SESSION_SETTLED.to_string(),
            entity: Entity::Scope,
            verb: Verb::Progressed,
            payload: json!({ "root_id": root_id }),
            produced_by: Some(user_seq),
        });
        RowStep::Facts(facts)
    }
}

symbio_plugin_api::submit_row!(CloseTurn);
