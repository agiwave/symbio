//! `wal` 单测 —— S4 两步的出口判据（[roadmap/S05 §6](../../../../../docs/009-task/125-长会话与断点恢复.md "T-125-长会话与断点恢复")
//! 四条验收逐条落地）+ C11 / N2（崩溃恢复后投影 == 崩溃前投影）。

use super::*;
use crate::symbio_core::Budget;
use crate::symbio_core::{check_all, unresolved_turns};
use crate::symbio_core::{checkpoint, CheckpointState};
use crate::symbio_core::{
    transcript, verify_prompt_fidelity, Entity, Event, PromptMessage, Verb, EVENT_ARTIFACT_ADDED,
    EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use crate::symbio_core::{turnstate, TurnState};

/// 造一条 pending 事件（入库前无 seq；带溯源与正文载荷）。
fn ev(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64, text: &str) -> Event {
    Event::pending(id, kind, entity, verb, turn, "session/main")
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
    let before = before_store.range(Seq::new(0));
    let before_turn = turnstate().apply(&before, 0, Budget::generous());
    let before_cp = checkpoint().apply(&before, 0, Budget::generous());

    // 「重启」：全新实例（同一文件）。
    let after = EventWalStore::open(&path).unwrap();
    let snapshot = after.range(Seq::new(0));
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
    use crate::symbio_core::{ActorSpec, EventStore, Pattern, Store as _};

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
    let snapshot = store.range(Seq::new(0));
    assert!(check_all(&snapshot).is_empty());
    let v = turnstate().apply(&snapshot, 0, Budget::generous());
    assert!(v.value.settled() && v.value.final_text.is_some());
    // 平凡值锚仍是规则模式（零 LLM 也能跑完整闭环）。
    assert_eq!(ActorSpec::trivial("session/main").pattern, Pattern::Decider);
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
    let snapshot = store.range(Seq::new(0));
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

/// 批 0 反向用例（[plan/11 §3 批 0](../../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") 判据）：
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
    // **读实现内部**的 live 段：`range` 归一后给的是完整史（含归档前缀），
    // 「哪一段在内存里」是实现自己的事，不再是公开读法（见 `Store::range` 文档）。
    store
        .inner
        .read()
        .expect("WalStore 锁中毒")
        .events
        .iter()
        .filter_map(|e| e.seq().map(Seq::value))
        .collect()
}

/// 段名带首末号、落在 `WAL_ARCHIVE_SUBDIR` 下——「这段覆盖哪一号区间」在文件名上
/// 直接可读。
fn segment(dir: &tempfile::TempDir, name: &str) -> std::path::PathBuf {
    archive_dir_of(dir.path()).join(name)
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
        !archive_dir_of(dir.path()).exists(),
        "没归档就不该建目录（空目录是「发生过归档」的假证据）"
    );
}

