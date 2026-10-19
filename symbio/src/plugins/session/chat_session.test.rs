//! 会话引擎测试 —— `replace_messages` 孤儿存档配对清理。
//!
//! 覆盖三条路径：正常配对删除（L2 压缩语义）、保留新列表仍引用的存档
//! （keep_recent 语义）、以及路径越界防护（`..` 逃逸 / 非 tool_archives 根）。

use super::super::config::SessionConfig;
use super::super::store::SessionStore;
use super::super::types::Session;
use super::write::{assign_seq, ensure_durable_states, max_seq, prune_historical_tool_calls};
use super::PersistentChatSession;
use crate::plugins::session::store::EventWalStore;
use crate::symbio_core::chat_node::{ChatNode, MessageContent, NodeKind, NodeRole};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 构造带 `archive_path` meta 的 Tool 消息（模拟 L0 守卫落库后的形态）。
fn tool_msg_with_archive(id: &str, archive_path: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        role: Some(NodeRole::Tool),
        msg_type: Some(NodeKind::Skeleton),
        content: Some(MessageContent::Text("已存档".to_string())),
        meta: Some(serde_json::json!({"style_tag": "run", "archive_path": archive_path})),
        ..Default::default()
    }
}

/// 构造普通文本消息（无存档引用）。
fn plain_msg(id: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        role: Some(NodeRole::User),
        msg_type: Some(NodeKind::Body),
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
///
/// 切主后「列表被整体重写」改由事实投影的判据钉住（`apply_*` + `store` 冷重建）——
/// 本测试只守文件侧：写路径再怎么重写列表，磁盘上那些文件一张都不能少。
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
        .replace_all(new_msgs)
        .await
        .expect("replace_messages 失败");

    assert!(
        orphan.exists(),
        "失去引用的存档不得由写入路径删除（交 L0 滚动回收）: {}",
        orphan.display()
    );
    assert!(kept.exists(), "仍被引用的存档必须保留: {}", kept.display());
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
        .replace_all(new_msgs)
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

    // 旧消息引用 `../outside_secret.txt`（相对路径逃逸）。// doc-link-allow D-010: 示意路径（被测契约的输入不是入链）
    let old_msgs = vec![tool_msg_with_archive("m1", "../outside_secret.txt")];
    session
        .append_messages(old_msgs)
        .await
        .expect("初始落库失败");

    let new_msgs = vec![plain_msg("s1")];
    session
        .replace_all(new_msgs)
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
        .replace_all(new_msgs)
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
fn tool_round(user_id: &str, tc_id: &str, tool_id: &str) -> Vec<ChatNode> {
    vec![
        ChatNode {
            id: user_id.to_string(),
            role: Some(NodeRole::User),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text(format!("q-{}", user_id))),
            ..Default::default()
        },
        ChatNode {
            id: tc_id.to_string(),
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            parent_id: Some(user_id.to_string()),
            meta: Some(serde_json::json!({"style_tag": "tool", "tool_name": "local/sh"})),
            ..Default::default()
        },
        ChatNode {
            id: tool_id.to_string(),
            role: Some(NodeRole::Tool),
            msg_type: Some(NodeKind::Skeleton),
            meta: Some(serde_json::json!({"style_tag": "run"})),
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
    let mut msgs: Vec<ChatNode> = Vec::new();
    prune_historical_tool_calls(&mut msgs, 0);
    assert!(msgs.is_empty());
}

/// 正常语义：窗口外的工具链被物理删除，窗口内保留。
#[tokio::test]
async fn prune_keeps_recent_turns_and_drops_older_tool_chain() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));

    let removed = prune_historical_tool_calls(&mut msgs, 1);

    assert!(
        !msgs.iter().any(|m| m.id == "r1"),
        "最近 1 轮之外的工具结果应被裁剪"
    );
    assert!(
        !msgs.iter().any(|m| m.id == "t1"),
        "对应的 ToolCall 应一并裁剪"
    );
    assert!(msgs.iter().any(|m| m.id == "r2"), "最近一轮工具结果应保留");
    // T-054：裁剪必须报出被删的 id（调用方据此落 `message.removed`）。
    assert!(
        removed.contains(&"r1".to_string()) && removed.contains(&"t1".to_string()),
        "被裁节点 id 必须全部报出（否则切主后它们会复活）：{removed:?}"
    );
    assert!(
        !removed.contains(&"r2".to_string()) && !removed.contains(&"u2".to_string()),
        "保留区节点不得进裁剪集合：{removed:?}"
    );
}

