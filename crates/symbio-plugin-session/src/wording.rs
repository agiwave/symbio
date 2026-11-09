//! 措辞往返的公共折叠：终答 / 首响 / 汇报三份落格行都经 route 问
//! `compose/wording`，差异只有各自构造的 `Verdict` 与 `RunSnapshot`。

use std::sync::Arc;

use symbio_kernel::event::Event;
use symbio_plugin_api::routes::ROUTE_COMPOSE_WORDING;
use symbio_plugin_api::{
    ComposeRequest, PluginInvokeRequest, PluginInvokeRequestExt, RunSnapshot, Verdict,
    WordingReply, PATH,
};
use symbio_sdk::keys::SESSION_ID;

use crate::convergence::dialog_context;

/// 经 route 取一次措辞回执；取不到（session_id 缺席 / set_payload 失败 / route
/// 失败 / 回执解析失败或为 `None`）一律返回 `None`，由调用方收敛成「不落格」。
///
/// 返回完整的 [`WordingReply`]：终答还要读 `cost_ms` / `reasoning` /
/// `reasoning_id`。空串早退留在各行——终答的生成路径（读 assist 回执）不经过
/// 本函数，仍须按同一契约判空串。
pub(crate) async fn compose_wording(
    ctx: &Arc<dyn PluginInvokeRequest>,
    facts: &[Event],
    verdict: Verdict,
    snapshot: RunSnapshot,
) -> Option<WordingReply> {
    let Some(session_id) = ctx.get(SESSION_ID) else {
        return None;
    };
    let request = ComposeRequest {
        session_id: session_id.clone(),
        verdict,
        context: dialog_context(facts),
        snapshot,
    };
    // 跨插件只走 route：子上下文是同一份 ctx 的 fork，父句柄也取自这份 ctx。
    let child = ctx.fork();
    child.set(PATH, ROUTE_COMPOSE_WORDING.to_string());
    if child.set_payload(request).is_err() {
        return None;
    }
    let Some(parent) = ctx.parent().and_then(|parent| parent.upgrade()) else {
        return None;
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
        return None;
    };
    Some(reply)
}
