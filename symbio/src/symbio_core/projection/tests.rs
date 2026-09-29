//! projection 域自测 —— 签名约束的**可执行判据**与双跑一致性。

use super::*;
use crate::symbio_core::{Fact, FactKind, FactPrincipal};

/// 造一条最小事实（只为测试折叠，不关心语义）。
fn fact(seq: u64, kind: FactKind) -> Fact {
    Fact {
        seq,
        kind,
        principal: FactPrincipal::new("s1"),
        caused_by: None,
        at_ms: seq as i64 * 10,
        payload: serde_json::Value::Null,
    }
}

/// 一个合法的投影：只读 input，无捕获（`fn` 指针）。
fn count_facts(input: &ProjectionInput<'_>) -> View<usize> {
    View::new(input.facts.len())
}

/// `Projection::new` 接受无捕获 `fn` 指针；`run` 得到预期值。
#[test]
fn projection_new_accepts_plain_fn() {
    let p = Projection::new(count_facts);
    let facts = vec![fact(1, FactKind::TurnUserMessage)];
    let v = p.run(&ProjectionInput::new(&facts, 0));
    assert_eq!(v.value, 1);
    assert!(!v.trivial);
}

/// **A4 双跑一致**：同一份输入双跑，逐字节相同（投影不读时钟）。
#[test]
fn projection_is_deterministic_across_runs() {
    let p = Projection::new(count_facts);
    let facts = vec![
        fact(1, FactKind::TurnUserMessage),
        fact(2, FactKind::TurnAssistantFinal),
    ];
    let input = ProjectionInput::new(&facts, 12_345);
    let a = p.run(&input);
    let b = p.run(&input);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "同一份输入双跑必须逐字节相同（A4）"
    );
}

/// `at_ms` 由调用方给定：**改变它不算"双跑不一致"的证据**，
/// 因为我们比的是"同一 input"下的两次运行。这里反向确认 input 不同则可能不同。
#[test]
fn different_input_may_differ() {
    fn uses_at_ms(input: &ProjectionInput<'_>) -> View<i64> {
        View::new(input.at_ms)
    }
    let p = Projection::new(uses_at_ms);
    let facts: Vec<Fact> = vec![];
    let a = p.run(&ProjectionInput::new(&facts, 1));
    let b = p.run(&ProjectionInput::new(&facts, 2));
    assert_ne!(a.value, b.value, "input 不同则结果可不同");
}

/// 平凡值可区分：`View::trivial` 折叠出来仍带 `trivial = true`。
#[test]
fn trivial_view_is_distinguishable() {
    fn degenerate(_input: &ProjectionInput<'_>) -> View<Vec<Fact>> {
        View::trivial(Vec::new())
    }
    let p = Projection::new(degenerate);
    let facts: Vec<Fact> = vec![];
    let v = p.run(&ProjectionInput::new(&facts, 0));
    assert!(v.value.is_empty());
    assert!(v.trivial, "平凡值必须与'折出来是空的'可区分（D3）");
}

/// 空表寻址：未登记的名字 ⇒ `Err(NotFound)`，不是 panic / 不是空结果。
#[test]
fn unknown_projection_is_not_found() {
    let facts: Vec<Fact> = vec![];
    let r = projection_run("__no_such_projection__", &ProjectionInput::new(&facts, 0));
    assert_eq!(
        r,
        Err(ProjectionError::NotFound(
            "__no_such_projection__".to_string()
        )),
        "未登记投影应返回 NotFound（可区分'未接入'）"
    );
}

/// `projection_has` 对未登记名返回 false（平凡值判定入口）。
#[test]
fn projection_has_reports_presence() {
    assert!(
        !projection_has("__no_such_projection__"),
        "未登记名应为 false"
    );
}

/// `projection_list` 排序去重后稳定（顺序确定，便于审计比对）。
#[test]
fn projection_list_is_sorted_and_deduped() {
    let a = projection_list();
    let mut sorted = a.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(a, sorted, "projection_list 应已排序去重");
}
