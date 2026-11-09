//! 终答（`chat.assistant.final`）：会话的债行 + 落格行。
//!
//! ## 为什么终答也是会话自己的一笔债
//!
//! `classify.answered` 只说「这句话能直接答」，不产出话术；话术在 compose。
//! 终答过去在 `project_final_turn`（**驱动空闲之后**）才问 compose / 读 assist 回执，
//! 于是 model 必须读 `compose_enabled` 才知道「本轮有没有人替我说话」——一个模型
//! 插件凭什么知道兄弟插件的开关？现在终答做成会话自己申报的一笔债：
//!
//! ```text
//! session.answer（plugin_row，债 id 恒为 `session.answer:id=<user.message id>`）
//!   ├─ when       `classify.answered:<user.message id>`（判决说能答才欠）
//!   ├─ unless     `chat.assistant.final:<user.message id>`（答过就不再欠）
//!   ├─ after_port `model.assist`：生成路径先等 compose 的 assist 债收束
//!   └─ 执行体：模板理由 → route `compose/wording`；生成理由 → 读 assist 回执
//! ```
//!
//! 债在 `plugin_row` 端口上认领：模型行的 `after_port(plugin_row)` 因此在本轮有人
//! 答话时自动等它，`unless_id(final)` 在终答落格后放行。模型不再需要
//! `compose_enabled`，也不需要认识本行；compose 缺席 / 关闭时本行不申报——
//! 「缺席即收束」，模型直接接管。
//!
//! ## 可观察差异：终答从「驱动空闲后」移到「驱动内」
//!
//! 终答在驱动内落格，`session.close-turn`（编码 / 巩固）在同一轮里接着还；收轮
//! 那一侧只投影、不再补写终答。中止 / 失败的收束格仍留在 `project_final_turn`：
//! 它们读的是**环的终态事实**（`aborted` / `failed`），只有驱动停下之后才存在。
//!
//! 转写仍只有会话写（D-020）：模板文本来自 compose，落格动作用本插件的行做。
//! 本文件不出现任何话术模板。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use symbio_kernel::event::{ClosureReason, Entity, Event, Verb};
use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep, PLUGIN_ROW};
use symbio_plugin_api::{
    FactDraft, PluginInvokeRequest, RunSnapshot, Verdict, WordingReply, EVENT_RESPONSE,
};
use symbio_sdk::authz::PRINCIPAL_MAIN;
use symbio_sdk::chat::chat_node as cm;

use crate::declarations::OWNER;
use crate::wording::compose_wording;

/// 终答行的行名 / 债名前缀。本行与判决行、首响行同在 `plugin_row` 端口；模型行
/// 只等端口（`after_port(plugin_row)`），不认这个行名。
pub(crate) const SESSION_ANSWER: &str = "session.answer";

/// 一次性问答端口名（数据）：名字住 `plugin-api` 的词汇面（D-364），提供者与
/// 各消费方 import 同一常量。
pub(crate) use symbio_plugin_api::vocab::PORT_MODEL_ASSIST as MODEL_ASSIST_PORT;

/// 上游判决事实（classify 插件所有）：名字由 owner 的清单生成（D-364）。
use symbio_plugin_api::events::EVENT_CLASSIFY_ANSWERED as CLASSIFY_ANSWERED;

/// 终答的落格行。
pub(crate) struct AnswerRow;

impl RowHandler for AnswerRow {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn row(&self) -> &'static str {
        SESSION_ANSWER
    }

    fn writes(&self) -> &'static [&'static str] {
        &[EVENT_CHAT_ASSISTANT_FINAL]
    }

    fn run(&self, _req: &RowRequest<'_>) -> RowStep {
        // 本行要经 route 取措辞（或读 assist 回执），同步入口只满足 trait 的契约位；
        // 派发永远走 `run_ctx`。
        RowStep::Facts(Vec::new())
    }

    fn run_ctx<'a>(
        &'a self,
        req: &'a RowRequest<'_>,
        ctx: Option<Arc<dyn PluginInvokeRequest>>,
    ) -> Pin<Box<dyn Future<Output = RowStep> + Send + 'a>> {
        Box::pin(async move { answer_step(req, ctx).await })
    }
}

