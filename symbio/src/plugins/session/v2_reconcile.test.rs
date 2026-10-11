//! `v2_reconcile` 的判据（轨 S 第一步）。
//!
//! 这个模块的失效形态是**反的**：它不生产正确性，它生产**信号**。坏了的样子不是
//! 「会话读错」，而是「该报的不报 / 不该报的乱报」——后者更subtle：它会把正常的
//! 时序差（流式期间 WAL 已开轮、落盘还没写）报成不一致，于是这个诊断天天响，
//! 三天后没人再看他。所以这里逐条钉三件事：
//!
//! 1. **一致 ⇒ 一声不吭**（平凡值不能有噪音）；
//! 2. **真不一致 ⇒ 一定报**（含"条数不同"这种只看长度会发现不了的形态）；
//! 3. **时序差 ⇒ 不报**（在途版本参与还原，见 `reconcile_sources` 的注释）。

use super::{reconcile, record, Reconciliation};
use crate::symbio_core::chat_message::{ChatMessage, MessageContent, MessageRole};
use crate::symbio_core::{Entity, Event, Verb};
use crate::symbio_core::{EventWalStore, Seq, Store, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};

/// 一条落库消息（对账眼里只有 role + 正文）。
fn msg(role: MessageRole, text: &str) -> ChatMessage {
    ChatMessage {
        id: crate::symbio_core::llm_short_id(),
        role: Some(role),
        content: Some(MessageContent::Text(text.into())),
        ..Default::default()
    }
}

/// 造一个临时会话目录，返回 `(dir, wal_path)`。
fn tmp(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("symbio-reconcile-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join(super::super::paths::V2_WAL_FILE);
    (dir, wal)
}

/// 手写 legacy 侧 `messages.json`（切源后生产已停写，只有 v1 遗留会话才有）。
fn seed_legacy(dir: &std::path::Path, msgs: &[ChatMessage]) {
    let json = serde_json::json!({ "messages": msgs });
    std::fs::write(
        dir.join(crate::plugins::session::store::MESSAGES_FILE_NAME),
        serde_json::to_string(&json).unwrap(),
    )
    .expect("写 messages.json");
}

/// 往 WAL 里写一段对话（user + assistant final）。
fn seed_wal(wal: &std::path::Path, turns: &[(&str, &str)]) {
    let tag = "e";
    let store = EventWalStore::open(wal).expect("open wal");
    for (i, (u, a)) in turns.iter().enumerate() {
        let t = i as u64;
        let s = store
            .append(
                Event::pending(
                    format!("u-{tag}-{t}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    t,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": u })),
            )
            .expect("user")
            .value();
        store
            .append(
                Event::pending(
                    format!("f-{tag}-{t}"),
                    EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    t,
                    "agent:main",
                )
                .with_produced_by(s)
                .with_payload(serde_json::json!({ "text": a })),
            )
            .expect("final");
    }
}

// ── ① 一致 ⇒ 一声不吭 ─────────────────────────────────────────────────

