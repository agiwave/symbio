//! 会话引擎测试 —— `replace_messages` 孤儿存档配对清理。
//!
//! 覆盖三条路径：正常配对删除（L2 压缩语义）、保留新列表仍引用的存档
//! （keep_recent 语义）、以及路径越界防护（`..` 逃逸 / 非 tool_archives 根）。

use super::super::config::SessionConfig;
use super::super::store::SessionStore;
use super::super::types::Session;
use super::{
    ensure_durable_states, prune_historical_tool_calls, ChatSession, PersistentChatSession,
};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 构造带 `archive_path` meta 的 Tool 消息（模拟 L0 守卫落库后的形态）。
fn tool_msg_with_archive(id: &str, archive_path: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Turn),
        content: Some(MessageContent::Text("已存档".to_string())),
        meta: Some(serde_json::json!({ "archive_path": archive_path })),
        ..Default::default()
    }
}

/// 构造普通文本消息（无存档引用）。
fn plain_msg(id: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("hello".to_string())),
        ..Default::default()
    }
}

/// 建立临时文件存储 + 会话实例；返回 (会话实例, 会话目录, 清理句柄)。
async fn setup() -> (PersistentChatSession, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "test_replace_cleanup".to_string();
    // 先落盘一次空会话，使 session_dir 能解析到实际目录。
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store
        .session_dir(&session_id)
        .expect("文件后端应返回会话目录");
    let session = PersistentChatSession::new(
        session_id,
        Arc::new(RwLock::new(SessionConfig::default())),
        store,
    );
    (session, dir, tmp)
}

/// 在会话 tool_archives/ 下创建一个存档文件，返回其绝对路径。
fn make_archive(dir: &Path, name: &str) -> PathBuf {
    let archives = dir.join("tool_archives");
    std::fs::create_dir_all(&archives).expect("tool_archives 创建失败");
    let p = archives.join(name);
    std::fs::write(&p, "archived content").expect("存档写入失败");
    p
}

/// 审计 A3 契约：`replace_messages` 整体重写消息列表时**不删任何存档文件**。
///
/// 归档的磁盘回收唯一归口 `tool_result_guard::prune_archive_dir`（每会话按 mtime
/// 保留最新 `TOOL_ARCHIVE_KEEP` 个）。写入路径历史上会删"失去引用"的孤儿存档，
/// 代价是把不可信的消息 meta 变成删除动作的输入（需要额外的路径逃逸校验），
/// 且与滚动保留职责重叠；本测试锁定"只改消息、不动文件"的新契约。
#[tokio::test]
async fn test_replace_messages_never_deletes_archives() {
    let (session, dir, _tmp) = setup().await;
    let orphan = make_archive(&dir, "tool_a.txt");
    let kept = make_archive(&dir, "tool_b.txt");

    // 旧列表：两条带存档引用的消息。
    let old_msgs = vec![
        tool_msg_with_archive("m1", orphan.to_str().unwrap()),
        tool_msg_with_archive("m2", kept.to_str().unwrap()),
    ];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    // 新列表：丢掉对 tool_a 的引用（模拟 L2 压缩 keep_recent 语义）。
    let new_msgs = vec![
        plain_msg("s1"),
        tool_msg_with_archive("m2", kept.to_str().unwrap()),
    ];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        orphan.exists(),
        "失去引用的存档不得由写入路径删除（交 L0 滚动回收）: {}",
        orphan.display()
    );
    assert!(kept.exists(), "仍被引用的存档必须保留: {}", kept.display());
    let stored = session.get_messages().await.expect("读取存储消息失败");
    assert_eq!(stored.len(), 2, "消息列表应被整体重写");
    assert!(
        !stored.iter().any(|m| m.id == "m1"),
        "被丢弃的消息节点不应留在存储里"
    );
}

