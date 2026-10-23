//! `governance` 单测 —— S3 两步的出口判据逐条钉死。
//!
//! 第 7 步：`grants_of` / `sees_of` 成对，只有写侧 → 拒绝；
//! 第 8 步：`vis_scope` 默认 `thread_private` → 越界读取 0。
//!
//! [04 §3.1 批⑥](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的接线面（判定在 core、
//! 构造与调用在 core 外，故各钉一半）：`from_names` 认不出的能力名**拒绝整体构造**、
//! `can_reply` 只问一个 `reply` 能力（调用点不复述）。

use super::*;
use crate::symbio_core::event::Entity;

/// 验收（第 7 步）：只有写侧 → 拒绝。
#[test]
fn write_only_policy_is_rejected() {
    let result = PermissionMatrix::new(vec![PrincipalPolicy {
        principal: "session/rogue".into(),
        grants: vec![Capability::Reply],
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
        vec![Capability::Reply, Capability::Reply],
        VisScope::ThreadPrivate,
    )])
    .expect("成对策略必过");
    assert_eq!(
        matrix.grants_of("session/main"),
        [Capability::Reply, Capability::Reply]
    );
    assert_eq!(
        matrix.sees_of("session/main"),
        Some(VisScope::ThreadPrivate)
    );
    assert!(matrix.can_write("session/main", Capability::Reply));
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
        vec![Capability::Reply],
        VisScope::Public,
    )])
    .expect("成对策略必过");
    assert!(!matrix.can_write("session/ghost", Capability::Reply));
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
            vec![Capability::Reply],
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
        &["reply", "reply"],
        VisScope::ThreadPrivate,
    )])
    .expect("认得出的能力名必过");
    assert_eq!(
        matrix.grants_of("session/main"),
        [Capability::Reply, Capability::Reply]
    );
    assert_eq!(
        matrix.sees_of("session/main"),
        Some(VisScope::ThreadPrivate)
    );
    // 闭集之外的行不复存在 ⇒ 未知主体照旧 fail-closed。
    assert!(!matrix.can_write("session/ghost", Capability::Reply));
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

/// `can_reply`：只问**一个**能力（`reply`），不再按轮次位置分叉。
#[test]
fn can_reply_reads_the_single_reply_capability() {
    // 持 `reply` ⇒ 放行；不持 ⇒ 拒（证明判的是能力，不是恒真）。
    let holder = PermissionMatrix::from_names(&[("session/main", &["reply"], VisScope::default())])
        .expect("成对策略必过");
    assert!(holder.can_reply("session/main"));

    let speaker_less =
        PermissionMatrix::from_names(&[("session/worker", &["define.work"], VisScope::default())])
            .expect("成对策略必过");
    assert!(!speaker_less.can_reply("session/worker"), "不持 reply");

    // 空矩阵（表构造失败时的降级兜底）⇒ 一概拒绝；未知主体 ⇒ fail-closed。
    let deny_all = PermissionMatrix::from_names(&[]).expect("空表无主体可校验");
    assert!(!deny_all.can_reply("session/main"));
    assert!(!holder.can_reply("session/ghost"));
}

