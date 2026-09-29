//! 会话投影登记自测 —— 三个既有纯函数经适配器的**双跑一致**与形状正确。

use crate::symbio_core::{
    projection_run, Fact, FactKind, FactPrincipal, ProjectionInput, FACT_NONE_SEQ,
};

/// 造一条"消息类"事实：payload 形状与 `fact_log` 派生的一致。
fn msg_fact(seq: u64, id: &str, role: &str, ty: Option<&str>) -> Fact {
    Fact {
        seq,
        kind: if role == "User" {
            FactKind::TurnUserMessage
        } else {
            FactKind::TurnAssistantFinal
        },
        principal: FactPrincipal::new("s1"),
        caused_by: None,
        at_ms: seq as i64 * 10,
        payload: serde_json::json!({
            "message_id": id,
            "role": role,
            "type": ty,
            "name": null,
            "status": null,
            "timestamp": seq as i64 * 10,
        }),
    }
}

/// 三个投影**全部已登记**（登记点齐备）。
#[test]
fn all_three_projections_registered() {
    for name in ["session.snapshot", "session.display", "session.checkpoint"] {
        assert!(
            crate::symbio_core::projection_has(name),
            "投影 {name} 应已登记"
        );
    }
}

/// 典型一轮对话：user → assistant ×2 轮。
fn sample_facts() -> Vec<Fact> {
    vec![
        msg_fact(1, "u1", "User", Some("Text")),
        msg_fact(2, "a1", "Assistant", Some("Text")),
        msg_fact(3, "u2", "User", Some("Text")),
        msg_fact(4, "a2", "Assistant", Some("Text")),
    ]
}

/// **A4 双跑一致**：三个投影各跑两次，逐字节相同。
#[test]
fn projections_are_deterministic() {
    let facts = sample_facts();
    let input = ProjectionInput::new(&facts, 1_234_567);
    for name in ["session.snapshot", "session.display", "session.checkpoint"] {
        let a = projection_run(name, &input).unwrap();
        let b = projection_run(name, &input).unwrap();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "{name} 双跑必须逐字节相同"
        );
    }
}

/// `session.snapshot`：窗口小于总轮数时只保留最近 N 轮。
#[test]
fn snapshot_projection_keeps_window() {
    // 造 10 轮 user/assistant（20 条）——超过窗口 8
    let mut facts = Vec::new();
    for i in 0..10u64 {
        facts.push(msg_fact(i * 2 + 1, &format!("u{i}"), "User", Some("Text")));
        facts.push(msg_fact(
            i * 2 + 2,
            &format!("a{i}"),
            "Assistant",
            Some("Text"),
        ));
    }
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("session.snapshot", &input).unwrap();
    assert!(!v.trivial, "正常路径不应是平凡值");
    assert_eq!(v.value["window_turns"], 8);
    assert_eq!(v.value["total"], 20);
    // 最近 8 轮 = 16 条，起点是 u2（第 3 个 user，索引 4）
    assert_eq!(v.value["kept"], 16);
    assert_eq!(v.value["kept_ids"][0], "u2");
}

/// `session.checkpoint`：切分点落在 User 边界（下标是偶数位置的 User）。
#[test]
fn checkpoint_projection_splits_on_user_boundary() {
    let facts = sample_facts();
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("session.checkpoint", &input).unwrap();
    // 只有 4 条、字符数小，切分点要么 0 要么是某个 User 下标
    let split = v.value["split"].as_u64().unwrap() as usize;
    assert!(split <= facts.len());
    if split > 0 {
        // 切分点处的消息应是 User（原函数只在 User 边界切）
        assert_eq!(facts[split].payload["role"], "User");
    }
}

/// 空事实：三个投影都不 panic，返回空/零结构。
#[test]
fn projections_on_empty_facts() {
    let facts: Vec<Fact> = Vec::new();
    let input = ProjectionInput::new(&facts, 0);
    for name in ["session.snapshot", "session.display", "session.checkpoint"] {
        let v = projection_run(name, &input).unwrap();
        assert_eq!(v.value["total"], 0, "{name} 空输入应有 total=0");
    }
}

/// `FACT_NONE_SEQ` 哨兵与投影输入共存（不误当消息）。
#[test]
fn none_seq_sentinel_is_not_a_message() {
    let facts = vec![Fact {
        seq: FACT_NONE_SEQ + 1,
        kind: FactKind::SystemHealth,
        principal: FactPrincipal::new("sys"),
        caused_by: None,
        at_ms: 0,
        payload: serde_json::json!({}), // 非消息载荷
    }];
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("session.snapshot", &input).unwrap();
    // 非消息事实被过滤 ⇒ total = 0
    assert_eq!(v.value["total"], 0);
}
