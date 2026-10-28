//! 分支投影的回归测试
//!
//! ## 它防的是哪种失效
//!
//! 「重试留下两支」这件事的全部价值在于**切得回去**，而它的对立面不是崩溃，是
//! **悄悄读错的那一支**：
//! - 默认选中算错 ⇒ 用户重试完看到的还是那条失败的（等于没重试）；
//! - 隐藏只按 `parent_id` 收子树 ⇒ `flat` 档下那一支的工具结果**漏在外面**，
//!   模型在同一段历史里读到两条矛盾的答复；
//! - 单支也盖标注 ⇒ 前端画出 `1/1` 的点不动的切换器；
//! - 重试不落读法 ⇒ 用户刚发起重试却还看着旧的那一支（界面像卡住）；
//! - 找锚时把「自身即锚」当成一条还要往上走的边 ⇒ 重建**挂死**（CPU 满载、永不返回，
//!   测试里表现为整条套件不结束）。这条是本套件的最低要求：它让其余各条根本没机会跑。
//!
//! 前三条都不报错、都不 panic（只有最后一条挂死）。所以这里按**读法**钉：谁是那一份、
//! 什么情况下一个字段都不该出现。
//!
//! 读法有**两个出口**（节点树的 `stamp_branches` 与请求包的 `unselected_roots`），它们
//! 对「这一支还在不在」的问法不同——不同在哪、为什么必须不同，见模块文档与本文件末段。

use super::{
    branch_candidates, branch_hide_unselected, branch_selected, drop_branch_subtrees,
    stamp_branches, unselected_roots, META_BRANCH,
};
use crate::chat_node::ChatNode;
use crate::test_fixtures::{CHAT, EVENT_TURN_BRANCH, EVENT_TURN_BRANCH_SELECTED};
use crate::{Entity, Event, Verb};

fn node(id: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        ..Default::default()
    }
}

/// 一条 `turn.branch`：`into` 是 `of` 的另一支。
fn branch(id: &str, of: &str, into: &str) -> Event {
    Event::pending(
        format!("branch-{id}"),
        EVENT_TURN_BRANCH,
        Entity::Turn,
        Verb::Asserted,
        1,
        "person/local",
    )
    .with_payload(serde_json::json!({ "of": of, "into": into }))
}

/// 一条 `turn.branch_selected`：这一组现在读 `selected`。
fn picked(id: &str, selected: &str) -> Event {
    Event::pending(
        format!("branch-selected-{id}"),
        EVENT_TURN_BRANCH_SELECTED,
        Entity::Turn,
        Verb::Asserted,
        1,
        "person/local",
    )
    .with_payload(serde_json::json!({ "selected": selected }))
}

/// 折叠一批事实后盖章，返回**盖章过**的那份列表。
fn stamped(nodes: &mut [ChatNode], events: &[Event]) {
    stamp_branches(nodes, events, &CHAT);
}

fn branch_of(n: &ChatNode) -> Option<&serde_json::Value> {
    n.meta.as_ref().and_then(|m| m.get(META_BRANCH))
}

fn ids(msgs: &[ChatNode]) -> Vec<String> {
    msgs.iter().map(|m| m.id.clone()).collect()
}

#[test]
fn 两支并存默认读最新那一次尝试() {
    let mut nodes = vec![node("t1"), node("t2")];
    stamped(&mut nodes, &[branch("a", "t1", "t2")]);

    assert_eq!(branch_of(&nodes[0]).unwrap()["index"], 0);
    assert_eq!(branch_of(&nodes[1]).unwrap()["index"], 1);
    assert_eq!(branch_of(&nodes[0]).unwrap()["total"], 2);
    assert_eq!(branch_of(&nodes[0]).unwrap()["group"], "t1");
    // 重试完什么都不点 ⇒ 读到的必须是新的一次（分支格自带读法，见 `BranchLedger::fold`）。
    assert!(!branch_selected(&nodes[0]));
    assert!(branch_selected(&nodes[1]));
}

#[test]
fn 单支不成组一个字段都不盖() {
    // 分支格引用了一支从没产出过节点的历史成员（格落地了、节点被截断了）⇒
    // 现存成员只有一条。盖出来的是 `1/1` 的假两支。
    let mut nodes = vec![node("t2")];
    stamped(&mut nodes, &[branch("a", "t1", "t2")]);
    assert!(nodes[0].meta.is_none());
    assert!(branch_candidates(&nodes[0]).is_none());
    // 没标注的节点**永远算选中**：它不在任何组里，就没有被隐藏的理由。
    assert!(branch_selected(&nodes[0]));
}

