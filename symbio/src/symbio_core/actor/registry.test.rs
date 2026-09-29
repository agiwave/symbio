//! `actor` 域单元测试：规格形态 / 覆盖语义 / 越层登记（A5）。

use super::super::spec::{ActorPattern, ActorScope};
use super::super::ActorSpec;
use super::*;

/// 每个用例前取全局闸并清表——`inventory` 表是进程级的，用例之间会串味。
///
/// 返回的 guard 必须**活到用例结束**（`let _g = setup();`），否则锁提前释放。
fn setup() -> std::sync::MutexGuard<'static, ()> {
    crate::symbio_core::actor_test_guard()
}

#[test]
fn builtin_row_is_current_behavior() {
    let _g = setup();
    let s = ActorSpec::builtin("session.reasoner");
    assert_eq!(s.pattern, ActorPattern::Reasoner);
    assert_eq!(s.budget_ms, 0);
    assert!(s.is_unbudgeted());
    assert!(s.scope.is_root());
    assert_eq!(s.principal, "", "内置行以占位符表达'运行期才知道'");
}

#[test]
fn resolve_fills_principal_and_preserves_rest() {
    let s = ActorSpec::builtin("session.reasoner");
    let r = s.resolve("sess-42");
    assert_eq!(r.principal, "sess-42");
    // 其余字段逐字段不变——resolve 只动 principal，这是它能安全用于运行期的前提
    assert_eq!(r.pattern, s.pattern);
    assert_eq!(r.budget_ms, s.budget_ms);
    assert_eq!(r.scope, s.scope);
    assert_eq!(r.name, s.name);
}

#[test]
fn register_then_get_roundtrips() {
    let _g = setup();
    let layer = ActorScope::Root;
    let mut spec = ActorSpec::builtin("session.reasoner");
    spec.principal = "".to_string();
    assert!(actor_register(&layer, spec.clone()).is_ok());

    let got = actor_get("session.reasoner").expect("刚登记的行必须能取到");
    assert_eq!(got, spec);
}

#[test]
fn same_name_overwrites() {
    let _g = setup();
    let layer = ActorScope::Root;
    let a = ActorSpec::builtin("dup");
    actor_register(&layer, a).unwrap();

    let mut b = ActorSpec::builtin("dup");
    b.pattern = ActorPattern::Verifier;
    b.budget_ms = 500;
    actor_register(&layer, b.clone()).unwrap();

    // 同 name 后者覆盖前者（运行期按真实 principal 覆盖登记行靠的就是这条）
    assert_eq!(actor_get("dup").unwrap(), b);
    assert_eq!(
        actor_list().iter().filter(|s| s.name == "dup").count(),
        1,
        "覆盖而非追加"
    );
}

#[test]
fn list_is_sorted_by_name() {
    let _g = setup();
    let layer = ActorScope::Root;
    for name in ["zeta", "alpha", "mu"] {
        actor_register(&layer, ActorSpec::builtin(name)).unwrap();
    }
    let names: Vec<String> = actor_list().into_iter().map(|s| s.name).collect();
    assert_eq!(names, vec!["alpha", "mu", "zeta"]);
}

#[test]
fn empty_table_lists_nothing_and_get_returns_none() {
    let _g = setup();
    assert!(actor_list().is_empty());
    assert!(actor_get("nothing.here").is_none());
    // 表为空**不是错误**——执行器回落到内置默认（J2）
}

#[test]
fn root_layer_cannot_register_sub_agent_row() {
    let _g = setup();
    let mut spec = ActorSpec::builtin("child.decider");
    spec.scope = ActorScope::SubAgent("child-1".into());

    let err = actor_register(&ActorScope::Root, spec).unwrap_err();
    assert!(matches!(err, ActorError::ScopeMismatch { .. }));
    // 越层行**没进表**——不是"进了表再被审计出来"
    assert!(actor_list().is_empty());
}

