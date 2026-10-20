//! `chat_loop/state.rs` 的单元测试 —— 请求快照 / 主体规格派生。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//! `StopSignal` 的「恰好一次」契约随实现留守 session
//! （`session/orchestrator/stop.test.rs`）。

use super::*;
use crate::symbio_core::PluginSimpleRequest;

/// 配置标量快照（与 `SessionConfig::default()` 同值；`TurnSettings` 没有
/// `Default`——字段必须逐项给出，缺省值由会话配置决定）。
fn default_settings() -> crate::symbio_core::TurnSettings {
    super::super::super::test_support::default_settings()
}

// ── 请求级默认值快照（配置标量经 `TurnSettings` 交付）────────────────────

#[test]
fn request_defaults_come_from_turn_settings() {
    let defaults = default_settings();
    let r = TurnRequest::new(&chat_request::Request::default(), defaults.auto_compress);
    assert_eq!(r.auto_compress, defaults.auto_compress);
    assert!(r.load_history, "该字段无配置对应项：缺省即加载历史");
    assert_eq!(r.system_prompt, None);
}

#[test]
fn explicit_request_values_win_over_defaults() {
    let defaults = default_settings();
    let r = TurnRequest::new(
        &chat_request::Request {
            auto_compress: Some(!defaults.auto_compress),
            system_prompt: Some("p".into()),
            ..Default::default()
        },
        defaults.auto_compress,
    );
    assert_eq!(r.auto_compress, !defaults.auto_compress);
    assert_eq!(r.system_prompt.as_deref(), Some("p"));
}

#[test]
fn load_history_false_is_honored() {
    // 心跳等无上下文场景：`load_history = false` 必须透传，不能被默认值覆盖。
    let defaults = default_settings();
    let r = TurnRequest::new(
        &chat_request::Request {
            load_history: Some(false),
            ..Default::default()
        },
        defaults.auto_compress,
    );
    assert!(!r.load_history);
}

// ── `request_actor_spec` 的五字段真值（T-107 exit ③）─────────────────────────
//
// 会话选定哪个 agent ⇒ 规格就从授权表那一行取。三个角色主体（工人 / 规划者 / 验证者）
// 逐一钉：**能力集从登记行取**（单一真源，不是空表也不是硬编码清单）、**模式按矩阵
// 谓词派生**（工人无发言权而有产物能力 ⇒ `Translator`，与对等行同形；规划者 / 验证者
// 两者皆无 ⇒ `Decider`，与 S03 §3 的 ActorSpec 行 `pattern: decider` 一致）、**预算
// 按档位**、**作用域由委派链给**（子会话 ⇒ `Child(父主体)`）。
//
// 硬编码任何一项的后果都是「表授予 A、规格带 B」静默成立——那一行就成了没人读的死记录。

/// 带 `AGENT_ID` 的请求上下文（会话选定了哪个 agent）。
fn agent_ctx(agent_id: &str) -> std::sync::Arc<dyn crate::symbio_core::PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
    std::sync::Arc::new(req)
}

#[test]
fn the_worker_spec_takes_its_registered_row() {
    use crate::symbio_core::{LatencyTier, Pattern, Scope};
    let spec = request_actor_spec(agent_ctx("worker").as_ref(), LatencyTier::Deep, Scope::Root);
    assert_eq!(spec.principal, "session/worker");
    assert_eq!(
        spec.capabilities,
        vec!["produce.artifact".to_string()],
        "能力集从授权表登记行取（单一真源）"
    );
    assert_eq!(
        spec.pattern,
        Pattern::Translator,
        "无发言权而有 produce.artifact ⇒ Translator（对等行同形，S02 行与派生不冲突）"
    );
    assert_eq!(spec.budget_ms, LatencyTier::Deep.budget_ms());
    assert_eq!(spec.scope, Scope::Root);
}

#[test]
fn the_planner_and_verifier_specs_take_their_registered_rows() {
    use crate::symbio_core::{LatencyTier, Pattern, Scope};
    let planner = request_actor_spec(
        agent_ctx("planner").as_ref(),
        LatencyTier::Deep,
        Scope::Root,
    );
    assert_eq!(planner.principal, "session/planner");
    assert_eq!(
        planner.capabilities,
        vec!["define.work".to_string()],
        "规划者的权威动作是立任务（S03 §3 grants 行）"
    );
    assert_eq!(
        planner.pattern,
        Pattern::Decider,
        "define.work 独持 ⇒ Decider（S03 §3 ActorSpec 行 pattern: decider）"
    );

    let verifier = request_actor_spec(
        agent_ctx("verifier").as_ref(),
        LatencyTier::Fast,
        Scope::Root,
    );
    assert_eq!(verifier.principal, "session/verifier");
    assert_eq!(
        verifier.capabilities,
        vec!["assert.verification".to_string()],
        "验证者的权威动作是独立判定（S03 §3 grants 行）"
    );
    assert_eq!(verifier.pattern, Pattern::Decider);
    assert_eq!(verifier.budget_ms, LatencyTier::Fast.budget_ms());
}

/// **会跑模型的那个主体派生 `Reasoner`**——这是 [ADR-079] 的反证。
///
/// `pattern` 由授权谓词派生（持 `reply` ⇒ `Reasoner`），而**每一个会跑模型的轮次
/// 主体都持 `reply`**。所以凡是要判「跑不跑模型」的地方，读到的永远是 `Reasoner`：
/// 把它接成分派闸门，等于把技能快路（S11 §4 参数表的同名参数，落地形态是
/// `SessionConfig::skill_fast_path`）永久关死。两者的区别见 [ADR-079]。
///
/// 机器判据不在这里（单测拦不住调用点的闸门），在 `e2e/cases/t41-skill-fast-path.mjs`：
/// 把 pattern 接成分派点 ⇒ 快路永不命中 ⇒ 那条用例的「零模型调用」断言不成立。
///
/// [ADR-079]: ../../../../../docs/010-adr/079.md "ADR-079"
#[test]
fn the_session_actor_is_a_reasoner_which_is_why_pattern_cannot_dispatch() {
    use crate::symbio_core::{LatencyTier, Pattern, Scope};
    let spec = request_actor_spec(agent_ctx("main").as_ref(), LatencyTier::Deep, Scope::Root);
    assert_eq!(spec.principal, "session/main");
    assert_eq!(
        spec.pattern,
        Pattern::Reasoner,
        "持 reply.* ⇒ Reasoner：这是**分类**，不是「这一轮走不走模型」的开关"
    );
    assert!(
        spec.capabilities.iter().any(|c| *c == "reply"),
        "分类的判据就是这条能力：{}",
        spec.capabilities.join(",")
    );
}

#[test]
fn a_delegated_worker_spec_is_scoped_to_its_parent() {
    use crate::symbio_core::{LatencyTier, Scope};
    // 委派链上（agent_run 派生的子会话）：作用域挂父主体——「委派不放大权威」的
    // 规格侧形态（T-105）。工人那一行不变，变的是 scope。
    let spec = request_actor_spec(
        agent_ctx("worker").as_ref(),
        LatencyTier::Deep,
        Scope::Child("session/main".to_string()),
    );
    assert_eq!(spec.scope, Scope::Child("session/main".to_string()));
    assert_eq!(spec.capabilities, vec!["produce.artifact".to_string()]);
}
