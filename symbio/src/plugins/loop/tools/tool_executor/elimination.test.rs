//! 淘汰闸门判据（S13 §3，T-073）——loop 侧只钉「主体派生 + 结论值搬运」。
//!
//! 部署级扫描（跨会话、嵌套子会话、恢复可逆、按主体精确命中、空根）是
//! session 存储布局的一半，由 `session/<sid>/_run/elimination-revoked` 的动作
//! 用例钉住（`session/plugin/run_actions.test.rs`）。这里只答一个问题：
//! session 交来的结论值是否**原样**落进闸门输入，以及主体从 `AGENT_ID` 的派生。

use super::EliminationInputs;
use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest};

/// 带 `AGENT_ID` 的请求上下文：闸门按它派生主体（同 `BreakerInputs` 的取值口径）。
fn agent_ctx(agent_id: Option<&str>) -> std::sync::Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    if let Some(id) = agent_id {
        req.set(crate::symbio_core::AGENT_ID, id.to_string());
    }
    std::sync::Arc::new(req)
}

/// `true` = 已被收回：结论值原样落进闸门输入，不被本层重新解释。
#[test]
fn a_revoked_verdict_is_carried_verbatim() {
    let inputs = EliminationInputs::of(true, &agent_ctx(None));
    assert!(inputs.delegation_revoked);
}

/// `false` = 覆盖层里没有生效的撤销（包括「从来没有裁决」与「恢复已翻假」两种）：
/// 闸门只答减法，表是权威。
#[test]
fn an_unrevoked_verdict_is_carried_verbatim() {
    let inputs = EliminationInputs::of(false, &agent_ctx(None));
    assert!(!inputs.delegation_revoked);
}

/// 主体派生：未绑智能体 ⇒ `session/main`；裸名归会话一族；带 namespace 的地址原样。
#[test]
fn the_principal_comes_from_the_agent_id() {
    assert_eq!(
        EliminationInputs::of(false, &agent_ctx(None)).principal,
        crate::symbio_core::PRINCIPAL_MAIN
    );
    assert_eq!(
        EliminationInputs::of(false, &agent_ctx(Some("ghost"))).principal,
        "session/ghost"
    );
    assert_eq!(
        EliminationInputs::of(false, &agent_ctx(Some("person/local"))).principal,
        "person/local"
    );
}