/// T-054：裁剪集合**按原列表顺序**给出（事实与回执的 id 序列必须稳定——
/// `to_remove` 是 `HashSet`，顺序不定；直接倒它会让同一会话的两次裁剪给出不同的序列）。
#[tokio::test]
async fn prune_removed_ids_follow_original_list_order() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    msgs.extend(tool_round("u3", "t3", "r3"));

    let removed = prune_historical_tool_calls(&mut msgs, 1);

    assert_eq!(
        removed,
        vec![
            "t1".to_string(),
            "r1".to_string(),
            "t2".to_string(),
            "r2".to_string(),
        ],
        "被裁 id 必须按原列表顺序（t1 在 r1 前、第一轮整体在第二轮前）"
    );
}

/// 无裁剪 ⇒ 空集合（`keep_turns = 0` / 空列表 / 轮数不超窗口，三条早退路径）。
#[tokio::test]
async fn prune_reports_nothing_when_it_removes_nothing() {
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    // 轮数不超窗口 ⇒ 不动
    assert!(prune_historical_tool_calls(&mut msgs, 2).is_empty());
    // 0 = 不限制 ⇒ 不动
    assert!(prune_historical_tool_calls(&mut msgs, 0).is_empty());
    // 空列表 ⇒ 不动
    let mut empty: Vec<ChatNode> = Vec::new();
    assert!(prune_historical_tool_calls(&mut empty, 1).is_empty());
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
///
/// 切主后（ADR-049）正文的真源是事实投影，`append_messages` 交回的**权威副本**
/// 就是写边界机制（窗口/裁剪/补号）的可观测面——持久判据另由 `apply_*` 与
/// `store` 测试钉。本判据只看「0 = 不限制」：回执里窗口外的工具结果仍在。
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

    let (kept, _removed) = session
        .append_messages(msgs)
        .await
        .expect("context_messages=0 时落库不得失败");
    assert!(
        kept.iter().any(|m| m.id == "r1"),
        "0 = 不限制：窗口外的工具结果仍应留在回执（写边界不裁它）"
    );
}

