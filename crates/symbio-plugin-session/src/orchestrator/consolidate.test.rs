//! `orchestrator::consolidate` 的用例：快照落格 / 取代 / 归档 / 幂等 / 水位。
//!
//! 这些判据的对象是**会话存储层**：`EventWalStore` 的写者令牌、归档段布局、
//! `range(0)` 的「归档前缀 + live」拼接。驱动器（loop）只负责把压缩结果经端口
//! 交过来。那一半由 `loop/turn_exec.test.rs` 钉住。

use crate::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};
use std::sync::Arc;
use symbio_plugin_api::vocab;

use super::*;
use crate::chat_session::PersistentChatSession;
use crate::config::SessionConfig;
use crate::store::{current_window, EventWalStore, SessionStore};
use crate::types::Session;
use symbio_core::chat_node as cm;
use symbio_core::principal::{Principal, PrincipalKind};
use symbio_core::{check_all, count_user_messages, Entity, Event, Seq, Store, Verb};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONSOLIDATED, EVENT_USER_MESSAGE,
};

/// 一份「压缩后的整份消息列表」（判据只关心 id 序列——快照的语义是
/// 「这份列表**取代**了它之前的一切」）。
fn consolidated(ids: &[&str]) -> Vec<cm::ChatNode> {
    ids.iter()
        .map(|id| cm::ChatNode {
            id: (*id).to_string(),
            principal: Some(Principal::new(PrincipalKind::Person, "person/local")),
            msg_type: Some(cm::NodeKind::Body),
            content: Some(cm::MessageContent::Text(format!(
                "[CONTEXT SNAPSHOT]\n{id}"
            ))),
            status: Some(cm::NodeStatus::Completed),
            ..Default::default()
        })
        .collect()
}

/// 每例独占存储根；WAL 直接开在会话目录下（本组测的就是这一层）。
fn open_wal(dir: &std::path::Path) -> EventWalStore {
    EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE)).expect("打开事实源失败")
}

/// 种一轮完整事实（用户格 + 收束格）——压缩要取代的「历史」。
fn seed_turn(store: &EventWalStore, turn: u64) {
    let user_seq = store
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                "user",
            )
            .with_payload(serde_json::json!({ "text": format!("第 {turn} 问") })),
        )
        .expect("用户格");
    store
        .append(
            Event::pending(
                format!("f-{turn}"),
                EVENT_CHAT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                turn,
                "user",
            )
            .with_produced_by(user_seq.value())
            .with_payload(serde_json::json!({ "text": "答" })),
        )
        .expect("收束格");
}

/// 压缩入格 + 归档的**三段判据**：落格（完整史里有快照，格子对）/ 取代（现行窗口
/// 以快照开头、被取代的前缀进了归档段）/ 完整史不丢（`range(0)` 仍含前缀）。
#[test]
fn a_consolidation_lands_a_snapshot_and_archives_the_prefix() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.path().to_path_buf();
    let store = open_wal(&dir);
    seed_turn(&store, 0);
    seed_turn(&store, 1);
    let before = store.len() as u64;
    assert_eq!(before, 4, "两轮 = 用户格 + 收束格 ×2");

    let seq = consolidate_into_wal(&store, &consolidated(&["snap-1"]), "session/main")
        .expect("快照应落格");

    // ── ① 落格：完整史里恰一条快照，格子 = message × progressed，turn = 0 ──
    let all = store.range(Seq::new(0));
    let snaps: Vec<&Event> = all
        .iter()
        .filter(|e| e.kind == EVENT_SESSION_CONSOLIDATED)
        .collect();
    assert_eq!(snaps.len(), 1, "恰一条快照：{snaps:?}");
    assert_eq!(snaps[0].entity, Entity::Message);
    assert_eq!(snaps[0].verb, Verb::Progressed);
    assert_eq!(snaps[0].turn, 0, "压缩不属于任何一轮（跨轮边界）");
    assert_eq!(snaps[0].payload["messages"][0]["id"], "snap-1");
    assert_eq!(snaps[0].payload["turns"], 2, "水位 = 被取代前缀里的轮数");
    assert_eq!(
        snaps[0].produced_by,
        Some(before - 1),
        "溯源锚 = 被取代前缀的末号"
    );
    assert_eq!(seq.value(), before, "快照接在前缀之后");

    // ── ② 取代：现行窗口以快照开头，被取代的前缀进了归档段 ──
    assert_eq!(
        current_window(&all)[0].kind,
        EVENT_SESSION_CONSOLIDATED,
        "现行窗口以快照开头——被取代的前缀已移走"
    );
    let archive_dir = dir.join("transcripts").join("wal-archive");
    let mut segs: Vec<String> = std::fs::read_dir(&archive_dir)
        .expect("归档段目录应存在")
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    segs.sort();
    assert_eq!(
        segs,
        vec![format!("v2-events.wal.0-{}", before - 1)],
        "段名带首末号，末号 = 被取代前缀的末号（= 落格前的 head − 1）"
    );

    // ── ③ 完整史不丢：`range(0)` = 归档段 + live，差集恰是被取代的前缀 ──
    assert_eq!(
        all.len() as u64,
        store.len() as u64 + before,
        "归档段 {before} 条 + 没被取代的那段——「零回退损失」的可执行形态"
    );
    assert!(
        check_all(vocab(), &all).is_empty(),
        "{:?}",
        check_all(vocab(), &all)
    );
}