#[test]
fn 选择格覆盖默认且后到胜出() {
    let events = [
        branch("a", "t1", "t2"),
        picked("s1", "t1"),
        picked("s2", "t2"),
    ];
    let mut nodes = vec![node("t1"), node("t2")];
    stamped(&mut nodes, &events);
    assert!(branch_selected(&nodes[1]));
    assert!(!branch_selected(&nodes[0]));

    // 把最后一条去掉 ⇒ 用户的选择生效
    let mut nodes = vec![node("t1"), node("t2")];
    stamped(&mut nodes, &events[..2]);
    assert!(branch_selected(&nodes[0]));
    assert!(!branch_selected(&nodes[1]));
}

#[test]
fn 选择指向不存在的成员时退回默认() {
    // 手改的 WAL / 那一支后来被截断删掉了：选择格读不出「现在读哪一支」，
    // 就退回默认（最新一支），而不是把整组判成「哪支都没选中」。
    let mut nodes = vec![node("t1"), node("t2")];
    stamped(&mut nodes, &[branch("a", "t1", "t2"), picked("s", "t9")]);
    assert!(branch_selected(&nodes[1]));
    assert!(!branch_selected(&nodes[0]));
}

#[test]
fn 切回旧支再重试仍是同一组锚不变() {
    let mut nodes = vec![node("t1"), node("t2"), node("t3")];
    stamped(
        &mut nodes,
        &[
            branch("a", "t1", "t2"),
            picked("s", "t1"),
            branch("b", "t1", "t3"),
        ],
    );
    assert_eq!(branch_of(&nodes[2]).unwrap()["group"], "t1");
    assert_eq!(branch_of(&nodes[2]).unwrap()["total"], 3);
    assert_eq!(branch_of(&nodes[2]).unwrap()["index"], 2);
    // 用户先选了 t1，又重试出新的一支 ⇒ 新的一支接管默认（选择格已被后一条分支
    // 动作的事实顺序超过）。
    assert!(branch_selected(&nodes[2]));
}

#[test]
fn 在新支上再重试仍是同一组() {
    // 链式的第二次重试：`of` 是上一次重试产出的那支（组内成员），新支并进**同一个组**、
    // 锚仍是第一次尝试的根。开出第二个组会让「三支」变成「两组各两支」，切换器画不对。
    let mut nodes = vec![node("t1"), node("t2"), node("t3")];
    stamped(
        &mut nodes,
        &[branch("a", "t1", "t2"), branch("b", "t2", "t3")],
    );
    assert_eq!(branch_of(&nodes[2]).unwrap()["group"], "t1");
    assert_eq!(branch_of(&nodes[2]).unwrap()["total"], 3);
    let cands = branch_candidates(&nodes[0]).unwrap();
    assert_eq!(cands, ["t1", "t2", "t3"].map(String::from));
    assert!(branch_selected(&nodes[2]));
}

#[test]
fn 跨组并进来时收成一组并读最后落地的分支格() {
    // 事实面不产出这个形状（`into` 恒是新根，不可能已经属于别组），所以它是防御面：
    // 手写/半截的 WAL 把两组并上时，**只能剩下一个组**，读法跟着最后那条分支格。
    let events = [
        branch("a", "t1", "t2"),
        picked("s1", "t2"),
        branch("b", "u1", "u2"),
        picked("s2", "u1"),
        branch("c", "t1", "u1"),
    ];
    let mut nodes = vec![node("t1"), node("t2"), node("u1"), node("u2")];
    stamped(&mut nodes, &events);
    assert_eq!(branch_of(&nodes[3]).unwrap()["total"], 4);
    assert_eq!(branch_of(&nodes[3]).unwrap()["group"], "t1");
    // 最后落地的是 `turn.branch{of:t1, into:u1}` ⇒ 读 u1；被并掉那组的旧记录不再作数
    // （一条组只能有一个答案）。
    assert!(branch_selected(&nodes[2]));
    assert!(!branch_selected(&nodes[1]));
}

#[test]
fn 一条分支格都没有时列表零改动() {
    // 绝大多数会话走这条：盖章不得给无分支的节点凭空造出 `meta`。
    let mut nodes = vec![node("t1"), node("t2")];
    stamped(
        &mut nodes,
        &[Event::pending(
            "x",
            "assistant.final",
            Entity::Turn,
            Verb::Closed,
            1,
            "session/main",
        )],
    );
    assert!(nodes.iter().all(|n| n.meta.is_none()));
}

#[test]
fn 隐藏丢掉未选中支的整棵子树() {
    let mut a = node("ta");
    a.meta = Some(serde_json::json!({}));
    let mut nodes = vec![node("u1"), a, node("tb"), node("ra"), node("rb")];
    // ta 的子孙挂 parent_id；tb 的子孙是 `flat` 档升根的（靠 `meta.turn_root` 认回）
    nodes[3].parent_id = Some("ta".to_string());
    nodes[4].meta = Some(serde_json::json!({ "turn_root": "tb" }));
    stamped(&mut nodes, &[branch("a", "ta", "tb")]);
    // 默认选中 tb ⇒ 丢掉 ta 那一支**连同它的子孙**，`u1`（共同的分支点）留着
    branch_hide_unselected(&mut nodes);
    assert_eq!(ids(&nodes), vec!["u1", "tb", "rb"]);
}