/// 新开关（P1-②）：`prune_tool_history = false` 时保留完整工具链。
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
    let (kept, _removed) = session.append_messages(msgs).await.expect("初始落库失败");
    assert!(
        kept.iter().any(|m| m.id == "r1"),
        "关闭写入期裁剪后，窗口外的工具结果应保留在回执里"
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

/// **T-054 的承重判据**：写入期裁剪是**列表变更**，必须落 `message.removed`。
///
/// 不落格的后果是**静默的**（这是它被登记成任务的原因）：WAL（唯一真源）里那些
/// 被裁节点还在，只是 `messages.json` 缓存里少了。切主读后重建照 WAL 折回 ⇒
/// **被裁节点复活**——用户会看到早已淘汰的旧工具链重新出现在历史里。
///
/// 本测试走**生产路径**（`apply(Born)` → `append_messages` 的存储临界区），
/// 判据落在事实源上：裁剪之后，被裁的那些 id 必须在 WAL 里有 `message.removed`。
#[tokio::test]
async fn prune_on_write_lands_a_removed_fact() {
    let config = SessionConfig {
        context_messages: 1,
        ..Default::default()
    };
    let (session, dir, _tmp) = setup_with_config(config).await;
    // 溯源锚：断言类事实（`message.removed`）必须有 `produced_by`（I2），
    // 空的切片连锚都解不出 ⇒ 事实被按口径丢弃（WARN 有据）。先种一条事实。
    seed_fact(&dir);

    // 两轮工具链，`context_messages = 1` ⇒ 第一轮的工具链应被裁掉。
    let mut msgs = Vec::new();
    msgs.extend(tool_round("u1", "t1", "r1"));
    msgs.extend(tool_round("u2", "t2", "r2"));
    session
        .apply(super::MessageChange::Born {
            nodes: msgs,
            carrier: super::Carrier::Here,
        })
        .await
        .expect("落库失败");

    // 缓存侧：被裁节点确实不在列表里。
    let list = session.get_messages().await.expect("读列表失败");
    assert!(
        !list.iter().any(|m| m.id == "t1" || m.id == "r1"),
        "窗口外工具链应被裁剪：{list:?}"
    );

    // **事实侧**：被裁的 id 必须进了网格。
    let events = events_of(&dir);
    let removed_ids: Vec<String> = events
        .iter()
        .filter(|e| e.kind == "message.removed")
        .filter_map(|e| e.payload.get("ids"))
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();
    for id in ["t1", "r1"] {
        assert!(
            removed_ids.contains(&id.to_string()),
            "被裁节点 {id} 必须落 `message.removed`（否则切主后复活）：{removed_ids:?}"
        );
    }
    assert!(
        !removed_ids.contains(&"r2".to_string()),
        "保留区节点不得进删除事实：{removed_ids:?}"
    );
}

/// **T-054 的另一半**：轮次窗口淘汰（`max_messages`）同样是列表变更，必须落事实。
#[tokio::test]
async fn turn_window_eviction_lands_a_removed_fact() {
    let config = SessionConfig {
        max_messages: 2,
        prune_tool_history: false,
        context_messages: 0,
        ..Default::default()
    };
    let (session, dir, _tmp) = setup_with_config(config).await;
    seed_fact(&dir);

    let mut msgs = Vec::new();
    for i in 1..=5 {
        msgs.push(plain_msg(&format!("u{}", i)));
    }
    session
        .apply(super::MessageChange::Born {
            nodes: msgs,
            carrier: super::Carrier::Here,
        })
        .await
        .expect("落库失败");

    let events = events_of(&dir);
    let removed_ids: Vec<String> = events
        .iter()
        .filter(|e| e.kind == "message.removed")
        .filter_map(|e| e.payload.get("ids"))
        .filter_map(|v| v.as_array())
        .flatten()
        .filter_map(|v| v.as_str())
        .map(str::to_string)
        .collect();
    // max_messages=2 ⇒ 前 3 条被 FIFO 淘汰。
    for id in ["u1", "u2", "u3"] {
        assert!(
            removed_ids.contains(&id.to_string()),
            "被窗口淘汰的 {id} 必须落 `message.removed`：{removed_ids:?}"
        );
    }
    assert!(
        !removed_ids.contains(&"u5".to_string()),
        "最近一轮不得进删除事实：{removed_ids:?}"
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
    let (kept, _removed) = session.append_messages(msgs).await.expect("落库失败");
    assert!(
        !kept.iter().any(|m| m.id == "u1"),
        "max_messages=2 应触发 FIFO 淘汰；被 500 下限钳制时 u1 会留在回执里"
    );
    assert!(
        kept.len() < 5,
        "max_messages=2 应裁剪写边界列表，实际保留 {} 条",
        kept.len()
    );
    assert!(
        kept.iter().any(|m| m.id == "u5"),
        "最近一轮用户消息必须保留"
    );
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
    let (kept, _removed) = session.append_messages(msgs).await.expect("落库失败");
    assert_eq!(kept.len(), 5, "max_messages=0 表示不限制");
}

/// 存储是 `seq` 的**唯一分配者**，且分配的号必须严格递增（Lamport 计数器）。
///
/// 这条不变量是「前端只有一套序号空间」的全部前提：前端为尚未落库的节点自己发
/// 本地号，只有**落库回包**才能把它换成权威号。因此两件事都得成立，且都得钉住：
///
/// 1. `append_messages` 交回的权威副本**带 `seq`**——发入参那条等于把「没有号」
///    写进前端；
/// 2. 跨次追加**严格递增**：若新消息拿到比已有消息小的号，前端的顺序锚点就会
///    把新消息排到旧消息之前（“刚发的跑到中间去”）。
///
/// 切主后（ADR-049）正文的真源是事实投影：**只有落了事实的节点才进投影**，跨次
/// 递增的「已有水位」也由投影供。所以本判据走生产路径（`apply` 的 Born{Here}
/// ——事实先落、再折列表、回包带存储号）：「读回存储」那半归 `apply_*` +
/// `store` 冷重建，这里盯写边界当场交出的那份。
///
/// 缺少本测试时，`append_messages` 停止补号不会有任何测试失败：前端只是
/// 「偶尔顺序错乱、刷新才恢复」——正是这类静默退化最难定位。
#[tokio::test]
async fn append_messages_assigns_monotonic_authoritative_seq() {
    let (session, dir, _tmp) = setup().await;
    seed_fact(&dir);

    let born = |id: &str| super::MessageChange::Born {
        nodes: vec![plain_msg(id)],
        carrier: super::Carrier::Here,
    };
    let s1 = session
        .apply(born("u1"))
        .await
        .expect("首次落库失败")
        .into_iter()
        .find(|m| m.id == "u1")
        .expect("u1 应在回执中")
        .seq
        .expect("存储必须为落库消息分配 seq（前端唯一的顺序锚点）");
    let s2 = session
        .apply(born("u2"))
        .await
        .expect("二次落库失败")
        .into_iter()
        .find(|m| m.id == "u2")
        .expect("u2 应在回执中")
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
/// 断言的是**回执上的号**——边界只对"交出去的是什么"负责。
///
/// 生产路径同右：节点经 `apply` Born{Here} 落库（事实先落、投影随后长大），
/// 跨次递增的「已有水位」由投影供。
#[tokio::test]
async fn append_messages_replaces_inflight_placeholder_seq() {
    let (session, dir, _tmp) = setup().await;
    let inflight = super::super::transcript::INFLIGHT_SEQ_BASE;
    seed_fact(&dir);

    let born = |node: ChatNode| super::MessageChange::Born {
        nodes: vec![node],
        carrier: super::Carrier::Here,
    };
    let seq_of = |landed: Vec<ChatNode>, id: &str| {
        landed
            .into_iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("{id} 应在回执中"))
            .seq
            .expect("存储必须分配 seq")
    };

    let s1 = seq_of(
        session
            .apply(born(plain_msg("u1")))
            .await
            .expect("首次落库失败"),
        "u1",
    );

    // 模拟压缩节点：带着在途号进来（这正是 `finish` 的返回形态）
    let mut compact = plain_msg("compact1");
    compact.seq = Some(inflight + 7);
    let s2 = seq_of(
        session
            .apply(born(compact))
            .await
            .expect("在途节点落库失败"),
        "compact1",
    );
    assert!(
        s2 < inflight,
        "在途号不得落库：compact1 落库后仍是 {s2}（在途号段从 {inflight} 起）"
    );
    assert!(s2 > s1, "落库号必须严格递增：u1={s1} → compact1={s2}");

    // 撞号的另一半：紧接其后的真实追加必须继续往上，而不是撞回同一个号
    let s3 = seq_of(
        session
            .apply(born(plain_msg("u2")))
            .await
            .expect("后续落库失败"),
        "u2",
    );
    assert!(s3 > s2, "后续追加必须继续递增：compact1={s2} → u2={s3}");
}

/// 落库回包契约（`docs/022-vdfs-session-messages.md` §3.4）：`append_messages` 必须
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

    let (returned, _removed) = session
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

    // 切主后「回执与磁盘逐字段一致」这半移入事实侧判据（`apply_*` 落格 +
    // `store` 测试的冷重建）：正文的真源是事实投影，本测试盯写边界当场交出的那份。
}

/// 交回的是**留在列表里**的那些：被轮次窗口淘汰的不得出现在回执里。
///
/// 交回一条列表里并不存在的消息，等于让前端"对齐"到一个幻影——它的号在存储里
/// 属于别人，之后每一次追加都会再错一次。切主后「存储里真的没有 u1」由事实侧
/// 判据钉（`apply_*` 只落活下来的节点），这里盯回执的当场形状。
#[tokio::test]
async fn append_messages_omits_messages_dropped_by_turn_window() {
    let config = SessionConfig {
        max_messages: 1,
        ..Default::default()
    };
    let (session, _dir, _tmp) = setup_with_config(config).await;

    let (returned, _removed) = session
        .append_messages(vec![plain_msg("u1"), plain_msg("u2"), plain_msg("u3")])
        .await
        .expect("落库失败");

    assert_eq!(
        returned.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["u3"],
        "窗口只留最后 1 轮 ⇒ 回执里只应有 u3"
    );
}

// `replace_messages` 的旧判据（在途号不得随整表重写落库）**随切源退役**：
// 切主后正文的真源是事实投影，整表重写只写元数据——「重写后的列表」不再有
// 可观测面。同一条边界改由事实侧的判据钉住：`apply_*` 只落定稿节点，在途节点
// 由 `converge_inflight` 收敛后才落；上一条（append 回执）钉同一条边界。
//
// 归档删除纪律（下方 `test_replace_messages_never_deletes_archives`）仍然有效：
// 它守的是 `replace_all` 剩下的文件侧契约。

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

// ==================== 序号分配（`max_seq` / `assign_seq`）====================
//
// 自 `symbio_core/schemas/session/chat_node.test.rs` 迁入：`seq` **字段**是跨栈
// schema（留 core），而「怎么补号」是会话存储的实现策略（随 `assign_seq` 迁到这里）。
//
// 前置条件（见 `assign_seq` 文档）：无号项只出现在**开头**或**末尾**。下面每条用例的
// 输入都取自某个真实调用点的列表形态，不是随手构造的数组。

fn msg(id: &str, seq: Option<i64>) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        seq,
        ..Default::default()
    }
}

