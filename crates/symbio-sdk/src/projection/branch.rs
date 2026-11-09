//! 分支投影 —— 「同一个问题的几个候选答复」与「此刻读的是哪一支」。
//!
//! ## 为什么需要它
//!
//! 重试若是**删除式**的（失败轮连子树一起删掉，再按普通新轮重来），转写就永远只有一条
//! 竖直的历史，于是「同 parent 多候选可切换」这件事**没有数据来源**——不是界面缺一个
//! `‹ ›`，是树上没有第二个孩子。裁决与理由见
//! [D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。
//!
//! 现在重试不删了：它落一条 `EVENT_TURN_BRANCH`（`{of, into}`：新根是那一条的另一
//! 支），用户点切换器落 `EVENT_BRANCH_SELECTED`。**「读哪一支」由这两类事实共同说**，
//! 组内后到的胜出（[`symbio_kernel::event::branch`]）——分支格自带读法，否则「切回旧支再重试」会
//! 停在用户刚发起重试的那一支上。两者都是**事实**，本模块把它们折回节点
//! 形状——重建是纯函数，所以「哪一支在当前历史里」必须能从事实里读出来，而不是由
//! 某个调用方记得传一个参数。
//!
//! ## 分工（与建树策略、`message.removed` 都正交）
//!
//! | 格 | 做什么 | 谁调 |
//! |---|---|---|
//! | [`stamp_branches`] | 把组结构盖成成员根节点的 `meta.branch`（`candidates` / `index` / `total` / `selected`） | [`super::rebuild`] 末尾一次 |
//! | [`unselected_roots`] | 说出**此刻不被读**的那几支的根 id | 请求包的两个出口：[`super::transcript`] 与 `prompt_fidelity` |
//! | [`branch_hide_unselected`] | 按标注丢弃未选中支的**整棵子树** | 节点树进模型的那条路（`get_context_messages`） |
//! | 呈现读侧 | **不丢**：全树带标注交出去，前端按 `selected` 决定画哪一支 | — |
//!
//! 「看不见」与「不存在」是分开的两件事：前者是呈现（前端留白、切回去还在），后者才
//! 需要 `message.removed`。给模型的那一份必须只有一支——同一个问题在请求包里出现两个
//! 答复，模型读到的是自己没说过的话。
//!
//! ## 「这一支还在不在」有两条问法，挑法只有一份
//!
//! 节点树那边问的是**有没有对应的节点**（[`stamp_branches`]），请求包那边问的是
//! **有没有被 `message.removed` 删掉**（[`unselected_roots`]）。两条问法都必须存在：
//! 重试那一幕在备输入那一步（`stage::prepare`）就落了分支格，而新支的 Turn 根那时
//! **还没有任何节点**——
//! 若请求包也按「有没有节点」问，被替换掉的那一支会在模型眼前活到本轮结束，
//! 用户刚按下的重试在模型眼里等于没发生。
//!
//! 分岔的只有这一句问话；**挑哪一支的算术**住在
//! [`symbio_kernel::event::branch::branch_marks`] 一处（事实层判断，与节点形状无关；
//! 插件侧不该为了它依赖应用门面，DE-051 D-362）。本模块只负责把结论盖成
//! `meta.branch`。两个出口各算一遍就会对同一段历史给出两种读法，而那种分叉是静默的。
//!
//! ## 为什么按**节点 id** 归组而不是轮号
//!
//! 一幕起一个新根，幕间续写让同一轮开出多个根（轮号与根不是一对一）。按轮号归组会把
//! 「同一轮的第二幕」误读成「另一支」。载荷里因此只出现根 id（节点身份是事实的一部分，
//! 与 `root_id` / `result_id` 同一条纪律）。

use symbio_kernel::event::branch::branch_marks;

use crate::event::Event;
use crate::projection::vocab::ChatVocab;
use crate::turn::node_build::subtree_of;
use symbio_kernel::chat::chat_node::ChatNode;

/// 成员根节点上的分支标注键（`meta.branch`）。
///
/// 键里五个字段：`group`（组锚 = 组内最早那支的根 id）、`candidates`（组内**存在节点**
/// 的成员 id，按转写顺序）、`index`（自己在 `candidates` 里的下标）、`total`
/// （= `candidates.len()`）、`selected`（是不是当前读的那一支）。
///
/// 前端只读 `index` / `total` 画 `‹ ›`、只读 `selected` 决定画哪一支，**不认识**
/// `candidates` 的内容——切换动作把目标候选的 id 原样交回后端，后端按它认人。
pub const META_BRANCH: &str = "branch";

/// 把分支结构盖成成员根节点的 `meta.branch`（**就地改**，不改列表长度与顺序）。
///
/// 在世的证据这里是「**有没有对应的节点**」：一支从没产出过根节点就没有可画的东西，
/// 把它列进 `candidates` 只会多出一个点不动的候选。
pub fn stamp_branches(nodes: &mut [ChatNode], events: &[Event], v: &ChatVocab) {
    let marks = branch_marks(
        events,
        |id| nodes.iter().any(|n| n.id == id),
        v.branch,
        v.branch_selected,
    );
    if marks.is_empty() {
        return;
    }
    // 逐条节点查表 ⇒ 组的遍历顺序不参与产出（同一批事实恒得同一份标注）。
    for n in nodes.iter_mut() {
        if let Some(m) = marks.get(&n.id) {
            stamp_one(n, &m.group, &m.candidates, m.index, m.total, m.selected);
        }
    }
}

