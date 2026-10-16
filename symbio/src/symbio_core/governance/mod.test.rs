//! `governance` 单测 —— S3 两步的出口判据逐条钉死。
//!
//! 第 7 步：`grants_of` / `sees_of` 成对，只有写侧 → 拒绝；
//! 第 8 步：`vis_scope` 默认 `thread_private` → 越界读取 0。
//!
//! [04 §3.1 批⑥](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 的接线面（判定在 core、
//! 构造与调用在 core 外，故各钉一半）：`from_names` 认不出的能力名**拒绝整体构造**、
//! `can_reply` 的「轮次位置 → 能力」映射（调用点不复述）。

use super::*;
use crate::symbio_core::event::Entity;

/// 验收（第 7 步）：只有写侧 → 拒绝。
#[test]
fn write_only_policy_is_rejected() {
    let result = PermissionMatrix::new(vec![PrincipalPolicy {
        principal: "session/rogue".into(),
        grants: vec![Capability::ReplyFirst],
        vis_scope: None,
    }]);
    assert_eq!(
        result,
        Err(PairingViolation::WriteOnly {
            principal: "session/rogue".into()
        }),
        "只有写侧 = 看得到一切——必须在构造时拒绝"
    );
}

/// 验收（第 7 步反向侧）：只有读侧 → 同样拒绝。
#[test]
fn read_only_policy_is_rejected() {
    let result = PermissionMatrix::new(vec![PrincipalPolicy {
        principal: "session/snooper".into(),
        grants: vec![],
        vis_scope: Some(VisScope::ThreadPrivate),
    }]);
    assert_eq!(
        result,
        Err(PairingViolation::ReadOnly {
            principal: "session/snooper".into()
        })
    );
}

/// 验收（第 7 步）：成对策略构造成功，读写两侧都可查。
#[test]
fn paired_policy_serves_both_sides() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "session/main",
        vec![Capability::ReplyFirst, Capability::ReplyAppend],
        VisScope::ThreadPrivate,
    )])
    .expect("成对策略必过");
    assert_eq!(
        matrix.grants_of("session/main"),
        [Capability::ReplyFirst, Capability::ReplyAppend]
    );
    assert_eq!(
        matrix.sees_of("session/main"),
        Some(VisScope::ThreadPrivate)
    );
    assert!(matrix.can_write("session/main", Capability::ReplyFirst));
    assert!(
        !matrix.can_write("session/main", Capability::AssignWork),
        "未持有的能力必拒"
    );
}

/// 验收（第 7 步）：fail-closed——未知主体写与读全拒。
#[test]
fn unknown_principal_is_fail_closed() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "session/main",
        vec![Capability::ReplyFirst],
        VisScope::Public,
    )])
    .expect("成对策略必过");
    assert!(!matrix.can_write("session/ghost", Capability::ReplyFirst));
    assert_eq!(matrix.sees_of("session/ghost"), None);
    assert!(!matrix.can_see("session/ghost", "session/main", VisScope::Shared));
    // Public 是唯一例外：它的语义就是任何人可见。
    assert!(matrix.can_see("session/ghost", "session/main", VisScope::Public));
}

/// C10：`vis_scope` 的缺省值是 `thread_private`（默认隔离是缺省值，不是配置项）。
#[test]
fn vis_scope_defaults_to_thread_private() {
    assert_eq!(VisScope::default(), VisScope::ThreadPrivate);
}

