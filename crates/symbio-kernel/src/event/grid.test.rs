//! `grid` 单测 —— 名字表自身的三条判据（段四）。
//!
//! 写侧闸门的拒绝行为在 `plugins/session/store/wal.test.rs`（判它的地方就是强制点
//! 所在的那一层），本文件只钉**表**：一行一名、精确拼写、与读侧三目 `match` 同格。

use super::super::event_node_of;
use super::*;
use crate::event::holds_role;
use crate::Event;
use serde_json::json;

/// 表上事件名的条数 = 内核仍在 `event/mod.rs` 声明的事件名条数（协议五格 + 未迁出的域账本）。
///
/// `event/mod.rs` 里另有两条 `EVENT_TRIGGER_*` 常量：`meta.trigger` 的**键名**与它的
/// `kind` 取值（[D-077](../../../symbio-plugin-heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README")）。它们不是事件名，
/// 所以不进这张表。已迁出的名字（owner 的 `EventDecl`）也不在这里——这条断言把内核
/// 名字表与 `CELLS` 的行数钉在一起。
const DECLARED_EVENT_NAMES: usize = 30;

#[test]
fn the_table_is_one_row_per_name_and_covers_every_declared_name() {
    assert_eq!(
        CELLS.len(),
        DECLARED_EVENT_NAMES,
        "名字表的行数与事件名常量数漂移：加名字要同批加一行（闸门会拒未登记的名字）"
    );
    let mut seen = std::collections::HashSet::new();
    for decl in CELLS {
        assert!(
            seen.insert(decl.name),
            "一个名字主张了两个格位：{} —— 一个名字只有一格",
            decl.name
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

/// 角色是**表上的数据**，不是判据里抄的名字（DE-024 D-203 裁决二）。
///
/// 一张插件自己造的表：两个名字落同一格，只有其中一个扮演「定稿发言」。
/// 判据按表回答 ⇒ 一个新 owner 声明自己的名字并带上角色，守卫就认它——
/// 这条用例是「加能力不改架构」的最小可执行证据。
#[test]
fn roles_are_data_on_the_table_not_names_in_a_guard() {
    const VOCAB: &[EventDecl] = &[
        EventDecl::with_roles(
            "fixture.finalizes",
            (Entity::Turn, Verb::Closed),
            &[FactRole::FinalizesTurn],
        ),
        EventDecl::new("fixture.other", (Entity::Turn, Verb::Closed)),
    ];
    assert!(
        holds_role(VOCAB, "fixture.finalizes", FactRole::FinalizesTurn),
        "声明了角色的名字，守卫必须经表认出来"
    );
    assert!(
        !holds_role(VOCAB, "fixture.other", FactRole::FinalizesTurn),
        "同格的另一个名字没声明角色，守卫不得替它猜"
    );
    // 表上根本没有的名字 ⇒ 不扮演任何东西（不是「默认算一份」）。
    assert!(!holds_role(VOCAB, "turn.closed", FactRole::FinalizesTurn));
}

/// 内核表**不再**被要求住满每一档角色（DE-024 D-203）。
///
/// 「回退臂要能答出每一档角色，否则落回它的判据空转」这条要求把内核表钉成守卫的
/// 词汇源，而词汇源是**装配表**：名字随 owner 迁出，这张表上的角色就跟着变少，
/// 这是迁移的正常形状（最后一批业务名字走完后这张表只剩协议五格，回退臂同批删除）。
/// 「每一档都有人住」判在装配后的那张表上（`Frozen::unmanned_roles` ⇒
/// `Gap::UnmannedRole`，装载入口拒绝空转；内核侧的最小可执行证据在
/// `decl::tests`），不判在迁移存量上。
#[test]
fn the_kernel_table_carries_roles_only_for_the_names_it_still_holds() {
    // 表上的每一行：格位答得出，角色只可能挂在**这一行**的名字上。
    for decl in kernel_vocab() {
        for role in FactRole::ALL {
            if decl.roles.contains(role) {
                assert!(
                    holds_role(kernel_vocab(), decl.name, *role),
                    "{} 上声明了 {role:?}，判据却读不出——表与 `holds_role` 不同源",
                    decl.name
                );
            } else {
                assert!(
                    !holds_role(kernel_vocab(), decl.name, *role),
                    "{} 没声明 {role:?}，判据却替它认下——守卫不得替名字猜角色",
                    decl.name
                );
            }
        }
    }
    // 未迁出的名字里，至少 `FinalizesTurn` 与 `StopsTurn` 还有归宿：装配表缺席时
    // （纯 session 二进制走回退臂）收束与停机两道守卫不许整体空转。
    // 这一条随回退臂一起退役。
    for role in [FactRole::FinalizesTurn, FactRole::StopsTurn] {
        assert!(
            kernel_vocab().iter().any(|e| e.roles.contains(&role)),
            "回退臂上 {role:?} 无人居住：未装配的进程里那道守卫会静默不判"
        );
    }
}
