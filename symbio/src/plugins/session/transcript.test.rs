//! `transcript.rs` 的单元测试（图语义 / 位置序号 / 投递形状）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//!
//! 投递的观测方式：给 `Transcript` 一张带 sink 的 `ChangeSubscriptions`，sink 把
//! 每条变更收进数组。断言的是**投递出去的形状**（`path` = 那条消息节点自身的地址、
//! `data` = 那条 `ChatMessage`），而不是内部字段——形状才是线上契约。
//!
//! ⚠️ 投递有**合帧窗口**（[`DELIVER_WINDOW_MS`]）：相邻的同节点纯增量会并成一帧。
//! 因此断言「条数」的用例要么显式 `flush_pending`，要么让边界帧（终态 / 删除）
//! 把窗口顶出去；断言「正文」的用例则与窗口无关（合帧逐字等价）。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    MessageContent, MessageRole, MessageStatus, MessageType,
};
use std::sync::Mutex;

/// 一条带正文的完整消息（`content` = 整条替换）。
fn text_msg(id: &str, status: MessageStatus, text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        status: Some(status),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 一帧增量（`delta` = 追加）。
fn delta_msg(id: &str, delta: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        delta: Some(delta.to_string()),
        ..Default::default()
    }
}

/// 删除帧。
fn removed_msg(id: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        status: Some(MessageStatus::Removed),
        ..Default::default()
    }
}

/// 一张**带收件盒**的变更表：`seen` 收到每一条被投递的变更。
///
/// 订阅路径取 `""`（provider 根）——`related` 对空串恒命中，因此所有变更都会进来。
fn sink_subs() -> (ChangeSubscriptions, Arc<Mutex<Vec<VdfsChange>>>) {
    let subs = ChangeSubscriptions::default();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    subs.watch(
        "",
        Arc::new(move |c: VdfsChange| sink.lock().unwrap().push(c)),
    );
    (subs, seen)
}

/// 一份转写 + 它的收件盒。
fn transcript() -> (Transcript, Arc<Mutex<Vec<VdfsChange>>>) {
    let (subs, seen) = sink_subs();
    (Transcript::new("s1".into(), Arc::new(subs)), seen)
}

/// 收件盒快照（顺序即投递顺序）。
fn seen_of(seen: &Arc<Mutex<Vec<VdfsChange>>>) -> Vec<VdfsChange> {
    seen.lock().unwrap().clone()
}

/// 帧计数器单调：每个被投递的帧 +1；违例帧（同帧既带增量又带完整正文）不占号。
#[test]
fn frame_counter_is_monotonic_and_violations_consume_no_frame() {
    let (mut tr, seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, "你"));
    tr.apply(delta_msg("a", "好"));
    // 违例：同帧既带 delta 又带 content —— 该拼接还是该替换？语义不可判定
    tr.apply(cm::ChatMessage {
        id: "a".into(),
        delta: Some("!".into()),
        content: Some(MessageContent::Text("你好!".into())),
        ..Default::default()
    });
    tr.apply(text_msg("a", MessageStatus::Completed, "你好"));
    assert_eq!(tr.frame_no, 3, "违例帧不得消耗帧号（3 个合法帧）");
    assert_eq!(seen_of(&seen).len(), 3, "违例帧不得投递");
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "你好", "content 帧是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// 两个内容字段各自的语义：`delta` 追加、`content` 整条替换——互不干扰。
#[test]
fn delta_appends_and_content_replaces() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "ab"));
    tr.apply(delta_msg("a", "cd"));
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(ref t)) => assert_eq!(t, "abcd", "delta 逐字累积"),
        _ => panic!("应为 Text"),
    }
    // 替换：把累积结果整条换掉（工具输出被改写 / 编辑消息 / 权威副本对齐都走这里）
    tr.apply(text_msg("a", MessageStatus::Completed, "XYZ"));
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "XYZ", "content 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// 未知 id 的增量帧自给自足：就地建占位再追加，不依赖任何先行帧——
/// **位置序号也在这里就有**（不必等落库）。
#[test]
fn delta_for_unknown_id_creates_placeholder() {
    let (mut tr, seen) = transcript();
    tr.apply(delta_msg("ghost", "片段"));
    let n = tr.get("ghost").expect("增量帧自给自足，应建占位");
    assert_eq!(n.id, "ghost");
    match n.content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "片段"),
        _ => panic!("应为 Text"),
    }
    // 首帧发**全量副本**（图里已合并完毕）：消费端零回读即得完整基线
    let delivered = seen_of(&seen);
    assert_eq!(delivered.len(), 1);
    assert_eq!(
        delivered[0].path,
        message_path("s1", "ghost"),
        "落点是那条消息节点自身的地址"
    );
    let v = delivered[0].data.as_ref().expect("带载荷");
    assert!(v.get("delta").is_none(), "首帧不携带 delta");
    assert_eq!(v["content"], "片段", "载荷是合并后的全量正文");
    assert_eq!(
        n.seq,
        Some(INFLIGHT_SEQ_BASE),
        "在途消息创建时即拿到位置序号"
    );
}

