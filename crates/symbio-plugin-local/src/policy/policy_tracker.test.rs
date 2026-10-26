//! `policy_tracker.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `policy_tracker.rs` 只保留生产代码，测试全部放本文件。
//!
//! 放在本文件而不是 `mod.rs` 的测试模块里，是因为要**直接构造窗口**——
//! `window_secs` 是私有字段，只有同模块的测试能改它。

use super::*;

/// 窗口内计数 / 到限判定 / `0` = 关闭限流
#[test]
fn window_count_and_limit() {
    let t = ActionTracker::new();
    assert_eq!(t.count(), 0);
    for _ in 0..3 {
        t.record();
    }
    assert_eq!(t.count(), 3);
    assert!(!t.is_at_limit(5));
    assert!(t.is_at_limit(3));
    assert!(t.is_at_limit(1));
    assert!(!t.is_at_limit(0), "0 视为关闭限流");
}

/// **窗口比时钟基准到此刻的时长还长**时，没有一条记录算旧——不得清空。
///
/// 这是 `cleanup_old_actions` 里 `checked_sub` 下溢分支的回归测试。旧写法
/// （`.unwrap_or_else(Instant::now)`）会在这里返回 0：cutoff 变成「此刻」，
/// `retain` 把全部记录丢掉 ⇒ 限流静默失效。
///
/// 用 1e9 秒（≈31 年）作窗口把下溢**确定性地**造出来：`Instant` 的基准点没有保证，
/// 但没有任何实现会早到 31 年之前。
#[test]
fn window_longer_than_clock_origin_keeps_every_action() {
    let t = ActionTracker {
        actions: Mutex::new(Vec::new()),
        window_secs: 1_000_000_000,
    };
    for _ in 0..3 {
        t.record();
    }
    assert_eq!(
        t.count(),
        3,
        "窗口早于时钟基准 ⇒ 没有一条是旧的，全部保留（下溢不是「窗口从此刻开始」）"
    );
    assert!(t.is_at_limit(3));
}

/// 窗口**短于**记录年龄时照常清理：把记录手动改老，验证清理方向没被改反
#[test]
fn stale_actions_are_dropped() {
    let t = ActionTracker {
        actions: Mutex::new(Vec::new()),
        window_secs: 0,
    };
    // window_secs = 0 ⇒ cutoff = 此刻 ⇒ 严格大于才算窗口内 ⇒ 已记录的都被判旧
    t.record();
    assert_eq!(t.count(), 0, "窗口为 0 时任何记录都在窗口外");
}

/// 克隆出来的追踪器带**独立的**记录表与同一窗口
#[test]
fn clone_is_independent() {
    let t = ActionTracker::new();
    t.record();
    t.record();
    let c = t.clone();
    assert_eq!(c.count(), 2, "克隆继承既有记录");
    c.record();
    assert_eq!(c.count(), 3);
    assert_eq!(t.count(), 2, "克隆之后两者互不影响");
}
