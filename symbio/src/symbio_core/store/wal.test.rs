//! `wal` 单测 —— S4 两步的出口判据（[roadmap/S05 §6](../../../../docs/plan/roadmap/S05-长会话与断点恢复.md)
//! 四条验收逐条落地）+ C11 / N2（崩溃恢复后投影 == 崩溃前投影）。

use super::*;
use crate::symbio_core::event::{Entity, Event, Verb, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE};
use crate::symbio_core::invariants::{check_all, unresolved_turns};
use crate::symbio_core::projection::checkpoint::{checkpoint, CheckpointState};
use crate::symbio_core::projection::turnstate::{turnstate, TurnState};
use crate::symbio_core::view::Budget;

/// 造一条 pending 事件（入库前无 seq；带溯源与正文载荷）。
fn ev(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64, text: &str) -> Event {
    Event::pending(id, kind, entity, verb, turn, "agent:main")
        .with_produced_by(0)
        .with_payload(serde_json::json!({ "text": text }))
}

/// 验收 1：写到一半杀进程 → 重启后 `head()` 与崩溃前最后一条**已提交**事件一致。
#[test]
fn crash_recovery_head_matches_last_committed_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        for i in 0..5 {
            store
                .append(ev(
                    &format!("e{i}"),
                    "user.message",
                    Entity::Turn,
                    Verb::Opened,
                    i as u64,
                    "hi",
                ))
                .unwrap();
        }
        // store 在此 drop（模拟杀进程）——每条 append 已落盘，全部已提交。
    }
    let recovered = EventWalStore::open(&path).unwrap();
    assert_eq!(recovered.head().value(), 5, "恢复后 head == 已提交条数");
    assert_eq!(recovered.len(), 5);
}

/// 验收 1 的撕裂行变体：进程死在半行 ⇒ 撕裂部分不算已提交。
#[test]
fn torn_tail_line_is_not_committed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        for i in 0..3 {
            store
                .append(ev(
                    &format!("e{i}"),
                    "user.message",
                    Entity::Turn,
                    Verb::Opened,
                    i as u64,
                    "hi",
                ))
                .unwrap();
        }
    }
    // 模拟写到一半被杀：追加半行 JSON。
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        use std::io::Write;
        f.write_all(b"{\"event_id\":\"e3\", \"pay").unwrap();
    }
    let recovered = EventWalStore::open(&path).unwrap();
    assert_eq!(recovered.head().value(), 3, "撕裂尾行不算已提交");
    // 恢复后继续写入：seq 从 3 继续，无跳号（文件已截断到提交边界）。
    recovered
        .append(ev(
            "e3",
            "user.message",
            Entity::Turn,
            Verb::Opened,
            3,
            "hi",
        ))
        .unwrap();
    assert_eq!(recovered.head().value(), 4);
    let again = EventWalStore::open(&path).unwrap();
    assert_eq!(again.head().value(), 4, "截断后重启不重放撕裂残片");
}

/// 验收 2 / C11 / N2：从 `seq=0` 重放得到的投影 == 崩溃前投影（逐字节）。
#[test]
fn recovery_projection_is_byte_identical_to_pre_crash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    // 崩溃前的完整对话（S01 闭环形状）。
    {
        let store = EventWalStore::open(&path).unwrap();
        store
            .append(ev(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "你好",
            ))
            .unwrap();
        store
            .append(ev(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "你好，我能做什么？",
            ))
            .unwrap();
        store
            .append(ev(
                "u1",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                1,
                "帮我查天气",
            ))
            .unwrap();
        store
            .append(ev(
                "f1",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                1,
                "今天晴，25 度。",
            ))
            .unwrap();
    }
    // 崩溃前的投影。
    let before_store = EventWalStore::open(&path).unwrap();
    let before = before_store.range(crate::symbio_core::event::Seq::new(0));
    let before_turn = turnstate().apply(&before, 0, Budget::generous());
    let before_cp = checkpoint().apply(&before, 0, Budget::generous());

    // 「重启」：全新实例（同一文件）。
    let after = EventWalStore::open(&path).unwrap();
    let snapshot = after.range(crate::symbio_core::event::Seq::new(0));
    let after_turn: TurnState = turnstate().apply(&snapshot, 0, Budget::generous()).value;
    let after_cp: CheckpointState = checkpoint().apply(&snapshot, 0, Budget::generous()).value;

    assert_eq!(
        before_turn.value, after_turn,
        "C11：恢复后 turnstate == 崩溃前（逐字节）"
    );
    assert_eq!(
        before_cp.value, after_cp,
        "C11：恢复后 checkpoint == 崩溃前（逐字节）"
    );
    // 恢复后 seq 仍单调（S05 §5：恢复后 seq 单调的 CI 断言）。
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
    assert!(unresolved_turns(&snapshot, false).is_empty());
    // 断点状态本身也在账上（4 条事实 ⇒ event_count = 4）。
    assert_eq!(after_cp.event_count, 4, "断点按事实条数计");
}

