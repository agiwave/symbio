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