/// 归档掉全部事件：live 变**空文件**（不是删除），号空间照旧，读口照旧给完整史。
#[test]
fn archiving_past_the_last_event_empties_the_live_file_without_losing_the_numbers() {
    let (dir, path, store) = seeded(4);
    assert_eq!(store.archive_before(Seq::new(99)).unwrap(), Some(4));
    assert_eq!(store.len(), 0);
    assert_eq!(live_seqs(&store), Vec::<u64>::new(), "没被取代的那段空了");
    assert_eq!(
        store.range(Seq::new(0)).len(),
        4,
        "读口给的是完整史：**全归档不等于全丢**，那 4 条仍在归档段里，读者一条不少"
    );
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
    assert!(!archive_dir_of(dir.path()).exists());
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

// ── 一个读口的口径：归档段 + live ─────────────────────────────────────────

/// **一个读口**：`range(from)` 给的是完整史（归档段 + live，按号升序）。
///
/// 累计口径的读者（不变量 C1 / SLO / 声誉 / 成本 / 记忆 / 任务状态）必须看得见被
/// 归档的前缀，否则压缩后**静默**少算；消息 / 上下文类读者读的也是同一份完整史，
/// 「被取代」由折叠侧处理（`mutations_latest_snapshot` 从最近一份快照起折）。
/// 按布局分叉的第二个口（只给 live 窗口）会让「读哪个」取决于调用方是否知道
/// 「这条会话压缩过没有」——那是布局泄漏进抽象
/// （[plan/14 §2.2](../../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 推论 1）。
#[test]
fn range_unions_archives_with_live_in_seq_order() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();

    // live 段只是尾部两条——那是实现的内部形态，不再是公开读法。
    assert_eq!(live_seqs(&store), vec![10, 11]);

    // 完整史是原样的 12 条（逐条不重不漏——ADR-053 迁移表 b2 行的「零回退损失」）。
    let all = store.range(Seq::new(0));
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

/// `range(from)` 的 `from` 是**号**，不是 live 段里的下标（trait 契约 `[from, head)`）。
///
/// 归档把被取代的前缀移出 live ⇒ live 首号不再是 0。按下标切会把「从 seq 11 起」
/// 读成「从 live 的第 11 条起」= 空集，调用方要的那一条因此在压缩过的会话里
/// **静默**消失（P6-① 修的就是这一处；唯一的 `from > 0` 调用点在
/// `transcript/autonomous`，它取 `.next()`，压缩过才显形）。
#[test]
fn range_after_archival_selects_by_seq_not_by_position() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();
    assert_eq!(live_seqs(&store), vec![10, 11], "前置：live 是尾部两条");

    let seqs = |from: u64| -> Vec<u64> {
        store
            .range(Seq::new(from))
            .iter()
            .filter_map(|e| e.seq().map(Seq::value))
            .collect()
    };
    assert_eq!(seqs(11), vec![11], "从 seq 11 起 = 一条");
    assert_eq!(seqs(10), vec![10, 11], "从归档 cut 起 = 尾部两条");
    assert_eq!(
        seqs(9),
        vec![9, 10, 11],
        "**归档段也认号**：从 seq 9 起包括已进归档的那一条"
    );
    assert_eq!(
        seqs(12),
        Vec::<u64>::new(),
        "`[from, head)` 右开：head 起为空"
    );
}

/// 没压过 ⇒ `range(0)` 与 live 段逐条相同（归一不引入第二份内容，只是并掉两个口）。
#[test]
fn range_without_archival_is_the_live_segment() {
    let (_dir, _path, store) = seeded(5);
    let by_range: Vec<String> = store
        .range(Seq::new(0))
        .iter()
        .map(|e| e.event_id.clone())
        .collect();
    let in_live: Vec<String> = store
        .inner
        .read()
        .expect("WalStore 锁中毒")
        .events
        .iter()
        .map(|e| e.event_id.clone())
        .collect();
    assert_eq!(by_range, in_live);
}

/// 两次归档 ⇒ 两个段 + live，仍还原完整史（段与段不重叠）。
#[test]
fn range_spans_two_disjoint_segments() {
    let (_dir, _path, store) = seeded(12);
    store.archive_before(Seq::new(4)).unwrap();
    store.archive_before(Seq::new(9)).unwrap();

    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter()
            .filter_map(|e| e.seq().map(Seq::value))
            .collect::<Vec<_>>(),
        (0..12).collect::<Vec<u64>>(),
        "两段 + live 拼接后无重无漏"
    );
}

/// 本实例归档之后，缓存必须失效——否则「归一后的 `range` 含前缀」会读到自己刚写进去
/// 的那段的**旧**并集（少一段）。这条钉的是缓存与 `archive_before` 的联动。
#[test]
fn archiving_invalidates_the_archive_cache_of_this_instance() {
    let (_dir, _path, store) = seeded(12);
    assert_eq!(store.range(Seq::new(0)).len(), 12, "前置：没归档");
    store.archive_before(Seq::new(4)).unwrap();
    assert_eq!(
        store.range(Seq::new(0)).len(),
        12,
        "归档后完整史仍 12 条（缓存已作废，新段被读进来）"
    );
    store.archive_before(Seq::new(9)).unwrap();
    assert_eq!(store.range(Seq::new(0)).len(), 12, "第二次归档同样");
}

