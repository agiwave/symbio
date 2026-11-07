//! 行为名收集面的判据：没有手写名单、一个只在测试里存在的能力也能走完整条链。

use super::*;
use crate::effects::{
    apply, expects_facts, EffectCall, EffectEnv, EffectInput, FactDraft, ToolBehavior,
};
use symbio_kernel::event::{Entity, Verb};

/// 一个**只存在于测试夹具**的能力名：不进仓库任何清单，也不被任何插件认识。
const FIXTURE_PROBE: &str = "fixture_probe";

/// 夹具能力：成功的调用 ⇒ 一条 `fixture.recorded` 草稿。
struct FixtureProbe;

impl ToolBehavior for FixtureProbe {
    fn owner(&self) -> &'static str {
        "tests/fixture"
    }

    fn behavior(&self) -> &'static str {
        FIXTURE_PROBE
    }

    fn kind(&self) -> Kind {
        Kind::LandsFacts
    }

    fn after(&self, call: &EffectCall<'_>) -> Vec<FactDraft> {
        vec![FactDraft {
            key: format!("fixture-{}", call.call_id),
            kind: "fixture.recorded".to_string(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({ "ok": call.success }),
            produced_by: Some(call.user_seq),
        }]
    }
}

crate::submit_behavior!(FixtureProbe);

/// 「新能力只改自己 crate」的可执行证明：夹具只提交实现，收集面立刻认识它。
#[test]
fn a_capability_declared_only_in_its_own_crate_is_collected() {
    assert!(is_known(FIXTURE_PROBE));
    assert!(
        known().contains(&FIXTURE_PROBE),
        "收集面没有这个夹具能力：{:?}",
        known()
    );
    assert_eq!(kind_of(FIXTURE_PROBE), Some(Kind::LandsFacts));
    assert_eq!(lands_facts(FIXTURE_PROBE), Some(true));
}

/// 被收集到的能力可以走完整条链：声明 → 落事实草稿 → 违约判据认可。
#[test]
fn the_collected_capability_lands_its_fact() {
    let input = EffectInput {
        tool: "fixture/tool".into(),
        call_id: "call-1".into(),
        args: serde_json::json!({}),
        success: true,
        result: "ok".into(),
        why: String::new(),
        declared: &[FIXTURE_PROBE],
    };
    let env = EffectEnv {
        anchor: "t1",
        user_seq: 9,
        principal: "session/a",
        source: "user",
        snapshot: &[],
        applied: &[],
    };

    let drafts = apply(&input.call(&env));
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].kind, "fixture.recorded");
    assert!(expects_facts(&[FIXTURE_PROBE]), "造格型必须被判据认可");
}

/// `known()` 是收集结果，不是手写清单：名字只来自已提交的实现。
#[test]
fn known_names_are_the_collected_handlers() {
    let names = known();
    for behavior in crate::effects::handlers() {
        assert!(
            names.contains(&behavior.behavior()),
            "{} 报的名字 {:?} 没进收集结果",
            behavior.owner(),
            behavior.behavior()
        );
    }
}

/// `is_known` 只认整词：前缀 / 大小写变体都不是已知行为。
///
/// 闸门按单个常量判，校验面按本函数判；两处若一处写宽，未知行为就会被当成
/// 「已登记」静默放行。
#[test]
fn is_known_matches_whole_words_only() {
    assert!(is_known(FIXTURE_PROBE));
    for raw in [
        "fixture",
        "fixture_probe_extra",
        "FIXTURE_PROBE",
        "",
        " fixture_probe",
    ] {
        assert!(!is_known(raw), "{raw:?} 不应被判为已知行为");
    }
}

/// 收集面里仍是 snake_case 线格式（名字随工具元数据跨进程走）。
#[test]
fn collected_names_are_snake_case() {
    for name in known() {
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{name:?} 不是 snake_case 线格式"
        );
    }
}

/// 判据：本模块不得再长出任何字符串常量（能力名只能由实现自报）。
///
/// 扫的是**源码**而不是运行时的收集结果：后者只能证明「此刻没长回来」，
/// 前者让下一次有人往这里加常量时在单测里立刻红。
#[test]
fn this_module_holds_no_capability_names() {
    let src = include_str!("behaviors.rs");
    for (index, line) in src.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with('*') || code.starts_with("/*") {
            continue;
        }
        assert!(
            !code.contains(": &str =") && !code.contains(": &[&str] ="),
            "behaviors.rs:{} 出现字符串常量（能力名必须由实现自报）：{line}",
            index + 1
        );
    }
}
