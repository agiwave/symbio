//! 首响（`chat.assistant.opening`）：会话的债行 + 落格行。
//!
//! ## 为什么是会话自己的债行
//!
//! 首响发生在 `classify.escalate` 之后、模型派发之前（`docs/003-design/022`）。环里
//! **没有**那个「派发前窗口」可以钩：计划一次算全、按批派发，任何执行体都不许在发送
//! 前再查一次事实（D-342）。所以首响做成一条**独立的事实与一笔独立的债**：
//!
//! ```text
//! session.opening（plugin_row，债 id 恒为 `session.opening:id=<user.message id>`）
//!   ├─ when  `classify.escalate:<user.message id>`（只有升级路径才欠首响）
//!   ├─ unless `chat.assistant.opening:<user.message id>`（已说过就不再欠）
//!   └─ 执行体经 route 调 `compose/wording` 取措辞 → 落 `chat.assistant.opening`
//! ```
//!
//! 模型行随后按 `after(session.opening:id=…)` 等这笔债收束——顺序由债表排出来，
//! 不是执行体临时决定（`model/debt.rs`）。
//!
//! ## 行为什么能拿 ctx（D-344）
//!
//! 行拿到的是**派发这次调用的那份 `exec.ctx`**（同一份，不另造句柄、不加别的字段）。
//! 它唯一的用途是：`fork` 子上下文、设 `PATH` 与载荷，经 `ctx.parent()` 的插件路由
//! 去问 compose。**跨插件调用只走 route，不直连插件**；谁想用 `ctx` 绕过路由去摸
//! 别处的数据，按 D-344 驳回。
//!
//! 转写仍只有会话写（D-020）：措辞文本来自 compose（模板），落格动作用本插件的行做。
//! 本文件不出现任何话术模板。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};

use symbio_plugin_api::debt::{session_config_id, DebtFacts, DebtSource, DebtSpec, DriveRequest};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_OPENING, EVENT_SESSION_CONFIG, EVENT_USER_MESSAGE,
};
use symbio_plugin_api::routes::ROUTE_COMPOSE_WORDING;
use symbio_plugin_api::rows::{RowHandler, RowRequest, RowStep, PLUGIN_ROW};
use symbio_plugin_api::{
    ComposeRequest, Entity, FactDraft, PluginInvokeRequest, PluginInvokeRequestExt, RunSnapshot,
    Verb, Verdict, WordingReply, PATH, REASON_UNCLASSIFIED,
};
use symbio_sdk::keys::SESSION_ID;

use crate::convergence::dialog_context;
use crate::declarations::OWNER;

/// 首响行的行名 / 债名前缀。本行与判决行同在 `plugin_row` 端口；模型行只等端口
/// （`after_port(plugin_row)`），不认这个行名。
pub(crate) const SESSION_OPENING: &str = "session.opening";

/// 上游判决事实（classify 插件所有）：会话只按 id 等它，不解释载荷。
const CLASSIFY_ESCALATE: &str = "classify.escalate";

/// 首响的落格行。
pub(crate) struct OpeningRow;

impl RowHandler for OpeningRow {
    fn owner(&self) -> &'static str {
        OWNER
    }

    fn row(&self) -> &'static str {
        SESSION_OPENING
    }

    fn writes(&self) -> &'static [&'static str] {
        &[EVENT_CHAT_ASSISTANT_OPENING]
    }

    fn run(&self, _req: &RowRequest<'_>) -> RowStep {
        // 本行要经 route 取措辞，同步入口只满足 trait 的契约位；派发永远走 `run_ctx`。
        RowStep::Facts(Vec::new())
    }

    fn run_ctx<'a>(
        &'a self,
        req: &'a RowRequest<'_>,
        ctx: Option<Arc<dyn PluginInvokeRequest>>,
    ) -> Pin<Box<dyn Future<Output = RowStep> + Send + 'a>> {
        Box::pin(async move { opening_step(req, ctx).await })
    }
}