/// 验收（第 8 步）：默认隔离下越界读取 0。
#[test]
fn thread_private_yields_zero_out_of_bounds_reads() {
    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "session/main",
            vec![Capability::ReplyFirst],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "session/worker",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("成对策略必过");
    // 别人的 thread_private 内容：worker 看不到 main 的，main 也看不到 worker 的。
    assert!(!matrix.can_see("session/worker", "session/main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("session/main", "session/worker", VisScope::ThreadPrivate));
    // 本人可见。
    assert!(matrix.can_see("session/main", "session/main", VisScope::ThreadPrivate));
    // 提权必须显式：shared 对矩阵内主体开放。
    assert!(matrix.can_see("session/worker", "session/main", VisScope::Shared));
}

/// `from_names`（授权表构造面）：能力**名** → 枚举的闭集翻译，成功时照样过成对性。
///
/// 授权表（`crate::symbio_core::authz`）与本域同住一棵模块树，但**表与翻译仍是两半**
/// ——所以本测钉的是翻译那半边：名字认得出 ⇒ grants 与可见域都落位（`paired` 仍被
/// 调用，成对性不绕过）。
#[test]
fn from_names_translates_a_name_table_into_a_paired_matrix() {
    let matrix = PermissionMatrix::from_names(&[(
        "session/main",
        &["reply.first", "reply.append"],
        VisScope::ThreadPrivate,
    )])
    .expect("认得出的能力名必过");
    assert_eq!(
        matrix.grants_of("session/main"),
        [Capability::ReplyFirst, Capability::ReplyAppend]
    );
    assert_eq!(
        matrix.sees_of("session/main"),
        Some(VisScope::ThreadPrivate)
    );
    // 闭集之外的行不复存在 ⇒ 未知主体照旧 fail-closed。
    assert!(!matrix.can_write("session/ghost", Capability::ReplyFirst));
}

/// `from_names` 的反向：**认不出的能力名拒绝整体构造**，且错误指名道姓。
///
/// 不能「跳过那一行 / 当作没这个能力」——那会把授权表里的笔误变成一条**悄悄生效的
/// 拒绝**（生产上线后才发现某项能力从未生效），与 fail-closed 是两回事：这里拒绝的是
/// 「表本身不合法」，于是宿主降级空矩阵 + 告警（`crate::symbio_core::authz::production_matrix`）。
#[test]
fn a_name_outside_the_closed_set_refuses_construction() {
    let err = PermissionMatrix::from_names(&[(
        "session/main",
        &["reply.fisrt"],
        VisScope::ThreadPrivate,
    )])
    .expect_err("拼写错误必须拒绝整体构造");
    let text = err.to_string();
    assert!(text.contains("reply.fisrt"), "错误要点名那个能力：{text}");
    assert!(text.contains("session/main"), "错误要点名那一行：{text}");
}

/// `can_reply`：**轮次位置 → 能力**的映射口径只在这一处（调用点不复述）。
#[test]
fn can_reply_maps_turn_position_to_one_capability() {
    // 只有追加能力 ⇒ 首条被拒、追加放行（证明判的是能力，不是恒真）。
    let append_only =
        PermissionMatrix::from_names(&[("session/worker", &["reply.append"], VisScope::default())])
            .expect("成对策略必过");
    assert!(
        !append_only.can_reply("session/worker", true),
        "缺 reply.first"
    );
    assert!(
        append_only.can_reply("session/worker", false),
        "有 reply.append"
    );

    // 只有首响能力 ⇒ 反过来。
    let first_only =
        PermissionMatrix::from_names(&[("session/main", &["reply.first"], VisScope::default())])
            .expect("成对策略必过");
    assert!(first_only.can_reply("session/main", true));
    assert!(
        !first_only.can_reply("session/main", false),
        "缺 reply.append"
    );

    // 空矩阵（表构造失败时的降级兜底）⇒ 一概拒绝；未知主体 ⇒ fail-closed。
    let deny_all = PermissionMatrix::from_names(&[]).expect("空表无主体可校验");
    assert!(!deny_all.can_reply("session/main", true));
    assert!(!first_only.can_reply("session/ghost", true));
}

/// 与 S1/S2 彩排链路对接的形状预演：主体对**事件切片**的可见子集为空 ⇒ 越界读取 0。
#[test]
fn foreign_principal_reads_nothing_from_a_private_thread() {
    use crate::symbio_core::event::{Event, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
    use crate::symbio_core::store::{EventStore, Store};

    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "session/main",
            vec![Capability::ReplyFirst],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "session/intruder",
            vec![Capability::DefineWork],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("成对策略必过");

    let store = EventStore::new();
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            crate::symbio_core::event::Verb::Opened,
            0,
            "user",
        ))
        .expect("首插必成");
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                crate::symbio_core::event::Verb::Closed,
                0,
                "session/main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    // thread_private：事件只对**产生它的主体**可见。
    let visible: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("session/intruder", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(visible.len(), 0, "越界读取必须为 0：{:?}", visible);
    // 本人可见自己产出的事件（口径=按事件作者；「线程参与者」模型随 S4 thread
    // 实体落地——user 发言的共享归属是 thread 的属性，不是 author 的）。
    let own: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("session/main", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(own.len(), 1, "main 可见自己的 final；user 消息归 user");
    assert_eq!(own[0].kind, EVENT_ASSISTANT_FINAL);
}

// ── T-094：可见域的字符串形态（内容侧产地的书写面）─────────────────────

/// 书写形态 round-trip：三个取值各自 `name → from_name` 逐字回来。
/// 判据不是「能转回来」而是「**只有**这三个能转回来」——载荷里出现第四个值
/// （拼写错误 / 将来的新取值没有同批登记）必须落回缺省，不能静默变成更宽。
#[test]
fn vis_scope_name_round_trips_and_unknown_falls_back_to_private() {
    for scope in [VisScope::ThreadPrivate, VisScope::Shared, VisScope::Public] {
        assert_eq!(
            VisScope::from_name(scope.name()),
            scope,
            "{} 必须逐字转回",
            scope.name()
        );
    }
    assert_eq!(VisScope::from_name("shared"), VisScope::Shared);
    assert_eq!(VisScope::from_name("public"), VisScope::Public);
    // 认不出的值 = 缺省 thread_private（不是放宽、不是报错后放行）——
    // 把笔误读成 shared/public 等于把载荷的拼写错误变成一次越权读取。
    for bad in ["Shared", "SHARED", "shard", "", "thread-private"] {
        assert_eq!(
            VisScope::from_name(bad),
            VisScope::ThreadPrivate,
            "`{bad}` 必须落回缺省（fail-closed 的方向是更窄）"
        );
    }
}

/// `widen` 是全序的「取宽」：满足交换律、幂等、单调——
/// 读侧整列闸（T-094 ②）拿它合并一批内容的可见域，性质不成立就会
/// 「同两份事实源因为顺序不同读出不同的闸答案」，而两种答案都合法。
#[test]
fn widen_is_commutative_idempotent_and_monotone() {
    use VisScope::{Public, Shared, ThreadPrivate};
    let all = [ThreadPrivate, Shared, Public];
    for a in all {
        for b in all {
            assert_eq!(a.widen(b), b.widen(a), "交换律：{a:?} × {b:?}");
        }
        assert_eq!(a.widen(a), a, "幂等：{a:?}");
        assert_eq!(a.widen(ThreadPrivate), a, "私有是底：{a:?}");
        assert_eq!(Public.widen(a), Public, "公开是顶：{a:?}");
    }
    assert_eq!(ThreadPrivate.widen(Shared), Shared);
    assert_eq!(Shared.widen(Public), Public);
}

// ── T-073：授权覆盖层（淘汰回路的机制半边）─────────────────────────────

/// 撤销回路（S13 §6 断言 3「淘汰 = 一条可审计、可撤销的 grants 变更事件」）：
/// 收回 ⇒ `is_revoked` 翻真 ⇒ 恢复 ⇒ 翻假 ⇒ **再收回** ⇒ 又翻真。
///
/// last-wins 是这条回路的全部语义：恢复之后再违约必须还能再淘汰
/// （否则第一次申诉之后主体永久免死），反之恢复必须真的生效
/// （否则「可申诉」只是一句文档话）。
#[test]
fn grants_overlay_supports_the_revoke_restore_revoke_loop() {
    let revoke = |subject: &str| GrantsChange::Revoke {
        principal: subject.to_string(),
        capability: "assign.work".to_string(),
    };
    let restore = |subject: &str| GrantsChange::Restore {
        principal: subject.to_string(),
        capability: "assign.work".to_string(),
    };

    let overlay = GrantsOverlay::fold([]);
    assert!(
        !overlay.is_revoked("session/main", "assign.work"),
        "空覆盖层 = 什么都没收回（表本身是权威，覆盖层只答减法）"
    );

    let overlay = GrantsOverlay::fold([revoke("session/main")]);
    assert!(overlay.is_revoked("session/main", "assign.work"));
    assert!(
        !overlay.is_revoked("session/market", "assign.work"),
        "撤销按 (主体, 能力) 精确命中，不波及别的主体"
    );
    assert!(
        !overlay.is_revoked("session/main", "reply.first"),
        "也不波及同一主体的别的能力——淘汰只收回 assign.work"
    );

    let overlay = GrantsOverlay::fold([revoke("session/main"), restore("session/main")]);
    assert!(
        !overlay.is_revoked("session/main", "assign.work"),
        "恢复是撤销的逆操作（申诉成立 ⇒ 重新可委派）"
    );

    let overlay = GrantsOverlay::fold([
        revoke("session/main"),
        restore("session/main"),
        revoke("session/main"),
    ]);
    assert!(
        overlay.is_revoked("session/main", "assign.work"),
        "恢复之后再违约 ⇒ 再淘汰（last-wins，不是一次性事件）"
    );
}

/// 覆盖层的 fail-closed 方向：认不出的能力名 ⇒ 未收回。
///
/// 与 `from_name` 相反的方向感——能力名笔误在这里**不**变成一次静默撤销
/// （那等于把载荷的拼写错误变成一次越权拒绝），而是「这条撤销不存在」，
/// 由调用方的表谓词继续把关。两个 fail-closed 各自守自己那一侧。
#[test]
fn overlay_never_guesses_a_capability_name() {
    let overlay = GrantsOverlay::fold([GrantsChange::Revoke {
        principal: "session/main".to_string(),
        capability: "assign.wrok".to_string(),
    }]);
    assert!(
        !overlay.is_revoked("session/main", "assign.work"),
        "拼错的撤销不命中正确的能力名"
    );
    assert!(
        overlay.is_revoked("session/main", "assign.wrok"),
        "但它自己那一格照旧生效（审计面不能丢）"
    );
}
