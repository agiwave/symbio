//! 转写的**折叠**判据（S2.7-b1，D-053）。
//!
// doc-link-allow D-004: 同上，「四格 → 三角色」是静态映射非版本改造
//! 与「四格 → 三角色」的基本口径分开成文件：那些是**追加**，折叠是**改写**。
//! 混在一个文件里，后来的人会以为折叠只是又一种投影臂——而它补的是一个**正确性**
//! 缺口（少了它，一次截断在 prompt 面上等于没发生过）。

use super::transcript;
use crate::event::{Entity, Event, Verb};
use crate::test_fixtures::{
    CHAT, EVENT_CHAT_ASSISTANT_FINAL, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
    EVENT_USER_MESSAGE, MESSAGE,
};
use crate::Budget;

/// 投影成 `(role, text)` 序列（折叠关心的就是这两格）。
fn entries(events: &[Event]) -> Vec<(String, String)> {
    transcript(CHAT, MESSAGE)
        .apply(events, i64::MAX, Budget::generous())
        .value
        .entries
        .into_iter()
        .map(|e| (e.role, e.text))
        .collect()
}

fn user(turn: u64, node: &str, text: &str) -> Event {
    Event::pending(
        format!("u-{turn}"),
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        "user",
    )
    .with_payload(serde_json::json!({ "text": text, "tier": "deep", "id": node }))
}

/// 一条收束格。`text_id` 就是它在读侧的**节点身份**（与写侧 `produced_payload` 同源）。
fn assistant(turn: u64, text_id: &str, text: &str) -> Event {
    Event::pending(
        format!("f-{turn}"),
        EVENT_CHAT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "session/main",
    )
    .with_payload(serde_json::json!({
        "text": text,
        "model": "deep",
        "turn_id": format!("t-{turn}"),
        "text_id": text_id,
    }))
}

fn edited(node: &str, role: &str, text: &str) -> Event {
    Event::pending(
        format!("m-edit-{node}"),
        EVENT_MESSAGE_EDITED,
        Entity::Message,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({
        "message": { "id": node, "role": role, "content": text }
    }))
}

fn removed(ids: &[&str]) -> Event {
    Event::pending(
        "m-del-0",
        EVENT_MESSAGE_REMOVED,
        Entity::Message,
        Verb::Closed,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "ids": ids }))
}

/// 编辑：**换正文，不动位置**。
///
/// 「不动位置」是判据的一半：转写按事件顺序出，编辑若把条目挪到末尾，多轮 prompt
/// 的对话次序就乱了——而次序错乱会让模型把后说的话当成先说的。
#[test]
fn an_edit_replaces_the_text_in_place() {
    let events = vec![
        user(0, "u-0", "原始问题"),
        assistant(0, "f-0-text", "原始回答"),
        edited("u-0", "user", "改过的问题"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "改过的问题".to_string()),
            ("assistant".to_string(), "原始回答".to_string()),
        ],
        "编辑只换正文、不动位置"
    );
}

/// 编辑助手那条同理（节点身份走 `text_id`，与重建同一套 id）。
#[test]
fn an_edit_reaches_the_assistant_entry() {
    let events = vec![
        user(0, "u-0", "问"),
        assistant(0, "f-0-text", "原始回答"),
        edited("f-0-text", "assistant", "改过的回答"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "问".to_string()),
            ("assistant".to_string(), "改过的回答".to_string()),
        ]
    );
}

/// 删除：条目**没了**（截断的读侧后果）。
///
/// 这是本批最重要的一条：少了折叠，模型下一轮仍看得见被删掉的那几句——
/// 而用户以为自己删掉了。
#[test]
fn a_removal_drops_the_entry() {
    let events = vec![
        user(0, "u-0", "第一问"),
        assistant(0, "f-0-text", "第一答"),
        user(1, "u-1", "第二问"),
        assistant(1, "f-1-text", "第二答"),
        // 从第二问起截断（起点 + 其后全部）
        removed(&["u-1", "t-1", "f-1-text"]),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "第一问".to_string()),
            ("assistant".to_string(), "第一答".to_string()),
        ],
        "被截断的那一轮不得留在转写里"
    );
}

/// 折叠**不依赖事件顺序**：变更格排在它改的那条之前也照折。
///
/// 生产里变更格恒在后面（先有事实才有「它变了」），但切片可能被裁过 / 手工改过。
/// 依赖顺序的实现在那些情形下会静默漏折——而「静默」正是本条要消灭的形态。
#[test]
fn folding_does_not_depend_on_event_order() {
    let events = vec![
        removed(&["f-0-text"]),
        edited("u-0", "user", "改过的问题"),
        user(0, "u-0", "原始问题"),
        assistant(0, "f-0-text", "原始回答"),
    ];
    assert_eq!(
        entries(&events),
        vec![("user".to_string(), "改过的问题".to_string())]
    );
}