/// 与 S1/S2 彩排链路对接的形状预演：主体对**事件切片**的可见子集为空 ⇒ 越界读取 0。
#[test]
fn foreign_principal_reads_nothing_from_a_private_thread() {
    use crate::symbio_core::event::{Event, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
    use crate::symbio_core::store::{EventStore, Store};

    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "session/main",
            vec![Capability::Reply],
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

// ── ：可见域的字符串形态（内容侧产地的书写面）─────────────────────

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

/// 能力名 round-trip（⑥）：拼写只活在 [`closed_set!`] 的声明表里，六项各自
/// `name → parse_name` 逐字回来——两个方向共用一张表，「读作 A、写作 B」在这里
/// 就断掉了。判据的另一半是闭集**外**的名字必须 `None`：它的两个下游方向都是拒
/// （`from_names` 拒绝整体构造、[`GrantsOverlay::is_revoked`] 按已收回判），
/// 认不出却静默放行才是事故。
#[test]
fn capability_names_round_trip_through_the_declaration_table() {
    use Capability::{
        AssertVerification, AssignWork, DefineWork, JudgeIntent, ProduceArtifact, Reply,
    };
    for cap in [
        JudgeIntent,
        Reply,
        DefineWork,
        ProduceArtifact,
        AssertVerification,
        AssignWork,
    ] {
        assert_eq!(
            Capability::parse_name(cap.name()),
            Some(cap),
            "{} 必须逐字转回",
            cap.name()
        );
    }
    for bad in [
        "Reply",
        "assign_work",
        "reply.append",
        "produce.artifacts",
        "",
    ] {
        assert_eq!(
            Capability::parse_name(bad),
            None,
            "`{bad}` 不在闭集内，必须认不出而不是读成某个能力"
        );
    }
}

/// `widen` 是全序的「取宽」：满足交换律、幂等、单调——
/// 读侧整列闸（②）拿它合并一批内容的可见域，性质不成立就会
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

// ── ：授权覆盖层（淘汰回路的机制半边）─────────────────────────────

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
        !overlay.is_revoked("session/main", "reply"),
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

/// 覆盖层的 fail-closed（[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")）：
/// **查询侧**的能力名不在 `Capability` 闭集内 ⇒ 告警并按**已收回**判。
///
/// 方向与其余 fail-closed 面同侧：查询名认不出时「现在准不准」无从查起，
/// 宁红不漏——笔误的查询名不许变成一次静默放行。
#[test]
fn overlay_revokes_on_an_unrecognizable_query_name() {
    // 空覆盖层（表是权威、覆盖层只答减法）：认得出的查询名照旧答「未收回」，
    // 认不出的查询名按已收回判。
    let overlay = GrantsOverlay::fold([]);
    assert!(!overlay.is_revoked("session/main", "reply"));
    assert!(
        overlay.is_revoked("session/main", "reply.fisrt"),
        "认不出的查询名必须按已收回判（fail-closed）"
    );
    // 流水里只有恢复也一样：认不出的查询名无格可查，仍按已收回判。
    let overlay = GrantsOverlay::fold([GrantsChange::Restore {
        principal: "session/main".to_string(),
        capability: "assign.work".to_string(),
    }]);
    assert!(overlay.is_revoked("session/main", "assign.wrok"));
    assert!(!overlay.is_revoked("session/main", "assign.work"));
}

/// 变更流水里的**收回**项带认不出的能力名 ⇒ 该主体对**任何**能力查询都按已收回判
/// （[D-099](../../../../docs/003-design/011-Agent目录规范.md#d-099-授权默认翻-fail-closed未登记主体不再派生主智能体能力集认不出的能力名按已收回判 "DE-011-Agent目录规范")：一次笔误的收回不得静默落空）。
///
/// 读法（ADR 字面「变更流水里带认不出能力名的收回项对该主体的任何能力查询都按
/// 已收回判」）：认不出名字的收回不是「某一格被收回」，而是「这次收回落不到任何
/// 一格」，故按**主体整体**生效——之后同名的 `Restore` 也**不解除**。这是宁红不漏：
/// 极端情况下一次载荷损坏会让该主体暂时什么都做不了，但不会让一次本该生效的收回
/// 静默消失（代价划界见该 ADR「代价」节）。
#[test]
fn a_typoed_revoke_revokes_the_whole_principal() {
    let typoed_revoke = || GrantsChange::Revoke {
        principal: "session/main".to_string(),
        capability: "assign.wrok".to_string(),
    };

    // 该主体的任何能力查询——认得出的名字也一样——都按已收回判。
    let overlay = GrantsOverlay::fold([typoed_revoke()]);
    assert!(
        overlay.is_revoked("session/main", "assign.work"),
        "笔误的收回对该主体整体生效，不是只对那一格"
    );
    assert!(
        overlay.is_revoked("session/main", "reply"),
        "任何能力查询都按已收回判"
    );
    // 只波及那一个主体：别人的查询照旧按流水答。
    assert!(!overlay.is_revoked("session/market", "assign.work"));

    // 同名 `Restore` 也不能解除（读法见文档注释）。
    let overlay = GrantsOverlay::fold([
        typoed_revoke(),
        GrantsChange::Restore {
            principal: "session/main".to_string(),
            capability: "assign.wrok".to_string(),
        },
    ]);
    assert!(
        overlay.is_revoked("session/main", "assign.work"),
        "同名恢复不解除整体收回"
    );

    // 认得出名字的 `Restore` 同样解除不了——整体生效不按名字折叠。
    let overlay = GrantsOverlay::fold([
        typoed_revoke(),
        GrantsChange::Restore {
            principal: "session/main".to_string(),
            capability: "assign.work".to_string(),
        },
    ]);
    assert!(overlay.is_revoked("session/main", "assign.work"));
}

/// 载荷读数的**唯一取值口**（⑧）：三处复写的那一段现在只有一份。
/// 测试侧**故意写字面量键**（不用 `VIS_SCOPE_KEY`）：它是线上键名，产侧改了名而这里
/// 没跟着改，本用例必须红——与 `event_bus` 里「手搓信封」那两条判据同一条纪律。
/// 它的缺省方向必须钉住——把「认不出」读宽 = 一次静默越权读取，而两边都是合法值，
/// 没有任何东西会为此变红（[`VisScope::from_name`] 文档里的同向判据）。
#[test]
fn vis_scope_of_event_only_ever_widens_on_an_exact_spelling() {
    use crate::symbio_core::event::{Entity, Event, Verb};
    let base = || {
        Event::pending(
            "e1",
            "memory.encoded",
            Entity::Memory,
            Verb::Closed,
            0,
            "user",
        )
    };

    // 缺字段（存量事实）⇒ 最窄。
    assert_eq!(VisScope::of_event(&base()), VisScope::ThreadPrivate);
    // 逐字对上的书写形态 ⇒ 读回它自己（与 `name()` 共用声明表那一份拼法）。
    for scope in [VisScope::Shared, VisScope::Public] {
        let mut e = base();
        e.payload = serde_json::json!({ "vis_scope": scope.name() });
        assert_eq!(
            VisScope::of_event(&e),
            scope,
            "写的是这一格、读的必须是同一格"
        );
    }
    // 认不出（大小写错 / 造的词）⇒ 落回最窄，不读宽。
    for typo in ["Shared", "sharred", "publick", ""] {
        let mut e = base();
        e.payload = serde_json::json!({ "vis_scope": typo });
        assert_eq!(
            VisScope::of_event(&e),
            VisScope::ThreadPrivate,
            "笔误「{typo}」不得被读成更宽的可见域"
        );
    }
    // 非字符串取值同样落回最窄（`{ "vis_scope": 1 }` 这种载荷不该让读数变宽）。
    let mut e = base();
    e.payload = serde_json::json!({ "vis_scope": 1 });
    assert_eq!(VisScope::of_event(&e), VisScope::ThreadPrivate);
}
