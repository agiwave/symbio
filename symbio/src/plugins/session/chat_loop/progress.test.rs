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

// ── RoundProgress：轮边界回调持有的那份状态（缺口 4）────────────────
//
// 为什么有这组用例：`due` 的四条与关系上面已穷举，但它们都假设
// `tool_rounds` **有人递增**。缺口 4 的病根就是 `full` 档下那个递增
// 消失了（v1 的轮循环不存在），而 `due` 本身照绿——判定对、输入恒零。
// 这组用例钉的是「推进 → 判定」这条链：轮数不到 `min_rounds` 不说、
// 到了说、说了静默时钟归零、配额耗尽闭嘴。

/// 距"上次说话"已 60 秒的策略状态（`interval_ms = 60_000` ⇒ 静默条件恒成立）。
fn progressed() -> RoundProgress {
    let last_spoke_at = clock_now_ms() - 60_000;
    RoundProgress::new(policy(), last_spoke_at)
}

#[test]
fn round_boundary_counts_up_and_gates_the_report() {
    let mut p = progressed();

    // 第 1 个轮边界：轮数 1 < `min_rounds`（2）⇒ 不说。
    let first = p.advance();
    assert_eq!(first.tool_rounds, 1, "轮边界推进一格");
    assert!(!p.due(&first), "轮数不到 min_rounds：进展还谈不上");

    // 第 2 个轮边界：轮数达标 ⇒ 说。
    let second = p.advance();
    assert_eq!(second.tool_rounds, 2);
    assert!(p.due(&second), "轮数达标且静默够久：该说");
    assert!(
        second.quiet_ms >= 60_000,
        "静默时长从构造时的 last_spoke_at 起算"
    );
}

#[test]
fn speaking_resets_the_quiet_clock_and_spends_the_quota() {
    let mut p = progressed();
    p.advance();
    let snap = p.advance();
    assert!(p.due(&snap));

    p.spoke();

    // 静默时钟归零 ⇒ 下一个轮边界「刚说过话」，即静默条件不成立。
    let next = p.advance();
    assert!(
        next.quiet_ms < 60_000,
        "说了之后时钟归零：{} 不该还顶着 60 秒的静默",
        next.quiet_ms
    );
    assert!(!p.due(&next), "归零后同一段静默不能连续吃配额");

    // 配额：max_per_turn = 5，汇报一次后还剩 4 次额度。
    p.last_spoke_at = clock_now_ms() - 60_000;
    let due_again = p.advance();
    assert!(p.due(&due_again), "时钟重新计时后还能再说");
    assert_eq!(p.reports, 1, "配额计了一次");
}
