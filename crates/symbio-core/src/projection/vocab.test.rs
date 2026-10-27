//! 三张词汇表自身的判据：三个「按名字点收」的判断收在 [`ChatVocab`] 上，
//! 只有一处口径可漂；三张表的字段两两不同名，否则某个折叠会静默错位。

use super::{ChatVocab, CommitmentVocab, MessageVocab};

/// 夹具表：值就是线上名字。用例不引内核常量——那正是这三张表要替掉的东西。
pub(super) const CHAT: ChatVocab = ChatVocab {
    user_message: "user.message",
    dispatch: "turn.dispatch",
    supplemented: "turn.supplemented",
    branch: "turn.branch",
    branch_selected: "turn.branch_selected",
    reported: "chat.assistant.reported",
    opening: "chat.assistant.opening",
    final_reply: "chat.assistant.final",
    fallback: "chat.assistant.fallback",
    progress: "chat.assistant.progress",
    denied: "authz.denied",
};

pub(super) const MESSAGE: MessageVocab = MessageVocab {
    artifact_added: "artifact.added",
    message_added: "message.added",
    message_edited: "message.edited",
    message_removed: "message.removed",
    consolidated: "session.consolidated",
};

pub(super) const COMMITMENT: CommitmentVocab = CommitmentVocab {
    opened: "commitment.opened",
    released: "commitment.released",
    broken: "commitment.broken",
    asserted: "commitment.asserted",
};

#[test]
fn only_the_final_counts_as_a_finished_turn() {
    assert!(CHAT.closes_as_final(CHAT.final_reply));
    // 兜底是一条输出，但它不是「这一轮答完了」——收束闸的读侧比写侧宽（I3）。
    assert!(!CHAT.closes_as_final(CHAT.fallback));
    assert!(!CHAT.closes_as_final(CHAT.progress));
}

#[test]
fn closure_covers_both_names_on_the_closed_cell() {
    assert!(CHAT.is_closure(CHAT.final_reply));
    assert!(CHAT.is_closure(CHAT.fallback));
    // 收束格上的新名字**应当**算收束（这是按名字点收会静默漏掉的那一半）。
    assert!(!CHAT.is_closure("chat.assistant.whatever"));
}

#[test]
fn spoken_covers_the_five_ways_the_assistant_says_something() {
    for kind in [
        CHAT.reported,
        CHAT.opening,
        CHAT.final_reply,
        CHAT.fallback,
        CHAT.progress,
    ] {
        assert!(CHAT.is_spoken(kind), "{kind} 是助手说的一句话");
    }
    // 用户说的话、被拒都不是「助手说话了」。
    for kind in [CHAT.user_message, CHAT.denied] {
        assert!(!CHAT.is_spoken(kind), "{kind} 不是助手说的话");
    }
}

/// 三张表的字段两两不同名：同一条名字占两个字段的表现是**某个折叠静默错位**，
/// 而它不会有任何运行期信号。表都是 `Copy` 的纯值，这里逐字段读出来比一遍。
#[test]
fn no_event_name_is_claimed_by_two_fields() {
    let all = [
        // ChatVocab（11）
        CHAT.user_message,
        CHAT.dispatch,
        CHAT.supplemented,
        CHAT.branch,
        CHAT.branch_selected,
        CHAT.reported,
        CHAT.opening,
        CHAT.final_reply,
        CHAT.fallback,
        CHAT.progress,
        CHAT.denied,
        // MessageVocab（5）
        MESSAGE.artifact_added,
        MESSAGE.message_added,
        MESSAGE.message_edited,
        MESSAGE.message_removed,
        MESSAGE.consolidated,
        // CommitmentVocab（4）
        COMMITMENT.opened,
        COMMITMENT.released,
        COMMITMENT.broken,
        COMMITMENT.asserted,
    ];
    assert_eq!(
        all.len(),
        20,
        "三张表的闭集是 20 条名字（verdict 是单名字形参）"
    );
    let mut seen = all.to_vec();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), all.len(), "同一条名字占了两处：{all:?}");
}

/// 领域扩展位在**读侧**也是「不解释」：`Domain × 任意动词` 一律不是对话槽位。
///
/// 这条判据钉的是 DE-025 D-209 的另一半。写侧的证据在 `plugins/session` 的写闸
/// （`a_domain_cell_is_gated_like_any_other_declared_cell`）：owner 声明了就放行。
/// 读侧的证据在这里：**放行之后 core 也不会因此「懂」这个领域**——它不进对话折叠、
/// 不产节点、不参与任何核心判据。
///
/// 若哪天有人为了「让 notes 也显示在转写上」在这里加一档 `ChatSlot`，那正是
/// 「加一个领域要改架构」的回流形状；这条用例会红，逼那条改动去走决策。
#[test]
fn a_domain_cell_is_never_a_chat_slot() {
    use super::slot_of;
    use crate::event::{Entity, Event, Verb};

    for verb in [
        Verb::Opened,
        Verb::Progressed,
        Verb::Held,
        Verb::Closed,
        Verb::Asserted,
    ] {
        // 名字特意挑成「看起来像对话」的形态：判据认的是 `(entity, verb)` 与词汇面，
        // 不是名字长得像不像——名字由 owner 声明，core 不认识它。
        let e = Event::pending(
            "d-0",
            "notes.created",
            Entity::Domain,
            verb,
            0,
            "session/main",
        );
        assert_eq!(
            slot_of(&e, &CHAT, &MESSAGE),
            None,
            "Domain × {verb:?} 不该被 core 解释成任何对话槽位"
        );
    }
}