#[tokio::test]
async fn test_replace_messages_keeps_archives_when_all_referenced() {
    let (session, dir, _tmp) = setup().await;
    let a = make_archive(&dir, "tool_a.txt");

    let old_msgs = vec![tool_msg_with_archive("m1", a.to_str().unwrap())];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    // 新列表仍引用同一存档（消息 id 变了但引用未丢）。
    let new_msgs = vec![tool_msg_with_archive("m1", a.to_str().unwrap())];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(a.exists(), "仍被引用的存档不应被删除");
}

/// 安全回归：消息 meta 里的 `archive_path` 是**不可信输入**，任何情况下都不得被
/// 写入路径解析成删除动作（含 `..` 相对逃逸）。当前实现根本不删文件，本测试作为
/// 防止将来重新引入"按 meta 删文件"的护栏。
#[tokio::test]
async fn test_replace_messages_rejects_path_traversal() {
    let (session, dir, _tmp) = setup().await;
    // 会话目录外的目标文件（模拟攻击者可写的共享位置）。
    let outside = dir.parent().unwrap().join("outside_secret.txt");
    std::fs::write(&outside, "secret").expect("外部文件写入失败");

    // 旧消息引用 `../outside_secret.txt`（相对路径逃逸）。
    let old_msgs = vec![tool_msg_with_archive("m1", "../outside_secret.txt")];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    let new_msgs = vec![plain_msg("s1")];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        outside.exists(),
        "`..` 逃逸路径必须被拒绝，外部文件不得被删除: {}",
        outside.display()
    );
}

#[tokio::test]
async fn test_replace_messages_rejects_absolute_escape() {
    let (session, dir, _tmp) = setup().await;
    // 绝对路径指向会话目录外的任意文件。
    let outside = dir.parent().unwrap().join("abs_secret.txt");
    std::fs::write(&outside, "secret").expect("外部文件写入失败");

    let old_msgs = vec![tool_msg_with_archive("m1", outside.to_str().unwrap())];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    let new_msgs = vec![plain_msg("s1")];
    session
        .replace_messages(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        outside.exists(),
        "tool_archives/ 根之外的绝对路径必须被拒绝: {}",
        outside.display()
    );
}

// ── 写入期工具链裁剪与配置生效性（审计止血项 P0-2 / P0-3 / P1-②）────────

/// 同 `setup`，但注入自定义会话配置（验证 `prune_tool_history` / `max_messages` 等开关）。
async fn setup_with_config(
    config: SessionConfig,
) -> (PersistentChatSession, PathBuf, tempfile::TempDir) {
    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));
    let session_id = "test_config_semantics".to_string();
    let seed = Session::new(&session_id);
    store.save_session(&seed).await.expect("种子会话落盘失败");
    let dir = store
        .session_dir(&session_id)
        .expect("文件后端应返回会话目录");
    let session = PersistentChatSession::new(session_id, Arc::new(RwLock::new(config)), store);
    (session, dir, tmp)
}

/// 构造一轮完整工具调用链：User → ToolCall → Tool 结果。
fn tool_round(user_id: &str, tc_id: &str, tool_id: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            id: user_id.to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(format!("q-{}", user_id))),
            ..Default::default()
        },
        ChatMessage {
            id: tc_id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            parent_id: Some(user_id.to_string()),
            meta: Some(serde_json::json!({ "tool_name": "local/sh" })),
            ..Default::default()
        },
        ChatMessage {
            id: tool_id.to_string(),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Turn),
            parent_id: Some(tc_id.to_string()),
            content: Some(MessageContent::Text(format!("result-{}", tool_id))),
            ..Default::default()
        },
    ]
}

/// 回归（P0-2）：`keep_turns = 0`（语义"不限制"）不得越界 panic。
/// 旧实现直接取 `user_indices[len - 0]` → 下标 `len` 越界。
#[tokio::test]
async fn prune_zero_keep_turns_does_not_panic() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    prune_historical_tool_calls(&mut msgs, 0);

    assert_eq!(msgs.len(), 6, "keep_turns=0 应保留全部消息（不限制）");
    assert!(msgs.iter().any(|m| m.id == "r1"));
    assert!(msgs.iter().any(|m| m.id == "r2"));
}