/// 只读实例（不持令牌）也能读完整史：归档段的读不依赖写者身份。
#[test]
fn range_is_readable_from_a_readonly_instance() {
    let (_dir, path, store) = seeded(12);
    store.archive_before(Seq::new(10)).unwrap();
    drop(store);

    let ro = EventWalStore::open_readonly(&path).unwrap();
    assert_eq!(live_seqs(&ro), vec![10, 11], "live 只有尾部");
    assert_eq!(ro.range(Seq::new(0)).len(), 12, "完整史把归档段一并读回来");
}

/// 生产路径：真落一格 `artifact.added` 再从**重开的 WAL** 复核双向完整性。
///
/// 判据住在会话侧而不是 core：它必须开一个真的 WAL，而 WAL 是会话的落事实实现；
/// core 只出让 `verify_prompt_fidelity` / `transcript` 这两个纯函数。
/// 走 WAL 而不是内存事件，是为了让「窗口裁剪 / 可见域过滤」在生产里经过的
/// 同一段代码也被覆盖到——内存切片绕过那一层。
#[test]
fn an_artifact_round_is_complete_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let wal = dir.path().join("v2-events.wal");

    let user = Event::pending(
        "u-0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "帮我回显", "tier": "deep", "id": "u-0" }));
    let artifact = Event::pending(
        "a-0-0",
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({ "tool": "vdfs_read", "text": "文件内容：hello" }));
    let assistant = Event::pending(
        "f-0",
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({
        "text": "回显完成",
        "model": "deep",
        "turn_id": "t-0",
        "text_id": "f-0-text",
    }));

    let store = EventWalStore::open(&wal).expect("open");
    let seq = store.append(user).expect("append user").value();
    store
        .append(artifact.with_produced_by(seq))
        .expect("append artifact");
    store
        .append(assistant.with_produced_by(seq))
        .expect("append final");
    drop(store);

    let reopened = EventWalStore::open(&wal).expect("reopen");
    let events = reopened.range(Seq::new(0));

    let view = transcript()
        .apply(&events, i64::MAX, Budget::generous())
        .value;
    let entries: Vec<PromptMessage> = view.to_messages();
    assert_eq!(entries.len(), 3, "三条都要进 prompt：{entries:?}");
    assert_eq!(entries[1].role, "tool");
    assert_eq!(
        entries[1].tool.as_deref(),
        Some("vdfs_read"),
        "工具名是事实"
    );

    let rep = verify_prompt_fidelity(&events, &entries, None);
    assert!(rep.is_complete(), "工具轮必须双向完整：{}", rep.summary());
}

// ==================== 写侧不变量闸门（T-188）====================
//
// 闸门挂在 `WalStore::append` 这个**唯一写入口**内：三条真实写路径（用户输入 /
// 工具结果 / 收尾落格）都从这里过。这一组钉五件事：
//
// 1. I2 缺溯源的声明被拒，且被拒的事件**不消耗 seq、不落盘**；
// 2. I1 同轮第二条 final 被拒；
// 3. I1 同轮第二条开轮被拒（不同 id——同 id 会先被 `Duplicate` 拦下）；
// 4. I3 新轮越过未收束的旧轮**不拒**（崩溃恢复就是这个形状），但计数 + 出声。
// 5. 名字与格位对不上被拒（[T-206 段四](../../../../../docs/009-task/206-事件总线读侧与协议收敛批.md "T-206-事件总线读侧与协议收敛批")）：
//    表上没这个名字、或这个名字不住在这一格——两条都是 `OffGrid`，不是 `Invariant`。

/// 一条**没有** `produced_by` 的 final（I2 的反例）。
fn final_without_provenance(turn: u64) -> Event {
    Event::pending(
        format!("f-{turn}"),
        crate::symbio_core::EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "session/main",
    )
    .with_payload(serde_json::json!({ "text": "答", "status": "completed" }))
}