/// 正常路径**不得**重排：数组顺序本就等于 seq 顺序时，既有序号原样保留，
/// 只有末尾的缺号项续接水位。修单调性不能以"每次落库都重排历史"为代价。
#[test]
fn assign_seq_leaves_already_ordered_seqs_untouched() {
    let mut msgs = vec![msg("a", Some(5)), msg("b", Some(6)), msg("c", None)];
    assign_seq(&mut msgs, 0);
    assert_eq!(msgs[0].seq, Some(5), "已有序的历史不得被改写");
    assert_eq!(msgs[1].seq, Some(6));
    assert_eq!(msgs[2].seq, Some(7), "缺号者续接水位");
}

/// **压缩契约**：`base` 高于既有序号时，既有序号依然一个都不许动。
///
/// 这正是实测会话 `mtmae8j2wxam4dhrei` 的病态：`replace_messages` 传
/// `base = max_seq(旧列表)`（868），而新列表是 `[快照(槽位 630), 保留区(631..642)]`。
/// 旧实现从 868 起步，把保留区整段抬到 870..881；正确行为是原样保留。
#[test]
fn assign_seq_never_rewrites_existing_seqs_even_when_base_is_higher() {
    let old = vec![msg("old", Some(868))];
    let mut msgs = vec![
        msg("snapshot", Some(630)), // 快照接替被压缩内容的槽位：keep[0].seq - 1
        msg("keep1", Some(631)),
        msg("keep2", Some(632)),
    ];
    assign_seq(&mut msgs, max_seq(&old));
    assert_eq!(msgs[0].seq, Some(630), "快照的槽位序号必须原样保留");
    assert_eq!(
        msgs[1].seq,
        Some(631),
        "保留区序号必须原样保留（压缩不改号）"
    );
    assert_eq!(msgs[2].seq, Some(632));
}