/// 回归（P0-2）：空列表 + `keep_turns = 0` 同样是空操作。
#[tokio::test]
async fn prune_empty_messages_zero_keep_turns_is_noop() {
    let mut msgs: Vec<ChatMessage> = Vec::new();
    prune_historical_tool_calls(&mut msgs, 0);
    assert!(msgs.is_empty());
}

/// 正常语义：窗口外的工具链被物理删除，窗口内保留。
#[tokio::test]
async fn prune_keeps_recent_turns_and_drops_older_tool_chain() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    prune_historical_tool_calls(&mut msgs, 1);

    assert!(
        !msgs.iter().any(|m| m.id == "r1"),
        "最近 1 轮之外的工具结果应被裁剪"
    );
    assert!(
        !msgs.iter().any(|m| m.id == "t1"),
        "对应的 ToolCall 应一并裁剪"
    );
    assert!(msgs.iter().any(|m| m.id == "r2"), "最近一轮工具结果应保留");
}

/// 回归（审计 A3）：写入期裁剪**只删节点、不删文件**。
///
/// 旧实现在淘汰旧轮消息时按 `meta.archive_path` 物理删除 L0 存档，既把不可信的
/// 消息 meta 当删除依据，又与 `tool_result_guard` 的每会话滚动保留职责冲突（被淘汰
/// 的"旧轮"消息往往持有最新存档，反而被误删）。文件回收统一归口 `replace_messages`
/// 的孤儿清理 + 守卫自身的滚动保留。
#[tokio::test]
async fn prune_drops_nodes_but_never_deletes_archive_files() {
    let (session, dir, _tmp) = setup_with_config(SessionConfig {
        prune_tool_history: true,
        context_messages: 1,
        ..Default::default()
    })
    .await;
    let archive = make_archive(&dir, "prune_me.txt");

    // 两轮工具链，第一轮持有真实存档文件。
    let mut msgs = tool_round("u1", "t1", "r1");
    msgs[2].meta = Some(serde_json::json!({ "archive_path": archive.to_str().unwrap() }));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let stored = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !stored.iter().any(|m| m.id == "r1"),
        "窗口外的工具结果节点应被裁剪"
    );
    assert!(
        archive.exists(),
        "被裁剪节点的存档文件不得由写入路径删除: {}",
        archive.display()
    );
}

