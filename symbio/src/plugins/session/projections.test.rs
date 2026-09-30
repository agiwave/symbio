//! 会话投影登记自测 —— 三个既有纯函数经适配器的**双跑一致**与形状正确。

use crate::symbio_core::{
    projection_run, Fact, FactKind, FactPrincipal, ProjectionInput, FACT_NONE_SEQ,
};

/// 造一条"消息类"事实：payload 形状与 `fact_log` 派生的一致。
fn msg_fact(seq: u64, id: &str, role: &str, ty: Option<&str>) -> Fact {
    msg_fact_as("s1", seq, id, role, ty)
}

/// 同上，但指定 principal（多会话用例需要）。
fn msg_fact_as(principal: &str, seq: u64, id: &str, role: &str, ty: Option<&str>) -> Fact {
    Fact {
        seq,
        kind: if role == "User" {
            FactKind::TurnUserMessage
        } else {
            FactKind::TurnAssistantFinal
        },
        principal: FactPrincipal::new(principal),
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
    for name in [
        "session.snapshot",
        "session.display",
        "session.checkpoint",
        "memory.recall",
    ] {
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

/// **A4 双跑一致**：四个投影各跑两次，逐字节相同。
#[test]
fn projections_are_deterministic() {
    let facts = sample_facts();
    let input = ProjectionInput::new(&facts, 1_234_567);
    for name in [
        "session.snapshot",
        "session.display",
        "session.checkpoint",
        "memory.recall",
    ] {
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

/// 空事实：四个投影都不 panic，返回空/零结构。
#[test]
fn projections_on_empty_facts() {
    let facts: Vec<Fact> = Vec::new();
    let input = ProjectionInput::new(&facts, 0);
    for name in [
        "session.snapshot",
        "session.display",
        "session.checkpoint",
        "memory.recall",
    ] {
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

// ==================== memory.recall（B4 / S06）====================

/// 造一条记忆类事实（`memory.*` 格子，payload 不必是消息形状）。
fn memory_fact(seq: u64, kind: FactKind) -> Fact {
    memory_fact_as("s1", seq, kind)
}

/// 同上，但指定 principal。
fn memory_fact_as(principal: &str, seq: u64, kind: FactKind) -> Fact {
    Fact {
        seq,
        kind,
        principal: FactPrincipal::new(principal),
        caused_by: None,
        at_ms: seq as i64 * 10,
        payload: serde_json::json!({ "note": "encoded" }),
    }
}

/// **平凡值**：没有任何 `memory.*` 事实 ⇒ `trivial = true`（退化成只看当前窗口）。
#[test]
fn recall_is_trivial_without_memory_facts() {
    let facts = sample_facts();
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("memory.recall", &input).unwrap();
    assert!(v.trivial, "无记忆事实时 recall 必须是平凡值");
    assert_eq!(v.value["memory_verbs"].as_array().unwrap().len(), 0);
    // 窗口内事实仍被列为候选（"只看当前窗口"的那一半）
    assert!(v.value["candidates"].as_u64().unwrap() > 0);
}

/// **完整路径**：有 `memory.*` 事实 ⇒ `trivial = false`，且动词列被折出。
#[test]
fn recall_lists_memory_verbs() {
    let mut facts = sample_facts();
    facts.push(memory_fact(5, FactKind::MemoryEncoded));
    facts.push(memory_fact(6, FactKind::MemoryRecalled));
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("memory.recall", &input).unwrap();
    assert!(!v.trivial, "有记忆事实时不应是平凡值");
    let verbs: Vec<&str> = v.value["memory_verbs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(verbs.contains(&"memory.encoded"));
    assert!(verbs.contains(&"memory.recalled"));
    // 候选按 seq 严格递增
    let seqs: Vec<u64> = v.value["candidate_seqs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_u64())
        .collect();
    for w in seqs.windows(2) {
        assert!(w[0] < w[1], "候选 seq 必须严格递增");
    }
}

/// 窗口折叠：超过 8 轮的会话，`window_from_seq` 指向第（总轮数-8）轮的用户事实。
#[test]
fn recall_window_keeps_last_turns() {
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
    let v = projection_run("memory.recall", &input).unwrap();
    // 第 3 个 user 事实的 seq = 5（i=2 → 2*2+1）；窗口从它起
    assert_eq!(v.value["window_from_seq"], 5);
    // 窗口内 = 8 轮 × 2 条 = 16；无 memory 事实 ⇒ 候选恰为这些
    assert_eq!(v.value["candidates"], 16);
    // 单会话：windows 数组恰一行，且与兼容标量一致
    let windows = v.value["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0]["principal"], "s1");
    assert_eq!(windows[0]["window_from_seq"], 5);
}

// ==================== 多会话窗口（跨会话召回的核心语义）====================

/// 造 10 轮 user/assistant 事实，seq 从 `base` 起（模拟 retrieval 的会话高位编码）。
fn ten_turns(principal: &str, base: u64) -> Vec<Fact> {
    let mut facts = Vec::new();
    for i in 0..10u64 {
        facts.push(msg_fact_as(
            principal,
            base + i * 2 + 1,
            &format!("{principal}-u{i}"),
            "User",
            Some("Text"),
        ));
        facts.push(msg_fact_as(
            principal,
            base + i * 2 + 2,
            &format!("{principal}-a{i}"),
            "Assistant",
            Some("Text"),
        ));
    }
    facts
}

/// **跨会话窗口**：两个会话各有 10 轮，合并后每个主体的窗口必须各算各的。
///
/// 旧实现全局取尾部 8 轮 ⇒ 窗口只罩住 seq 更大的会话（字典序靠后者），
/// 另一会话的全部事实落窗。本用例在旧实现下必失败（锁住修复）。
#[test]
fn recall_windows_are_per_principal() {
    const B2: u64 = 1 << 40;
    let mut facts = ten_turns("s1", 0); // seq 1..=20
    facts.extend(ten_turns("s2", B2)); // seq 2^40+1 ..= 2^40+20
    facts.push(memory_fact_as("s1", 21, FactKind::MemoryEncoded));
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("memory.recall", &input).unwrap();
    assert!(!v.trivial);

    // windows：两行，按 principal 字典序，各行下界 = 自己的第 3 轮用户事实
    let windows = v.value["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0]["principal"], "s1");
    assert_eq!(windows[0]["window_from_seq"], 5);
    assert_eq!(windows[1]["principal"], "s2");
    assert_eq!(windows[1]["window_from_seq"], B2 + 5);

    // 兼容标量 = 各主体下界的最小值
    assert_eq!(v.value["window_from_seq"], 5);

    // 两个会话的窗口内事实都在候选里（跨会话召回成立）
    let seqs: Vec<u64> = v.value["candidate_seqs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_u64())
        .collect();
    assert!(seqs.contains(&5), "s1 的窗口起点必须在候选中");
    assert!(seqs.contains(&(B2 + 5)), "s2 的窗口起点必须在候选中");
    // 各自窗口外的前两轮不在候选中
    assert!(!seqs.contains(&1), "s1 窗口外的事实不应在候选中");
    assert!(!seqs.contains(&(B2 + 1)), "s2 窗口外的事实不应在候选中");
    // 记忆事实永远在候选中
    assert!(seqs.contains(&21));

    // **A4**：多会话输入下双跑逐字节一致
    let b = projection_run("memory.recall", &input).unwrap();
    assert_eq!(
        serde_json::to_string(&v).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "多会话下双跑必须逐字节相同"
    );
}

/// 没有用户事实的主体取哨兵下界（`FACT_NONE_SEQ` = 0）⇒ 其事实全部入选。
///
/// 与旧全局语义同形：单会话无用户轮时窗口为"无下界"。
#[test]
fn recall_principal_without_user_turns_is_fully_in_window() {
    let facts = vec![
        Fact {
            seq: 1,
            kind: FactKind::SystemHealth,
            principal: FactPrincipal::new("sys"),
            caused_by: None,
            at_ms: 0,
            payload: serde_json::json!({}),
        },
        memory_fact_as("sys", 2, FactKind::MemoryEncoded),
    ];
    let input = ProjectionInput::new(&facts, 0);
    let v = projection_run("memory.recall", &input).unwrap();
    let windows = v.value["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0]["principal"], "sys");
    assert_eq!(windows[0]["window_from_seq"], FACT_NONE_SEQ);
    // 非记忆事实 seq=1 >= 0 ⇒ 入选
    let seqs: Vec<u64> = v.value["candidate_seqs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_u64())
        .collect();
    assert!(seqs.contains(&1));
}