/// 取措辞 → 落首响事实；取不到就**不落**（收敛为「不说话」，绝不自己编一句）。
async fn opening_step(req: &RowRequest<'_>, ctx: Option<Arc<dyn PluginInvokeRequest>>) -> RowStep {
    let Some(user_id) = req.key_of("id") else {
        return RowStep::Facts(Vec::new());
    };
    // 升级原因来自判决事实（`classify.escalate` 的载荷），本行不另造理由。
    let reason = req
        .facts
        .iter()
        .rev()
        .find(|e| {
            e.kind == CLASSIFY_ESCALATE
                && e.payload.get("id").and_then(Value::as_str) == Some(user_id)
        })
        .and_then(|e| e.payload.get("reason").and_then(Value::as_str))
        .unwrap_or(REASON_UNCLASSIFIED)
        .to_string();
    let Some(ctx) = ctx else {
        return RowStep::Facts(Vec::new());
    };
    let Some(session_id) = ctx.get(SESSION_ID) else {
        return RowStep::Facts(Vec::new());
    };
    let compose = ComposeRequest {
        session_id: session_id.clone(),
        verdict: Verdict::Escalate {
            reason: reason.clone(),
        },
        context: dialog_context(req.facts),
        snapshot: RunSnapshot::default(),
    };
    // 跨插件只走 route：子上下文是同一份 ctx 的 fork，父句柄也取自这份 ctx。
    let child = ctx.fork();
    child.set(PATH, ROUTE_COMPOSE_WORDING.to_string());
    if child.set_payload(compose).is_err() {
        return RowStep::Facts(Vec::new());
    }
    let Some(parent) = ctx.parent().and_then(|parent| parent.upgrade()) else {
        return RowStep::Facts(Vec::new());
    };
    let reply = match parent.route(child).await {
        Ok(response) => response.get::<WordingReply>().ok(),
        Err(e) => {
            symbio_plugin_api::plugin_warn!(
                "session",
                "compose/wording 不可用（{session_id}）：{e}"
            );
            None
        }
    };
    let Some(reply) = reply else {
        return RowStep::Facts(Vec::new());
    };
    let text = reply.text;
    if text.trim().is_empty() {
        return RowStep::Facts(Vec::new());
    }
    RowStep::Facts(vec![FactDraft {
        key: format!("{EVENT_CHAT_ASSISTANT_OPENING}:{user_id}"),
        kind: EVENT_CHAT_ASSISTANT_OPENING.to_string(),
        entity: Entity::Scope,
        verb: Verb::Asserted,
        payload: json!({
            // 节点身份与用户消息区分（用户节点 id 就是 `user.message` 的事件 id）。
            "id": format!("opening:{user_id}"),
            "reason": reason,
            "text": text,
            "exclude_from_context": true,
        }),
        produced_by: Some(req.user_seq),
    }])
}

/// 首响的**运行时债表**（无状态：每轮从事实视图现算）。
///
/// 装载期只登记「这一格归会话看护」（`declarations.rs` 的归宿行）；这里在每轮
/// `rows()` 里申报哪一轮欠首响。开关读**当轮** `session.config` 的 `opening_enabled`
/// （与 `ClassifyDebts` 同一口径：上一轮的开关不算这一轮的账）。
pub(crate) struct OpeningDebts;

impl DebtSource for OpeningDebts {
    fn rows(&self, _request: &DriveRequest, facts: &DebtFacts) -> Vec<DebtSpec> {
        let Some(user_id) = facts.latest_fact_id(EVENT_USER_MESSAGE) else {
            return Vec::new();
        };
        let enabled = facts
            .fact_payload(EVENT_SESSION_CONFIG, &session_config_id(user_id))
            .and_then(|c| c.get("opening_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !enabled {
            return Vec::new();
        }
        let escalate_id = format!("{CLASSIFY_ESCALATE}:{user_id}");
        let opening_id = format!("{EVENT_CHAT_ASSISTANT_OPENING}:{user_id}");
        vec![DebtSpec::new(
            SESSION_OPENING,
            PLUGIN_ROW,
            format!("{SESSION_OPENING}:id={user_id}"),
            json!({ "row": SESSION_OPENING, "key": [["id", user_id]] }),
        )
        .when_id(CLASSIFY_ESCALATE, escalate_id)
        .unless_id(EVENT_CHAT_ASSISTANT_OPENING, opening_id)
        .lease(60_000, 1)]
    }
}

// 链接期提交：本实现随本 crate 进入二进制，宿主按行名选它。
symbio_plugin_api::submit_row!(OpeningRow);
