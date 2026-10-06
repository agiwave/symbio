//! `authz` 单测 —— 钉死**本机授权表的形状**（04 §3.1 批⑥ 的生产接线）。
//!
//! 这里不复述 core 的判定语义（`governance/mod.test.rs` 钉成对性、闭集校验与
//! 轮次→能力的映射），只钉四件只有这张表能回答的事：
//!
//! 1. `agent:main` 持 6 项、**不持验证**（C15 不自验），首响与追加发言都放行；
//! 2. **自主发起者那一行比主智能体窄**（只持 `define.work`、不持 `reply.*`，
//!    S12 §3/§5「自主行为不得冒充用户对话」）；
//! 3. `matrix_for` 的两条分支：**登记过的主体走表**（表是权威，派生不覆盖它），
//!    未登记的 `agent:<id>` 才派生主智能体那一行；
//! 4. 读写两侧的闸按**属主 / 矩阵成员**分：主智能体与属主各得其所，`user` 不入表
//!    （发言不是能力），矩阵外主体拒绝；表构造失败时的降级面（空矩阵）一概拒绝
//!    ——fail-closed 不因降级而松开。

use super::*;
use crate::symbio_core::governance::Capability;

/// 表的形状：主智能体持 6 项、缺 `assert.verification`，可见域 `thread_private`。
#[test]
fn main_agent_row_is_the_documented_shape() {
    let matrix = production_matrix();
    assert_eq!(
        matrix.grants_of(PRINCIPAL_MAIN).len(),
        6,
        "judge.intent / reply.first / reply.append / define.work / produce.artifact / assign.work"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_MAIN),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// 自主发起者那一行（S12 §3）：**比主智能体窄**——只持 `define.work`，不持任何 `reply.*`。
///
/// 这一行是本表里第一条「少给」的记录。它曾经**不存在**，而 `matrix_for` 对一切
/// `agent:*` 都派生主智能体的整套能力集（含 `reply.*`）——于是「自主行为不得冒充
/// 用户对话」在生产里被静默推翻，文档与验收断言却都还写着它成立。
#[test]
fn the_autonomous_row_is_narrower_than_main() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_AUTONOMOUS);
    assert_eq!(caps.len(), 1, "只持 define.work：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_AUTONOMOUS, Capability::DefineWork),
        "自主层必须能定义工作"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_AUTONOMOUS, true)
            && !matrix.can_reply(PRINCIPAL_AUTONOMOUS, false),
        "自主行为不得冒充用户对话（首响与追加都不放行）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_AUTONOMOUS),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );
}

/// `matrix_for` 两条分支：**登记过的主体走表**（表是权威，派生不覆盖它）；
/// 未登记的 `agent:<id>` 才派生主智能体那一行。
#[test]
fn matrix_for_prefers_the_registered_row_over_derivation() {
    // 登记过：拿自己那一行——窄行不被主智能体那套覆盖（覆盖就成「表授予 A、闸门判 B」）。
    let autonomous = matrix_for(PRINCIPAL_AUTONOMOUS);
    assert_eq!(
        autonomous.grants_of(PRINCIPAL_AUTONOMOUS).len(),
        1,
        "派生不得把主智能体的 reply.* 塞回这一行"
    );
    assert!(!autonomous.can_reply(PRINCIPAL_AUTONOMOUS, true));

    // 未登记的子智能体：派生主智能体那一行（身份分层 ≠ 权限分层，S08 §4 平凡值）。
    let sub = matrix_for("agent:sub");
    assert_eq!(
        sub.grants_of("agent:sub").len(),
        production_matrix().grants_of(PRINCIPAL_MAIN).len(),
        "未登记主体派生主智能体的整套能力"
    );
    assert!(
        sub.can_reply("agent:sub", true),
        "子智能体要能写自己的收束格，否则它什么都写不了"
    );

    // 非 `agent:*` 的主体走表：表里没有它 ⇒ fail-closed。
    assert!(!matrix_for(PRINCIPAL_USER).can_reply(PRINCIPAL_USER, true));
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
