//! 分支事实的读法：一组重试留下的几支，此刻读的是哪一支。
//!
//! ## 为什么在 kernel，而不在投影层
//!
//! 「哪一支是活的」是**事实层判断**：只读 `turn.branch` / `turn.branch_selected` 两类
//! 事实的载荷，与节点形状（`meta.branch`、`ChatNode`）无关。投影层先用到它，是因为
//! 投影是第一个消费者，不是因为这条判断属于呈现。把它放在投影层（`symbio-sdk`）会让
//! 「按事实自己回答问题」的插件被迫依赖应用侧门面 `symbio-sdk`——那是插件→应用的反向
//! 耦合，正是 E-009 / HS-001 拦的形状（DE-051 D-362）。
//!
//! 两条名字由调用方给（owner 是会话插件的事件常量）：内核不认识业务词，只认识
//! 「哪两种事实在说读法」。
//!
//! ## 一份数据一个构造点
//!
//! 折法只有这一处：投影侧（`symbio_sdk::projection::branch`）把它盖成
//! `meta.branch`，请求包侧（[`unselected_roots`]）拿它裁掉不读的支，模型端口的
//! 请求装配也调同一个函数。三处各写一遍，必然有一份先漏——重试/切换后的裁剪与
//! 转写投影就会给出不同的答案。
//!
//! ## 「这一支还在不在」有两条问法
//!
//! 同一个折法，两种在世证据，都对：
//!
//! - **有没有对应的节点**（投影盖章时）：一支从没产出过根节点就没有可画的东西，
//!   列进候选只会多出一个点不动的点；
//! - **有没有被 `message.removed` 删掉**（请求包裁剪时）：它不要求那一支已经产出，
//!   否则「刚切过去、还没产出」的那一支会被当成不在世，于是旧的一支被读。
//!
//! 两条问法共用 [`branch_marks`]，差别只在 `live` 闭包。谁都不能另抄一份折法。

use std::collections::{HashMap, HashSet};

use crate::event::{Entity, Event, Verb};

/// 一个成员根在组里的位置与此刻读法（字面 = 投影侧 `meta.branch` 的五个字段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchMark {
    /// 组锚 = 最早出现的成员根 id（第一次尝试的那一支）。
    pub group: String,
    /// 组里**在世**的成员根 id，按首次出现（= 尝试）顺序。
    pub candidates: Vec<String>,
    /// 本成员在 `candidates` 里的位置。
    pub index: usize,
    /// `candidates` 的长度。
    pub total: usize,
    /// 本成员是不是组里此刻读的那一支。
    pub selected: bool,
}

/// 折出**每个在世成员**在组里的位置与此刻读法（成员根 id → 标注）。
///
/// `live` 是「这一支还在不在」的证据，由调用方给（两条问法见模块文档）。
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
pub fn branch_marks(
    events: &[Event],
    live: impl Fn(&str) -> bool,
    branch: &str,
    branch_selected: &str,
) -> HashMap<String, BranchMark> {
    let ledger = BranchLedger::fold(events, branch, branch_selected);
    if ledger.is_empty() {
        return HashMap::new();
    }
    let mut parent = ledger.parent;
    let mut seen: Vec<String> = Vec::new();
    for e in events {
        if e.kind != branch {
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
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for id in &seen {
        let anchor = anchor_of(&mut parent, id);
        groups.entry(anchor).or_default().push(id.clone());
    }
    let mut out: HashMap<String, BranchMark> = HashMap::new();
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

/// 分支组里**此刻不被读**的那些支的根 id（请求包读侧专用）。
///
/// 在世的证据由 `live` 给（请求包侧是「有没有被 `message.removed` 删掉」，见
/// [`removed_ids`]）。调用方拿它把那些支的正文标成不进请求包（转写里照留，切回去还在）。
pub fn unselected_roots(
    events: &[Event],
    live: impl Fn(&str) -> bool,
    branch: &str,
    branch_selected: &str,
) -> HashSet<String> {
    branch_marks(events, live, branch, branch_selected)
        .into_iter()
        .filter(|(_, m)| !m.selected)
        .map(|(id, _)| id)
        .collect()
}

/// 某类**收束事实**点名的 id 集合（`message.removed` 的读法）。
///
/// 这是分支读法需要的「还在不在」最小词汇，也是消息变更折法（编辑 / 删除）用的同一份
/// 扫描——两边共用，避免「哪条事实算删除」出现第二份实现。
pub fn removed_ids(events: &[Event], message_removed: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for e in events {
        if e.entity != Entity::Message || e.verb != Verb::Closed || e.kind != message_removed {
            continue;
        }
        let Some(ids) = e.payload.get("ids").and_then(|v| v.as_array()) else {
            continue;
        };
        out.extend(ids.iter().filter_map(|v| v.as_str()).map(str::to_string));
    }
    out
}

/// 折叠事实得到的分支结构。
struct BranchLedger {
    /// 成员 id → 父（并查集；自身 = 尚未并过任何支的孤立成员）。
    parent: HashMap<String, String>,
    /// 组锚 → 该组**最后一条**选择格说的那一支。
    picks: HashMap<String, String>,
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
    fn fold(events: &[Event], branch: &str, branch_selected: &str) -> Self {
        let mut parent: HashMap<String, String> = HashMap::new();
        let mut picks: HashMap<String, String> = HashMap::new();
        for e in events {
            if e.kind == branch {
                let (Some(of), Some(into)) = (payload_str(e, "of"), payload_str(e, "into")) else {
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
            } else if e.kind == branch_selected {
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
        }
        Self { parent, picks }
    }

    /// 一个分支格都没有 ⇒ 本模块对这批事实零改动（绝大多数会话走这条）。
    fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }
}

/// 并查集找锚（带路径压缩）。
///
/// 迭代而非递归：链长 = 重试次数，它不是栈预算；一次重试压一层，走到今天不会有事，
/// 但把栈深交给数据形状是白送的一个坑。
///
/// **自身即锚**是集合里的表示法，不是待走的边：`fold` 用 `parent[x] = x` 标记「x 是这个
/// 组的成员」（孤立成员也占这一格，`contains_key` / `is_empty` 都靠它）。两处循环都必须在
/// `p == cur` 处停下——把它当成「还要往上走一步」就是死循环。
fn anchor_of(parent: &mut HashMap<String, String>, id: &str) -> String {
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