/// I2：声明类事件缺 `produced_by` ⇒ 拒绝，且拒绝发生在赋号 / 落盘之前。
#[test]
fn gate_rejects_a_declaration_without_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let store = EventWalStore::open(&path).unwrap();

    let err = store
        .append(final_without_provenance(0))
        .expect_err("无溯源的 final 必须被拒");
    match err {
        crate::symbio_core::AppendError::Invariant {
            invariant,
            event_id,
            why,
        } => {
            assert_eq!(invariant, crate::symbio_core::StoreInvariant::I2);
            assert_eq!(event_id, "f-0");
            assert!(why.contains("produced_by"), "原因要说清缺什么：{why}");
        }
        other => panic!("拒错形态：{other:?}"),
    }
    assert_eq!(store.head().value(), 0, "被拒的事件不消耗 seq");
    assert_eq!(store.len(), 0, "被拒的事件不落盘");
}

/// I1：同一轮的第二条 final 被拒；**产物收束 + final 是合法组合**（不是重复）。
#[test]
fn gate_rejects_a_second_final_but_allows_artifact_then_final() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let store = EventWalStore::open(&path).unwrap();

    store
        .append(ev(
            "u-0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "问",
        ))
        .unwrap();
    // 产物收束（ADR-067）：它让「这一轮算收束」，但 final 还没落——必须还能落。
    store
        .append(
            Event::pending(
                "a-0-0",
                EVENT_ARTIFACT_ADDED,
                Entity::Artifact,
                Verb::Asserted,
                0,
                "session/main",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "tool": "echo", "text": "out" })),
        )
        .expect("产物格不是 final，不能被当成重复收束");
    store
        .append(ev(
            "f-0",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "答",
        ))
        .expect("产物之后仍应有且只有一条 final");

    let err = store
        .append(ev(
            "f-0-again",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "答二",
        ))
        .expect_err("同轮第二条 final 必须被拒");
    match err {
        crate::symbio_core::AppendError::Invariant { invariant, .. } => {
            assert_eq!(invariant, crate::symbio_core::StoreInvariant::I1);
        }
        other => panic!("拒错形态：{other:?}"),
    }
}

/// I1：同一轮两条**不同 id** 的开轮事件被拒（同 id 的重放更早就会被 `Duplicate` 拦下）。
#[test]
fn gate_rejects_a_second_open_for_the_same_turn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let store = EventWalStore::open(&path).unwrap();

    store
        .append(ev(
            "u-0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "问",
        ))
        .unwrap();
    let err = store
        .append(ev(
            "u-0-retry",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "又问",
        ))
        .expect_err("同轮第二条开轮必须被拒");
    match err {
        crate::symbio_core::AppendError::Invariant { invariant, .. } => {
            assert_eq!(invariant, crate::symbio_core::StoreInvariant::I1);
        }
        other => panic!("拒错形态：{other:?}"),
    }
}

/// I3：新轮越过未收束的旧轮 = **崩溃恢复的形状** ⇒ 照常入格，但计数（不静默）。
#[test]
fn gate_degrades_instead_of_blocking_a_new_turn_over_an_open_one() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    let store = EventWalStore::open(&path).unwrap();

    store
        .append(ev(
            "u-0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "问",
        ))
        .unwrap();
    // 上一轮停在半路（没有 final）：重启后开下一轮。拒绝会把这个会话钉死。
    store
        .append(ev(
            "u-1",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            1,
            "第二个问题",
        ))
        .expect("崩溃恢复形状必须还能写");
    assert_eq!(store.degraded_i3(), 1, "越过未收束轮要计数，不能静默");
    assert_eq!(store.len(), 2);

    // 收束之后就不再有缺口：第三轮不该再计一次。**两条**都要收——0 是崩溃残轮，
    // 1 是本轮；只收 1 的话，0 仍然是「被越过的旧轮」。
    store
        .append(ev(
            "f-0",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "补上的收束",
        ))
        .unwrap();
    store
        .append(ev(
            "f-1",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            1,
            "答",
        ))
        .unwrap();
    store
        .append(ev(
            "u-2",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            2,
            "第三个问题",
        ))
        .unwrap();
    assert_eq!(store.degraded_i3(), 1, "旧轮收束后不再算越界");
}