#[test]
fn sub_agent_layer_cannot_register_root_row() {
    let _g = setup();
    let spec = ActorSpec::builtin("root.reasoner"); // scope 默认 Root

    let err = actor_register(&ActorScope::SubAgent("child-1".into()), spec).unwrap_err();
    assert!(matches!(err, ActorError::ScopeMismatch { .. }));
    assert!(actor_list().is_empty());
}

#[test]
fn sub_agent_layer_cannot_register_other_agent_row() {
    let _g = setup();
    let mut spec = ActorSpec::builtin("child.decider");
    spec.scope = ActorScope::SubAgent("child-2".into());

    // 自己的行、但 id 是别人的 → 同样越层
    let err = actor_register(&ActorScope::SubAgent("child-1".into()), spec).unwrap_err();
    assert!(matches!(err, ActorError::ScopeMismatch { .. }));
    assert!(actor_list().is_empty());
}

#[test]
fn sub_agent_layer_can_register_own_row() {
    let _g = setup();
    let mut spec = ActorSpec::builtin("child.decider");
    spec.scope = ActorScope::SubAgent("child-1".into());

    let layer = ActorScope::SubAgent("child-1".into());
    actor_register(&layer, spec).unwrap();
    assert_eq!(actor_list().len(), 1);
}

#[test]
fn pattern_wire_matches_serde() {
    for p in ActorPattern::ALL {
        assert_eq!(
            serde_json::to_value(p).unwrap(),
            serde_json::Value::String(p.wire().to_string()),
            "{p:?} 的 wire() 与 serde 必须一致"
        );
    }
}

#[test]
fn scope_wire_is_flat_and_stable() {
    assert_eq!(ActorScope::Root.wire(), "root");
    assert_eq!(
        ActorScope::SubAgent("child-1".into()).wire(),
        "sub_agent:child-1"
    );
}

#[test]
fn to_wire_carries_all_four_fields_plus_name() {
    let mut s = ActorSpec::builtin("session.decider");
    s.pattern = ActorPattern::Decider;
    s.budget_ms = 80;
    let w = s.to_wire();

    assert_eq!(w["name"], "session.decider");
    assert_eq!(w["principal"], "");
    assert_eq!(w["pattern"], "decider");
    assert_eq!(w["budget_ms"], 80);
    assert_eq!(w["scope"], "root");
}

#[test]
fn spec_serde_roundtrips() {
    let mut s = ActorSpec::builtin("child.verifier");
    s.scope = ActorScope::SubAgent("child-1".into());
    s.budget_ms = 1200;

    let json = serde_json::to_string(&s).unwrap();
    let back: ActorSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(back, s);
}

// ── ActorSource：层由装配方赋予，登记者改不了 ──────────────────────────────

struct RootRegistrar;
impl ActorSource for RootRegistrar {
    fn layer(&self) -> ActorScope {
        ActorScope::Root
    }
    fn specs(&self) -> Vec<ActorSpec> {
        vec![ActorSpec::builtin("session.reasoner")]
    }
}

/// 一个"想冒充 root"的子 Agent 登记者 —— 它的 `specs` 声明 Root（越层）。
struct ImpostorRegistrar;
impl ActorSource for ImpostorRegistrar {
    fn layer(&self) -> ActorScope {
        ActorScope::SubAgent("child-1".into())
    }
    fn specs(&self) -> Vec<ActorSpec> {
        vec![ActorSpec::builtin("fake.root")] // scope = Root，越层
    }
}

#[test]
fn actor_source_registers_all_rows() {
    let _g = setup();
    let n = RootRegistrar.register_all().unwrap();
    assert_eq!(n, 1);
    assert!(actor_get("session.reasoner").is_some());
}

#[test]
fn actor_source_cannot_self_declare_a_higher_layer() {
    let _g = setup();
    // 越层由**层**（装配方赋予）判定，而不是由行的自我声明决定
    let err = ImpostorRegistrar.register_all().unwrap_err();
    assert!(matches!(err, ActorError::ScopeMismatch { .. }));
    assert!(actor_list().is_empty());
}