/// 回归（P0-2）：`context_messages = 0` 走完整 `append_messages` 链路也不得失败。
#[tokio::test]
async fn append_messages_with_zero_context_messages_does_not_fail() {
    let config = SessionConfig {
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    session
        .append_messages(msgs)
        .await
        .expect("context_messages=0 时落库不得失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        ctx.iter().any(|m| m.id == "r1"),
        "0 = 不限制：窗口外的工具结果仍应留在存储中"
    );
}

/// 新开关（P1-②）：`prune_tool_history = false` 时存储保留完整工具链。
#[tokio::test]
async fn append_messages_keeps_tool_chain_when_prune_disabled() {
    let config = SessionConfig {
        context_messages: 1,
        prune_tool_history: false,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        ctx.iter().any(|m| m.id == "r1"),
        "关闭写入期裁剪后，窗口外的工具结果应保留在存储中"
    );
}

/// 默认行为不回退：`prune_tool_history = true` 时窗口外工具链仍被物理裁剪。
#[tokio::test]
async fn append_messages_prunes_tool_chain_by_default() {
    let config = SessionConfig {
        context_messages: 1,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session.append_messages(msgs).await.expect("初始落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !ctx.iter().any(|m| m.id == "r1"),
        "默认开启写入期裁剪：窗口外的工具结果应被删除"
    );
}

/// 回归（P0-3）：`max_messages` 不再被存储层 `.max(500)` 硬下限钳制，小值真实生效。
#[tokio::test]
async fn max_messages_below_legacy_store_floor_is_honored() {
    let config = SessionConfig {
        max_messages: 2,
        prune_tool_history: false,
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(plain_msg(&format!("u{}", i)));
    }
    session.append_messages(msgs).await.expect("落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !ctx.iter().any(|m| m.id == "u1"),
        "max_messages=2 应触发 FIFO 淘汰；被 500 下限钳制时 u1 会留在存储中"
    );
    assert!(
        ctx.len() < 5,
        "max_messages=2 应裁剪存储轮次，实际保留 {} 条",
        ctx.len()
    );
    assert!(ctx.iter().any(|m| m.id == "u5"), "最近一轮用户消息必须保留");
}

/// `max_messages = 0`：不限制，全部保留。
#[tokio::test]
async fn max_messages_zero_means_unlimited() {
    let config = SessionConfig {
        max_messages: 0,
        prune_tool_history: false,
        context_messages: 0,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(plain_msg(&format!("u{}", i)));
    }
    session.append_messages(msgs).await.expect("落库失败");

    let ctx = session.get_messages().await.expect("读取存储消息失败");
    assert_eq!(ctx.len(), 5, "max_messages=0 表示不限制");
}

/// 存储是 `seq` 的**唯一分配者**，且分配的号必须严格递增（Lamport 计数器）。
///
/// 这条不变量是「前端只有一套序号空间」的全部前提：前端为尚未落库的节点自己发
/// 本地号，只有**落库回包**才能把它换成权威号。因此两件事都得成立，且都得钉住：
///
/// 1. `get_messages()` 读回来的消息**带 `seq`**——落库回包发的就是 `append_messages`
///    交回的这份权威版本（发入参那条等于把「没有号」写进前端）；
/// 2. 跨次追加**严格递增**：若新消息拿到比已有消息小的号，前端的顺序锚点就会
///    把新消息排到旧消息之前（“刚发的跑到中间去”）。
///
/// 缺少本测试时，`append_messages` 停止补号不会有任何测试失败：前端只是
/// 「偶尔顺序错乱、刷新才恢复」——正是这类静默退化最难定位。
#[tokio::test]
async fn append_messages_assigns_monotonic_authoritative_seq() {
    let (session, _dir, _tmp) = setup().await;

    session
        .append_messages(vec![plain_msg("u1")])
        .await
        .expect("首次落库失败");
    let first = session.get_messages().await.expect("读取存储消息失败");
    let s1 = first
        .iter()
        .find(|m| m.id == "u1")
        .expect("u1 应在存储中")
        .seq
        .expect("存储必须为落库消息分配 seq（前端唯一的顺序锚点）");

    session
        .append_messages(vec![plain_msg("u2")])
        .await
        .expect("二次落库失败");
    let second = session.get_messages().await.expect("读取存储消息失败");
    let s2 = second
        .iter()
        .find(|m| m.id == "u2")
        .expect("u2 应在存储中")
        .seq
        .expect("存储必须为落库消息分配 seq（前端唯一的顺序锚点）");

    assert!(
        s2 > s1,
        "追加分配必须严格递增（Lamport 计数器从已有最大 seq 续上）：u2={s2} 应大于 u1={s1}"
    );
}

/// 存储边界：**在途占位号不得落库**（事故现场见 `transcript::INFLIGHT_SEQ_BASE`）。
///
/// `CompressionEmitter::finish` 交给 `append_messages` 的是**在途图里的节点**——
/// 它的 `seq` 已被 `Transcript::apply` 填成在途号。存储若把它当权威号存下，存储
/// 水位就被抬进在途号段；此后存储计数器与在途计数器在同一数值区间各自递增，
/// **必然撞号**：实测同一会话里「本轮用户消息」与「压缩节点」各持 `1099511627781`，
/// `assertTranscriptInvariants` 的「seq 严格递增」当场失败。
///
/// 断言的是**从存储读回来的号**——边界只对"落盘的是什么"负责。
#[tokio::test]
async fn append_messages_replaces_inflight_placeholder_seq() {
    let (session, _dir, _tmp) = setup().await;
    let inflight = super::super::transcript::INFLIGHT_SEQ_BASE;

    session
        .append_messages(vec![plain_msg("u1")])
        .await
        .expect("首次落库失败");
    let s1 = session.get_messages().await.unwrap()[0]
        .seq
        .expect("存储必须分配 seq");

    // 模拟压缩节点：带着在途号进来（这正是 `finish` 的返回形态）
    let mut compact = plain_msg("compact1");
    compact.seq = Some(inflight + 7);
    session
        .append_messages(vec![compact])
        .await
        .expect("在途节点落库失败");

    let stored = session.get_messages().await.expect("读取存储消息失败");
    let s2 = stored
        .iter()
        .find(|m| m.id == "compact1")
        .expect("compact1 应在存储中")
        .seq
        .expect("存储必须分配 seq");
    assert!(
        s2 < inflight,
        "在途号不得落库：compact1 落库后仍是 {s2}（在途号段从 {inflight} 起）"
    );
    assert!(s2 > s1, "落库号必须严格递增：u1={s1} → compact1={s2}");

    // 撞号的另一半：紧接其后的真实追加必须继续往上，而不是撞回同一个号
    session
        .append_messages(vec![plain_msg("u2")])
        .await
        .expect("后续落库失败");
    let stored = session.get_messages().await.expect("读取存储消息失败");
    let s3 = stored
        .iter()
        .find(|m| m.id == "u2")
        .expect("u2 应在存储中")
        .seq
        .expect("存储必须分配 seq");
    assert!(s3 > s2, "后续追加必须继续递增：compact1={s2} → u2={s3}");
}

/// 落库回包契约（`docs/vdfs-session-messages.md` §3.4）：`append_messages` 必须
/// **把存储里的权威副本交回调用方**——带存储分配的 `seq` / `timestamp`，在途号已换掉。
///
/// 这条契约是正确性而非便利：`seq` 只在存储写入时分配，前端又只认存储号
/// （`Transcript::apply` 仅在帧带 `seq` 时采用外来值）。调用方拿不到权威副本 ⇒
/// 那条消息在前端**永远只有在途号** ⇒ 与存储号并存两套序号空间 ⇒ 排序错位
/// （`user-user-assistant-assistant`，只有整份回读才恢复）。
/// 端到端回归钉：`e2e/cases/t16-live-order.mjs`。
#[tokio::test]
async fn append_messages_returns_storage_authoritative_copies() {
    let (session, _dir, _tmp) = setup().await;
    let inflight = super::super::transcript::INFLIGHT_SEQ_BASE;

    // 助手侧节点的真实形态：号是在途图发的；用户那条干净（本地乐观副本也没有号）
    let mut assistant = plain_msg("a1");
    assistant.seq = Some(inflight + 1);

    let returned = session
        .append_messages(vec![plain_msg("u1"), assistant])
        .await
        .expect("落库失败");

    assert_eq!(
        returned.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["u1", "a1"],
        "交回的条数与顺序必须与入参一致"
    );
    for m in &returned {
        let seq = m.seq.expect("交回的消息必须带存储分配的 seq");
        assert!(
            seq < inflight,
            "交回的必须是存储号、不是在途号：{} = {seq}（在途号段从 {inflight} 起）",
            m.id
        );
        assert!(
            m.timestamp.unwrap_or(0) > 0,
            "交回的消息必须带存储回填的 timestamp：{}",
            m.id
        );
    }

    // 与磁盘逐字段一致——回执不是入参副本（入参那份没有号）
    let stored = session.get_messages().await.expect("读取存储消息失败");
    for m in &returned {
        let s = stored
            .iter()
            .find(|s| s.id == m.id)
            .unwrap_or_else(|| panic!("交回的消息必须在存储中：{}", m.id));
        assert_eq!(m.seq, s.seq, "交回的 seq 必须与存储一致：{}", m.id);
        assert_eq!(
            m.timestamp, s.timestamp,
            "交回的 timestamp 必须与存储一致：{}",
            m.id
        );
    }
}

/// 交回的是**留在存储里**的那些：被轮次窗口淘汰的不得出现在回执里。
///
/// 交回一条存储里并不存在的消息，等于让前端"对齐"到一个幻影——它的号在存储里
/// 属于别人，之后每一次追加都会再错一次。
#[tokio::test]
async fn append_messages_omits_messages_dropped_by_turn_window() {
    let config = SessionConfig {
        max_messages: 1,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let returned = session
        .append_messages(vec![plain_msg("u1"), plain_msg("u2"), plain_msg("u3")])
        .await
        .expect("落库失败");

    assert_eq!(
        returned.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["u3"],
        "窗口只留最后 1 轮 ⇒ 回执里只应有 u3"
    );
    let stored = session.get_messages().await.expect("读取存储消息失败");
    assert!(
        !stored.iter().any(|m| m.id == "u1"),
        "u1 应已被窗口淘汰（前置条件）"
    );
}

/// 存储边界（整表重写）：在途号同样不得落库，且必须在 `assign_seq` **之前**摘掉。
///
/// `assign_seq` 对既有序号是"原样保留"（稳定不变式），所以放进去就等于把在途号
/// 当成权威号写进存储。真实路径：`converge_inflight` 把在途图里尚未落库的节点
/// `clone()` 后追加到存储列表尾部再 `replace_messages`——那些节点带的正是在途号。
///
/// 同时钉住另一半：**既有的权威号一个都不许改**（否则前端手里的顺序锚点会错位）。
#[tokio::test]
async fn replace_messages_replaces_inflight_placeholder_seq() {
    let (session, _dir, _tmp) = setup().await;
    let inflight = super::super::transcript::INFLIGHT_SEQ_BASE;

    session
        .append_messages(vec![plain_msg("u1")])
        .await
        .expect("首次落库失败");
    let base = session.get_messages().await.unwrap()[0]
        .seq
        .expect("存储必须分配 seq");

    let mut kept = plain_msg("u1");
    kept.seq = Some(base);
    let mut live = plain_msg("turn1");
    live.seq = Some(inflight + 3);
    session
        .replace_messages(vec![kept, live])
        .await
        .expect("整表重写失败");

    let stored = session.get_messages().await.expect("读取存储消息失败");
    let seq_of = |id: &str| {
        stored
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("{id} 应在存储中"))
            .seq
            .expect("存储必须分配 seq")
    };
    assert_eq!(seq_of("u1"), base, "既有序号必须原样保留（稳定不变式）");
    assert!(
        seq_of("turn1") < inflight,
        "在途号不得落库：turn1 落库后仍是 {}",
        seq_of("turn1")
    );
    assert!(
        seq_of("turn1") > base,
        "在途节点应接在末尾：base={base}，turn1={}",
        seq_of("turn1")
    );
}

/// 持久层不变量：**增量不得落盘**。
///
/// `delta` 是帧的形态（"这一段"），不是消息的形态（"全部"）。放行它落库就等于把
/// 半截消息写进历史；而正文只落在 `content`（图内累积后的结果），所以一条带
/// `delta` 的待落库消息必定意味着某处把增量当成了全部。
#[test]
fn durable_layer_rejects_stream_delta() {
    let mut with_delta = plain_msg("m1");
    with_delta.delta = Some("半截".into());
    let err = ensure_durable_states(&[with_delta], "append_messages")
        .expect_err("必须拒绝携带 delta 的消息");
    assert!(
        err.to_string().contains("delta"),
        "错误信息要指明是 delta：{err}"
    );

    // 同一条消息去掉 delta（正文落在 content）后必须放行
    assert!(ensure_durable_states(&[plain_msg("m1")], "append_messages").is_ok());
}