/// 闸门的索引**随重放重建**：重启后同轮第二条 final 仍然被拒。
#[test]
fn gate_index_is_rebuilt_from_the_wal_on_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events.jsonl");
    {
        let store = EventWalStore::open(&path).unwrap();
        store
            .append(ev(
                "u-0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "问",
            ))
            .unwrap();
        store
            .append(ev(
                "f-0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "答",
            ))
            .unwrap();
    }

    let reopened = EventWalStore::open(&path).unwrap();
    let err = reopened
        .append(ev(
            "f-0-late",
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            0,
            "迟到的第二条",
        ))
        .expect_err("重放必须把「这一轮已有 final」带回来");
    match err {
        crate::symbio_core::AppendError::Invariant { invariant, .. } => {
            assert_eq!(invariant, crate::symbio_core::StoreInvariant::I1);
        }
        other => panic!("拒错形态：{other:?}"),
    }
}

/// 段四·名字不在表上 ⇒ 拒，且答的是 `OffGrid { declared: None }`。
///
/// 这条同时是「加名字忘了登记表」的形状：写入口直接拒绝，不留下「事件在盘上、
/// 投影读不到」的静默中间态。
#[test]
fn the_grid_gate_rejects_a_name_that_is_not_on_the_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = EventWalStore::open(dir.path().join("events.jsonl")).unwrap();

    let err = store
        .append(ev(
            "u-0",
            "user.messag",
            Entity::Turn,
            Verb::Opened,
            0,
            "拼错的名字",
        ))
        .expect_err("表上没有的名字不得入库");
    match err {
        AppendError::OffGrid {
            event_id,
            kind,
            declared,
            given,
        } => {
            assert_eq!(event_id, "u-0");
            assert_eq!(kind, "user.messag");
            assert_eq!(declared, None, "不在表上 = 没有格位，不是猜一格");
            assert_eq!(given, (Entity::Turn, Verb::Opened));
        }
        other => panic!("拒错形态：{other:?}"),
    }
    assert_eq!(store.head().value(), 0, "被拒的事件不消耗 seq");
    assert_eq!(store.len(), 0, "被拒的事件不落盘");
}

/// 段四·名字在表上、但**不住在这一格** ⇒ 同一形态，`declared` 给出表上那一格。
///
/// 故意挑一条**同时**缺 `produced_by` 的事件：名字表这一臂排在 I2 之前——坐标错位时
/// I2 读到的输入本身就是错位的，先判 I2 会把「事件自述矛盾」误报成「缺溯源」。
#[test]
fn the_grid_gate_rejects_a_name_claiming_another_cell() {
    let dir = tempfile::tempdir().unwrap();
    let store = EventWalStore::open(dir.path().join("events.jsonl")).unwrap();
    let given = (Entity::Turn, Verb::Opened);
    let e = Event::pending(
        "f-0",
        EVENT_ASSISTANT_FINAL,
        given.0,
        given.1,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({ "text": "答" }));

    let err = store.append(e).expect_err("名字与格位不同源的事件不得入库");
    match err {
        AppendError::OffGrid {
            kind,
            declared,
            given,
            ..
        } => {
            assert_eq!(kind, EVENT_ASSISTANT_FINAL);
            assert_eq!(declared, Some((Entity::Turn, Verb::Closed)));
            assert_eq!(given, (Entity::Turn, Verb::Opened));
        }
        other => panic!("拒错形态：{other:?}"),
    }
    assert_eq!(store.len(), 0, "拒了就不落盘");

    // 同一个名字回到表上那一格（带溯源）⇒ 正常入库：拒的是错位，不是这个名字。
    let ok = Event::pending(
        "f-1",
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        1,
        "session/main",
    )
    .with_produced_by(0)
    .with_payload(serde_json::json!({ "text": "答" }));
    store.append(ok).expect("同格的名字要能落");
    assert_eq!(store.len(), 1);
}
