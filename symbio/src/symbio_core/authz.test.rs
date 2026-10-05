//! `authz` 单测 —— 钉死**本机授权表的形状**（04 §3.1 批⑥ 的生产接线）。
//!
//! 这里不复述 core 的判定语义（`governance/mod.test.rs` 钉成对性、闭集校验与
//! 轮次→能力的映射），只钉三件只有这张表能回答的事：
//!
//! 1. `agent:main` 持 6 项、**不持验证**（C15 不自验），首响与追加发言都放行；
//! 2. 读写两侧的闸按**属主 / 矩阵成员**分：主智能体与属主各得其所，`user` 不入表
//!    （发言不是能力），矩阵外主体拒绝；
//! 3. 表构造失败时的降级面（空矩阵）一概拒绝——fail-closed 不因降级而松开。

use super::*;

/// 表的形状：主智能体持 6 项、缺 `assert.verification`，可见域 `thread_private`。
#[test]
fn main_agent_row_is_the_documented_shape() {
    let matrix = production_matrix();
    assert_eq!(
        matrix.grants_of("agent:main").len(),
        6,
        "judge.intent / reply.first / reply.append / define.work / produce.artifact / assign.work"
    );
    assert_eq!(
        matrix.sees_of("agent:main"),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// 写侧闸：收束的两种位置都放行；矩阵外主体两种位置都拒。
#[test]
fn only_the_main_agent_may_close_a_turn() {
    let matrix = production_matrix();
    assert!(
        matrix.can_reply("agent:main", true) && matrix.can_reply("agent:main", false),
        "首响与追加都必须放行，否则生产对话直接断"
    );
    assert!(
        !matrix.can_reply("agent:ghost", true) && !matrix.can_reply("agent:ghost", false),
        "未知主体 fail-closed"
    );
    assert!(
        !matrix.can_reply("user", true),
        "人不入能力表（发言不是能力），故人没有权威收束"
    );
}

/// 读侧闸：`thread_private` 下只有属主本人；`shared` 只认矩阵成员。
#[test]
fn read_side_is_scoped_by_owner_and_membership() {
    let matrix = production_matrix();
    assert!(matrix.can_see("agent:main", "agent:main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("user", "agent:main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("agent:ghost", "agent:main", VisScope::Shared));
}

/// 降级面：空矩阵（表构造失败时的 fail-closed 兜底）一概拒绝。
#[test]
fn the_degraded_empty_matrix_denies_everything() {
    let deny_all = PermissionMatrix::from_names(&[]).expect("空表无主体可校验");
    assert!(!deny_all.can_reply("agent:main", true));
    assert!(
        !deny_all.can_see("user", "agent:main", VisScope::ThreadPrivate),
        "非属主读不到（thread_private 比的是属主，空矩阵不给任何人开例外）"
    );
    assert!(!deny_all.can_see("agent:main", "agent:main", VisScope::Shared));
}
