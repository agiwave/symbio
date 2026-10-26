//! 分支投影 —— 「同一个问题的几个候选答复」与「此刻读的是哪一支」。
//!
//! ## 为什么需要它
//!
//! 重试若是**删除式**的（失败轮连子树一起删掉，再按普通新轮重来），转写就永远只有一条
//! 竖直的历史，于是「同 parent 多候选可切换」这件事**没有数据来源**——不是界面缺一个
//! `‹ ›`，是树上没有第二个孩子。裁决与理由见
//! [D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")。
//!
//! 现在重试不删了：它落一条 [`EVENT_TURN_BRANCH`]（`{of, into}`：新根是那一条的另一
//! 支），用户点切换器落 [`EVENT_BRANCH_SELECTED`]。**「读哪一支」由这两类事实共同说**，
//! 组内后到的胜出（[`BranchLedger::fold`]）——分支格自带读法，否则「切回旧支再重试」会
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
//! 重试那一幕在 `prepare-turn` 就落了分支格，而新支的 Turn 根那时**还没有任何节点**——
//! 若请求包也按「有没有节点」问，被替换掉的那一支会在模型眼前活到本轮结束，
//! 用户刚按下的重试在模型眼里等于没发生。
//!
//! 分岔的只有这一句问话；**挑哪一支的算术**住在 [`marks`] 一处。两个出口各算一遍就会
//! 对同一段历史给出两种读法，而那种分叉是静默的。
//!
//! ## 为什么按**节点 id** 归组而不是轮号
//!
//! 一幕起一个新根，幕间续写让同一轮开出多个根（轮号与根不是一对一）。按轮号归组会把
//! 「同一轮的第二幕」误读成「另一支」。载荷里因此只出现根 id（节点身份是事实的一部分，
//! 与 `turn_id` / `result_id` 同一条纪律）。

use crate::chat_node::ChatNode;
use crate::event::{Event, EVENT_BRANCH_SELECTED, EVENT_TURN_BRANCH};
use crate::node_build::subtree_of;

/// 成员根节点上的分支标注键（`meta.branch`）。
///
/// 键里五个字段：`group`（组锚 = 组内最早那支的根 id）、`candidates`（组内**存在节点**
/// 的成员 id，按转写顺序）、`index`（自己在 `candidates` 里的下标）、`total`
/// （= `candidates.len()`）、`selected`（是不是当前读的那一支）。
///
/// 前端只读 `index` / `total` 画 `‹ ›`、只读 `selected` 决定画哪一支，**不认识**
/// `candidates` 的内容——切换动作把目标候选的 id 原样交回后端，后端按它认人。
pub const META_BRANCH: &str = "branch";

/// 并查集找锚（带路径压缩）。
///
/// 迭代而非递归：链长 = 重试次数，它不是栈预算；一次重试压一层，走到今天不会有事，
/// 但把栈深交给数据形状是白送的一个坑。
///
/// **自身即锚**是集合里的表示法，不是待走的边：`fold` 用 `parent[x] = x` 标记「x 是这个
/// 组的成员」（孤立成员也占这一格，`contains_key` / `is_empty` 都靠它）。两处循环都必须在
/// `p == cur` 处停下——把它当成「还要往上走一步」就是死循环。
fn anchor_of(parent: &mut std::collections::HashMap<String, String>, id: &str) -> String {
    let mut root = id.to_string();
    while let Some(p) = parent.get(&root) {
        if p == &root {
            break;
        }
        root = p.clone();
    }
    let mut cur = id.to_string();
    while let Some(p) = parent.get(&cur) {
        if p == &cur {
            break;
        }
        let next = p.clone();
        parent.insert(cur, root.clone());
        cur = next;
    }
    root
}

/// 一条事件载荷里的**非空**字符串字段（空串读作「这个字段没记」，与 `event` 同口径）。
fn payload_str(e: &Event, key: &str) -> Option<String> {
    e.payload
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 折叠事实得到的分支结构。
struct BranchLedger {
    /// 成员 id → 父（并查集；自身 = 尚未并过任何支的孤立成员）。
    parent: std::collections::HashMap<String, String>,
    /// 组锚 → 该组**最后一条**选择格说的那一支。
    picks: std::collections::HashMap<String, String>,
}

impl BranchLedger {
    /// 两类事实都在说「读哪一支」，后到的覆盖先到的（事实顺序 = 用户动作顺序）。
    ///
    /// - `turn.branch{of, into}`：一次重试 = 一组多了一支 + 读的就是这**新落地的**一支。
    ///   它必须自己带读法，否则「切回旧支再重试」会停在用户刚发起重试的那一支上——
    ///   用户看到的是「重试没反应」。
    /// - `turn.branch_selected{selected}`：用户点切换器，明确指定这一支。
    ///
    /// 组锚 = 最早出现的成员 id：`of` 是被重试的那一支，它先入集，所以锚天然是**第一次
    /// 尝试**的根。在已属于某组的成员上再重试时两组并成一组、锚取较早的那个——
    /// 「切回旧支再重试一次」因此不会开出第二个组。
    fn fold(events: &[Event]) -> Self {
        let mut parent: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut picks: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        for e in events {
            match e.kind.as_str() {
                EVENT_TURN_BRANCH => {
                    let (Some(of), Some(into)) = (payload_str(e, "of"), payload_str(e, "into"))
                    else {
                        continue;
                    };
                    if of == into {
                        continue;
                    }
                    // 各自先入集（没见过同伴时自成一组），再并。
                    parent.entry(of.clone()).or_insert_with(|| of.clone());
                    parent.entry(into.clone()).or_insert_with(|| into.clone());
                    let ra = anchor_of(&mut parent, &of);
                    let rb = anchor_of(&mut parent, &into);
                    if ra != rb {
                        parent.insert(rb.clone(), ra.clone());
                        // 被并掉的锚不再是锚，它的读法记在锚上才有人读得到；而这一条分支格
                        // 马上就要在合并后的锚上写下新的读法 ⇒ 旧的那份直接作废。
                        picks.remove(&rb);
                    }
                    picks.insert(anchor_of(&mut parent, &into), into);
                }
                EVENT_BRANCH_SELECTED => {
                    let Some(sel) = payload_str(e, "selected") else {
                        continue;
                    };
                    if !parent.contains_key(&sel) {
                        // 选择的是一组里从来没有过的 id（手改的 WAL / 已整段截断）⇒ 不认。
                        continue;
                    }
                    let anchor = anchor_of(&mut parent, &sel);
                    picks.insert(anchor, sel);
                }
                _ => {}
            }
        }
        Self { parent, picks }
    }

    /// 一个分支格都没有 ⇒ 本模块对这批事实零改动（绝大多数会话走这条）。
    fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }
}