/// 删除帧（`status = removed`）与 persisted 的图语义。
#[test]
fn removed_and_persisted_evict() {
    let (mut tr, seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, "1"));
    tr.apply(text_msg("b", MessageStatus::Streaming, "2"));
    tr.persisted(&["a".to_string()]);
    assert!(tr.get("a").is_none(), "persisted 后节点离开在途图");
    tr.apply(removed_msg("b"));
    assert!(tr.snapshot().is_empty(), "删除帧把节点移出在途图");
    // 删除帧照常占帧号：**删了什么**在时间线上必须可追溯
    assert_eq!(tr.frame_no, 3);
    let delivered = seen_of(&seen);
    let last = delivered.last().unwrap();
    assert_eq!(
        last.path,
        message_path("s1", "b"),
        "落点是那条消息节点自身的地址"
    );
    let v = last.data.as_ref().expect("带载荷");
    assert_eq!(
        v["status"], "removed",
        "删除语义在载荷状态上（信封无 deleted）"
    );
    assert!(v.get("delta").is_none(), "删除帧不携带正文增量");
}

/// 图里只留累积后的 `content`：`delta` 是传输形态，不得被留在节点上。
///
/// 这条断言保护的是持久层的不变量（`ensure_durable_states` 直接拒绝带 `delta`
/// 的消息）——图是在途投影的读取源，它一旦留住 `delta`，落库就会踩雷。
#[test]
fn delta_is_never_retained_on_graph_nodes() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "abc"));
    assert!(
        tr.get("a").unwrap().delta.is_none(),
        "图内节点不得携带 delta"
    );
}

/// 位置序号在**创建时**分配：在途消息（存储尚未写入）从此也有权威顺序。
///
/// 这条是本轮（ADR-025 §3.3）的核心：两条并行工具的消息在同一毫秒创建时，
/// 前端只能回退 `timestamp` ⇒ 并列、顺序不定。有了在途号，`ordered` 的
/// 「`seq` 升序」对在途消息同样成立。
#[test]
fn inflight_messages_get_their_position_seq_at_creation() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("t1", MessageStatus::Streaming, ""));
    tr.apply(text_msg("t2", MessageStatus::Streaming, ""));
    // 顺序与到达顺序一致，且都落在 `INFLIGHT_SEQ_BASE` 起的段内
    assert_eq!(tr.get("t1").unwrap().seq, Some(INFLIGHT_SEQ_BASE));
    assert_eq!(tr.get("t2").unwrap().seq, Some(INFLIGHT_SEQ_BASE + 1));
    // 同一节点的后续帧**不重新分配**（位置是节点属性，不是帧属性）
    tr.apply(delta_msg("t1", "x"));
    assert_eq!(tr.get("t1").unwrap().seq, Some(INFLIGHT_SEQ_BASE));

    // 存储分配的权威值（落库后回发的对齐帧）覆盖在途号
    tr.apply(cm::ChatMessage {
        id: "t1".into(),
        seq: Some(7),
        ..Default::default()
    });
    assert_eq!(tr.get("t1").unwrap().seq, Some(7));
}