/// 验收 3：`store = memory` 平凡值——重启即丢，但系统仍能完成 S01 闭环。
#[test]
fn memory_store_trivial_value_still_completes_the_loop() {
    use crate::symbio_core::actors::{ActorSpec, Pattern};
    use crate::symbio_core::store::{EventStore, Store as _};

    // 「重启」= 新进程 = 全新 MemoryStore（事件为空）。
    let store = EventStore::new();
    assert!(store.is_empty(), "memory 重启即丢");
    // 但 S01 闭环照常完整运行。
    store
        .append(ev(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "你好，帮我看看",
        ))
        .unwrap();
    store
        .append(ev(
            "f0",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "我可以帮你查资料、跑任务、写东西。",
        ))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(check_all(&snapshot).is_empty());
    let v = turnstate().apply(&snapshot, 0, Budget::generous());
    assert!(v.value.settled() && v.value.final_text.is_some());
    // 平凡值锚仍是规则模式（零 LLM 也能跑完整闭环）。
    assert_eq!(ActorSpec::trivial("agent:main").pattern, Pattern::Decider);
}

/// 验收 4（反向）：故意丢掉最后一条已提交事件 → `head()` 必须不同
/// （证明「已提交」边界真实存在，不是恢复逻辑假绿）。
#[test]
fn dropping_last_committed_event_changes_head() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        for i in 0..3 {
            store
                .append(ev(
                    &format!("e{i}"),
                    "user.message",
                    Entity::Turn,
                    Verb::Opened,
                    i as u64,
                    "hi",
                ))
                .unwrap();
        }
    }
    assert_eq!(EventWalStore::open(&path).unwrap().head().value(), 3);
    // 丢掉最后一条：按行截断（模拟「最后一条未真正落盘」的崩法）。
    let content = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = content.lines().collect();
    lines.pop();
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    assert_eq!(
        EventWalStore::open(&path).unwrap().head().value(),
        2,
        "边界必须真实：丢一条，head 就得变"
    );
}

/// 幂等在恢复后依然成立：重放期间重建的 id 集合继续拦截重复。
#[test]
fn duplicate_is_rejected_after_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        store
            .append(ev(
                "e0",
                "user.message",
                Entity::Turn,
                Verb::Opened,
                0,
                "hi",
            ))
            .unwrap();
    }
    let recovered = EventWalStore::open(&path).unwrap();
    assert_eq!(
        recovered.append(ev(
            "e0",
            "user.message",
            Entity::Turn,
            Verb::Opened,
            0,
            "hi"
        )),
        Err(AppendError::Duplicate),
        "恢复后幂等键仍然有效"
    );
    assert_eq!(
        recovered.head().value(),
        1,
        "Duplicate 不消耗 seq（无跳号）"
    );
}

/// 断点状态可序列化且可回读（S05 §5：checkpoint 可序列化——序列化失败有信号）。
///
/// 往返直接走 JSON：断点是**派生视图**（恢复 = 重放 + 投影），没有「写回事件」
/// 这一步，所以序列化的对象就是状态本身。
#[test]
fn checkpoint_state_round_trips_through_json() {
    let dir = tempfile::tempdir().unwrap();
    let store = EventWalStore::open(dir.path().join("events.jsonl")).unwrap();
    store
        .append(ev(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "hi",
        ))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let cp: CheckpointState = checkpoint().apply(&snapshot, 0, Budget::generous()).value;
    let value = serde_json::to_value(&cp).expect("CheckpointState 必可序列化");
    assert_eq!(value["event_count"], 1, "断点状态按事实计数");
    let round: CheckpointState = serde_json::from_value(value).unwrap();
    assert_eq!(round, cp, "断点状态 JSON 往返必须无损");
}

/// 只读打开：撕裂尾行同样不算已提交，但**不改文件**——截断是写方的恢复语义。
///
/// 为什么必须分开：[`Store::append`] 用 append 模式（写点恒在当前 EOF），
/// 读方若在写方落一行的中途把它截掉，剩下的字节会接在新的 EOF 上 ⇒ 那一行
/// 静默损坏。读数口（`session/stats`）会在会话进行中随时打开同一个 WAL。
#[test]
fn open_readonly_drops_a_torn_tail_without_truncating_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        for i in 0..2 {
            store
                .append(ev(
                    &format!("e{i}"),
                    "user.message",
                    Entity::Turn,
                    Verb::Opened,
                    i as u64,
                    "hi",
                ))
                .unwrap();
        }
    }
    // 追加半行：模拟进程死在写到一半。
    let mut raw = std::fs::read_to_string(&path).unwrap();
    raw.push_str("{\"event_id\":\"e2\",\"ki");
    std::fs::write(&path, &raw).unwrap();
    let torn_len = std::fs::metadata(&path).unwrap().len();

    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(ro.len(), 2, "撕裂尾行不算已提交（同一提交边界）");
    assert_eq!(ro.head().value(), 2);
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        torn_len,
        "只读打开不许截断文件"
    );

    // 对照：写方打开才执行恢复截断——恢复语义归写方，读方只解析。
    let _rw = EventWalStore::open(&path).unwrap();
    assert!(
        std::fs::metadata(&path).unwrap().len() < torn_len,
        "写方打开照旧截断到最后一行完整记录"
    );
}

/// 只读打开遇上「还没有事实源」= 空仓，且**不创建文件**。
#[test]
fn open_readonly_on_a_missing_file_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(ro.len(), 0);
    assert_eq!(ro.head().value(), 0);
    assert!(
        !path.exists(),
        "读方不该为了读而写下东西——那会把「从没跑过」变成「跑过一轮空的」"
    );
}
