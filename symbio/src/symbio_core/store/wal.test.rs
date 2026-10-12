//! `wal` 单测 —— S4 两步的出口判据（[roadmap/S05 §6](../../../../docs/stage/ST-005-长会话与断点恢复.md "ST-005-长会话与断点恢复")
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

/// 批 0 反向用例（[plan/11 §3 批 0](../../../../docs/plan/PL-009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") 判据）：
/// 两个写者交错 ⇒ 第二个拿不到写者令牌、`append` 返回 `NotTheWriter`，且**盘上零丢失**。
///
/// 这是本批的核心：在此之前「单写者」只由进程内 `RwLock` 保证，第二个写者会静默地
/// 写出重复 seq——错误形态（`NotTheWriter`）早已定义，却**从不可达**。
#[test]
fn second_writer_gets_not_the_writer_and_nothing_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");

    let writer = EventWalStore::open(&path).unwrap();
    writer
        .append(ev(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "hi",
        ))
        .unwrap();

    // 第二个写者：同一进程、同一文件。令牌已被占用 ⇒ 降级为非写者。
    let intruder = EventWalStore::open(&path).unwrap();
    assert_eq!(
        intruder.append(ev(
            "u1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            1,
            "hi"
        )),
        Err(AppendError::NotTheWriter),
        "第二个写者必须拿到 NotTheWriter，而不是静默写出重复 seq"
    );
    // 降级不是失败：读面照常。
    assert_eq!(intruder.head().value(), 1, "非写者仍可读（降级 ≠ 报错）");

    // 持令牌者继续写，不受干扰。
    writer
        .append(ev(
            "f0",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "yo",
        ))
        .unwrap();
    assert_eq!(writer.head().value(), 2);
    drop(intruder);
    drop(writer);

    // 盘上零丢失、无重号。
    let reopened = EventWalStore::open(&path).unwrap();
    let ids: Vec<String> = reopened
        .range(Seq::new(0))
        .iter()
        .map(|e| e.event_id().to_string())
        .collect();
    assert_eq!(ids, vec!["u0".to_string(), "f0".to_string()], "盘上零丢失");
    assert_eq!(reopened.head().value(), 2, "seq 无跳号、无重号");
}

/// 读方在类型上就不该写：[`EventWalStore::open_readonly`] 从不取令牌 ⇒ 写入口恒关闭。
#[test]
fn readonly_store_refuses_to_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
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
    }
    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(ro.head().value(), 1, "读方看得见已提交的事实");
    assert_eq!(
        ro.append(ev(
            "u1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            1,
            "hi"
        )),
        Err(AppendError::NotTheWriter),
        "只读档的写入口必须关闭"
    );
}

/// 令牌随句柄关闭自动释放（不是粘死的）：前一个写者 drop 之后，新的 `open` 成为写者，
/// 且它的 `head` 来自**重放**而不是从 0 重新计数——否则接手者会重号。
#[test]
fn writer_token_is_released_when_the_store_drops() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");

    let first = EventWalStore::open(&path).unwrap();
    first
        .append(ev(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "hi",
        ))
        .unwrap();
    // 令牌在手上时，第二个写者进不来。
    assert_eq!(
        EventWalStore::open(&path).unwrap().append(ev(
            "u1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            1,
            "hi"
        )),
        Err(AppendError::NotTheWriter),
        "令牌未释放前，第二个写者仍被拒"
    );
    drop(first);

    // 令牌释放 ⇒ 下一个写者接手。
    let second = EventWalStore::open(&path).unwrap();
    assert_eq!(
        second.head().value(),
        1,
        "接手者的 head 来自重放，不是从 0 起"
    );
    second
        .append(ev(
            "u1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            1,
            "hi",
        ))
        .unwrap();
    assert_eq!(second.head().value(), 2, "接手后继续单调递增");
}

// ── 段归档（S2.7-b2）：被取代的前缀移入归档段，live 从 cut 起 ──────────────