/// 平凡值：没有变更格 ⇒ 逐字等于旧行为（本批的零行为变更承诺）。
#[test]
fn without_mutations_nothing_changes() {
    let events = vec![user(0, "u-0", "问"), assistant(0, "f-0-text", "答")];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "问".to_string()),
            ("assistant".to_string(), "答".to_string()),
        ]
    );
}

/// 定位不上的编辑是**空操作**，不是「多出一条」。
///
/// 载荷里的 id 与任何条目都对不上（v1 时代的历史消息 / 切片被裁过）⇒ 什么也不做。
/// 「硬插一条」会让一段历史在错误的时刻冒出一句话。
#[test]
fn an_edit_for_an_unknown_node_is_a_no_op() {
    let events = vec![user(0, "u-0", "问"), edited("ghost", "user", "谁都不是")];
    assert_eq!(
        entries(&events),
        vec![("user".to_string(), "问".to_string())]
    );
}

// ── 压缩快照：**换基**（取代格，S2.7-b2）──────────────────────────────

/// 一条 `session.consolidated`（载荷 `messages` = 压缩后的整份列表）。
fn snapshot(tag: &str, msgs: Vec<serde_json::Value>) -> Event {
    Event::pending(
        format!("snap-{tag}"),
        crate::test_fixtures::EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({ "messages": msgs }))
}

/// 快照列表里的一条消息（`ty = None` 即不写 `type`，与折进的补充同形）。
fn msg(id: &str, role: &str, ty: Option<&str>, text: &str) -> serde_json::Value {
    let mut v = serde_json::json!({ "id": id, "role": role, "content": text });
    if let Some(t) = ty {
        v["type"] = serde_json::json!(t);
    }
    v
}

/// 投影出的**完整条目**（折叠关心的不止 `(role, text)`——工具名与「不进请求包」
/// 标记也是读侧语义）。
fn full(events: &[Event]) -> Vec<super::TranscriptEntry> {
    transcript(CHAT, MESSAGE)
        .apply(events, i64::MAX, Budget::generous())
        .value
        .entries
}

/// 快照**取代**前缀：它之前的事件不参与，条目从载荷那份列表起。
///
/// 少了这一段，一次压缩在 prompt 面上等于**没发生过**——被压掉的历史全部复活，
/// 而压缩的全部意义就是让它们不再进入上下文。
#[test]
fn a_snapshot_replaces_the_superseded_prefix() {
    let events = vec![
        user(0, "u-0", "被压掉的问题"),
        assistant(0, "f-0", "被压掉的答复"),
        snapshot(
            "a",
            vec![
                msg("snap-root", "user", Some("body"), "[CONTEXT SNAPSHOT] 摘要"),
                msg("u-1", "user", Some("body"), "保留的近轮问题"),
                msg("f-1", "assistant", Some("body"), "保留的近轮答复"),
            ],
        ),
        user(2, "u-2", "压缩之后的问题"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "[CONTEXT SNAPSHOT] 摘要".to_string()),
            ("user".to_string(), "保留的近轮问题".to_string()),
            ("assistant".to_string(), "保留的近轮答复".to_string()),
            ("user".to_string(), "压缩之后的问题".to_string()),
        ],
        "换基之后：快照载荷 + 其后的事实，被压掉的一条都不在"
    );
}

/// **换基边界上的同一个节点只产一行**（自动压缩把本轮的用户发言**原文**留在快照载荷
/// 尾部，而那条发言同时是本轮的开场格 `turn × opened`——它落在快照**之后**）。
///
/// 少了这一步，每次压缩之后对话网格里就多一条重复的用户发言，而 `rebuild` 那边只认
/// 一条（它的 `at` 先被快照播种）——两个出口对同一段历史给两种读法，正是本仓反复栽的
/// **静默**分叉。这条判据由 e2e 的真实会话负载抓出来（`e2e/cases/t47-real-session-soak.mjs`）。
#[test]
fn the_user_turn_a_snapshot_carries_is_not_emitted_twice() {
    let events = vec![
        user(0, "u-0", "被压掉的问题"),
        snapshot(
            "a",
            vec![
                msg("snap-root", "user", Some("body"), "摘要"),
                msg(
                    "u-1",
                    "user",
                    Some("body"),
                    "本轮的发言（快照尾部留着原文）",
                ),
            ],
        ),
        user(1, "u-1", "本轮的发言（快照尾部留着原文）"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "摘要".to_string()),
            (
                "user".to_string(),
                "本轮的发言（快照尾部留着原文）".to_string()
            ),
        ],
        "基线占了的节点身份，快照之后的同 id 事实不再产第二行"
    );
}

