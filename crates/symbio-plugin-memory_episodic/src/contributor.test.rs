//! 编码贡献的判据（DE-036 §3 / DE-026 D-263）：两条草稿各按标签取值、空不编、
//! 截断按字符、同文去重读**全量史**（跨标签、含巩固产物、排除式遗忘后可重记）。

use serde_json::json;
use symbio_plugin_api::events::{
    EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
};
use symbio_plugin_api::{
    Contribution, Contributor, Entity, Event, Seq, SettleFace, Settling, Verb, PRINCIPAL_USER,
};

use super::EpisodicEncode;
use crate::decl::{ENCODE_MAX_CHARS, JUDGMENT_TAG, MEMORY_TAG};

fn ev(seq: u64, kind: &str, entity: Entity, verb: Verb, payload: serde_json::Value) -> Event {
    let mut e = Event::pending(format!("seed-{seq}"), kind, entity, verb, 0, PRINCIPAL_USER);
    e.seq = Some(Seq::new(seq));
    e.ts = 1000 + seq as i64;
    e.payload = payload;
    e
}

/// 一条**活**的经验记忆（与写方形态一致：带 content / tag / generation）。
fn encoded(seq: u64, content: &str, tag: &str) -> Event {
    ev(
        seq,
        EVENT_MEMORY_ENCODED,
        Entity::Memory,
        Verb::Opened,
        json!({ "content": content, "tag": tag, "generation": 0u32 }),
    )
}

/// 一条排除式遗忘（`produced_by` 指向被遗忘那条的 seq）。
fn forgotten(seq: u64, target: u64) -> Event {
    let mut e = ev(
        seq,
        EVENT_MEMORY_FORGOTTEN,
        Entity::Memory,
        Verb::Closed,
        json!({ "why": "consolidated", "into": 99 }),
    );
    e.produced_by = Some(target);
    e
}

fn settling<'a>(
    history: &'a [Event],
    user_text: &'a str,
    response: Option<&'a str>,
) -> Settling<'a> {
    Settling {
        face: SettleFace::Generated,
        now_ms: 1,
        principal: "session/main",
        user_seq: 41,
        anchor: "t3",
        user_text,
        response,
        history,
    }
}

fn payload_str<'a>(draft: &'a Contribution, key: &str) -> Option<&'a str> {
    draft.fact.payload.get(key).and_then(|v| v.as_str())
}

/// 一问一答 ⇒ 两条草稿：经验（shared）与判断（thread_private），同锚同溯源。
#[test]
fn one_settlement_encodes_question_and_conclusion() {
    let history: [Event; 0] = [];
    let drafts = EpisodicEncode.contribute(&settling(
        &history,
        "你好",
        Some("连接池调到 50 并加了索引"),
    ));
    assert_eq!(drafts.len(), 2, "成功收束编两条：经验 + 判断");

    let (m, j) = (&drafts[0], &drafts[1]);
    assert_eq!(m.fact.key, "v2m-t3");
    assert_eq!(j.fact.key, "v2j-t3");
    for d in &drafts {
        assert_eq!(d.fact.kind, EVENT_MEMORY_ENCODED);
        assert_eq!(d.fact.entity, Entity::Memory);
        assert_eq!(d.fact.verb, Verb::Opened);
        assert_eq!(d.fact.produced_by, Some(41), "溯源同指本轮开口格（I2）");
        assert_eq!(d.actor, Some(PRINCIPAL_USER), "记忆的 actor 是认知归属人");
        assert!(
            d.fact.payload.get("vec").is_none(),
            "载荷里没有 vec 键：恒空的预留键不算承诺（D-110 / D-152）"
        );
        assert_eq!(
            d.fact.payload.get("generation").and_then(|v| v.as_u64()),
            Some(0)
        );
    }
    assert_eq!(payload_str(m, "tag"), Some(MEMORY_TAG));
    assert_eq!(
        payload_str(m, "vis_scope"),
        Some("shared"),
        "经验 = 可借用的认知"
    );
    assert_eq!(payload_str(m, "content"), Some("你好"));
    assert_eq!(payload_str(j, "tag"), Some(JUDGMENT_TAG));
    assert_eq!(
        payload_str(j, "vis_scope"),
        Some("thread_private"),
        "判断 = 会话私有的工作记忆（C10）"
    );
}

