//! 判决行的执行体：**规则可判 ⇒ 直接落判决；规则不可判 ⇒ 交一条提示等模型回执**。
//!
//! ## 两条路径共用一笔债、一个稳定的债 id
//!
//! 债的**身份**是 [`super::debt`] 给的 `classify.decide:id=<本轮 user.message id>`——
//! 两种路径用同一个 id（模型侧要按这个 id 等它收束，见 `model/debt.rs` 的 `after`）。
//! 路径差别只在**债键**：
//!
//! - 规则命中：键里带 `reason`（规则表的**确定性结论**，同一输入恒定），本行直接回一条
//!   [`CLASSIFY_ANSWERED`]；
//! - 规则未命中：键里只有 `id`，本行交 [`RowStep::Ask`]（`plugin.row.asked` 事实），
//!   往返由 [`super::debt`] 申报的 `model.assist` 债走端口；回执回来后再派发同一行，
//!   走 [`RowHandler::parse`] 落判决事实。
//!
//! 模型路径的 `reason` 在**解析时**才出现，所以它不能进债键：键一变，`ask_id`
//! 与「等哪一笔债」的身份就跟着变——身份必须只由「本轮哪句话」决定。
//!
//! ## `parse` 的兜底方向
//!
//! 回执不可解析 / 往返失败 ⇒ `Escalate{unclassified}`（= 卸载分类插件时的行为，
//! 全部进工具循环，功能完整）。**不返回 `Answered`**：一次分类故障若表现成
//! 「用户什么都收不到」，那是静默失败，没有信号（与 `decide.rs` 模块文档同一条理由）。

use serde_json::{json, Value};
use symbio_plugin_api::events::EVENT_USER_MESSAGE;
use symbio_plugin_api::rows::{ModelAsk, RowHandler, RowReply, RowRequest, RowStep};
use symbio_plugin_api::{Entity, FactDraft, Verb};
use symbio_plugin_api::{Verdict, REASON_UNCLASSIFIED};

use super::decide::parse_verdict;
use super::decl::{CLASSIFY_ANSWERED, CLASSIFY_DECIDE, CLASSIFY_ESCALATE};

pub(crate) struct ClassifyRow;

/// 一条判决草稿：`id` = 本轮用户发言的事实 id。
fn verdict_fact(kind: &str, id: &str, reason: &str, user_seq: Option<u64>) -> FactDraft {
    FactDraft {
        key: format!("{kind}:{id}"),
        kind: kind.to_string(),
        entity: Entity::Verdict,
        verb: Verb::Asserted,
        payload: json!({ "id": id, "reason": reason }),
        produced_by: user_seq,
    }
}

impl RowHandler for ClassifyRow {
    fn owner(&self) -> &'static str {
        "classify"
    }

    fn row(&self) -> &'static str {
        CLASSIFY_DECIDE
    }

    fn writes(&self) -> &'static [&'static str] {
        // 两种判决都可能由本行产生：规则路径只写 answered，模型路径按回执写其中一格。
        &[CLASSIFY_ANSWERED, CLASSIFY_ESCALATE]
    }

    fn run(&self, req: &RowRequest<'_>) -> RowStep {
        let Some(id) = req.key_of("id") else {
            return RowStep::Facts(Vec::new());
        };

        // 规则路径：债源已把**规则的确定性结论**放进键里，本行只需落格。
        if let Some(reason) = req.key_of("reason") {
            return RowStep::Facts(vec![verdict_fact(
                CLASSIFY_ANSWERED,
                id,
                reason,
                Some(req.user_seq),
            )]);
        }

        // 模型路径：提示只带本轮原话；往返的执行体（`model.assist`）从事实读面拼转写，
        // 因此提问正文不是本文件的事（本章只负责「问什么由本能力决定」）。
        let utterance = req
            .facts
            .iter()
            .rev()
            .find(|e| e.kind == EVENT_USER_MESSAGE)
            .and_then(|e| e.payload.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        RowStep::Ask(ModelAsk {
            prompt: utterance.to_string(),
        })
    }

    fn parse(&self, req: &RowRequest<'_>, reply: RowReply<'_>) -> Vec<FactDraft> {
        let Some(id) = req.key_of("id") else {
            return Vec::new();
        };
        let verdict = match reply {
            RowReply::Text(text) => parse_verdict(text),
            RowReply::Failed(_) => None,
        }
        .unwrap_or(Verdict::Escalate {
            reason: REASON_UNCLASSIFIED.to_string(),
        });
        let (kind, reason) = match verdict {
            Verdict::Answered { reason } => (CLASSIFY_ANSWERED, reason),
            Verdict::Escalate { reason } => (CLASSIFY_ESCALATE, reason),
            // `parse_verdict` 只产出上面两格；`Report` 不在本行的话表里，按兜底走。
            Verdict::Report => (CLASSIFY_ESCALATE, REASON_UNCLASSIFIED.to_string()),
        };
        vec![verdict_fact(kind, id, &reason, Some(req.user_seq))]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(ClassifyRow);