#[test]
fn identical_sources_are_consistent() {
    let (dir, wal) = tmp("same");
    seed_wal(&wal, &[("问 A", "答 A"), ("问 B", "答 B")]);
    let store_side = vec![
        msg(MessageRole::User, "问 A"),
        msg(MessageRole::Assistant, "答 A"),
        msg(MessageRole::User, "问 B"),
        msg(MessageRole::Assistant, "答 B"),
    ];
    seed_legacy(&dir, &store_side);

    let r = reconcile(&dir);
    assert!(r.is_consistent(), "同内容必须判一致：{}", r.summary());
    assert_eq!(r.store_msgs, 4);
    assert_eq!(r.wal_entries, 4);
    assert_eq!(r.mismatches, 0);
    // record() 在一致时**不落事件**（没什么可记的）
    assert!(!record(&dir, "s1", &r), "一致不该落对账事件");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ② 真不一致 ⇒ 一定报 ───────────────────────────────────────────────

#[test]
fn a_changed_line_is_reported() {
    let (dir, wal) = tmp("changed");
    seed_wal(&wal, &[("问 A", "答 A")]);
    // 落库镜像里那句"答 A"被改成了别的（同一句话两个版本 = 真分歧）
    let store_side = vec![
        msg(MessageRole::User, "问 A"),
        msg(MessageRole::Assistant, "答 A'"),
    ];
    seed_legacy(&dir, &store_side);

    let r = reconcile(&dir);
    assert!(!r.is_consistent(), "正文不同必须判不一致");
    assert_eq!(r.mismatches, 1, "逐条比，差的那一处要数出来");
    assert!(record(&dir, "s1", &r), "不一致 ⇒ 落一条事件");

    // 落下去的那条真在 WAL 里（判据要能读它——这正是"落事件而非打日志"的理由）
    let events = EventWalStore::open(&wal)
        .expect("reopen")
        .range(Seq::new(0));
    let rec = events
        .iter()
        .find(|e| e.kind == crate::symbio_core::EVENT_SESSION_RECONCILED)
        .expect("对账事件已落格");
    assert_eq!(
        rec.entity,
        Entity::System,
        "落在 System（诊断，不开新实体）"
    );
    assert_eq!(rec.payload["mismatches"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_line_is_reported_too() {
    let (dir, wal) = tmp("missing");
    seed_wal(&wal, &[("问 A", "答 A"), ("问 B", "答 B")]);
    // 落库镜像少了一整轮（WAL 有 4 条、store 只有 2 条）
    let store_side = vec![
        msg(MessageRole::User, "问 A"),
        msg(MessageRole::Assistant, "答 A"),
    ];
    seed_legacy(&dir, &store_side);

    let r = reconcile(&dir);
    assert!(!r.is_consistent(), "条数不同必须判不一致");
    assert_eq!(
        r.mismatches, 2,
        "多出来的那段每一格都算一处——只比前 N 条会让「WAL 少一段」报 0"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ②′ 空正文的呈现态占位不算分歧（e2e 探针照出来的那一格）──────────

#[test]
fn empty_placeholder_nodes_are_not_a_mismatch() {
    let (dir, wal) = tmp("placeholder");
    seed_wal(&wal, &[("问 A", "答 A")]);
    // messages.json 的真实形状：每轮多一条**空正文的 assistant**（turn 容器 /
    // 流式占位）。WAL 侧没有对应事件——ADR-049：呈现态不入格、也不是对账对象。
    //
    // ⚠️ 这一条是 e2e 探针照出来的：第一版对账拿全部消息比，于是 store 6 条 vs
    // wal 4 条， diagnostic 在真跑一步就红。**探针的价值正在于此**——它把
    // "我以为两源同形状"变成了"实测差两条"。
    let store_side = vec![
        msg(MessageRole::User, "问 A"),
        msg(MessageRole::Assistant, ""),
        msg(MessageRole::Assistant, "答 A"),
    ];
    seed_legacy(&dir, &store_side);

    let r = reconcile(&dir);
    assert_eq!(
        r.store_msgs, 2,
        "空正文那条不计入（对账眼里它有话才算一句话）"
    );
    assert_eq!(r.wal_entries, 2);
    assert!(r.is_consistent(), "呈现态占位不该判成分歧：{}", r.summary());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── ③ 时序差 ⇒ 不报 ───────────────────────────────────────────────────

#[test]
fn a_missing_wal_is_not_a_mismatch() {
    let (dir, _wal) = tmp("nowal");
    seed_legacy(&dir, &[msg(MessageRole::User, "问 A")]);
    let r = reconcile(&dir);

    assert!(r.wal_missing, "WAL 不在要单独标（还没跑过任何一轮）");
    assert_eq!(r.mismatches, 0, "没有第二源可比 ⇒ 不算不一致");
    assert!(
        r.is_consistent(),
        "WAL 不在时判「一致」——它不该让诊断天天响"
    );
    assert!(!record(&dir, "s1", &r), "WAL 不在不落事件");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 幂等：重复对账不产生第二份噪音 ─────────────────────────────────────

#[test]
fn recording_twice_is_idempotent() {
    let (dir, wal) = tmp("idem");
    seed_wal(&wal, &[("问 A", "答 A")]);
    seed_legacy(&dir, &[msg(MessageRole::Assistant, "答 A'")]);
    let r = reconcile(&dir);

    assert!(record(&dir, "s1", &r), "第一次落");
    assert!(
        !record(&dir, "s1", &r),
        "第二次撞幂等键必须静默跳过——重复记录一条诊断没有信息量"
    );
    let count = EventWalStore::open(&wal)
        .expect("reopen")
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == crate::symbio_core::EVENT_SESSION_RECONCILED)
        .count();
    assert_eq!(count, 1, "WAL 里只有一条对账记录");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 平凡值：两个都空 ──────────────────────────────────────────────────

#[test]
fn both_empty_is_consistent() {
    let (dir, wal) = tmp("empty");
    seed_wal(&wal, &[]);
    seed_legacy(&dir, &[]);
    let r = reconcile(&dir);
    assert!(r.is_consistent(), "两个源都空 ⇒ 一致（有据的零，不是错误）");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 切源后常态：`messages.json` 不在 = 无第二源可比，不出声 ─────────────

#[test]
fn absent_legacy_messages_is_not_a_mismatch() {
    let (dir, wal) = tmp("nolegacy");
    seed_wal(&wal, &[("问 A", "答 A"), ("问 B", "答 B")]);
    // 切源后**生产不再写** messages.json ⇒ 它不存在。此时读侧看到的 WAL 内容是
    // 唯一真源，没有任何"第二份存储"跟它打架 ⇒ 判一致，绝不能让诊断天天误报。
    let r = reconcile(&dir);
    assert!(r.store_missing, "legacy 不在要单独标（切源后的常态）");
    assert_eq!(r.mismatches, 0, "没有第二源可比 ⇒ 不算不一致");
    assert!(r.is_consistent(), "legacy 不在时判「一致」");
    assert!(
        !record(&dir, "s1", &r),
        "legacy 不在不落事件——这正是 I2 误报的修复点"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── 反向：Reconciliation 的判据本身要能反向 ────────────────────────────

#[test]
fn is_consistent_requires_both_count_and_content_to_match() {
    // 内容全同但条数不同 ⇒ 不能判一致（这是上一条 `a_missing_line...` 的程序化表述）
    let r = Reconciliation {
        store_msgs: 2,
        wal_entries: 4,
        mismatches: 0,
        wal_missing: false,
        store_missing: false,
    };
    assert!(!r.is_consistent(), "条数不同即使逐条都相同也不算一致");
    // WAL 不在时恒判一致（否则还没跑过轮的会话会让它天天响）
    let w = Reconciliation {
        store_msgs: 9,
        wal_entries: 0,
        mismatches: 0,
        wal_missing: true,
        store_missing: false,
    };
    assert!(w.is_consistent());
    // legacy 不在时恒判一致（切源后的常态，同一条纪律的另一侧）
    let l = Reconciliation {
        store_msgs: 0,
        wal_entries: 9,
        mismatches: 0,
        wal_missing: false,
        store_missing: true,
    };
    assert!(l.is_consistent());
}
