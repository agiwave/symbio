//! 双向完整性判据的**反向自检**与形状判据。
//!
//! ## 反向自检是这个模块的全部意义
//!
//! 一个恒绿的守卫证明不了自己有用。本文件的第一条用例**故意制造四类缺口**并断言判据
//! **逐类报出来**——把四条判据从「代码写了」变成「报得出」。
//!
//! 第二条用例用**真实运行器**跑一轮带工具调用的对话，断言双向完整——它是这套判据在
//! 生产路径上的第一条绿线，后续每一步（prompt 结构化、`turn.supplemented`）都必须保持
//! 它绿。

use super::super::transcript::{transcript, PromptMessage};
use super::{verify, PromptGap as Gap};
use crate::event::{Entity, Event, Verb};
use crate::test_fixtures::{
    CHAT, EVENT_ARTIFACT_ADDED, EVENT_CHAT_ASSISTANT_FINAL, EVENT_CHAT_ASSISTANT_REPORTED,
    EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_USER_MESSAGE, MESSAGE,
};

/// 一条送进模型的消息。
///
/// ⚠️ 这里**必须**构造 `PromptMessage` 而不是 `TranscriptEntry`：判据验的是
/// 「送进模型的那批」，而那条路上 `tool` 消息还带着 `tool_call_id`。用投影条目
/// 构造会让判据少验一样东西（而那恰恰是 D-048a 要保的东西）。
fn entry(role: &str, text: &str, tool: Option<&str>) -> PromptMessage {
    PromptMessage {
        role: role.to_string(),
        text: text.to_string(),
        // `role == "tool"` 必带调用 id（模型侧协议要求）——见 transcript 的
        // `From<&TranscriptEntry>`：缺了会被 provider 整条拒绝。
        tool_call_id: tool.map(|t| format!("call_{t}_0")),
        tool: tool.map(|t| t.to_string()),
        tool_calls: None,
    }
}

fn user_turn(turn: u64, text: &str) -> Event {
    Event::pending(
        format!("u-{turn}"),
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        "user",
    )
    .with_payload(serde_json::json!({ "text": text, "tier": "deep", "id": format!("u-{turn}") }))
}

fn assistant_turn(turn: u64, text: &str) -> Event {
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
        "text_id": format!("f-{turn}-text"),
    }))
}

fn artifact(turn: u64, tool: &str, text: &str) -> Event {
    Event::pending(
        format!("a-{turn}-0"),
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        turn,
        "session/main",
    )
    .with_payload(serde_json::json!({ "tool": tool, "text": text }))
}

/// **反向自检**：四类缺口各造一个，断言判据**逐类**报出来。
///
/// 把「新增一条事件 kind」也纳入：它落在 `expected_role` 的 `_ => None` 分支上，
/// 不改判据就会被静默当成「不在投影范围内」——而那正是**丢事实**的一种形态。
#[test]
fn the_fidelity_check_reports_every_gap_it_claims_to() {
    let events = vec![
        user_turn(0, "帮我回显"),
        assistant_turn(0, "回显完成"),
        artifact(0, "vdfs_read", "文件内容：hello"),
        // 投影不认识的新 kind。
        //
        // ⚠️ 这一条**换过**：原先用 `turn.supplemented`（P3c 落地前它确实不在投影里）。
        // P3c 补进投影后忘了登记进 `expected_role`，于是它一直被当成「不在投影范围」
        // 跳过——**补充漏进请求包时判据不会红**。所以现在补了登记，而这条用例必须
        // 换一个真正未知的 kind，否则它钉的是「有东西没登记」而不是「投影不认识它」。
        Event::pending(
            "x-0",
            "future.kind.not_in_projection",
            Entity::Turn,
            Verb::Asserted,
            0,
            "user",
        )
        .with_payload(serde_json::json!({ "text": "不认识的一格" })),
    ];

    // 三条投影内的 + 一条网格外的（无出处），另：tool entry 不带工具名
    let entries = vec![
        entry("user", "帮我回显", None),
        // assistant 那条被丢掉 ⇒ 网格→prompt 缺口
        entry("tool", "文件内容：hello", None), // 没带工具名 ⇒ role 退化
        entry("assistant", "事实源里没有这句", None), // 无出处
    ];

    let rep = verify(&events, &entries, None, &CHAT, &MESSAGE);
    assert!(!rep.is_complete(), "判据必须报出缺口（否则它恒绿、无用）");
    assert_eq!(rep.checked_events, 3, "只把投影内的三条计入");
    assert_eq!(rep.checked_entries, 3);

    // ① 网格 → prompt：assistant 那条被丢
    assert!(
        rep.gaps.iter().any(|g| matches!(
            g,
            Gap::DroppedFromPrompt { kind, turn: 0 } if kind == EVENT_CHAT_ASSISTANT_FINAL
        )),
        "必须报出「网格有、prompt 没有」：{}",
        rep.summary()
    );
    // ② role 退化：tool 没带工具名
    assert!(
        rep.gaps
            .iter()
            .any(|g| matches!(g, Gap::ToolEntryWithoutName { .. })),
        "必须报出「role=tool 却没带工具名」：{}",
        rep.summary()
    );
    // ③ prompt → 网格：无出处
    assert!(
        rep.gaps
            .iter()
            .any(|g| matches!(g, Gap::NoSourceInGrid { role, .. } if role == "assistant")),
        "必须报出「prompt 里有事实源没有的东西」：{}",
        rep.summary()
    );
    // ④ 角色不符
    let role_bad = {
        let evs = vec![user_turn(0, "问")];
        let msgs = vec![entry("assistant", "问", None)];
        verify(&evs, &msgs, None, &CHAT, &MESSAGE)
    };
    assert!(
        role_bad.gaps.iter().any(|g| matches!(
            g,
            Gap::RoleMismatch { expected, .. } if expected == "user"
        )),
        "必须报出「角色不符」：{}",
        role_bad.summary()
    );
    // ⑤ 界面文本泄漏进请求包（缺口 4 的判据方向）
    let leaked = {
        let evs = vec![user_turn(0, "问"), report(0, "已经完成 1 轮工具调用")];
        let msgs = vec![
            entry("user", "问", None),
            entry("assistant", "已经完成 1 轮工具调用", None),
        ];
        verify(&evs, &msgs, None, &CHAT, &MESSAGE)
    };
    assert!(
        leaked.gaps.iter().any(
            |g| matches!(g, Gap::ExcludedLeakedIntoPrompt { kind, .. } if kind == EVENT_CHAT_ASSISTANT_REPORTED)
        ),
        "必须报出「界面文本漏进请求包」：{}",
        leaked.summary()
    );
}