/// 在途号**排在全部历史之后**——`ordered` 按 `seq` 升序，而存储分配的序号是
/// 「第几条消息」量级（远小于 `INFLIGHT_SEQ_BASE`）。于是「最新的消息在末尾」
/// 对在途消息同样成立。
#[test]
fn inflight_seq_sorts_after_all_stored_messages() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("old", MessageStatus::Completed, "历史"));
    tr.apply(text_msg("new", MessageStatus::Streaming, "在途"));
    let mut msgs = tr.snapshot();
    // `ordered` 的镜像（`nodes.rs::ordered`）：按 `seq` 升序
    msgs.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
    let ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["old", "new"], "在途消息排在历史之后");
}

/// 线上格式：`path`（**被变更节点自身**的地址）+ `data`（那条 `ChatMessage`）。
///
/// 语义全在 `data` 的字段上：`delta` 追加 / `content` 替换 / `status = removed`
/// 移除——不从类型反推，信封没有操作枚举。首帧发图里合并后的**全量**副本，
/// 消费端零回读即得完整基线。
#[test]
fn message_change_wire_shape_is_node_address_and_message_payload() {
    let (mut tr, seen) = transcript();
    // ① 首见：全量帧（正文基线）
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    // ② 增量：窄载荷 delta（**唯一**的零回读追加形态）
    tr.apply(delta_msg("a", "你"));
    // ③ 整条替换：全量载荷（content 整条给出，无需回读）
    tr.apply(text_msg("a", MessageStatus::Completed, "你好"));
    // ④ 删除
    tr.apply(removed_msg("a"));

    let d = seen_of(&seen);
    assert_eq!(d.len(), 4);
    let addr = message_path("s1", "a");
    assert!(
        d.iter().all(|c| c.path == addr),
        "落点恒为**被变更节点自身**的地址（同一条消息的四帧共用它）"
    );
    let v0 = d[0].data.as_ref().expect("①带载荷");
    assert_eq!(v0["id"], "a");
    assert!(v0.get("delta").is_none(), "全量帧不带 delta");
    let v1 = d[1].data.as_ref().expect("②带载荷");
    assert_eq!(v1["delta"], "你", "增量帧只带 delta（窄载荷）");
    let v2 = d[2].data.as_ref().expect("③带载荷");
    assert!(v2.get("delta").is_none(), "全量帧不带 delta");
    assert_eq!(v2["content"], "你好", "content 整条给出，消费端无需回读");
    let v3 = d[3].data.as_ref().expect("④带载荷");
    assert_eq!(
        v3["status"], "removed",
        "删除语义在载荷状态上（信封无 deleted）"
    );

    // 线上 JSON：`delta` 缺省时不出现该键（可选字段，`skip_serializing_if`）
    let v = serde_json::to_value(&d[0]).unwrap();
    assert!(v["data"].get("delta").is_none(), "无增量时不落 `delta` 键");
}

/// 首帧恰是纯增量 ⇒ 发图里合并后的**全量副本**，delta 不落线。
///
/// 为什么不是照原帧发窄增量：未知 id 上的 `{id, delta}` 让消费端拿不到正文
/// 基线（追加到什么之上不可知）——而图里的节点已经合并完毕，把这份自给自足的
/// 副本发出去，消费端零回读。为什么不是整条丢弃：丢掉首帧会让一个节点凭空
/// 消失，比丢一段增量难查得多。
#[test]
fn a_first_frame_carrying_delta_is_delivered_as_full_copy() {
    let (mut tr, seen) = transcript();
    tr.apply(delta_msg("a", "片段"));
    let d = seen_of(&seen);
    assert_eq!(d.len(), 1, "变更照常投递，不整条丢弃");
    let v = d[0].data.as_ref().expect("带载荷");
    assert!(v.get("delta").is_none(), "首帧不携带 delta");
    assert_eq!(v["content"], "片段", "首帧带合并后的全量正文");
    // 图里的正文不受影响（首帧形状只影响**投递**）
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "片段"),
        _ => panic!("应为 Text"),
    }
}

// ============================================================================
// 日志折行（`DeltaLogCoalescer`）
//
// 折行**只影响日志**：帧号不变，也不参与投递。下面这组用例一半在钉折行本身，
// 一半在钉「折行没有碰到图与正文」这条边界。
// ============================================================================