/// 全新列表（无任何既有序号）才使用 `base`：整批新节点接在旧水位之后。
#[test]
fn assign_seq_uses_base_only_for_fresh_lists() {
    let mut msgs = vec![msg("n1", None), msg("n2", None)];
    assign_seq(&mut msgs, 868);
    assert_eq!(msgs[0].seq, Some(869));
    assert_eq!(msgs[1].seq, Some(870));
}

/// 空列表（`replace_messages(Vec::new())`）：无号可补，水位原样回传。
///
/// 这条锁的是「把整份转写替换成空」这条路：若实现改成"无论如何先 +1"，替换就会把
/// 会话水位推高一格，下一次追加的号与前端手里的号出现空档。
#[test]
fn assign_seq_on_empty_list_returns_base() {
    let mut msgs: Vec<ChatNode> = Vec::new();
    assert_eq!(assign_seq(&mut msgs, 868), 868);
}

/// 开头段有**多个**无号项时，整段落在首号之前且沿数组递增。
///
/// 逐个"往上找空位"会先占用首号本身、把既有序号顶成下一个号；正确做法是
/// 整段一次算好（`首号 − k … 首号 − 1`）。
#[test]
fn assign_seq_places_leading_run_below_first_existing() {
    let mut msgs = vec![msg("s1", None), msg("s2", None), msg("keep", Some(95))];
    assign_seq(&mut msgs, 100);
    assert_eq!(msgs[0].seq, Some(93));
    assert_eq!(msgs[1].seq, Some(94));
    assert_eq!(msgs[2].seq, Some(95), "既有序号不得被开头段顶走");
}