/// 轮边界汇报一格事实（缺口 4）。
fn report(turn: u64, text: &str) -> Event {
    Event::pending(
        format!("r-{turn}"),
        EVENT_CHAT_ASSISTANT_REPORTED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "session/main",
    )
    .with_payload(serde_json::json!({ "text": text, "tool_rounds": 1, "quiet_ms": 0 }))
}

/// 缺口 4 的**正向**判据：汇报落了格 ⇒ 转写里有它、送模型的那批里**没有**它。
///
/// 两侧都要钉，缺一不可：
/// - 只有「不在请求包里」⇒ 事件压根没落格也绿（那是缺口的**原症状**）；
/// - 只有「在转写里」⇒ 界面文本污染对话也绿（那是**反向**症状，线上会出现连续两条
///   assistant）。
#[test]
fn a_progress_report_lands_but_stays_out_of_the_request() {
    let events = vec![
        user_turn(0, "跑个长任务"),
        report(0, "已经完成 1 轮工具调用，用时约 3 秒，还在继续。"),
    ];
    let view = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, crate::Budget::generous())
        .value;

    let excluded = view.excluded();
    assert_eq!(
        excluded.len(),
        1,
        "汇报必须被投影收下（用户看得见）：{view:?}"
    );
    assert_eq!(excluded[0].role, "assistant", "它是助手说的话");

    let messages = view.to_messages();
    assert_eq!(
        messages.len(),
        1,
        "送模型的那批**不得**含汇报（否则线上出现连续两条 assistant）：{messages:?}"
    );
    assert_eq!(messages[0].role, "user");

    let rep = verify(&events, &messages, None, &CHAT, &MESSAGE);
    assert!(rep.is_complete(), "汇报轮必须双向完整：{}", rep.summary());
    assert_eq!(rep.checked_events, 2, "汇报也在判据覆盖范围内");
}

/// 平凡值：单轮无工具 ⇒ 双向完整（判据在最简单的形状上不能有假红）。
#[test]
fn a_single_turn_is_complete() {
    let events = vec![user_turn(0, "你好"), assistant_turn(0, "你好呀")];
    let entries = vec![
        entry("user", "你好", None),
        entry("assistant", "你好呀", None),
    ];
    let rep = verify(&events, &entries, None, &CHAT, &MESSAGE);
    assert!(rep.is_complete(), "平凡值必须无假红：{}", rep.summary());
}

