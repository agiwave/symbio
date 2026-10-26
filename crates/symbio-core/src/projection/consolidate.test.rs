//! `consolidate` 单测 —— 双约束各配正反用例（拒收 = 保真度防线真实存在）。

use super::*;

#[test]
fn authority_defaults_match_plan_01() {
    let p = ConsolidateParams::default();
    assert_eq!(p.max_gen, 3, "01 §8：consolidate:max_gen = 3");
    assert!(
        (p.min_fidelity - 0.7).abs() < f64::EPSILON,
        "01 §8：min_fidelity = 0.7"
    );
}

#[test]
fn healthy_consolidation_is_accepted() {
    let p = ConsolidateParams::default();
    assert_eq!(accept(&p, 1, 0.9), Ok(()));
    // 边界值：恰好等于下界与上界都放行。
    assert_eq!(accept(&p, 3, 0.7), Ok(()));
}

#[test]
fn over_generation_is_rejected() {
    let p = ConsolidateParams::default();
    assert_eq!(
        accept(&p, 4, 1.0),
        Err(Rejection::RejectedMaxGen {
            generation: 4,
            max: 3
        }),
        "代数上界防无限推进（G7）"
    );
}

#[test]
fn low_fidelity_is_rejected() {
    let p = ConsolidateParams::default();
    assert!(
        matches!(
            accept(&p, 1, 0.5),
            Err(Rejection::RejectedLowFidelity { fidelity, min }) if (fidelity - 0.5).abs() < 1e-9 && (min - 0.7).abs() < 1e-9
        ),
        "保真度下界防失真固化：0.5 < 0.7 必拒"
    );
}
