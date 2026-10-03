//! 轮首判决的调用点 —— `session` ⇄ `classify` 的那条边。
//!
//! ## 它做三件事
//!
//! 1. 把**对话线**投影出来（[`conversation_view`]）——插件不读存储，读的是投影；
//! 2. 经容器 `route` 调 `classify/decide`（`ctx.fork()` + `PATH` 常量 + 契约载荷）；
//! 3. 把判决回读成 [`Verdict`]（**枚举**，不是文本——编排层要能执行它）。
//!
//! ## 为什么经容器 `route` 而不是直连插件实例
//!
//! 直连要求 session 按值持有 `classify`，那会绕过地址分发（`plugin-entry-audit` 的
//! E-007 正是拦这个），并在插件重建后钉住旧实例。`parent` 已经是容器句柄
//! （[`ChatOrchestrator::parent`]），本方案不需要新增任何持有关系。
//!
//! ## 「没有这个插件」是一个**正常状态**，不是错误
//!
//! `classify` 的卸载语义就是「全部输入直接进工具循环」（= 引入它之前的行为）。
//! 因此这里对路由失败的处理是**返回 `None` 并放行**，而不是报错：
//!
//! - `NotFound`（未挂载 / 已停用）：**静默**——这是预期的装配形态，
//!   每次判决都告警会变成噪声；
//! - 其它错误（契约不符 / 内部失败）：告警，但**仍然放行**——判决是增强，
//!   不是正确性的前提。把它升级成会话失败，等于让一个可选插件能拖垮主路径。
//!
//! 这两条合起来就是「可卸载」在调用侧的落地形态：**调用方按缺插件处理**。

use std::sync::Arc;

use crate::symbio_core::chat_message::ChatMessage;
use crate::symbio_core::{DecideRequest, Verdict};
use crate::symbio_core::{
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PATH, ROUTE_CLASSIFY_DECIDE,
    SESSION_ID,
};

use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::state::{ChatOrchestrator, SessionContext};

/// 轮首判决。
///
/// 返回 `None` = **没有判决**（没挂载 `classify` / 路由失败 / 响应不是合法契约）——
/// 调用方按「引入本插件之前的行为」继续，即全部输入进工具循环。
pub(crate) async fn decide_turn(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    utterance: &str,
) -> Option<Verdict> {
    let parent = orchestrator.parent.as_ref()?;
    let session_id = ctx.get(SESSION_ID).unwrap_or_default();

    let conversation: Vec<ChatMessage> =
        conversation_view(&context.messages, CONVERSATION_VIEW_LIMIT);

    let req = ctx.fork();
    req.set(PATH, ROUTE_CLASSIFY_DECIDE.to_string());
    req.set(SESSION_ID, session_id.clone());
    req.set_payload(DecideRequest {
        session_id,
        utterance: Some(utterance.to_string()),
        context: conversation,
    })
    .ok()?;

    match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<Verdict>() {
            Ok(verdict) => Some(verdict),
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Classify] 判决载荷不是合法契约，按「无判决」处理：{e}"
                );
                None
            }
        },
        Err(PluginError::NotFound(_)) => {
            // 未挂载 / 已停用：这正是「卸载平凡值」的形态，不是故障。
            crate::plugin_debug!("session", "[Classify] 未挂载判决插件，本轮直接进工具循环");
            None
        }
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Classify] 判决调用失败，本轮直接进工具循环：{e}"
            );
            None
        }
    }
}