/// 变更格（**第三类**）：既不产 entry、也不是「按设计不进请求包」。
///
/// 三条断言各钉一件事，缺一条这类就会被**静默跳过**（那正是本模块自己警告过的
/// 失败形态：`turn.supplemented` 漏登记那次）：
///
/// 1. **计入**（`mutations`）——不是「不在投影范围内」；
/// 2. 编辑过的原事件按**新**正文复核 ⇒ 折叠正确时**完整**（否则每次编辑都会报
///    「网格→prompt 丢事实」：网格里是旧正文、prompt 里是新正文，两边都「对」）；
/// 3. 没折叠时**报得出来**（prompt 里还是旧正文 ⇒ 丢事实；被删的那句还在
///    ⇒ 无出处）。
#[test]
fn mutation_events_are_folded_not_silently_skipped() {
    let events = vec![
        user_turn(0, "原始问题"),
        assistant_turn(0, "原始回答"),
        edited("u-0", "user", "改过的问题"),
        removed(&["f-0-text"]),
    ];

    // ① + ② 正确折叠后的那批（转写的产出）：改过的那句在，被删的那句没了。
    let folded = vec![entry("user", "改过的问题", None)];
    let rep = verify(&events, &folded, None, &CHAT, &MESSAGE);
    assert_eq!(rep.mutations, 2, "两个变更格都要计入：{}", rep.summary());
    assert!(
        rep.is_complete(),
        "折叠正确时必须完整（否则每次编辑都假红）：{}",
        rep.summary()
    );

    // ③a 没折叠：prompt 里还是**旧**正文 ⇒ 网格→prompt 丢事实。
    let unfolded = vec![entry("user", "原始问题", None)];
    let rep_unfolded = verify(&events, &unfolded, None, &CHAT, &MESSAGE);
    assert!(
        rep_unfolded
            .gaps
            .iter()
            .any(|g| matches!(g, Gap::DroppedFromPrompt { .. })),
        "没折叠必须报出来：{}",
        rep_unfolded.summary()
    );

    // ③b 没折叠：**被删的那句**还在 prompt 里 ⇒ 反方向报「无出处」
    //（它已经没有事件认领了）。
    let leaked = vec![
        entry("user", "改过的问题", None),
        entry("assistant", "原始回答", None),
    ];
    let rep_leaked = verify(&events, &leaked, None, &CHAT, &MESSAGE);
    assert!(
        rep_leaked
            .gaps
            .iter()
            .any(|g| matches!(g, Gap::NoSourceInGrid { role, .. } if role == "assistant")),
        "被删的那句留在 prompt 里必须报出来：{}",
        rep_leaked.summary()
    );
}

/// 变更格的载荷构造函数（与写侧 `message_events` 落下的形状逐字同源）。
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

// ── 第四类：取代格（压缩快照）────────────────────────────────

/// 一条 `session.consolidated`（载荷 `messages` = 压缩后的整份列表）。
fn snapshot(msgs: &[(&str, &str, &str)]) -> Event {
    let messages: Vec<serde_json::Value> = msgs
        .iter()
        .map(|(id, role, text)| {
            serde_json::json!({ "id": id, "role": role, "type": "body", "content": text })
        })
        .collect();
    Event::pending(
        "snap-0",
        crate::test_fixtures::EVENT_SESSION_CONSOLIDATED,
        Entity::Message,
        Verb::Progressed,
        0,
        "session/main",
    )
    .with_payload(serde_json::json!({ "messages": messages }))
}

/// **压缩之后判据必须仍为绿**：快照的出处**就是它自己**（被压掉的那些事件已不在
/// 主张范围内，快照载荷承载了它们的历史）。
///
/// 少了这条分支，每次压缩之后判据都会把「快照 + 被保留的近轮」整批报成
/// `NoSourceInGrid`——一个**恒红**的守卫，与没有守卫等价。
#[test]
fn a_compacted_history_is_complete_after_the_snapshot() {
    let events = vec![
        user_turn(0, "被压掉的问题"),
        assistant_turn(0, "被压掉的答复"),
        snapshot(&[
            ("snap-root", "user", "[CONTEXT SNAPSHOT] 摘要"),
            ("u-1", "user", "保留的近轮问题"),
            ("f-1-text", "assistant", "保留的近轮答复"),
        ]),
        user_turn(2, "压缩之后的问题"),
    ];
    let msgs = vec![
        entry("user", "[CONTEXT SNAPSHOT] 摘要", None),
        entry("user", "保留的近轮问题", None),
        entry("assistant", "保留的近轮答复", None),
        entry("user", "压缩之后的问题", None),
    ];
    let rep = verify(&events, &msgs, None, &CHAT, &MESSAGE);
    assert!(rep.is_complete(), "压缩之后必须双向完整：{}", rep.summary());
    assert_eq!(
        rep.consolidations, 1,
        "换基要计入——并进 checked_events 就看不出「换基有没有发生」"
    );
}