#[test]
fn 隐藏对没有分支的列表是空操作() {
    let mut nodes = vec![node("u1"), node("t1")];
    branch_hide_unselected(&mut nodes);
    assert_eq!(ids(&nodes), vec!["u1", "t1"]);
}

#[test]
fn 点名的那一支连同子孙一起被剪掉而另一支不动() {
    // `branch_hide_unselected` 用的那把尺单独钉一次：判据是**点名的根**，不看标注。
    // `flat` 档升根的子孙（没有 `parent_id`，只带 `meta.turn_root`）也必须够得着，
    // 否则模型在同一份历史里读到两条矛盾的答复。
    let mut nodes = vec![node("u1"), node("ta"), node("tb"), node("ra"), node("rb")];
    nodes[3].parent_id = Some("ta".to_string());
    nodes[4].parent_id = Some("tb".to_string());
    // 还有一条 `flat` 档升了根的
    let mut wa = node("wa");
    wa.meta = Some(serde_json::json!({ "turn_root": "ta" }));
    nodes.push(wa);
    drop_branch_subtrees(&mut nodes, &["ta".to_string()]);
    assert_eq!(ids(&nodes), vec!["u1", "tb", "rb"]);
}

#[test]
fn 点名空列表时不动一根() {
    let mut nodes = vec![node("u1"), node("t1")];
    drop_branch_subtrees(&mut nodes, &[]);
    assert_eq!(ids(&nodes), vec!["u1", "t1"]);
}

// ── 请求包读侧：`unselected_roots`（D-173 的读法判据）────────────────────
//
// 这条路是**送给模型的那一份**（`projection::transcript` 与 `prompt_fidelity` 共用）。
// 它与 `stamp_branches` 的差别只有一处、却是全部要害：在世的证据取「有没有被删」，
// **不取**「有没有节点」。

/// `live` 用「没被删」那一条证据（投影侧的口径：只有变更格能说什么都不在了）。
fn alive(events: &[Event]) -> std::collections::HashSet<String> {
    unselected_roots(events, |_| false, &CHAT)
}

#[test]
fn 新支还没有节点时请求包就已经读新支() {
    // 这是重试那一幕的真实形状：备输入那一步落了分支格，新支 `t2` 的 Turn 根**还没有
    // 产出任何节点**。按「有没有节点」问就只有 t1 一条在世成员 ⇒ 不成组 ⇒ 不隐藏 ⇒
    // 模型照旧读着用户刚刚不要的那一份（表现是「重试了但答案一模一样」）。
    let hidden = alive(&[branch("a", "t1", "t2")]);
    assert_eq!(hidden, ["t1".to_string()].into_iter().collect());
}

#[test]
fn 选择格把读法换回旧支时隐藏的是新支() {
    let hidden = alive(&[branch("a", "t1", "t2"), picked("s", "t1")]);
    assert_eq!(hidden, ["t2".to_string()].into_iter().collect());
}

#[test]
fn 一支都没有时请求包不改一个字() {
    assert!(alive(&[]).is_empty());
    // 只有一格 `branch_selected` 而没有任何分支格：它认不出组（`fold` 不认陌生 id），
    // 于是没有任何一支该被隐藏。
    assert!(alive(&[picked("s", "t1")]).is_empty());
}

#[test]
fn 被选中那支整个被删掉时退回在世最后一支而不是删空整组() {
    // 用户把当前读的那一支截断了：读法还指着它，但它已经不在世。此时**必须**退回
    // 在世的最后一支——若照读法剪，整组都进不了请求包，这一轮的问题只剩问句没有答复。
    let hidden = unselected_roots(&[branch("a", "t1", "t2")], |id| id == "t2", &CHAT);
    assert!(hidden.is_empty(), "只剩 t1 一支在世 ⇒ 不成组 ⇒ 一支都不剪");
}

#[test]
fn 读法与界面盖章对同一批事实给同一个答案() {
    // 两条问法的**分岔点**只有一个（在世证据），其余必须逐字一致：这里同一批事实
    // 走两侧，选中态不许给出两种说法。
    let events = [branch("a", "t1", "t2"), branch("b", "t2", "t3")];
    let mut nodes = vec![node("t1"), node("t2"), node("t3")];
    stamp_branches(&mut nodes, &events, &CHAT);
    let hidden = alive(&events);
    for n in &nodes {
        assert_eq!(
            branch_selected(n),
            !hidden.contains(&n.id),
            "{} 的读法在两侧不一致",
            n.id
        );
    }
}