/// 拿文本 → 落终答事实；拿不到就**不落**（交给模型的正常往返或既有降级分支，
/// 绝不自己编一句）。
async fn answer_step(req: &RowRequest<'_>, ctx: Option<Arc<dyn PluginInvokeRequest>>) -> RowStep {
    let Some(user_id) = req.key_of("id") else {
        return RowStep::Facts(Vec::new());
    };
    // 回答原因来自判决事实（`classify.answered` 的载荷），本行不另造理由。
    let Some(reason) = req
        .facts
        .iter()
        .rev()
        .find(|e| {
            e.kind == CLASSIFY_ANSWERED
                && e.payload.get("id").and_then(Value::as_str) == Some(user_id)
        })
        .and_then(|e| e.payload.get("reason").and_then(Value::as_str))
        .map(str::to_string)
    else {
        return RowStep::Facts(Vec::new());
    };
    let verdict = Verdict::Answered {
        reason: reason.clone(),
    };
    let reply = if verdict.needs_wording_generation() {
        // 生成路径：回执已经在事实面上（行债的派发与三格都归环），只读不写。
        let Some(reply) = assist_reply(req.facts, user_id, req.after_port) else {
            return RowStep::Facts(Vec::new());
        };
        reply
    } else {
        // 模板路径：文本由 compose 的模板表给出（本插件不产出话术）。
        let Some(ctx) = ctx else {
            return RowStep::Facts(Vec::new());
        };
        let snapshot = RunSnapshot::default();
        let Some(reply) = compose_wording(&ctx, req.facts, verdict, snapshot).await else {
            return RowStep::Facts(Vec::new());
        };
        reply
    };
    let text = reply.text;
    if text.trim().is_empty() {
        // 契约里的平凡值：空串 = 没有对话面文本 ⇒ 不写节点（不另造一句顶上）。
        return RowStep::Facts(Vec::new());
    }
    RowStep::Facts(vec![FactDraft {
        key: format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}"),
        kind: EVENT_CHAT_ASSISTANT_FINAL.to_string(),
        entity: Entity::Scope,
        verb: Verb::Closed,
        payload: json!({
            "root_id": user_id,
            "text": text,
            "principal": PRINCIPAL_MAIN,
            "cost_ms": reply.cost_ms,
            // 终结格必须标明这一轮为什么停（闭集取值，`closure_reason_closed`）；
            // 对话面自己的 `dialog.reason` 是另一回事（为什么这样措辞）。
            "reason": ClosureReason::Completed.as_str(),
            "reasoning": reply.reasoning,
            "reasoning_id": reply.reasoning_id,
            "dialog": {
                "id": format!("reply:{user_id}"),
                "reason": reason,
                "text": text,
                "exclude_from_context": false,
            },
        }),
        produced_by: Some(req.user_seq),
    }])
}

/// 本轮 `model.assist` 的回执：按**端口 + 开局事实身份**双认（与 compact 的
/// `summary_of` 同一读法），取最后一条。
///
/// 只读事实面上的 `response`；没有回执 = 这一次生成失败 / 被拒 ⇒ 本行不落终答，
/// 由既有降级分支接管，**不在这里造一句**。
fn assist_reply(events: &[Event], root_id: &str, port: Option<&str>) -> Option<WordingReply> {
    let port = port?;
    let fact = events.iter().rev().find(|e| {
        e.kind == EVENT_RESPONSE
            && e.payload.get("port").and_then(Value::as_str) == Some(port)
            && e.payload.get("root_id").and_then(Value::as_str) == Some(root_id)
    })?;
    let reply: cm::AssistReply = serde_json::from_value(fact.payload.clone()).ok()?;
    Some(WordingReply {
        text: reply.text,
        reasoning: reply.reasoning,
        reasoning_id: reply.reasoning_id,
        cost_ms: reply.cost_ms,
    })
}

/// 终答的**运行时债表**（无状态：每轮从事实视图现算）。
///
/// 开关读**当轮** `session.config` 的 `compose_enabled`（有效值 = 调用方开关 ∧
/// compose 债表在场）：关掉 / 未挂载时本行不申报，模型行按端口干净接管本轮。
/// 生成路径额外等 `model.assist` 端口：compose 的 assist 债收束后本行才去读回执。
pub(crate) struct AnswerDebts;

impl DebtSource for AnswerDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let enabled = facts
            .fact_payload(EVENT_SESSION_CONFIG, &session_config_id(user_id))
            .and_then(|c| c.get("compose_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !enabled {
            return Vec::new();
        }
        let answered_id = format!("{CLASSIFY_ANSWERED}:{user_id}");
        if !facts
            .facts_of(CLASSIFY_ANSWERED)
            .iter()
            .any(|(id, _)| *id == answered_id)
        {
            return Vec::new();
        }
        vec![DebtSpec::new(
            SESSION_ANSWER,
            PLUGIN_ROW,
            format!("{SESSION_ANSWER}:id={user_id}"),
            json!({
                "row": SESSION_ANSWER,
                "key": [["id", user_id]],
                "after_port": MODEL_ASSIST_PORT,
            }),
        )
        .when_id(CLASSIFY_ANSWERED, answered_id)
        .unless_id(
            EVENT_CHAT_ASSISTANT_FINAL,
            format!("{EVENT_CHAT_ASSISTANT_FINAL}:{user_id}"),
        )
        .after_port(MODEL_ASSIST_PORT)
        .lease(60_000, 1)]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(AnswerRow);
