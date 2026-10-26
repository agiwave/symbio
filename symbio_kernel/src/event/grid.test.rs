//! `grid` 单测 —— 名字表自身的三条判据（段四）。
//!
//! 写侧闸门的拒绝行为在 `plugins/session/store/wal.test.rs`（判它的地方就是强制点
//! 所在的那一层），本文件只钉**表**：一行一名、精确拼写、与读侧三目 `match` 同格。

use super::super::event_node_of;
use super::*;
use crate::Event;
use serde_json::json;

/// 表上事件名的条数 = 名字表里**事件名**常量的条数。
///
/// `event/mod.rs` 里另有两条 `EVENT_TRIGGER_*` 常量：`meta.trigger` 的**键名**与它的
/// `kind` 取值（[D-077](../../plugins/heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "symbio/src/plugins/heartbeat/README")）。它们不是事件名，
/// 所以不进这张表——这条断言把「43 个事件名」钉住，将来加名字要同批改这里。
const DECLARED_EVENT_NAMES: usize = 43;

#[test]
fn the_table_is_one_row_per_name_and_covers_every_declared_name() {
    assert_eq!(
        CELLS.len(),
        DECLARED_EVENT_NAMES,
        "名字表的行数与事件名常量数漂移：加名字要同批加一行（闸门会拒未登记的名字）"
    );
    let mut seen = std::collections::HashSet::new();
    for (name, _) in CELLS {
        assert!(
            seen.insert(*name),
            "一个名字主张了两个格位：{name} —— 一个名字只有一格"
        );
    }
}

#[test]
fn event_cell_answers_only_the_exact_spelling() {
    assert_eq!(
        event_cell(EVENT_USER_MESSAGE),
        Some((Entity::Turn, Verb::Opened))
    );
    // 拼错的、大小写不同的、只对上前半段的、空的：一律「不在表上」，不是「猜一个」。
    for near in [
        "user.messag",
        "User.Message",
        "user",
        "user.message ",
        " ",
        "",
        "chat.assistant.finaly",
    ] {
        assert_eq!(event_cell(near), None, "{near:?} 不是表上的名字");
    }
}

/// **表与读侧必须说同一个格位**——读侧（`event_node_of`）按 `(entity, verb, kind)`
/// 三目取节点，格位漂了的表现是「这条事实不再产节点」，而投影**不报错**。
///
/// 这条测试是这张表存在的理由的可执行版：正着读要 `Some`，把格位挪一格要 `None`
/// （那个 `None` 就是静默失效的现场，写侧闸门拒的就是它）。
#[test]
fn the_table_agrees_with_the_read_side_about_which_cell_carries_a_node() {
    let at = |name: &str, cell: Cell| {
        let e = Event::pending("e1", name, cell.0, cell.1, 1, "session/main")
            .with_payload(json!({ "id": "n1", "text": "hi" }));
        (event_node_of(&e), e.grid_cell())
    };
    for name in [
        EVENT_USER_MESSAGE,
        EVENT_TURN_SUPPLEMENTED,
        EVENT_ASSISTANT_REPORTED,
        EVENT_ASSISTANT_OPENING,
        EVENT_ASSISTANT_FINAL,
        EVENT_ASSISTANT_FALLBACK,
        EVENT_ASSISTANT_PROGRESS,
        EVENT_ARTIFACT_ADDED,
    ] {
        let cell = event_cell(name).expect("表上要有这一行");
        let (node, round_trip) = at(name, cell);
        assert_eq!(round_trip, Some(cell), "{name} 的格位要能从事件本身读回来");
        assert!(node.is_some(), "{name} 落在 {cell:?} 时必须产节点");
        // 同一名字挪一格 ⇒ 读侧认不出来（静默），所以写侧闸门要拒它。
        let shifted = (cell.0, shifted_verb(cell.1));
        assert!(
            at(name, shifted).0.is_none(),
            "{name} 挪到 {shifted:?} 竟仍产节点：读侧的格子键没在判"
        );
    }
}

/// 取同一条实体行上的**另一个**动词（只为造出「格位漂了一格」的反例）。
fn shifted_verb(v: Verb) -> Verb {
    match v {
        Verb::Opened => Verb::Closed,
        Verb::Closed => Verb::Asserted,
        Verb::Asserted => Verb::Progressed,
        Verb::Progressed => Verb::Held,
        Verb::Held => Verb::Opened,
    }
}
