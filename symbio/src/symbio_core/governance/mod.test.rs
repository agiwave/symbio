//! `governance` 单测 —— S3 两步的出口判据逐条钉死。
//!
//! 第 7 步：`grants_of` / `sees_of` 成对，只有写侧 → 拒绝；
//! 第 8 步：`vis_scope` 默认 `thread_private` → 越界读取 0。

use super::*;
use crate::symbio_core::event::Entity;

/// 验收（第 7 步）：只有写侧 → 拒绝。
#[test]
fn write_only_policy_is_rejected() {
    let result = PermissionMatrix::new(vec![PrincipalPolicy {
        principal: "agent:rogue".into(),
        grants: vec![Capability::ReplyFirst],
        vis_scope: None,
    }]);
    assert_eq!(
        result,
        Err(PairingViolation::WriteOnly {
            principal: "agent:rogue".into()
        }),
        "只有写侧 = 看得到一切——必须在构造时拒绝"
    );
}

/// 验收（第 7 步反向侧）：只有读侧 → 同样拒绝。
#[test]
fn read_only_policy_is_rejected() {
    let result = PermissionMatrix::new(vec![PrincipalPolicy {
        principal: "agent:snooper".into(),
        grants: vec![],
        vis_scope: Some(VisScope::ThreadPrivate),
    }]);
    assert_eq!(
        result,
        Err(PairingViolation::ReadOnly {
            principal: "agent:snooper".into()
        })
    );
}

/// 验收（第 7 步）：成对策略构造成功，读写两侧都可查。
#[test]
fn paired_policy_serves_both_sides() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:main",
        vec![Capability::ReplyFirst, Capability::ReplyAppend],
        VisScope::ThreadPrivate,
    )])
    .expect("成对策略必过");
    assert_eq!(
        matrix.grants_of("agent:main"),
        [Capability::ReplyFirst, Capability::ReplyAppend]
    );
    assert_eq!(matrix.sees_of("agent:main"), Some(VisScope::ThreadPrivate));
    assert!(matrix.can_write("agent:main", Capability::ReplyFirst));
    assert!(
        !matrix.can_write("agent:main", Capability::AssignWork),
        "未持有的能力必拒"
    );
}

/// 验收（第 7 步）：fail-closed——未知主体写与读全拒。
#[test]
fn unknown_principal_is_fail_closed() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:main",
        vec![Capability::ReplyFirst],
        VisScope::Public,
    )])
    .expect("成对策略必过");
    assert!(!matrix.can_write("agent:ghost", Capability::ReplyFirst));
    assert_eq!(matrix.sees_of("agent:ghost"), None);
    assert!(!matrix.can_see("agent:ghost", "agent:main", VisScope::Shared));
    // Public 是唯一例外：它的语义就是任何人可见。
    assert!(matrix.can_see("agent:ghost", "agent:main", VisScope::Public));
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
            "agent:main",
            vec![Capability::ReplyFirst],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "agent:worker",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("成对策略必过");
    // 别人的 thread_private 内容：worker 看不到 main 的，main 也看不到 worker 的。
    assert!(!matrix.can_see("agent:worker", "agent:main", VisScope::ThreadPrivate));
    assert!(!matrix.can_see("agent:main", "agent:worker", VisScope::ThreadPrivate));
    // 本人可见。
    assert!(matrix.can_see("agent:main", "agent:main", VisScope::ThreadPrivate));
    // 提权必须显式：shared 对矩阵内主体开放。
    assert!(matrix.can_see("agent:worker", "agent:main", VisScope::Shared));
}

/// 与 S1/S2 彩排链路对接的形状预演：主体对**事件切片**的可见子集为空 ⇒ 越界读取 0。
#[test]
fn foreign_principal_reads_nothing_from_a_private_thread() {
    use crate::symbio_core::event::{Event, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
    use crate::symbio_core::store::{EventStore, Store};

    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "agent:main",
            vec![Capability::ReplyFirst],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "agent:intruder",
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
                "agent:main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    // thread_private：事件只对**产生它的主体**可见。
    let visible: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("agent:intruder", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(visible.len(), 0, "越界读取必须为 0：{:?}", visible);
    // 本人可见自己产出的事件（口径=按事件作者；「线程参与者」模型随 S4 thread
    // 实体落地——user 发言的共享归属是 thread 的属性，不是 author 的）。
    let own: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("agent:main", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(own.len(), 1, "main 可见自己的 final；user 消息归 user");
    assert_eq!(own[0].kind, EVENT_ASSISTANT_FINAL);
}