/// 连续同 id 的纯增量折成一行：带帧号区间、帧数、累计字符数。
#[test]
fn consecutive_deltas_on_one_node_collapse_into_a_single_line() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "12345678"), 4, true), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "x"), 5, true), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "yz"), 6, true), (None, true));
    // 显式冲刷
    let line = c.flush().expect("待合并的 run 应被冲刷");
    assert_eq!(line, "[T#4..6] a - Update 3 帧 / +11c");
    assert_eq!(c.flush(), None, "冲刷是幂等的，不重复产出");
}

/// 换 id 立即冲刷上一段，并为新 id 开新 run。
#[test]
fn switching_node_flushes_the_previous_run() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "123"), 1, true), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "45"), 2, true), (None, true));
    // 换 id：冲刷 a 的 run，b 自己开一段
    let (flushed, absorbed) = c.feed(&delta_msg("b", "6"), 3, true);
    assert_eq!(flushed.as_deref(), Some("[T#1..2] a - Update 2 帧 / +5c"));
    assert!(absorbed, "纯增量帧总会被并入");
    assert_eq!(c.flush().as_deref(), Some("[T#3] b - Update +1c"));
}

/// 非纯增量帧冲刷待合并的 run，且**自身不被并入**（它的行必须排在统计行之后）。
#[test]
fn a_non_delta_frame_flushes_the_run() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "12"), 1, true), (None, true));
    assert_eq!(c.feed(&delta_msg("a", "34"), 2, true), (None, true));
    // 终态帧（content、带 status）：冲刷统计行，自身不并入。
    // 它的 `foldable` 传什么都不影响结论——非纯增量帧本就不可能被并入。
    let (flushed, absorbed) = c.feed(&text_msg("a", MessageStatus::Completed, "1234"), 3, true);
    assert_eq!(flushed.as_deref(), Some("[T#1..2] a - Update 2 帧 / +4c"));
    assert!(!absorbed, "带状态的帧要打自己的行");
    assert_eq!(c.flush(), None, "非纯增量帧不留待合并状态");
}

/// **骨架帧不可折**：即便它恰好是纯增量（节点首帧的一种可能形态——`apply` 明确
/// 允许"未知 id 的增量帧自给自足"），也必须留下自己的一行。
///
/// 否则时间线会缺掉"某节点何时出现"这个起点——那正是折行要保住的骨架。
/// `foldable` 由调用方按级别给出（骨架 → `false`），本用例钉住这条边界。
#[test]
fn a_skeleton_frame_is_never_folded() {
    let mut c = DeltaLogCoalescer::default();
    let (flushed, absorbed) = c.feed(&delta_msg("a", "abc"), 1, false);
    assert_eq!(flushed, None, "没有上一段可冲刷");
    assert!(!absorbed, "骨架帧必须留下自己的一行");
    assert_eq!(c.flush(), None, "骨架帧不留待合并状态");

    // 折行段中途插进一帧骨架（终态）：统计行先冲刷，骨架帧自己留行。
    assert_eq!(c.feed(&delta_msg("a", "de"), 2, true), (None, true));
    let (flushed, absorbed) = c.feed(&text_msg("a", MessageStatus::Completed, "abcde"), 3, false);
    assert_eq!(flushed.as_deref(), Some("[T#2] a - Update +2c"));
    assert!(!absorbed);
}

/// 单帧 run 的形状与折行前**逐字相同**（只有一帧时日志形态不变）。
#[test]
fn a_single_frame_run_renders_exactly_like_before() {
    let mut c = DeltaLogCoalescer::default();
    assert_eq!(c.feed(&delta_msg("a", "abc"), 7, true), (None, true));
    assert_eq!(c.flush().as_deref(), Some("[T#7] a - Update +3c"));
}

// ============================================================================
// 帧日志分级（`FrameLogLevel` / `frame_log_of` / `render_frame_line`）
//
// 日志分两级后，"哪些帧常看、哪些帧要开 `--verbose` 才看"就不再是散在各处的
// 措辞，而是一条可断言的函数。下面这组用例钉住那条分界线本身。
// ============================================================================