/// 造 `n` 条事件（号 0..n）并返回（临时目录, WAL 路径, 持令牌的实例）。
fn seeded(n: usize) -> (tempfile::TempDir, std::path::PathBuf, EventWalStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2-events.wal");
    let store = EventWalStore::open(&path).unwrap();
    for i in 0..n {
        store
            .append(ev(
                &format!("e{i}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                i as u64,
                &format!("t{i}"),
            ))
            .unwrap();
    }
    (dir, path, store)
}

fn live_seqs(store: &EventWalStore) -> Vec<u64> {
    store
        .range(Seq::new(0))
        .iter()
        .filter_map(|e| e.seq().map(Seq::value))
        .collect()
}

/// 段名带首末号、落在 `WAL_ARCHIVE_SUBDIR` 下——「这段覆盖哪一号区间」在文件名上
/// 直接可读。
fn segment(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    dir.path().join(WAL_ARCHIVE_SUBDIR).join(name)
}

/// 前缀进归档段、尾部留 live；`head` 是「下一个号」不因归档倒退。
#[test]
fn archive_before_moves_the_superseded_prefix_into_a_segment() {
    let (dir, _path, store) = seeded(12);
    assert_eq!(store.archive_before(Seq::new(10)).unwrap(), Some(10));
    assert_eq!(store.len(), 2, "live 只剩尾部两条");
    assert_eq!(live_seqs(&store), vec![10, 11], "live 的号从 cut 起");
    assert_eq!(store.head().value(), 12, "head 是「下一个号」，不倒退");

    let seg = std::fs::read_to_string(segment(&dir, "v2-events.wal.0-9")).unwrap();
    assert_eq!(seg.lines().count(), 10, "归档段**只装被取代的前缀**");
    assert!(seg.lines().next().unwrap().contains("\"e0\""));
    assert!(seg.lines().last().unwrap().contains("\"e9\""));
}

/// 归档**不回收号**：新事件接着 `max_seq + 1` 走，不与 live 里的号相撞（N2）。
///
/// 这条钉的是 `build` 的 head 不变量：若 `head = events.len()`，归档后 live 只剩
/// 两条 ⇒ 下一条分配 seq 2，与既有的 10 撞号且不单调——而**没有任何东西会红**。
#[test]
fn appending_after_archival_continues_the_number_space() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();
    let s = store
        .append(ev(
            "e12",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            12,
            "t12",
        ))
        .unwrap();
    assert_eq!(s.value(), 12, "接着号空间，不与 live 里的 10 / 11 撞号");
    assert_eq!(store.head().value(), 13);
    assert_eq!(live_seqs(&store), vec![10, 11, 12], "单调递增");
}

/// **零回退损失的依据**：归档段 + live 逐字节等于原文件（段与段不重叠）。
#[test]
fn archive_segment_plus_live_equals_the_original_stream() {
    let (dir, path, store) = seeded(12);
    let original = std::fs::read_to_string(&path).unwrap();
    store.archive_before(Seq::new(10)).unwrap();
    let seg = std::fs::read_to_string(segment(&dir, "v2-events.wal.0-9")).unwrap();
    let live = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        format!("{seg}{live}"),
        original,
        "拼接即原 WAL：回退时把各段与 live 依次接起来就还原了"
    );
}

/// 两次归档 ⇒ 两个**不重叠**的段，拼接仍无损。
#[test]
fn two_archivals_leave_two_disjoint_segments() {
    let (dir, path, store) = seeded(12);
    let original = std::fs::read_to_string(&path).unwrap();
    assert_eq!(store.archive_before(Seq::new(4)).unwrap(), Some(4));
    assert_eq!(store.archive_before(Seq::new(10)).unwrap(), Some(6));
    let a = std::fs::read_to_string(segment(&dir, "v2-events.wal.0-3")).unwrap();
    let b = std::fs::read_to_string(segment(&dir, "v2-events.wal.4-9")).unwrap();
    let live = std::fs::read_to_string(&path).unwrap();
    assert_eq!(a.lines().count(), 4);
    assert_eq!(b.lines().count(), 6);
    assert_eq!(live.lines().count(), 2);
    assert_eq!(
        format!("{a}{b}{live}"),
        original,
        "三段拼接仍逐字节等于原文件"
    );
}

/// `cut` 落在第一条上 ⇒ 没有可归档的前缀：**不是失败，是「这次不归档」**，
/// 且不留下空目录。
#[test]
fn a_cut_at_the_first_event_archives_nothing() {
    let (dir, _path, store) = seeded(3);
    assert_eq!(store.archive_before(Seq::new(0)).unwrap(), None);
    assert_eq!(store.len(), 3, "一条都没动");
    assert!(
        !dir.path().join(WAL_ARCHIVE_SUBDIR).exists(),
        "没归档就不该建目录（空目录是「发生过归档」的假证据）"
    );
}