/// 末尾段接在末号之后——本轮在途追加 / 恢复时新建子节点的常规路径。
///
/// `base` 取 `max_seq(旧列表)`（与 `replace_messages` 的真实调用形态一致）且**高于**
/// 末号：末尾段仍须接在**末号**之后，不得跳到 `base`。
#[test]
fn assign_seq_places_trailing_run_after_last_existing() {
    let old = vec![msg("old1", Some(5)), msg("old2", Some(6))];
    let mut msgs = vec![msg("a", Some(5)), msg("inflight", None)];
    assign_seq(&mut msgs, max_seq(&old));
    assert_eq!(msgs[0].seq, Some(5));
    assert_eq!(msgs[1].seq, Some(6));
}

// ── `apply`：唯一写入口（S2.7-b3 前置，ADR-054 决策 2 + 3）──────────────

use super::super::paths::V2_WAL_FILE;
use crate::symbio_core::{Event, Seq, Store};

/// 只读读回事实源里的全部事件（与 `resume.test.rs::events_of` 同款）。
fn events_of(dir: &std::path::Path) -> Vec<Event> {
    let wal = dir.join(V2_WAL_FILE);
    let store = EventWalStore::open_readonly(&wal).expect("只读打开事实源失败");
    store.range(Seq::new(0))
}

/// 往事实源里种一条事实，让 `message_events` 的溯源锚有处可站。
///
/// 空切片（会话还没有任何事实）时变更事实会被按口径丢弃（「不硬造一个号」）——
/// 生产里压缩记录出现前必然已有轮次事实，测试里补的就是这颗种子。
/// **作用域包住**：种子的写者令牌必须释放，否则 `message_events` 自己开 WAL 时
/// 拿到 `NotTheWriter` ⇒ 事实静默丢弃。
fn seed_fact(dir: &std::path::Path) {
    let wal = dir.join(V2_WAL_FILE);
    let s = EventWalStore::open(&wal).expect("打开事实源失败");
    s.append(
        Event::pending(
            "u-seed",
            crate::symbio_core::EVENT_USER_MESSAGE,
            crate::symbio_core::Entity::Turn,
            crate::symbio_core::Verb::Opened,
            1,
            "user",
        )
        .with_payload(serde_json::json!({ "text": "第一问", "id": "u-seed" })),
    )
    .expect("种子事实落格失败");
}

/// **Born + `Carrier::Here`**：落一条 `message.added` 事实 + 追加进列表。
///
/// 这条判据钉的就是「顺序反转」：事实与列表在**同一次调用**里都发生了，
/// 「改了列表却没落事实」在 `apply` 里表达不出来。
#[tokio::test]
async fn apply_born_here_lands_an_added_fact_and_appends() {
    let (session, dir, _tmp) = setup().await;
    seed_fact(&dir);

    let landed = session
        .apply(super::MessageChange::Born {
            nodes: vec![plain_msg("c1")],
            carrier: super::Carrier::Here,
        })
        .await
        .expect("apply(Born, Here) 失败");
    assert_eq!(landed.len(), 1, "返回受影响的权威副本：{landed:?}");
    assert_eq!(landed[0].id, "c1");

    // 切主读后列表 = **事实投影**：u-seed 与 c1 都在（旧形态只回 c1——那是
    // messages.json 与 WAL 双源并存的产物，已随切源消失）。
    let list = session.get_messages().await.expect("读列表失败");
    assert_eq!(
        list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["u-seed", "c1"],
        "列表即事实投影：种子与新生节点都在"
    );

    let events = events_of(&dir);
    let added: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == "message.added")
        .collect();
    assert_eq!(added.len(), 1, "落了恰好一条诞生事实：{added:?}");
    let first = added[0]
        .payload
        .get("messages")
        .and_then(|v| v.as_array())
        .expect("载荷有 messages")
        .first()
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str())
        .expect("载荷首条带 id");
    assert_eq!(first, "c1", "承载的正是这次诞生的节点");
}

/// **Born + `Carrier::Runner`**：**不落事实**——承载事实归运行器的格子
/// （用户发言 / 助手正文 / 工具结果），`apply` 再落一条就是第二个 owner。
///
/// 切主读后这条契约的可观察面就是「持久化面什么都没发生」：节点的出生记录归
/// 运行器的事实，这里的 append 只是在途合并，不构成出生记录。
#[tokio::test]
async fn apply_born_runner_lands_no_fact() {
    let (session, dir, _tmp) = setup().await;
    seed_fact(&dir);

    session
        .apply(super::MessageChange::Born {
            nodes: vec![plain_msg("u1")],
            carrier: super::Carrier::Runner,
        })
        .await
        .expect("apply(Born, Runner) 失败");

    assert!(
        events_of(&dir).iter().all(|e| e.kind != "message.added"),
        "运行器承载 ⇒ 这里不落诞生事实"
    );
    let list = session.get_messages().await.expect("读列表失败");
    assert!(
        list.iter().all(|m| m.id != "u1"),
        "没有出生事实 ⇒ 投影里没有它（持久化归运行器，这里不得自立门户）"
    );
}

