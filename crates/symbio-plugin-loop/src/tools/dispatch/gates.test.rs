//! 拒止骨架与声明闸映射的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。开窗判据（时延 / 预算）已搬进
//! `symbio-plugin-tool` 的执行体，本文件只钉**映射不猜**：声明闸（能力包的 `before`）
//! 与 PreToolUse 钩子的出口经 [`Refusal`] 如实交回，主体派生与落笔同源。

use super::*;
use symbio_kernel::capability::failure_kind;
use symbio_kernel::plugin::{PluginInvokeRequestExt, PluginSimpleRequest};
use symbio_plugin_api::effects::BeforeRefusal;

/// 声明闸的默认出口是**可审计拒绝**：拒因进结果，违约承诺照记（淘汰即此类）。
#[test]
fn from_before_keeps_the_audited_default() {
    let judged = BeforeRefusal::new(
        "淘汰",
        "主体已收回".to_string(),
        failure_kind::PERMISSION_DENIED,
    );
    let refusal = Refusal::from_before(judged);
    assert_eq!(refusal.gate, "淘汰");
    assert_eq!(refusal.reason, "主体已收回");
    assert_eq!(refusal.prefix, "Refused");
    assert_eq!(refusal.kind, failure_kind::PERMISSION_DENIED);
    assert!(refusal.notes_delegation);
}

/// 实现显式去掉承诺记账 ⇒ 骨架不记（任务表：工具没跑，声明无从说起）。
#[test]
fn from_before_honours_the_implementations_trace() {
    let judged = BeforeRefusal::new(
        "任务表",
        "不持资格".to_string(),
        failure_kind::PERMISSION_DENIED,
    )
    .without_delegation_note();
    let refusal = Refusal::from_before(judged);
    assert!(!refusal.notes_delegation);
}

/// 主体派生（原 `EliminationInputs::of` 的那一条判据）：未绑智能体 ⇒ `session/main`；
/// 裸名归会话一族；带 namespace 的地址原样。
///
/// 声明闸与落笔读的是**同一个主体**——派生点因此留在环侧，而不是各闸自算。
#[test]
fn the_principal_comes_from_the_agent_id() {
    fn with_agent(agent_id: Option<&str>) -> std::sync::Arc<dyn PluginInvokeRequest> {
        let req = PluginSimpleRequest::new(None, None);
        if let Some(id) = agent_id {
            req.set(symbio_sdk::keys::AGENT_ID, id.to_string());
        }
        std::sync::Arc::new(req)
    }

    assert_eq!(
        batch_principal(&with_agent(None)),
        symbio_sdk::authz::PRINCIPAL_MAIN
    );
    assert_eq!(batch_principal(&with_agent(Some("ghost"))), "session/ghost");
    assert_eq!(
        batch_principal(&with_agent(Some("person/local"))),
        "person/local"
    );
}

/// PreToolUse 钩子共用同一套失败语义：`Blocked:` + `blocked` + 违约理由保留钩子原措辞。
#[test]
fn blocked_refusal_keeps_hook_wording_and_semantics() {
    let refusal = Refusal::blocked("策略不允许".to_string());
    assert_eq!(refusal.gate, "PreToolUse");
    assert_eq!(refusal.prefix, "Blocked");
    assert_eq!(refusal.kind, "blocked");
    assert!(refusal.notes_delegation);
    assert_eq!(
        refusal.note_why.as_deref(),
        Some("被 PreToolUse 钩子拦下：策略不允许")
    );
}
