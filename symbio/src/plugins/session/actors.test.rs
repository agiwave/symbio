//! `session/actors.rs` 的单元测试：层判定 / 登记 / 解析回落。

use super::*;
use crate::symbio_core::{PluginSimpleRequest, PLUGIN_ID_SESSION};
use std::sync::Arc;

fn setup() -> std::sync::MutexGuard<'static, ()> {
    crate::symbio_core::actor_test_guard()
}

/// root 层上下文（无 AGENT_ID）。
fn root_ctx() -> Arc<PluginSimpleRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 子智能体层上下文（带 AGENT_ID）。
fn sub_ctx(agent_id: &str) -> Arc<PluginSimpleRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
    Arc::new(req)
}

#[test]
fn layer_is_root_without_agent_id() {
    assert_eq!(layer_of(&*root_ctx()), ActorScope::Root);
}

#[test]
fn layer_is_sub_agent_with_agent_id() {
    assert_eq!(
        layer_of(&*sub_ctx("child-1")),
        ActorScope::SubAgent("child-1".into())
    );
}

#[test]
fn blank_agent_id_is_treated_as_root() {
    // 空串与缺失同义——与 entry.rs 的 agent_id 判定一致（trim + 非空）
    assert_eq!(layer_of(&*sub_ctx("")), ActorScope::Root);
    assert_eq!(layer_of(&*sub_ctx("   ")), ActorScope::Root);
}

#[test]
fn register_records_resolved_principal_in_the_table() {
    let _g = setup();
    let ctx = root_ctx();
    register_reasoner(&*ctx, "sess-live");

    // 表里那一行就是**本轮在跑的 Actor**，不是登记期的空占位符 ——
    // 否则 `actor/list` 排障时看到的全是常量空串，等于看不出哪一路会话。
    let row = crate::symbio_core::actor_get(ACTOR_NAME_SESSION_REASONER).unwrap();
    assert_eq!(row.principal, "sess-live");
    // 其余字段与模板一致
    assert_eq!(row.pattern, ActorPattern::Reasoner);
    assert_eq!(row.budget_ms, REASONER_BUDGET_MS);
    assert_eq!(row.scope, ActorScope::Root);
}

#[test]
fn register_then_resolve_fills_principal() {
    let _g = setup();
    let ctx = root_ctx();
    register_reasoner(&*ctx, "s1");

    let actor = resolve_reasoner(&*ctx, "sess-7");
    assert_eq!(actor.name, ACTOR_NAME_SESSION_REASONER);
    assert_eq!(actor.pattern, ActorPattern::Reasoner);
    assert_eq!(actor.budget_ms, REASONER_BUDGET_MS);
    assert_eq!(actor.scope, ActorScope::Root);
    assert_eq!(
        actor.principal, "sess-7",
        "运行期必须用真实会话 id 覆盖占位符"
    );
}

#[test]
fn resolve_falls_back_to_builtin_when_table_empty() {
    let _g = setup();
    let ctx = root_ctx();
    // 不登记，直接解析——必须回落到内置默认（J2）
    let actor = resolve_reasoner(&*ctx, "sess-9");
    assert_eq!(actor.name, ACTOR_NAME_SESSION_REASONER);
    assert_eq!(actor.budget_ms, 0);
    assert_eq!(actor.principal, "sess-9");
    assert_eq!(actor.scope, ActorScope::Root);
}

/// 内置默认行与登记行的取值必须**逐字段相同** —— 这是"移除登记即退化"的前提。
#[test]
fn builtin_fallback_equals_registered_row() {
    let _g = setup();
    let ctx = root_ctx();

    register_reasoner(&*ctx, "s1");
    let registered = resolve_reasoner(&*ctx, "s1");

    crate::symbio_core::actor_clear();
    let fallback = resolve_reasoner(&*ctx, "s1");

    assert_eq!(
        registered, fallback,
        "登记行的取值必须与内置默认完全一致，否则'移除即退化'不成立"
    );
}

#[test]
fn sub_agent_layer_registers_sub_agent_row() {
    let _g = setup();
    let ctx = sub_ctx("child-1");
    register_reasoner(&*ctx, "s1");

    let row = crate::symbio_core::actor_get(ACTOR_NAME_SESSION_REASONER).unwrap();
    assert_eq!(row.scope, ActorScope::SubAgent("child-1".into()));

    let actor = resolve_reasoner(&*ctx, "child-sess");
    assert_eq!(actor.scope, ActorScope::SubAgent("child-1".into()));
}

/// 表里若有**别的层**登记的行，本层解析必须把 scope 校准回本层。
#[test]
fn resolve_calibrates_scope_to_current_layer() {
    let _g = setup();
    crate::symbio_core::actor_register(
        &ActorScope::SubAgent("other".into()),
        ActorSpec::new(
            ACTOR_NAME_SESSION_REASONER,
            "",
            ActorPattern::Reasoner,
            0,
            ActorScope::SubAgent("other".into()),
        ),
    )
    .unwrap();

    let ctx = sub_ctx("mine");
    let actor = resolve_reasoner(&*ctx, "s");
    assert_eq!(
        actor.scope,
        ActorScope::SubAgent("mine".into()),
        "执行器必须按本层运行，不信任表里可能属于别层的 scope"
    );
}

#[test]
fn reasoner_name_is_stable() {
    // 名字是表里的契约标识（`actor/list` 会列出它），不该悄悄改
    assert_eq!(ACTOR_NAME_SESSION_REASONER, "session.reasoner");
    assert_eq!(REASONER_BUDGET_MS, 0, "0 = 无墙钟上限，即现行语义");
}

#[test]
fn plugin_id_constant_is_the_session_id() {
    assert_eq!(PLUGIN_ID_SESSION, "session");
}