/// 相位与级别的分界：**骨架** = 出现 / 等待用户 / 终态，**细节** = 其余。
#[test]
fn frame_log_splits_skeleton_from_detail() {
    use FrameLogLevel::{Detail, Skeleton};
    let some = Some;

    // 出现：不论带不带状态，首帧都是骨架（"某节点何时出现"必须在默认输出里）
    assert_eq!(frame_log_of(false, None), ("Start", Skeleton));
    assert_eq!(
        frame_log_of(false, some(&MessageStatus::Streaming)),
        ("Start", Skeleton)
    );
    // 等待用户：节点还在，但**要人做事**——必须可见
    assert_eq!(
        frame_log_of(true, some(&MessageStatus::WaitingUserAction)),
        ("Wait", Skeleton)
    );
    // 终态：增长停止
    for st in [
        MessageStatus::Completed,
        MessageStatus::Failed,
        MessageStatus::Aborted,
    ] {
        assert_eq!(
            frame_log_of(true, Some(&st)),
            ("End", Skeleton),
            "{st:?} 是终态，进骨架"
        );
    }
    // 其余（`pending → streaming` 的迁移、正文替换、仅 `meta` / 身份变更）都是细节
    assert_eq!(
        frame_log_of(true, some(&MessageStatus::Pending)),
        ("Update", Detail)
    );
    assert_eq!(
        frame_log_of(true, some(&MessageStatus::Streaming)),
        ("Update", Detail)
    );
    assert_eq!(frame_log_of(true, None), ("Update", Detail));
}

/// 一行里不落尾随空格——日志要能直接复制、能整齐对齐。
#[test]
fn render_frame_line_is_trimmed() {
    assert_eq!(
        render_frame_line(7, "abc", "completed", ""),
        "[T#7] abc completed"
    );
    assert_eq!(
        render_frame_line(7, "abc", "completed", "Start =12c"),
        "[T#7] abc completed Start =12c"
    );
}

/// 两类合并（日志折行 + 投递合帧）都**不改图、不丢正文**。
///
/// 这是合并改动的真正风险边界：折行是日志的统计，合帧是载荷的合并，两者都不能
/// 让正文少一个字符，也不能碰内存图。**帧号照逐帧计**——日志的时间线不因合帧
/// 而变短，那正是「一次回复有几百帧」这个事实要留下的地方。
#[test]
fn coalescing_keeps_the_graph_and_loses_no_text() {
    let frames: Vec<cm::ChatMessage> = vec![
        text_msg("a", MessageStatus::Streaming, ""),
        delta_msg("a", "你"),
        delta_msg("a", "好"),
        delta_msg("a", "世界"),
        text_msg("a", MessageStatus::Completed, "你好世界"),
        text_msg("b", MessageStatus::Streaming, ""),
        delta_msg("b", "!"),
        removed_msg("b"),
    ];

    let (mut tr, seen) = transcript();
    for f in frames {
        tr.apply(f);
    }

    // 8 个合法帧 ⇒ 帧号 8（合帧不得影响日志的帧计数）
    assert_eq!(tr.frame_no, 8, "合帧不得影响帧号");
    assert_eq!(tr.snapshot().len(), 1, "b 已被删除，在途图只剩 a");
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(ref t)) => {
            assert_eq!(t, "你好世界", "content 帧整条替换")
        }
        _ => panic!("应为 Text"),
    }

    // 正文不丢：把节点 a 的增量帧按投递顺序拼起来，必须恰好是它的完整正文
    //（窗口可能因调度被切成多段，因此这里断言**累积值**而不是投递条数）
    let appended: String = seen_of(&seen)
        .iter()
        .filter(|c| c.path == message_path("s1", "a"))
        .filter_map(|c| Some(c.data.as_ref()?.get("delta")?.as_str()?.to_string()))
        .collect();
    assert_eq!(appended, "你好世界", "合帧后正文仍逐字完整");
    assert!(
        seen_of(&seen).len() <= 8,
        "投递条数只可能少于帧数（合帧），不可能多出来"
    );

    // 折行器与投递窗口都不留残留状态
    assert_eq!(
        tr.delta_log.flush(),
        None,
        "apply 走完后不应残留待合并的 run"
    );
    assert!(tr.pending.is_none(), "apply 走完后不应残留待投递的增量");
}

