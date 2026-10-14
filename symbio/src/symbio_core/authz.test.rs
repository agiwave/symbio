//! `authz` 单测 —— 钉死**本机授权表的形状**（04 §3.1 批⑥ 的生产接线）。
//!
//! 这里不复述 core 的判定语义（`governance/mod.test.rs` 钉成对性、闭集校验与
//! 轮次→能力的映射），只钉四件只有这张表能回答的事：
//!
//! 1. `agent:main` 持 6 项、**不持验证**（C15 不自验），首响与追加发言都放行；
//! 2. **自主发起者那一行比主智能体窄**（只持 `define.work`、不持 `reply.*`，
//!    S12 §3/§5「自主行为不得冒充用户对话」）；
//! 3. **抢占判定者那一行比主智能体窄**（只持 `judge.intent`、不持 `reply.*`，
//!    S07 §5「判定者不得直接发言」）；
//! 4. **对等主体那一行比主智能体窄**（只持 `produce.artifact`，S08 §3——第一个
//!    对等/图侧主体，窄于「派生整套」）；
//! 5. `matrix_for` 的两条分支：**登记过的主体走表**（表是权威，派生不覆盖它），
//!    未登记的 `agent:<id>` 才派生主智能体那一行；
//! 6. 读写两侧的闸按**属主 / 矩阵成员**分：主智能体与属主各得其所，`user` 不入表
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

/// 抢占判定者那一行（S07 §5）：**比主智能体窄**——只持 `judge.intent`，不持任何 `reply.*`。
///
/// 与自主行同形的静默面（本批修掉的正是这个）：`agent:reflex` 曾经**不在表里**，
/// 而 `matrix_for` 对一切未登记 `agent:*` 都派生主智能体的整套能力（含 `reply.*`）——
/// 于是「判定者不得发言」在生产里被静默推翻。判据落在**生产表**上（`matrix_for`），
/// 不再是自造矩阵：自造矩阵只证明「我拒我」，表里少一行它照样绿。
#[test]
fn the_reflex_row_is_narrower_than_derivation() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_REFLEX);
    assert_eq!(caps.len(), 1, "只持 judge.intent：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_REFLEX, Capability::JudgeIntent),
        "判定者必须能判决意图，否则抢占判定这一步直接失效"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_REFLEX, true) && !matrix.can_reply(PRINCIPAL_REFLEX, false),
        "判定者不得直接发言（首响与追加都不放行，S07 §5）"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_REFLEX),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 反向判据：**登记过的主体走表**，派生不得覆盖它——`matrix_for` 拿到的必须是
    // 上面那一行（窄），而不是派生出来的主智能体整套。去掉 ROWS 里那一行，这条断言
    // 立刻变红（派生分支会把 `reply.first` 塞回来）。
    let derived = matrix_for(PRINCIPAL_REFLEX);
    assert_eq!(
        derived.grants_of(PRINCIPAL_REFLEX).len(),
        1,
        "派生不得把主智能体的 reply.* 塞回判定者这一行"
    );
    assert!(!derived.can_reply(PRINCIPAL_REFLEX, true));
}

/// 对等主体那一行（S08 §3）：**第一个对等（图侧）主体的窄行**——只持 `produce.artifact`，
/// 严格窄于主智能体，也严格窄于 `matrix_for` 的派生整套。
///
/// S08 §4 的平凡值「身份分层 ≠ 权限分层」是给**子智能体**的（它要能写自己的收束格，
/// 否则什么都写不了）；对等体拿主智能体整套（含 `reply.*`）则是**静默超权**——
/// 与自主行 / 判定行同形的静默面。S08 §3.1 的纪律：对等主体出现必须**自带窄行同批落地**，
/// 不许靠派生。
#[test]
fn the_peer_row_is_narrower_than_the_derived_set() {
    let matrix = production_matrix();
    let caps = matrix.grants_of(PRINCIPAL_PEER);
    assert_eq!(caps.len(), 1, "只持 produce.artifact：{caps:?}");
    assert!(
        matrix.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "对等体的权威动作是产生产物（S08 §3 的 ActorSpec 行），否则这一行没有理由存在"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_PEER, true) && !matrix.can_reply(PRINCIPAL_PEER, false),
        "对等体的协作经 commitment.* 而非 chat.*——不以本机主控的身份发言"
    );
    assert_eq!(
        matrix.sees_of(PRINCIPAL_PEER),
        Some(VisScope::ThreadPrivate),
        "可见域显式给出（成对性），不是 None"
    );

    // 「严格窄于派生整套」：主智能体持 6 项、对等行 1 项；且 `matrix_for` 必须拿**登记行**
    // 而非派生出来的整套——去掉 ROWS 那一行，派生分支会把 `reply.*` 与其余能力一起塞回来。
    let main_len = matrix.grants_of(PRINCIPAL_MAIN).len();
    assert!(
        caps.len() < main_len,
        "对等行必须严格窄于主智能体那一行（{main_len} 项）"
    );
    let derived = matrix_for(PRINCIPAL_PEER);
    assert_eq!(
        derived.grants_of(PRINCIPAL_PEER).len(),
        1,
        "派生不得把主智能体整套塞回对等行"
    );
    assert!(!derived.can_reply(PRINCIPAL_PEER, true));
    assert!(
        derived.can_write(PRINCIPAL_PEER, Capability::ProduceArtifact),
        "登记行是权威：对等体仍持它那一项能力"
    );
}

/// 收束格按**种类**判（[ADR-067](../../../docs/adr/ADR-067.md "ADR-067")，T-072 的单测判据）：
/// 产物收束问 `produce.artifact`、发言收束仍按 `reply.*`——同一张表的两道闸。
///
/// 钉的是谓词面的**分派**，不是每个主体自己的窄行（那些在上面各测里）：对等主体
/// 产物闸放行、发言闸仍拒；主智能体两道都放；判定者 / 自主层产物闸也拒（它们
/// 持的不是 `produce.artifact`）。没有这道分派，对等主体跑一轮永远收束不了
/// （ADR-065 的批②③ 合并理由）；反过来若产物闸误用了 `can_reply`，对等主体的
/// 产物也落不了格——两种错位都被这条钉住。
#[test]
fn the_closure_gate_is_dispatched_by_kind() {
    let matrix = production_matrix();

    // 对等主体：产物收束放行、发言收束仍拒（「没有发言权」≠「留不下任何事实」）。
    assert!(
        matrix.can_produce_artifact(PRINCIPAL_PEER),
        "对等体以产物收束一轮（ADR-065 裁决的落地前提）"
    );
    assert!(
        !matrix.can_reply(PRINCIPAL_PEER, true),
        "产物闸放行不改变发言闸：对等体仍无发言权"
    );

    // 主智能体：两道都放（发言与产物都是它的权威动作）。
    assert!(matrix.can_reply(PRINCIPAL_MAIN, true));
    assert!(matrix.can_produce_artifact(PRINCIPAL_MAIN));

    // 判定者 / 自主层：两道都拒——它们持的能力与收束无关（fail-closed 的方向不变）。
    for p in [PRINCIPAL_REFLEX, PRINCIPAL_AUTONOMOUS] {
        assert!(
            !matrix.can_produce_artifact(p) && !matrix.can_reply(p, true),
            "{p} 既不产生产物也不发言（各自的窄行）"
        );
    }

    // 未知主体 fail-closed：两道闸同拒。
    assert!(!matrix.can_produce_artifact("agent:ghost"));
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