/// **换基边界上的同一条发言只认领一次**（自动压缩把本轮的用户发言原文留在快照载荷尾部，
/// 而它就是本轮的开场格 ⇒ 那条事实落在快照**之后**）。
///
/// 少了这一步，复核会对同一条发言主张两次：第一次用掉请求包里那一条，第二次找不到
/// 「还没被用掉」的匹配，于是每次压缩之后每轮都报一次「网格→prompt 丢事实」——而
/// prompt 一个字都不缺。这条恒假警报由 e2e 的真实会话负载暴露
/// （`e2e/cases/t47-real-session-soak.mjs`），它与 `transcript` 的换基边界去重是**同一条**
/// 纪律的两个出口。
#[test]
fn the_user_turn_a_snapshot_carries_claims_only_once() {
    let events = vec![
        user_turn(0, "被压掉的问题"),
        snapshot(&[
            ("snap-root", "user", "[CONTEXT SNAPSHOT] 摘要"),
            // 快照尾部留着**本轮**发言的原文，节点身份与下面那条开场格同一个。
            ("u-1", "user", "本轮的发言"),
        ]),
        user_turn(1, "本轮的发言"),
    ];
    let msgs = vec![
        entry("user", "[CONTEXT SNAPSHOT] 摘要", None),
        entry("user", "本轮的发言", None),
    ];
    let rep = verify(&events, &msgs, None, &CHAT, &MESSAGE);
    assert!(
        rep.is_complete(),
        "同一条发言两个出处不是两个缺口：{}",
        rep.summary()
    );
}

/// 反向：快照载荷里的一条**没进 prompt** ⇒ 报「丢事实」。
///
/// 这条证明上面那条绿不是恒绿：把一条摘要消息从 prompt 里拿掉，判据当场红。
#[test]
fn a_snapshot_entry_missing_from_the_prompt_is_reported() {
    let events = vec![snapshot(&[
        ("s", "user", "摘要"),
        ("u-1", "user", "保留的近轮"),
    ])];
    let msgs = vec![entry("user", "摘要", None)];
    let rep = verify(&events, &msgs, None, &CHAT, &MESSAGE);
    assert!(
        rep.gaps
            .iter()
            .any(|g| matches!(g, Gap::DroppedFromPrompt { .. })),
        "必须报出「快照条目没进 prompt」：{}",
        rep.summary()
    );
}

// ── 分支的读法（D-173）：未选中支**按设计**不在请求包里 ──────────────────
//
// 判据与投影必须认同一份折法（`branch::unselected_roots`）。这一节钉的是判据那一侧：
// 它若自己算一遍读法，重试过的会话就会每次请求报一串「丢事实」——恒红的守卫与没有
// 守卫等价；而它若跟着投影一起松掉，「旧支泄漏进请求包」就没人看着了。

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

fn branch_events() -> Vec<Event> {
    vec![
        user_turn(1, "问"),
        assistant_turn(1, "甲答复"),
        branch("t-1", "t-2"),
        assistant_turn(2, "乙答复"),
    ]
}

/// 重试过的会话：判据必须**绿**（旧支不在请求包是读法，不是丢事实）。
#[test]
fn a_replaced_branch_does_not_make_the_judge_red() {
    let events = branch_events();
    let msgs = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, crate::Budget::generous())
        .value
        .to_messages();
    assert_eq!(
        msgs.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
        vec!["问", "乙答复"],
        "投影只送当前那一支"
    );
    let rep = verify(&events, &msgs, None, &CHAT, &MESSAGE);
    assert!(rep.is_complete(), "重试轮必须双向完整：{}", rep.summary());
}

/// 反向自检：旧支**泄漏**进请求包时判据必须报出来。
///
/// 上面那条绿单独不构成证据——判据若跟着投影一起松掉，它会一样绿。泄漏的条目不被
/// 任何主张认领 ⇒ 报「无出处」，这正是「两个出口对同一段历史给两种读法」那种静默
/// 退化唯一的外部可见形态。
#[test]
fn a_leaked_replaced_branch_shows_up_as_an_unsourced_message() {
    let events = branch_events();
    let mut msgs = transcript(CHAT, MESSAGE)
        .apply(&events, i64::MAX, crate::Budget::generous())
        .value
        .to_messages();
    msgs.insert(1, entry("assistant", "甲答复", None));
    let rep = verify(&events, &msgs, None, &CHAT, &MESSAGE);
    assert!(
        rep.gaps
            .iter()
            .any(|g| matches!(g, Gap::NoSourceInGrid { .. })),
        "被替换那一支混进请求包时必须报无出处：{}",
        rep.summary()
    );
}