/// **投递合帧**：相邻的同节点纯增量合成一帧，正文逐字等价。
///
/// 合帧是投递层的优化（协议里没有它的位置）：线上形状仍是 `{id, delta}`，
/// 消费端无从、也无需知道这一帧由几帧并成。
#[test]
fn consecutive_pure_deltas_on_one_node_are_merged_into_a_single_frame() {
    let (mut tr, seen) = transcript();
    // 首帧是全量副本（带 content），不进窗口
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "你"));
    tr.apply(delta_msg("a", "好"));
    tr.apply(delta_msg("a", "世界"));
    // 窗口尚未过期 ⇒ 三帧仍在待投递状态（事件本身按帧号进了日志）
    assert_eq!(tr.frame_no, 4, "帧号照逐帧计");
    assert_eq!(seen_of(&seen).len(), 1, "只有首帧出去了");

    // 终态帧（非纯增量）冲刷窗口：增量先出，终态后出
    tr.apply(text_msg("a", MessageStatus::Completed, "你好世界"));
    let d = seen_of(&seen);
    assert_eq!(d.len(), 3, "首帧 + 合并后的增量帧 + 终态帧");
    assert_eq!(d[1].path, message_path("s1", "a"));
    assert_eq!(
        d[1].data.as_ref().unwrap()["delta"],
        "你好世界",
        "三帧增量并成一帧，正文逐字不丢"
    );
    assert_eq!(d[2].data.as_ref().unwrap()["content"], "你好世界");
}

/// **换节点立即开新窗口**：合并只在「同一个文件的一段连续增长」内部发生。
///
/// 两个节点都先建立（首帧是全量副本、必然立刻发，不进窗口），之后才交错送增量
/// ——这样断言到的才是「换节点冲刷窗口」这条规则本身。
#[test]
fn switching_node_starts_a_new_window() {
    let (mut tr, seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(text_msg("b", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "A1"));
    // 换 id ⇒ 冲刷 a 的窗口，b 自己开一个
    tr.apply(delta_msg("b", "B1"));
    tr.apply(delta_msg("b", "B2"));
    // 显式冲刷，免得断言依赖真实时钟（窗口本身有独立用例）
    tr.flush_pending();

    let deltas: Vec<(String, String)> = seen_of(&seen)
        .iter()
        .filter_map(|c| {
            let v = c.data.as_ref()?;
            let delta = v.get("delta")?.as_str()?.to_string();
            Some((v.get("id")?.as_str()?.to_string(), delta))
        })
        .collect();
    assert_eq!(
        deltas,
        vec![
            ("a".to_string(), "A1".to_string()),
            ("b".to_string(), "B1B2".to_string()),
        ],
        "a 的窗口在换节点时被冲刷，b 的两帧合为一帧"
    );
}

/// 运行态帧**排在待投递的增量之后**：先收敛为「本轮结束」再补正文，前端会把
/// 活动角标与正文的先后搞反（`sessionNodeSync` 在 `working → 非 working` 时就清角标）。
#[test]
fn session_state_flushes_pending_delta_first() {
    let (mut tr, seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "尾段"));
    tr.emit_session_state(None);

    let d = seen_of(&seen);
    assert_eq!(d.len(), 3, "首帧 + 冲刷出的增量帧 + 运行态帧");
    assert_eq!(
        d[1].data.as_ref().unwrap()["delta"],
        "尾段",
        "增量必须先出，且必须在运行态之前"
    );
    assert_eq!(d[2].path, "s1", "运行态帧落在会话叶子上");
    assert!(tr.pending.is_none(), "运行态帧冲刷掉待投递窗口");
}