/// 归档掉全部事件：live 变**空文件**（不是删除），号空间照旧。
#[test]
fn archiving_past_the_last_event_empties_the_live_file_without_losing_the_numbers() {
    let (dir, path, store) = seeded(4);
    assert_eq!(store.archive_before(Seq::new(99)).unwrap(), Some(4));
    assert_eq!(store.len(), 0);
    assert!(store.range(Seq::new(0)).is_empty());
    assert_eq!(store.head().value(), 4, "号空间不动");
    assert_eq!(
        std::fs::read_to_string(segment(&dir, "v2-events.wal.0-3"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "",
        "live 是空文件而不是被删除：文件在不在是「事实源存不存在」的信号"
    );
}

/// **只读实例从不归档**：它连写者令牌都没有（读方在类型上就不该动盘）。
#[test]
fn a_readonly_store_never_archives() {
    let (dir, path, store) = seeded(4);
    drop(store);
    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(ro.archive_before(Seq::new(2)).unwrap(), None);
    assert_eq!(ro.len(), 4, "只读实例不动任何东西");
    assert!(!dir.path().join(WAL_ARCHIVE_SUBDIR).exists());
}

/// 归档后重开：只看得见 live 尾部，`head` 从 `max_seq + 1` 续上。
#[test]
fn reopening_after_archival_sees_only_the_live_tail() {
    let (_dir, path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();
    drop(store);
    let re = EventWalStore::open(&path).unwrap();
    assert_eq!(re.len(), 2);
    assert_eq!(live_seqs(&re), vec![10, 11]);
    assert_eq!(
        re.head().value(),
        12,
        "重开后 head 从盘上的 max_seq + 1 续上——按条数算会倒退成 2"
    );
}

// ── 完整事件史（`history`）：归档段 + live ──────────────────────────────────

/// `history()` = 归档段 + live，按号升序；`range()` 只给 live 窗口。
///
/// 这条钉的是**两个读口的分工**：累计口径的读者（不变量 C1 / SLO / 声誉 / 成本 /
/// 记忆 / 任务状态）必须看得见被归档的前缀，否则压缩后**静默**少算。
#[test]
fn history_unions_archives_with_live_in_seq_order() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();

    // live 窗口只是尾部两条（`range` 的口径）。
    assert_eq!(live_seqs(&store), vec![10, 11]);

    // 完整史是原样的 12 条（逐条不重不漏——ADR-053 迁移表 b2 行的「零回退损失」）。
    let all = store.history();
    assert_eq!(
        all.iter()
            .filter_map(|e| e.seq().map(Seq::value))
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<u64>>(),
        "归档段 + live 恰好还原完整号空间"
    );
    assert_eq!(all[0].event_id, "e0", "首条来自归档段");
    assert_eq!(all[11].event_id, "e11", "末条来自 live");
}

/// 没压过 ⇒ `history()` 与 `range()` 逐条相同（本方法不引入第二份口径）。
#[test]
fn history_without_archival_equals_range() {
    let (_dir, _path, store) = seeded(5);
    let by_history: Vec<String> = store.history().iter().map(|e| e.event_id.clone()).collect();
    let by_range: Vec<String> = store
        .range(Seq::new(0))
        .iter()
        .map(|e| e.event_id.clone())
        .collect();
    assert_eq!(by_history, by_range);
}

/// 两次归档 ⇒ 两个段 + live，仍还原完整史（段与段不重叠）。
#[test]
fn history_spans_two_disjoint_segments() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(4)).unwrap();
    store.archive_before(Seq::new(9)).unwrap();

    let all = store.history();
    assert_eq!(
        all.iter()
            .filter_map(|e| e.seq().map(Seq::value))
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<u64>>(),
        "两段 + live 拼接后无重无漏"
    );
}

/// 只读实例（不持令牌）也能读完整史：归档段的读不依赖写者身份。
#[test]
fn history_is_readable_from_a_readonly_instance() {
    let (_dir, path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();
    drop(store);

    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(ro.len(), 2, "live 只有尾部");
    assert_eq!(ro.history().len(), 12, "完整史把归档段一并读回来");
}