/// 去重**只**作用在换基边界，不在事实事件彼此之间：兜底 / 汇报那几格的节点身份是按
/// `turn_id` 派生的合成 id（[`crate::event_produced_text_node_id`]），同一轮里
/// 两条不同事件可能撞到同一个合成 id，而它们在 `rebuild` 里是两个不同的节点——把它们
/// 合并就是**另一种**分叉。这条钉住判据的边界，免得后来人把它「顺手」改成全局去重。
#[test]
fn two_live_events_sharing_a_node_id_are_both_projected() {
    let events = vec![
        assistant(0, "t-0-text", "等用户时说的话"),
        assistant(0, "t-0-text", "同一轮后来的收束"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("assistant".to_string(), "等用户时说的话".to_string()),
            ("assistant".to_string(), "同一轮后来的收束".to_string()),
        ],
        "没有快照就没有换基边界：事实事件彼此不按时份 id 合并"
    );
}

/// **只认最近一次**：更早的快照连它自己的列表都已被后来的快照取代。
#[test]
fn only_the_latest_snapshot_counts() {
    let events = vec![
        user(0, "u-0", "更早的问题"),
        snapshot("a", vec![msg("s-a", "user", Some("body"), "第一份摘要")]),
        user(1, "u-1", "中间的问题"),
        snapshot(
            "b",
            vec![
                msg("s-b", "user", Some("body"), "第二份摘要"),
                msg("u-2", "user", Some("body"), "保留的近轮"),
            ],
        ),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "第二份摘要".to_string()),
            ("user".to_string(), "保留的近轮".to_string()),
        ]
    );
}

/// 空列表 / 坏载荷都**不是**快照 ⇒ 历史照常折。
///
/// 容错方向必须是「少折」而不是「多抹」：把一条坏载荷当快照，后果是整段历史
/// 被抹掉——而列表短了没有任何东西会红。
#[test]
fn an_empty_or_malformed_snapshot_does_not_wipe_the_history() {
    let broken = Event::pending(
        "snap-x",
        crate::test_fixtures::EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({ "messages": "我不是数组" }));
    let base = vec![user(0, "u-0", "问"), assistant(0, "f-0", "答")];
    let mut with_broken = base.clone();
    with_broken.push(broken);
    assert_eq!(entries(&base), entries(&with_broken), "坏载荷不得抹掉历史");
    let mut with_empty = base.clone();
    with_empty.push(snapshot("empty", Vec::new()));
    assert_eq!(entries(&base), entries(&with_empty), "空列表不得抹掉历史");
}

/// 快照**之后**的变更格照常折叠（换基不是终点，后面还有一整段 live 事实要折）。
#[test]
fn mutations_after_the_snapshot_still_fold() {
    let events = vec![
        user(0, "u-0", "被压掉的"),
        snapshot(
            "a",
            vec![
                msg("snap-root", "user", Some("body"), "摘要"),
                msg("u-keep", "user", Some("body"), "保留的"),
                msg("u-drop", "user", Some("body"), "保留但被删"),
            ],
        ),
        edited("u-keep", "user", "保留的（改过）"),
        removed(&["u-drop"]),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "摘要".to_string()),
            ("user".to_string(), "保留的（改过）".to_string()),
        ]
    );
}

/// 快照里的工具结果：`tool` 名取**父 ToolCall 节点**的 `meta.tool_name`
/// （结果子节点只记 `parent_id`——事件投影里工具名来自 `artifact.added` 载荷，
/// 两条路径取的是同一件事）。
#[test]
fn a_snapshot_tool_result_keeps_the_name_from_its_parent_call() {
    let mut call = msg("T-call", "assistant", Some("skeleton"), "{\"path\":\"a\"}");
    call["meta"] = serde_json::json!({ "style_tag": "tool", "tool_name": "vdfs_read" });
    let mut result = msg("T-res", "tool", Some("body"), "文件正文");
    result["parent_id"] = serde_json::json!("T-call");
    let events = vec![snapshot("a", vec![call, result])];
    let got = full(&events);
    assert_eq!(got.len(), 1, "组合节点（ToolCall）自己不产行");
    assert_eq!(got[0].role, "tool");
    assert_eq!(got[0].text, "文件正文");
    assert_eq!(
        got[0].tool.as_deref(),
        Some("vdfs_read"),
        "工具名必须带上：role=tool 没名字会让 provider 整条拒绝"
    );
}