/// 同一份快照**不落第二次**：判据是与完整史里最近一份快照逐 id 比对。
///
/// 同一轮里执行口可能被调多次（续写 / 答话），而压缩只发生一次。
#[test]
fn the_same_consolidation_is_not_landed_twice() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.path().to_path_buf();
    let store = open_wal(&dir);
    seed_turn(&store, 0);

    let msgs = consolidated(&["snap-1"]);
    assert!(
        consolidate_into_wal(&store, &msgs, "session/main").is_some(),
        "第一次落格"
    );
    assert!(
        consolidate_into_wal(&store, &msgs, "session/main").is_none(),
        "同一份快照（逐 id 相同）不落第二次"
    );
    let n = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == EVENT_SESSION_CONSOLIDATED)
        .count();
    assert_eq!(n, 1, "完整史里恰一条快照");
}

/// **不同**的一份快照（id 序列变了）⇒ 再落一条：套娃压缩（压了又压）由此成立。
///
/// 第二次落格会把**上一份快照**也归档（它落在被取代的前缀里）——所以两条判据分头看
/// 完整史（两份都在，审计可回溯）与现行窗口（只剩最新的那一份）。
#[test]
fn a_newer_consolidation_lands_again() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.path().to_path_buf();
    let store = open_wal(&dir);
    seed_turn(&store, 0);

    assert!(consolidate_into_wal(&store, &consolidated(&["a"]), "session/main").is_some());
    assert!(
        consolidate_into_wal(&store, &consolidated(&["b"]), "session/main").is_some(),
        "id 序列不同 ⇒ 是新的一份，该落"
    );
    let is_snap = |e: &Event| e.kind == EVENT_SESSION_CONSOLIDATED;
    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter().filter(|e| is_snap(e)).count(),
        2,
        "完整史里两份快照都在（审计 / 回溯可查）"
    );
    let window = current_window(&all);
    assert_eq!(
        window.iter().filter(|e| is_snap(e)).count(),
        1,
        "现行窗口只剩最新一份快照（上一份已被它取代、进了归档段）"
    );
    assert_eq!(window[0].payload["messages"][0]["id"], "b");
}

/// 持久会话 + 会话目录（`consolidate_outside_turn` 经 `session.facts()` 取唯一句柄）。
async fn setup_session() -> (PersistentChatSession, std::path::PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "consolidate_outside".to_string();
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘");
    let dir = store.session_dir(&session_id).expect("会话目录");
    let session = PersistentChatSession::new(
        session_id,
        Arc::new(tokio::sync::RwLock::new(SessionConfig::default())),
        store,
    );
    (session, dir, tmp)
}

/// **轮外**压缩（用户重试 `retry_compaction`）走自己的一条路：那里没有「下一个
/// 执行口」可以寄存，所以自己取一次事实源落格——落格实现仍是同一个。
#[tokio::test]
async fn consolidate_outside_turn_lands_and_archives() {
    let (session, dir, _tmp) = setup_session().await;
    let store = session.facts().expect("事实源").expect("持久会话有事实源");
    seed_turn(&store, 0);

    consolidate_outside_turn(&session, &consolidated(&["snap-r"]), "session/main");

    let all = store.range(Seq::new(0));
    assert_eq!(
        all.iter()
            .filter(|e| e.kind == EVENT_SESSION_CONSOLIDATED)
            .count(),
        1,
        "轮外也落了一条快照"
    );
    assert_eq!(current_window(&all)[0].kind, EVENT_SESSION_CONSOLIDATED);
    assert!(
        dir.join("transcripts").join("wal-archive").exists(),
        "轮外同样归档"
    );
}

/// **轮次水位**：归档把被取代的轮次移进了归档段，而轮号数的是**现行窗口**里的
/// `user.message` ⇒ 不补水位，压缩之后轮号会**从 1 重新数起**，而
/// `u-{turn}` 是幂等键。
///
/// 这条钉的就是「快照载荷必须带 `turns`」：去掉它，第二轮的 `user.message`
/// 会拿到 `turn = 0`（与归档段里的旧轮重号），事实网格里两轮撞成一轮。
#[test]
fn the_turn_number_continues_across_archival() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.path().to_path_buf();
    let store = open_wal(&dir);
    seed_turn(&store, 0);

    assert!(
        consolidate_into_wal(&store, &consolidated(&["snap-1"]), "session/main").is_some(),
        "压掉第 1 轮"
    );
    let snapshot = store
        .range(Seq::new(0))
        .into_iter()
        .find(|e| e.kind == EVENT_SESSION_CONSOLIDATED)
        .expect("快照已落");
    assert_eq!(snapshot.payload["turns"], 1, "快照带走水位：它取代了 1 轮");
    assert_eq!(
        count_user_messages(&store.range(Seq::new(0)), &CHAT_VOCAB, &MESSAGE_VOCAB),
        1,
        "现行窗口里没有用户格，靠水位才数得出 1"
    );

    // 第二轮：轮号接着水位走（不是从 0 重号）⇒ 幂等键不撞。
    store
        .append(
            Event::pending(
                "u-1",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                1,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "第二问" })),
        )
        .expect("u-1 不撞幂等键");
    let turns: Vec<u64> = store
        .range(Seq::new(0))
        .iter()
        .filter(|e| e.kind == EVENT_USER_MESSAGE)
        .map(|e| e.turn)
        .collect();
    assert_eq!(turns, vec![0, 1], "两轮不重号");
}
