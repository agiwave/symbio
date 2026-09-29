//! `session/chat_loop/progress.rs` 的单元测试 —— 汇报判定的**逐条件**钉法。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 为什么只测 `due`
//!
//! `report_if_due` 要 `ChatOrchestrator`（容器句柄）与真实转写，属 e2e 的射程
//! （`e2e/cases/t23-progress-report.mjs`）；单测在这里重复一遍只会得到一份会漂移的
//! 假副本。而 `due` 是**纯函数**，它承载本文件里唯一"错了不会报错"的决定：
//! 四条与关系里多一条、少一条、边界取 `>` 还是 `>=`，都不会有任何错误信号——
//! 只表现为"用户被打扰"或"用户收不到消息"。
//!
//! 因此每个条件都要有一条"**只差它不成立**"的用例：只验"全满足时汇报"是不够的，
//! 那在"恒返回 `true`"的实现下也通过。

use super::*;

/// 四个条件都成立的策略；各用例只改其中一项。
fn policy() -> ProgressPolicy {
    ProgressPolicy {
        enabled: true,
        interval_ms: 60_000,
        min_rounds: 2,
        max_per_turn: 5,
    }
}

#[test]
fn all_conditions_met_reports() {
    assert!(policy().due(60_000, 2, 0), "四个条件都成立就该汇报");
}

#[test]
fn disabled_never_reports() {
    let p = ProgressPolicy {
        enabled: false,
        ..policy()
    };
    assert!(
        !p.due(i64::MAX, usize::MAX, 0),
        "关掉总开关后其余条件再满足也不汇报（J2 平凡值：行为与引入汇报之前逐字一致）"
    );
}

#[test]
fn too_soon_does_not_report() {
    let p = policy();
    assert!(!p.due(59_999, 2, 0), "差 1ms 到阈值：用户还没等够");
    assert!(p.due(60_000, 2, 0), "阈值取 `>=`：到了就说，不是超过才说");
}

#[test]
fn too_few_rounds_does_not_report() {
    let p = policy();
    assert!(!p.due(60_000, 1, 0), "只走完一轮：进展还谈不上");
    assert!(p.due(60_000, 2, 0), "走到 `min_rounds` 就够");
}

#[test]
fn quota_exhausted_does_not_report() {
    let p = policy();
    assert!(
        p.due(60_000, 2, 4),
        "配额 5 ⇒ 第 5 次（已汇报 4 次）仍可汇报"
    );
    assert!(
        !p.due(60_000, 2, 5),
        "配额用完即停（判定是 `reports < max`）"
    );
}

#[test]
fn zero_quota_is_a_legitimate_degenerate_value() {
    // `progress_max_per_turn = 0` 与 `progress_enabled = false` **同效不同义**：
    // 前者是"上界为零"，后者是"这个特性关掉"。两条路径都要成立，不能靠其中一条
    // 短路掉另一条——否则配置面上会多出一个假开关。
    let p = ProgressPolicy {
        max_per_turn: 0,
        ..policy()
    };
    assert!(!p.due(60_000, 2, 0));
}

#[test]
fn a_negative_quiet_span_is_not_due() {
    // 时钟回拨（或调用方传了未初始化的值）会让差值为负。负值**不是**"等了很久"：
    // 它必须落回"不汇报"，否则一次时钟跳变就会让每个轮边界都冒出一句话。
    assert!(!policy().due(-1, 2, 0));
}