/// 一条事件属于**哪一支**（载荷 `root_id` = 本轮轮根 id）。
///
/// `None` = 它**不在任何支里**：用户格、轮内补充、快照基线都不带这个键（`root_id` 只由
/// 助手侧的产出事实记，见 `turn_runner/facts.rs`）。那些正是各支的**共同祖先**——按它剪
/// 会连用户的问题一起剪掉。
///
/// 键名与 [`symbio_kernel::event::branch`] 的载荷读法同一处，请求包的两个出口（[`super::transcript`]
/// 与 [`super::prompt_fidelity`]）引的是同一个判据。
pub fn event_branch_root(e: &Event) -> Option<&str> {
    e.payload
        .get("root_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// 分支组里**此刻不被读**的那些支的根 id（请求包读侧专用）。
///
/// 在世的证据这里是「**有没有被 `message.removed` 删掉**」——它不要求那一支已经产出过
/// 节点，理由见模块文档「『这一支还在不在』有两条问法」。调用方拿它把那些支的正文标成
/// 不进请求包（转写里照留，切回去还在）。
pub fn unselected_roots(
    events: &[Event],
    removed: impl Fn(&str) -> bool,
    v: &ChatVocab,
) -> std::collections::HashSet<String> {
    symbio_kernel::event::branch::unselected_roots(
        events,
        |id| !removed(id),
        v.branch,
        v.branch_selected,
    )
}

/// 盖一条节点的 `meta.branch`。
fn stamp_one(
    node: &mut ChatNode,
    group: &str,
    candidates: &[String],
    index: usize,
    total: usize,
    selected: bool,
) {
    let value = serde_json::json!({
        "group": group,
        "candidates": candidates,
        "index": index,
        "total": total,
        "selected": selected,
    });
    match &mut node.meta {
        Some(m) => match m.as_object_mut() {
            Some(obj) => {
                obj.insert(META_BRANCH.to_string(), value);
            }
            // meta 不是对象（历史脏数据）：整格换成只带分支标注的对象——非对象的那份
            // 本来就读不出任何键，留着只会让下一次合并继续失败。
            None => node.meta = Some(serde_json::json!({ META_BRANCH: value })),
        },
        None => node.meta = Some(serde_json::json!({ META_BRANCH: value })),
    }
}

/// 这条节点是不是「当前读的那一支」。
///
/// 没带标注的节点一律算「是」：它不属于任何分支组，就没有被隐藏的理由。
pub fn branch_selected(node: &ChatNode) -> bool {
    stamped(node)
        .and_then(|b| b.get("selected"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// 读 `meta.branch` 那一个键（没有标注 ⇒ `None`）。
///
/// 键名的唯一读法住在这里：写侧（本模块的 [`stamp_one`]）与任何要认「这条节点在
/// 哪个组、有哪些候选」的一方（切换动作的入参校验）共用同一份，避免第二处拼字符串。
fn stamped(node: &ChatNode) -> Option<&serde_json::Value> {
    node.meta.as_ref().and_then(|m| m.get(META_BRANCH))
}

/// 这条节点所在分支组的**候选列表**（`meta.branch.candidates`，按转写顺序）。
///
/// 不带标注 ⇒ `None`（它不在任何组里，也就没有「切到哪一支」这回事）。切换动作
/// 用它判定目标是否合法——**同组**是这条动作的全部边界：跨组跳转不是「换一支读」，
/// 是跳到另一条历史，那是 `truncate` / 重试各自的动作面。
pub fn branch_candidates(node: &ChatNode) -> Option<Vec<String>> {
    stamped(node)
        .and_then(|b| b.get("candidates"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
}

/// 丢弃未选中支的**整棵子树**（节点树进模型的那条路专用；呈现读侧不调它）。
///
/// 标注来自 [`stamp_branches`]，所以这条路读的是「**有没有对应的节点**」那一条问法：
/// 它拿到的已经是重建出来的树，节点身份就是它唯一有的证据。请求包那条路（事实切片 →
/// 转写投影）不经过这里，见 [`unselected_roots`]。
///
/// 子树收集用 [`subtree_of`]——`flat` 档下本轮节点升了根（`parent_id: None` +
/// `meta.turn_root` 记号），只按 `parent_id` 够不着它们。
pub fn branch_hide_unselected(messages: &mut Vec<ChatNode>) {
    let unselected: Vec<String> = messages
        .iter()
        .filter(|m| stamped(m).is_some() && !branch_selected(m))
        .map(|m| m.id.clone())
        .collect();
    drop_branch_subtrees(messages, &unselected);
}

/// 丢掉**点名的那几支**的整棵子树（与上面同一把尺，只是判据来自调用方而不是标注）。
fn drop_branch_subtrees(messages: &mut Vec<ChatNode>, roots: &[String]) {
    if roots.is_empty() {
        return;
    }
    let mut gone: std::collections::HashSet<String> = std::collections::HashSet::new();
    for root in roots {
        gone.extend(subtree_of(messages, root));
    }
    messages.retain(|m| !gone.contains(&m.id));
}

#[cfg(test)]
#[path = "branch.test.rs"]
mod tests;