/// 兜底 / 中止的收束没有正文（`response = None`）⇒ 只编经验一条；空正文两支都不写。
#[test]
fn empty_or_missing_response_encodes_nothing_extra() {
    let history: [Event; 0] = [];
    let drafts = EpisodicEncode.contribute(&settling(&history, "你好", None));
    assert_eq!(drafts.len(), 1, "没有成功收束 ⇒ 判断不编");

    let drafts = EpisodicEncode.contribute(&settling(&history, "   \n", Some("  ")));
    assert!(drafts.is_empty(), "空发言与空正文都不制造空事实");
}

/// 超长截断按**字符**数（不切 UTF-8 字节）——记忆存提炼，转写已存全文。
#[test]
fn truncation_is_by_characters() {
    let history: [Event; 0] = [];
    let long = "问".repeat(ENCODE_MAX_CHARS + 100);
    let drafts = EpisodicEncode.contribute(&settling(&history, &long, None));
    assert_eq!(drafts.len(), 1);
    assert_eq!(
        payload_str(&drafts[0], "content").map(|s| s.chars().count()),
        Some(ENCODE_MAX_CHARS)
    );
}

/// 同文去重**跨标签**、读**全量史**：窗口外（切片头部）的旧记忆与巩固产物都拦得住。
///
/// 判定范围不是近 N 条：窗口省掉的是这条链上最便宜的一节，代价是窗外的同文被再记
/// 一条、重复的那条继续挤占召回预算（原判据 `dedupe_covers_the_whole_history_not_the_recall_window`
/// 随写方搬到这里）。
#[test]
fn dedupe_is_cross_tag_and_over_the_whole_history() {
    let mut history = vec![encoded(1, "很久以前说过的一句", JUDGMENT_TAG)];
    for i in 0..600u64 {
        history.push(encoded(2 + i, &format!("中间的第 {i} 句"), MEMORY_TAG));
    }
    // 同文已以「判断」记过 ⇒ 「经验」不再记第二遍。
    let drafts = EpisodicEncode.contribute(&settling(&history, "很久以前说过的一句", None));
    assert!(drafts.is_empty(), "跨标签同文也算重复");

    // 巩固产物也是一条活记忆：它的正文同样拦重复（合并后的内容不再原样记回）。
    let consolidated = vec![ev(
        7,
        EVENT_MEMORY_CONSOLIDATED,
        Entity::Memory,
        Verb::Progressed,
        json!({ "content": "合并后的正文", "tag": MEMORY_TAG, "generation": 1u32 }),
    )];
    let drafts = EpisodicEncode.contribute(&settling(&consolidated, "合并后的正文", None));
    assert!(drafts.is_empty(), "巩固产物参与去重");
}

/// 排除式遗忘可逆：被 `memory.forgotten` 排除的那条不再拦重复——**放宽即重记**，
/// 与投影「Log 不删、只是不再包含」同一条口径。
#[test]
fn forgotten_content_is_dedupeable_again() {
    let history = vec![encoded(3, "曾经验过的一句", MEMORY_TAG), forgotten(4, 3)];
    let drafts = EpisodicEncode.contribute(&settling(&history, "  曾经验过的一句  ", None));
    assert_eq!(
        drafts.len(),
        1,
        "遗忘后再见同文 = 新的编码（可撤销、可审计）"
    );

    // 反向钉：没有那条遗忘 ⇒ 同文仍然被拦（遗忘不是「所有编码都失效」）。
    let only = vec![encoded(3, "曾经验过的一句", MEMORY_TAG)];
    assert!(EpisodicEncode
        .contribute(&settling(&only, "曾经验过的一句", None))
        .is_empty());
}

/// 同轮两条各编各的：经验与判断互不可见（两条草稿对着同一份史判），
/// 一问一答即便正文相同也各落一条——与旧实现（两次 `encode` 共享同一份
/// 交回切片）逐字同形。
#[test]
fn same_turn_pair_always_lands_together() {
    let history: [Event; 0] = [];
    let drafts = EpisodicEncode.contribute(&settling(&history, "同一句", Some("同一句")));
    assert_eq!(
        drafts.len(),
        2,
        "同轮一问一答各编一条（去重只判史，不判本轮的另一条）"
    );
}