/// `clear` 与 `persisted` 都是冲刷点：最后一段增量不会被吞掉（日志与投递两侧）。
#[test]
fn clear_and_persisted_flush_the_trailing_run() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "abc"));
    tr.apply(delta_msg("a", "de"));
    // clear 之后不得残留（轮次边界）
    tr.clear();
    assert_eq!(tr.delta_log.flush(), None, "clear 应冲刷最后一段");
    assert!(tr.pending.is_none(), "clear 应冲刷待投递窗口");

    tr.apply(text_msg("a", MessageStatus::Streaming, ""));
    tr.apply(delta_msg("a", "xy"));
    tr.persisted(&["a".to_string()]);
    assert_eq!(
        tr.delta_log.flush(),
        None,
        "persisted（落库回执）应冲刷最后一段"
    );
    assert!(tr.pending.is_none(), "persisted 应冲刷待投递窗口");
}

/// 会话运行态与消息走**同一张订阅表**（ADR-025 的实时面与历史面合流）。
///
/// 挂载点是会话叶子 `<sid>` 本身（`data` = 全量节点视图，与 `stat` 同源），
/// 与消息的 `<sid>/message` 目录在同一棵地址树上——消费端一张 `vdfs/watch`
/// 覆盖两者。
#[test]
fn session_state_change_lands_on_the_session_leaf() {
    let (mut tr, seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Completed, "你好"));
    let node = crate::symbio_core::VdfsNode::file(
        "s1",
        "会话",
        crate::symbio_core::VdfsAccess::READ_WRITE,
    );
    tr.emit_session_state(Some(node));

    let d = seen_of(&seen);
    assert_eq!(d.len(), 2, "消息变更 + 运行态变更");
    assert_eq!(d[1].path, "s1", "运行态挂在会话叶子上（不是消息目录）");
    let v = d[1].data.as_ref().expect("运行态随载荷携带全量节点视图");
    assert!(v.get("status").is_some(), "视图携带运行态字段");
    assert!(v.get("delta").is_none(), "运行态不带正文增量");
    assert_eq!(tr.frame_no, 2, "运行态同样占帧号（时间线可追溯）");
}

/// 视图缺席（会话已删）⇒ 退化为无载荷变更：消费端回读 `NotFound` 即收敛。
#[test]
fn session_state_without_view_is_a_bare_change() {
    let (mut tr, seen) = transcript();
    tr.emit_session_state(None);
    let d = seen_of(&seen);
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].path, "s1");
    assert!(d[0].data.is_none(), "无载荷：回读收敛");
}

/// 运行态**不碰消息图**：它只占帧号 + 投递，图上一条消息都不动。
#[test]
fn session_state_frame_does_not_touch_the_message_graph() {
    let (mut tr, _seen) = transcript();
    tr.apply(text_msg("a", MessageStatus::Completed, "你好"));
    let before = tr.snapshot().len();

    tr.emit_session_state(None);

    assert_eq!(tr.snapshot().len(), before, "运行态帧不得往消息图里加节点");
    assert_eq!(
        tr.get("a").and_then(|m| m.status),
        Some(MessageStatus::Completed),
        "既有节点的状态也不得被运行态改写（状态只有一个来源）"
    );
}

/// 在途号与存储号是**两个互不相交的号段**，判据是纯号段比较。
///
/// 它是存储边界的守卫（`chat_session` 的两条写入路径用它摘号），所以边界值本身
/// 就是契约：`base - 1` 仍是权威号、`base` 是第一个在途号。
#[test]
fn is_inflight_seq_splits_authoritative_and_inflight_ranges() {
    assert!(!is_inflight_seq(0), "0 是合法的「尚未分配」水位");
    assert!(
        !is_inflight_seq(INFLIGHT_SEQ_BASE - 1),
        "在途号段下界之前必须是权威号（存储号是「第几条消息」量级）"
    );
    assert!(is_inflight_seq(INFLIGHT_SEQ_BASE), "下界本身是第一个在途号");
    assert!(is_inflight_seq(INFLIGHT_SEQ_BASE + 1));

    // 存量会话被泄漏抬高的水位（旧号段 `1 << 40`）必须仍被判为**权威号**：
    // 它已经躺在存储里，边界再把它当在途号摘掉就等于改写既有消息的顺序锚点。
    // 这也是 `INFLIGHT_SEQ_BASE` 必须抬到 `1 << 50` 的原因。
    assert!(!is_inflight_seq(1 << 40), "存量泄漏水位是已落库的权威号");
}