/// **Edited**：每条落一个 `message.edited`，列表**原位改写**（不新增、不动序）。
#[tokio::test]
async fn apply_edited_lands_facts_and_rewrites_in_place() {
    let (session, dir, _tmp) = setup().await;
    seed_fact(&dir);
    // 种子走 `Here`（落出生事实）：切主读后列表 = 事实投影，被编辑的节点必须
    // 先以事实存在，编辑折叠才有对象。
    session
        .apply(super::MessageChange::Born {
            nodes: vec![plain_msg("m1"), plain_msg("m2")],
            carrier: super::Carrier::Here,
        })
        .await
        .expect("种子落库失败");
    let before = session.get_messages().await.expect("读列表失败");
    assert_eq!(before.len(), 3, "投影 = 种子 + 两个新生节点");

    let mut edited = plain_msg("m1");
    edited.content = Some(MessageContent::Text("改过的话".to_string()));
    let landed = session
        .apply(super::MessageChange::Edited(vec![edited]))
        .await
        .expect("apply(Edited) 失败");
    assert_eq!(landed.len(), 1, "返回改写后的权威副本");

    let list = session.get_messages().await.expect("读列表失败");
    assert_eq!(list.len(), before.len(), "编辑**不新增节点**");
    assert_eq!(list[1].id, "m1", "位置不变");
    assert_eq!(
        list[1].content.as_ref().map(|c| c.to_text()),
        Some("改过的话".to_string()),
        "内容换成新的"
    );
    assert_eq!(list[2].id, "m2", "其余节点不受影响");

    let events = events_of(&dir);
    let edits: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == "message.edited")
        .collect();
    assert_eq!(edits.len(), 1, "每条变更各落一条断言事实：{edits:?}");
}

/// **Removed**：落一条 `message.removed`（区间用 `ids` 表达），列表里节点消失。
#[tokio::test]
async fn apply_removed_lands_a_fact_and_drops_nodes() {
    let (session, dir, _tmp) = setup().await;
    seed_fact(&dir);
    // 种子走 `Here`（落出生事实）：切主读后列表 = 事实投影，被删的节点必须先以
    // 事实存在，删除折叠才有对象。
    session
        .apply(super::MessageChange::Born {
            nodes: vec![plain_msg("m1"), plain_msg("m2"), plain_msg("m3")],
            carrier: super::Carrier::Here,
        })
        .await
        .expect("种子落库失败");

    let landed = session
        .apply(super::MessageChange::Removed(vec![
            "m1".into(),
            "m3".into(),
        ]))
        .await
        .expect("apply(Removed) 失败");
    assert!(
        landed.is_empty(),
        "被删的节点不交回（交回等于让前端对齐幻影）"
    );

    let list = session.get_messages().await.expect("读列表失败");
    assert_eq!(
        list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["u-seed", "m2"],
        "只留没被删的（列表即事实投影）"
    );

    let events = events_of(&dir);
    let removed: Vec<&Event> = events
        .iter()
        .filter(|e| e.kind == "message.removed")
        .collect();
    assert_eq!(removed.len(), 1, "一次删除落**一条**事实（ids 表达区间）");
}