/// 快照里的**界面文本**（`meta.exclude_from_context`）进转写但**不进请求包**。
#[test]
fn a_snapshot_report_node_stays_out_of_the_request_package() {
    let mut report = msg("r-0", "assistant", Some("body"), "已经完成 1 轮工具调用");
    report["meta"] = serde_json::json!({ "exclude_from_context": true });
    let events = vec![
        user(0, "u-0", "问"),
        snapshot("a", vec![msg("s", "user", Some("body"), "摘要"), report]),
        user(2, "u-2", "压缩之后的问"),
    ];
    let view = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, Budget::generous())
        .value;
    assert_eq!(
        view.entries.len(),
        3,
        "摘要 + 界面文本 + 压缩后的问：界面文本仍在转写里（用户看得见）"
    );
    assert_eq!(view.excluded().len(), 1, "且被标出来");
    let sent: Vec<String> = view.to_messages().into_iter().map(|m| m.text).collect();
    assert!(
        !sent.iter().any(|t| t.contains("已经完成 1 轮")),
        "界面文本不得进请求包：模型会以为那是它自己刚说的"
    );
}

// ── 分支的读法（D-173）──────────────────────────────────────────────────
//
// 这一组钉的是「**送给模型的那一份**只有一支」——它是重试唯一的外部可见效果。
// 标注与选中态本身在 `projection::branch` 的用例里钉，这里只钉折叠之后的**请求包**。

/// 一条 `turn.branch`：`into` 那一支接管读法。
fn branch(of: &str, into: &str) -> Event {
    Event::pending(
        format!("branch-{into}"),
        crate::test_fixtures::EVENT_TURN_BRANCH,
        Entity::Turn,
        Verb::Asserted,
        2,
        "person/local",
    )
    .with_payload(serde_json::json!({ "of": of, "into": into }))
}

/// 重试之后：旧支的答复**仍在转写里**（切回去还看得见），但**不在请求包里**。
///
/// 反面是「重试了但答案一模一样」——那是这条折叠缺失唯一的外部症状，而它不报错：
/// 事实照常入格、`session/stats` 照常有数，只有模型读到的还是用户刚刚不要的那一份。
#[test]
fn the_replaced_branch_stays_out_of_the_request_package() {
    let events = vec![
        user(1, "u-1", "问"),
        assistant(1, "x-1", "甲答复"),
        branch("t-1", "t-2"),
        assistant(2, "x-2", "乙答复"),
    ];
    let view = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, Budget::generous())
        .value;
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "问".to_string()),
            ("assistant".to_string(), "甲答复".to_string()),
            ("assistant".to_string(), "乙答复".to_string()),
        ],
        "两支的正文都留在转写里（隐藏是读法，不是删除）"
    );
    let sent: Vec<String> = view.to_messages().into_iter().map(|m| m.text).collect();
    assert_eq!(
        sent,
        vec!["问".to_string(), "乙答复".to_string()],
        "请求包里只剩当前那一支，而**问句必须留着**——它是两支的共同祖先"
    );
}

/// 用户点切换器换回旧支 ⇒ 请求包跟着换：读法由**两类事实共同说**，后到的胜出。
#[test]
fn switching_back_moves_the_request_package_back() {
    let mut events = vec![
        user(1, "u-1", "问"),
        assistant(1, "x-1", "甲答复"),
        branch("t-1", "t-2"),
        assistant(2, "x-2", "乙答复"),
    ];
    events.push(
        Event::pending(
            "sel-1",
            crate::test_fixtures::EVENT_TURN_BRANCH_SELECTED,
            Entity::Turn,
            Verb::Asserted,
            3,
            "person/local",
        )
        .with_payload(serde_json::json!({ "selected": "t-1" })),
    );
    let sent: Vec<String> = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, Budget::generous())
        .value
        .to_messages()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        sent,
        vec!["问".to_string(), "甲答复".to_string()],
        "切回旧支后模型读的必须是旧支"
    );
}

/// 被选中那一支后来被整段截断 ⇒ 退回在世的最后一支，**不许把整组剪空**。
///
/// 剪空的后果比读错一支严重：那一轮的问题在请求包里一个答复都没有，模型不知道自己
/// 上一轮说了什么。
#[test]
fn a_truncated_selected_branch_falls_back_instead_of_hiding_everything() {
    let events = vec![
        user(1, "u-1", "问"),
        assistant(1, "x-1", "甲答复"),
        branch("t-1", "t-2"),
        assistant(2, "x-2", "乙答复"),
        removed(&["t-2", "x-2"]),
    ];
    let sent: Vec<String> = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, Budget::generous())
        .value
        .to_messages()
        .into_iter()
        .map(|m| m.text)
        .collect();
    assert_eq!(
        sent,
        vec!["问".to_string(), "甲答复".to_string()],
        "乙那支已被删掉（条目本身也不在），甲必须照读"
    );
}