/// 一个成员根在组里的位置与此刻读法（字面 = `meta.branch` 那五个字段）。
struct BranchMark {
    group: String,
    candidates: Vec<String>,
    index: usize,
    total: usize,
    selected: bool,
}

/// 折出**每个在世成员**在组里的位置与此刻读法（成员根 id → 标注）。
///
/// `live` 是「这一支还在不在」的证据，由调用方给（两条问法各自的理由见模块文档）。
/// 组里在世成员不足两条时**一个成员都不标注**：画出来的 `1/1` 是假的两支，而被请求包
/// 剪掉的「另一支」更是无中生有。
///
/// 读法指向的成员不在世时（那支后来被整段截断）退回**在世的最后一支**——宁读最新，
/// 不判成「哪一支都没被读」（那样等于把这一轮的问题连同所有答复一起抹掉）。
///
/// 成员顺序 = 该 id 在事实里**首次出现**的顺序 = 尝试顺序（`of` 总排在同一条分支格的
/// `into` 之前，所以较早的那一支总是先入列）。`BranchLedger::parent` 是哈希表，迭代它
/// 得不到这个顺序，所以这里单独扫一遍事件收列——**不能**改成「遍历 `parent`」，那会让
/// `index` / `candidates` 随哈希种子漂。
fn marks(
    events: &[Event],
    live: impl Fn(&str) -> bool,
) -> std::collections::HashMap<String, BranchMark> {
    let ledger = BranchLedger::fold(events);
    if ledger.is_empty() {
        return std::collections::HashMap::new();
    }
    let mut parent = ledger.parent;
    let mut seen: Vec<String> = Vec::new();
    for e in events {
        if e.kind != EVENT_TURN_BRANCH {
            continue;
        }
        for key in ["of", "into"] {
            if let Some(id) = payload_str(e, key) {
                if !seen.contains(&id) {
                    seen.push(id);
                }
            }
        }
    }
    let mut groups: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for id in &seen {
        let anchor = anchor_of(&mut parent, id);
        groups.entry(anchor).or_default().push(id.clone());
    }
    let mut out: std::collections::HashMap<String, BranchMark> = std::collections::HashMap::new();
    for (anchor, members) in groups {
        let candidates: Vec<String> = members.into_iter().filter(|id| live(id)).collect();
        if candidates.len() < 2 {
            continue;
        }
        // 读法 = 组里**最后一条读动作**说的那一支（`picks`：分支格与选择格同一种效力，
        // 见 [`BranchLedger::fold`]）。
        let chosen = ledger
            .picks
            .get(&anchor)
            .and_then(|sel| candidates.iter().position(|id| id == sel))
            .unwrap_or(candidates.len() - 1);
        let total = candidates.len();
        for (index, id) in candidates.iter().enumerate() {
            out.insert(
                id.clone(),
                BranchMark {
                    group: anchor.clone(),
                    candidates: candidates.clone(),
                    index,
                    total,
                    selected: index == chosen,
                },
            );
        }
    }
    out
}

/// 把分支结构盖成成员根节点的 `meta.branch`（**就地改**，不改列表长度与顺序）。
///
/// 在世的证据这里是「**有没有对应的节点**」：一支从没产出过根节点就没有可画的东西，
/// 把它列进 `candidates` 只会多出一个点不动的候选。
pub fn stamp_branches(nodes: &mut [ChatNode], events: &[Event]) {
    let marks = marks(events, |id| nodes.iter().any(|n| n.id == id));
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

/// 一条事件属于**哪一支**（载荷 `turn_id` = 本轮 Turn 根 id）。
///
/// `None` = 它**不在任何支里**：用户格、轮内补充、快照基线都不带这个键（`turn_id` 只由
/// 助手侧的产出事实记，见 `turn_runner/facts.rs`）。那些正是各支的**共同祖先**——按它剪
/// 会连用户的问题一起剪掉。
///
/// 键名与 [`BranchLedger::fold`] 的载荷读法同一处，请求包的两个出口（[`super::transcript`]
/// 与 [`super::prompt_fidelity`]）引的是同一个判据。
pub fn event_branch_root(e: &Event) -> Option<&str> {
    e.payload
        .get("turn_id")
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
) -> std::collections::HashSet<String> {
    marks(events, |id| !removed(id))
        .into_iter()
        .filter(|(_, m)| !m.selected)
        .map(|(id, _)| id)
        .collect()
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