/// **T-164 的承重判据**：事实落不下去 ⇒ `apply` 报失败，且**什么都没发生**。
///
/// 旧口径是「事实失败只记日志」——那在缓存是读侧真源时成立（影子失败不该让本体
/// 失败）。b3 把主读切到 WAL 之后事实就是本体：放过它等于「回执说改好了、重建后
/// 一看没改」，而且用户无感。步骤 ① 在写缓存**之前**，所以报失败是如实。
///
/// 复现手法：把锁文件路径换成目录 ⇒ `EventWalStore::open` 打不开（跨平台都失败）。
/// 断言两半：`apply` 返回错误（回执报失败）+ 缓存里没有那个节点（失败即未生效，
/// 不是「写了一半」）。
#[tokio::test]
async fn apply_reports_a_failed_fact_write_and_writes_nothing() {
    let (session, dir, _tmp) = setup().await;
    // 先种一条事实：没有锚的会话走的是 `NoAnchor` 豁免（v1 遗留），不是本条要验的路径。
    seed_fact(&dir);
    let lock = dir.join(format!("{V2_WAL_FILE}.lock"));
    std::fs::remove_file(&lock).expect("移走锁文件");
    std::fs::create_dir(&lock).expect("占住锁文件路径");

    let err = session
        .apply(super::MessageChange::Born {
            nodes: vec![plain_msg("born-1")],
            carrier: super::Carrier::Here,
        })
        .await
        .expect_err("事实源打不开 ⇒ 这次变更必须报失败");
    assert!(
        err.to_string().contains("未生效"),
        "错误要说清「本次变更未生效」：{err}"
    );

    // 恢复锁路径，读列表：失败的变更**一点都没写**。
    std::fs::remove_dir(&lock).expect("交还锁文件路径");
    let list = session.get_messages().await.expect("读列表失败");
    assert!(
        list.iter().all(|m| m.id != "born-1"),
        "事实失败 ⇒ 缓存也不写（失败即未生效，不是写了一半）：{list:?}"
    );
}

// ── 作用域的第一个生产构造点（[T-105]）──────────────────────────────────────
//
// `chat_loop::state::request_scope` 按会话元数据的**委派链**取值：子会话挂父主体，
// 顶层会话恒 `Root`。本段钉的是「取值链读得出真值」——语义（委派不放大权威）的
// 判据在 `turn_runner.test.rs::scope_ceiling_tests`。
//
// [T-105]: ../../../../docs/009-task/105-Scope-Child首产点批.md "T-105-Scope-Child首产点批"

/// 顶层会话 ⇒ `Root`；`agent_run` 登记形态的子会话 ⇒ `Child(父主体)`。
///
/// 父会话的主体取 `metadata.agent_id` + `principal_of`（与写事件的主体同一个派生点），
/// 故本用例同时钉住「子会话的作用域不是凭空一个字符串」——它跟着父会话的身份走。
#[tokio::test]
async fn scope_follows_the_delegation_chain_and_defaults_to_root() {
    use crate::symbio_core::Scope;

    let tmp = tempfile::tempdir().expect("临时目录创建失败");
    let store = Arc::new(SessionStore::new(tmp.path().to_path_buf()));

    // 父会话：`--agent peer-b` 的会话形态（主体 = 对等体）
    let mut parent = Session::new("parent-sess");
    parent.metadata["agent_id"] = serde_json::json!("peer-b");
    store.save_session(&parent).await.expect("父会话落盘失败");

    let top = PersistentChatSession::new(
        "parent-sess".to_string(),
        Arc::new(RwLock::new(SessionConfig::default())),
        store.clone(),
    );
    assert_eq!(
        super::super::chat_loop::request_scope(&top).await,
        Scope::Root,
        "没有归属声明的会话 = 顶层会话 ⇒ 根作用域"
    );

    // 子会话：`agent_run` 的登记形态（`parent_session_id` 指回父会话；自身未选智能体）
    let mut child = Session::new("child-sess");
    child.metadata["parent_session_id"] = serde_json::json!("parent-sess");
    store.save_session(&child).await.expect("子会话落盘失败");

    let sub = PersistentChatSession::new(
        "child-sess".to_string(),
        Arc::new(RwLock::new(SessionConfig::default())),
        store.clone(),
    );
    assert_eq!(
        super::super::chat_loop::request_scope(&sub).await,
        Scope::Child("session/peer-b".to_string()),
        "子会话挂**父会话主体**（不是父会话 id，也不是子会话自己的主体）"
    );

    // 反向：归属声明抹掉 ⇒ 同一实例必须回到 `Root`（取值链真的在算）
    let mut orphan = Session::new("orphan-sess");
    orphan.metadata = serde_json::json!({});
    store.save_session(&orphan).await.expect("孤儿会话落盘失败");
    let plain = PersistentChatSession::new(
        "orphan-sess".to_string(),
        Arc::new(RwLock::new(SessionConfig::default())),
        store.clone(),
    );
    assert_eq!(
        super::super::chat_loop::request_scope(&plain).await,
        Scope::Root
    );
}
