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
use crate::symbio_core::event::{
    Entity, Event, Seq, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_REPORTED, EVENT_USER_MESSAGE,
};
use crate::symbio_core::{EventWalStore, Store};

/// 一条送进模型的消息。
///
/// ⚠️ 这里**必须**构造 `PromptMessage` 而不是 `TranscriptEntry`：判据验的是
/// 「送进模型的那批」，而那条路上 `tool` 消息还带着 `tool_call_id`。用投影条目
/// 构造会让判据少验一样东西（而那恰恰是 ADR-048a 要保的东西）。
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
    .with_payload(serde_json::json!({ "text": text, "tier": "deep" }))
}

fn assistant_turn(turn: u64, text: &str) -> Event {
    Event::pending(
        format!("f-{turn}"),
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "agent:main",
    )
    .with_payload(serde_json::json!({ "text": text, "model": "deep" }))
}

fn artifact(turn: u64, tool: &str, text: &str) -> Event {
    Event::pending(
        format!("a-{turn}-0"),
        EVENT_ARTIFACT_ADDED,
        Entity::Artifact,
        Verb::Asserted,
        turn,
        "agent:main",
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

    let rep = verify(&events, &entries, None);
    assert!(!rep.is_complete(), "判据必须报出缺口（否则它恒绿、无用）");
    assert_eq!(rep.checked_events, 3, "只把投影内的三条计入");
    assert_eq!(rep.checked_entries, 3);

    // ① 网格 → prompt：assistant 那条被丢
    assert!(
        rep.gaps.iter().any(|g| matches!(
            g,
            Gap::DroppedFromPrompt { kind, turn: 0 } if kind == EVENT_ASSISTANT_FINAL
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
        verify(&evs, &msgs, None)
    };
    assert!(
        role_bad.gaps.iter().any(|g| matches!(
            g,
            Gap::RoleMismatch {
                expected: "user",
                ..
            }
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
        verify(&evs, &msgs, None)
    };
    assert!(
        leaked.gaps.iter().any(
            |g| matches!(g, Gap::ExcludedLeakedIntoPrompt { kind, .. } if kind == EVENT_ASSISTANT_REPORTED)
        ),
        "必须报出「界面文本漏进请求包」：{}",
        leaked.summary()
    );
}

/// 轮边界汇报一格事实（缺口 4）。
fn report(turn: u64, text: &str) -> Event {
    Event::pending(
        format!("r-{turn}"),
        EVENT_ASSISTANT_REPORTED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "agent:main",
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
    let view = transcript()
        .apply(&events, i64::MAX, crate::symbio_core::Budget::generous())
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

    let rep = verify(&events, &messages, None);
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
    let rep = verify(&events, &entries, None);
    assert!(rep.is_complete(), "平凡值必须无假红：{}", rep.summary());
}

/// 生产路径：真落一格 `artifact.added` 再从**重开的 WAL** 复核。
///
/// 走 `EventWalStore` 而不是内存事件，是为了让「窗口裁剪 / 可见域过滤」在生产里经过的
/// 同一段代码也被覆盖到——内存切片绕过那一层。
#[test]
fn an_artifact_round_is_complete_after_reopen() {
    let dir = std::env::temp_dir().join(format!("symbio-fidelity-{}-artifact", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时目录");
    let wal = dir.join("v2-events.wal");

    let store = EventWalStore::open(&wal).expect("open");
    let seq = store
        .append(user_turn(0, "帮我回显"))
        .expect("append user")
        .value();
    store
        .append(artifact(0, "vdfs_read", "文件内容：hello").with_produced_by(seq))
        .expect("append artifact");
    store
        .append(assistant_turn(0, "回显完成").with_produced_by(seq))
        .expect("append final");
    drop(store);

    let reopened = EventWalStore::open(&wal).expect("reopen");
    let events = reopened.range(Seq::new(0));

    // 投影（生产的那一个）+ 手工按结构化形态摆出 prompt 消息（ADR-048a 的目标形态）
    let view = transcript()
        .apply(&events, i64::MAX, crate::symbio_core::Budget::generous())
        .value;
    let entries: Vec<PromptMessage> = view.to_messages();
    assert_eq!(entries.len(), 3, "三条都要进 prompt：{entries:?}");
    assert_eq!(entries[1].role, "tool");
    assert_eq!(
        entries[1].tool.as_deref(),
        Some("vdfs_read"),
        "工具名是事实"
    );

    let rep = verify(&events, &entries, None);
    assert!(rep.is_complete(), "工具轮必须双向完整：{}", rep.summary());

    std::fs::remove_dir_all(&dir).ok();
}
