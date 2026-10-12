//! 事实网格 → prompt 的**双向完整性**判据（ADR-048）。
//!
//! ## 它防的是什么
//!
//! 三条已证实的缺口（请求视图三段不传 / `context_messages = 0` 清空历史 / 折进的补充
//! 不进事实源）**全部静默**：不报错、不告警、既有断言全绿（事实照样入格、`session/stats`
//! 照样有数），只有 e2e 恰好走到对应分支时偶然照到。本模块把「模型实际看到的东西」变成
//! **可断言的**，让这类缺口在单测层就红。
//!
//! ## 形态：为什么不是「两条路径对照」
//!
//! 执行只有一条路径（v2 运行器），没有第二条路径可作对照。且对照法是**双向的**——
//! 两边都可能错，判据没有唯一一侧。这里反过来：**事实网格是唯一真源**，所以每条判据都以网格为基准。
//!
//! ## 三个方向
//!
//! | 方向 | 断言 | 照出什么 |
//! |---|---|---|
//! | 网格 → prompt | 每条投影内的事件都有一条对应 entry，且 `role` 正确 | 投影层丢事实 |
//! | `role` 完好 | `role == "tool"` 的 entry 必须带工具名 | 工具结果的机器可读关联 |
//! | prompt → 网格 | 每条 entry 都能在网格里找到出处 | 「模型看到了事实源没有的东西」= 两条真源 |
//!
//! ## 第三类：变更格（S2.7-b1，ADR-053）
//!
//! `message.edited` / `message.removed` **既不产 entry**（它们不是新的对话内容），
//! **也不是**「按设计不进请求包」的界面文本（那是汇报 / 首响）。缺了这一类，
//! 判据会把它们当成「不在投影范围内」**静默跳过**——而本模块自己已经警告过
//! 「判据的覆盖范围要与投影的实现范围同时长」（`turn.supplemented` 漏登记那次）。
//!
//! 处理方式分两条，都是**共用同一份折叠索引**（[`super::mutations::MessageMutations`]）：
//!
//! - **编辑**：被编辑的那条**原事件**改按**新**正文复核（否则每次编辑都会报
//!   「网格→prompt 丢事实」——网格里是旧正文，prompt 里是新正文，两边都「对」）；
//! - **删除**：被删节点从「网格→prompt」这一向**跳过**（它**本就不该**在 prompt 里）。
//!   它若仍出现在 prompt 里，**反方向**的 [`PromptGap::NoSourceInGrid`] 会照出来——
//!   除非有别的格子认领那段正文，而那种情况恰恰是「删掉之后又被重新说了一遍」，
//!   本来就不该报。于是这条判据**不需要按正文去猜**「这句话是不是被删的那句」。
//!
//! ## 第四类：取代格（压缩快照，S2.7-b2）
//!
//! `session.consolidated` 是**换基**格：它不产 entry，但它载荷里那份列表的**每一条**
//! 都该在 prompt 里（投影已把它收成基线）。这一类的「出处」与别处不同——**出处就是
//! 这个事件本身**，不是某条更早的事件（那些事件已被移入归档段，不在本切片里）。
//!
//! 少了这一条分支，每次压缩之后判据都会把「快照 + 被保留的近轮」整批报成
//! `NoSourceInGrid`——一个**恒红**的守卫，与没有守卫等价（本模块开头那条纪律）。
//!
//! 认领仍走**同一份**翻译（[`super::transcript::snapshot_entries`]）与**同一份**
//! 折叠索引：两个出口不可能对同一条给出两种读法。
//!
//! ## 为什么它便宜到每轮都跑
//!
//! 纯函数 + 单测层：不起 CLI、不 mock LLM、不碰 IO。所以它不会被「太重」跳过——这一点
//! 是本条存在的意义，全量 e2e 反而做不到（每步都要重起进程、跑 42 个用例）。

use super::super::event::{
    event_node_of, Entity, Event, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS,
    EVENT_ASSISTANT_REPORTED, EVENT_MESSAGE_ADDED, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
    EVENT_SESSION_CONSOLIDATED, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};

/// 缺口的一条报告。**故意不 derive `PartialEq`**：报告要给门禁与日志读，字段顺序变动
/// 不该让「内容相同」的报告变成另一种类型。
#[derive(Debug, Clone)]
pub enum PromptGap {
    /// 网格里有一条投影内的事件，没有对应的 prompt entry（投影层把它丢了）。
    DroppedFromPrompt { kind: String, turn: u64 },
    /// prompt 里有一条 entry，网格里找不到出处（**两条真源**）。
    NoSourceInGrid { role: String, text: String },
    /// `role == "tool"` 却没带工具名——工具结果的机器可读关联丢了。
    ToolEntryWithoutName { text: String },
    /// **按设计不进请求包**的事件（界面文本）漏进了送模型的那批消息。
    ///
    /// 为什么独立成一条而不并进 DroppedFromPrompt：两者方向相反——那条是
    /// 「该有的没有」，这条是「不该有的有了」。合成一个方向就会漏掉其中一半，
    /// 而漏掉的那一半恰好是「界面文本污染了对话」这种用户直接看得见的退化。
    ExcludedLeakedIntoPrompt { kind: String, turn: u64 },
    /// 角色与网格里那条事件的坐标不符（投影把 user 说成 assistant 之类）。
    ///
    /// `expected` 是 `String` 而不是 `&'static str`：**快照条目**的期望角色来自
    /// [`super::transcript::snapshot_entries`] 的产出（运行时值），不是事件 kind
    /// 编译期定下的那几种。收成 `String` 让两类来源共用同一条报告——若为快照另立
    /// 一个变体，两类缺口的措辞与判定迟早各自演化。
    RoleMismatch {
        expected: String,
        got: String,
        kind: String,
        turn: u64,
    },
}

/// 网格事件 → 它在投影里应当成为的 entry（`None` = 不在投影范围内）。
///
/// **唯一真源**：事件 kind 的集合只在这里列一次。投影若漏了某个 kind，本函数是第一个
/// 知道的——而不是等 prompt 里少了东西才从症状往回猜。
fn expected_role(e: &Event) -> Option<&'static str> {
    match (e.entity, e.verb, e.kind.as_str()) {
        (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => Some("user"),
        // 轮边界折进的补充（缺口 3）：它**是用户说的话**，所以 role=user。
        //
        // (P3c 补记) 这一行是补的——补充落进投影时忘了登记进这里，于是
        // expected_role 返回 None、判据把它当成「不在投影范围内」直接跳过。
        // 后果很隐蔽：**补充漏进请求包时判据不会红**。凡是落进投影的 kind 都必须
        // 在这里登记——判据的覆盖范围要与投影的实现范围同时长。
        (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => Some("user"),
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => {
            // 空正文的收束格不进投影（同 `transcript` 的空正文跳过）⇒ 也不在复核范围。
            (!payload_text(e).trim().is_empty()).then_some("assistant")
        }
        // 进展格（等待用户）：同样投影成 assistant 行；空正文照跳。
        (Entity::Turn, Verb::Progressed, EVENT_ASSISTANT_PROGRESS) => {
            (!payload_text(e).trim().is_empty()).then_some("assistant")
        }
        (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => Some("assistant"),
        (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => Some("tool"),
        _ => None,
    }
}

/// **按设计不进请求包**的事件 kind（界面文本）。
///
/// 与 expected_role 是**互补**的两张表而不是一张表的两种取值：那张回答
/// 「它该是什么角色」，这张回答「它根本不该出现」。合成一张的话，「不该出现」
/// 就得靠某个假角色表达，而假角色会一路传到线格式去。
fn excluded_from_prompt(e: &Event) -> bool {
    matches!(
        (e.entity, e.verb, e.kind.as_str()),
        (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED)
            // 对话面开场白（T-056）：与汇报同为界面文本（`exclude_from_context`），
            // 按设计不进请求包——漏进去要红。
            | (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_OPENING)
            // 诞生格（ADR-054 决策 1）：本格的节点按定义是**界面文本**——只有
            // 「找不到别的格承载诞生」的节点才落这里（见 `EVENT_MESSAGE_ADDED`
            // 的边界），而承载对话内容的节点必须由它自己的事实落。
            | (Entity::Message, Verb::Opened, EVENT_MESSAGE_ADDED)
    )
}

/// **变更格**（第三类）：既不产 entry，也不是「按设计不进请求包」。
///
/// 与 `excluded_from_prompt` 分开成两个函数而不是合成一个「特殊格」判定：那张表
/// 说的是「它不该出现在 prompt 里」，这张说的是「它根本不是一条对话内容，它改的是
/// 别的东西」——两者的**检查动作相反**（前者要报「出现了」，后者要**跳过**并折叠）。
/// 合成一个 `matches!` 会让后来的人无从判断该往哪边加。
fn is_mutation(e: &Event) -> bool {
    matches!(
        (e.entity, e.verb, e.kind.as_str()),
        (Entity::Message, Verb::Asserted, EVENT_MESSAGE_EDITED)
            | (Entity::Message, Verb::Closed, EVENT_MESSAGE_REMOVED)
    )
}

/// **取代格**（第四类）：压缩快照。
///
/// 与前两类都不同：它不产 entry，但它**认领**一整批 entry（载荷里那份列表）。
/// 单列成函数而不是并进 `is_mutation`：那类的检查动作是「跳过并折叠既有条目」，
/// 这类的动作是「按载荷**新建**一批期望条目」——合成一个 `matches!` 会让后来的人
/// 无从判断该往哪边加。
fn is_snapshot(e: &Event) -> bool {
    e.entity == Entity::Message
        && e.verb == Verb::Progressed
        && e.kind == EVENT_SESSION_CONSOLIDATED
}

/// 事件载荷里该进 prompt 的正文（与 `transcript` 投影**同一取法**——两处各写一遍必然漂移，
/// 所以这里只做「有没有」，正文比对交给 `SourceRef` 的坐标）。
fn payload_text(e: &Event) -> &str {
    match e.kind.as_str() {
        EVENT_ASSISTANT_FALLBACK => e.payload.get("why").and_then(|v| v.as_str()).unwrap_or(""),
        k if k == EVENT_ARTIFACT_ADDED => {
            e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("")
        }
        _ => e.payload.get("text").and_then(|v| v.as_str()).unwrap_or(""),
    }
}

/// 一次完整性复核的结果。
#[derive(Debug, Clone, Default)]
pub struct PromptReport {
    pub gaps: Vec<PromptGap>,
    /// 参与复核的网格事件数（投影范围内）。
    pub checked_events: usize,
    /// 参与复核的 prompt 消息数。
    pub checked_entries: usize,
    /// **被显式声明**为「按设计不进网格」的消息数（当前只有 `prefix` 三段读视图）。
    pub declared: usize,
    /// 复核里遇到的**变更格**数（`message.edited` / `message.removed`）。
    ///
    /// 单列一格而不是并进 `checked_events`：这一类**不产 entry**（见模块文档的
    /// 「第三类」），所以「它们有多少条」是判断「折叠有没有真的发生」的唯一外部
    /// 可见量——并进去就看不出来了。
    pub mutations: usize,
    /// 复核里遇到的**取代格**数（`session.consolidated`）。
    ///
    /// 与 `mutations` 同理单列：这类同样**不产 entry**（它换基），
    /// 「它有多少条」是判断「换基有没有真的发生」的唯一外部可见量。
    pub consolidations: usize,
}

impl PromptReport {
    /// 是否完整（无缺口）。
    pub fn is_complete(&self) -> bool {
        self.gaps.is_empty()
    }

    /// 缺口的人读摘要（门禁 / 测试失败消息用）。
    pub fn summary(&self) -> String {
        if self.is_complete() {
            return format!(
                "完整：{} 条网格事件 → {} 条 prompt 消息（声明 {} 条请求级 / 折叠 {} 条变更 / 换基 {} 次）",
                self.checked_events, self.checked_entries, self.declared, self.mutations,
                self.consolidations
            );
        }
        let mut s = format!(
            "{} 处缺口（{} 条网格事件 / {} 条 prompt 消息）：\n",
            self.gaps.len(),
            self.checked_events,
            self.checked_entries
        );
        for g in &self.gaps {
            let one = match g {
                PromptGap::DroppedFromPrompt { kind, turn } => {
                    format!("网格→prompt 丢事实：{kind}（turn {turn}）没有对应消息")
                }
                PromptGap::NoSourceInGrid { role, text } => format!(
                    "prompt→网格 无出处：一条 role={role} 的消息在事实源里找不到对应（内容前 40 字：{:.40?}）",
                    text
                ),
                PromptGap::ToolEntryWithoutName { text } => format!(
                    "role 退化：role=tool 却没带工具名（内容前 40 字：{:.40?}）",
                    text
                ),
                PromptGap::ExcludedLeakedIntoPrompt { kind, turn } => format!(
                    "界面文本泄漏：{kind}（turn {turn}）标了「不进请求包」，却出现在送模型的消息里"
                ),
                PromptGap::RoleMismatch { expected, got, kind, turn } => format!(
                    "角色不符：{kind}（turn {turn}）应是 {expected}，prompt 里是 {got}"
                ),
            };
            s.push_str("  · ");
            s.push_str(&one);
            s.push('\n');
        }
        s
    }
}

/// **双向**完整性复核：网格 ↔ **送进模型的消息**。
///
/// `events` = 事实网格的可见切片（窗口 / 可见域已在上游裁好），`messages` = 交给模型
/// 的那批消息。**入参不含「投影」**——本函数自己按 `expected_role` 重算一遍，这样它能
/// 同时照出「投影漏了」与「prompt 里有网格之外的」。
///
/// 验的是**送出去的那批**（含 prefix 与本轮工具交换），不是投影中间产物——
///
/// 投影绿 ≠ 模型看得全。缺口 1 / 缺口 3 都发生在投影**之外**：前者是请求视图三段没进
/// 投影（所以判据看不见它），后者是补充根本没成为事实（网格里就没有它，判据同样看不见）。
/// 验「送出去的」能多抓住一类：**投影有、但组包时丢了**。
///
/// ## `declared`：按设计不进网格的内容
///
/// 请求视图三段（记忆召回 / 就绪任务 / 委派者真源）是**读视图**——本次请求临时投影出来的，
/// 不是事件，所以它们**理应**报「无出处」。但把已知的那一条放过，判据才有用：
/// **一个永远红的守卫等于没有守卫。**
///
/// 放过是**声明**（这个参数），不是「判据放宽」——声明之后判据仍会报「多了一条无出处
/// 消息」，只是不再把**已知的那一条**算成缺口，而且 `Report::declared` 会把条数记下来，
/// 于是「例外有没有悄悄变多」是看得见的。
/// 认领一条**期望**消息：在 prompt 里找**首个未用过**的同正文条目，标记已用并复核角色。
///
/// 抽成自由函数（而不是在 `verify` 里写两遍）是为了让「事件来的条目」与「快照来的
/// 条目」走**同一条**配对与复核——两类缺口（丢事实 / 角色错）若各写一套判据，
/// 迟早对同一件事给出两种结论。
///
/// ⚠️ **只按正文配对，不把 role 放进配对键**——放进去了，「角色不符」就永远匹配不上，
/// 于是会被误报成「丢事实」。那两类缺口的区别恰恰是 role：正文在、角色错，与正文
/// 根本不在，是两种不同的病，判据必须分得开。
///
/// 取**未用过**的第一条：同一轮可能有多个同正文事件（重试各成一格）；事件按 seq
/// 有序、prompt 消息也按序，于是「首个未用」保持原序配对。
fn claim(
    rep: &mut PromptReport,
    used: &mut [bool],
    messages: &[crate::symbio_core::PromptMessage],
    text: &str,
    role: &str,
    kind: &str,
    turn: u64,
) {
    let hit = messages
        .iter()
        .enumerate()
        .position(|(i, m)| !used[i] && m.text == text);
    match hit {
        None => rep.gaps.push(PromptGap::DroppedFromPrompt {
            kind: kind.to_string(),
            turn,
        }),
        Some(i) => {
            used[i] = true;
            let m = &messages[i];
            if m.role != role {
                rep.gaps.push(PromptGap::RoleMismatch {
                    expected: role.to_string(),
                    got: m.role.clone(),
                    kind: kind.to_string(),
                    turn,
                });
            }
            if m.role == "tool" && m.tool.as_deref().map(str::trim).unwrap_or("").is_empty() {
                rep.gaps.push(PromptGap::ToolEntryWithoutName {
                    text: m.text.clone(),
                });
            }
        }
    }
}

pub fn verify(
    events: &[Event],
    messages: &[crate::symbio_core::PromptMessage],
    declared: Option<&str>,
) -> PromptReport {
    let mut rep = PromptReport::default();
    // 变更格的折叠索引（与 `transcript` **共用同一份**）：编辑过的条目在网格侧
    // 按**新**正文复核，被删掉的从「网格→prompt」跳过。见模块文档「第三类」。
    let mutations = super::mutations::MessageMutations::collect(events);
    // 最近一次压缩快照（与 `transcript` **共用同一份**）：只有它是活的，更早的
    // 连自己的列表都已被取代。见模块文档「第四类」。
    let snapshot = super::mutations::mutations_latest_snapshot(events);
    // 快照基线**已经占住**的节点身份。复核与投影 `transcript` 共用同一条「换基边界不
    // 重复」纪律（见那里的文档）：本轮的用户发言既原文躺在快照载荷尾部、又在快照之后
    // 作为开场格落了一事实，于是它会被**两条来源各认领一次**——第一次用掉请求包里那
    // 一条，第二次找不到「还没被用掉」的匹配，报成「网格→prompt 丢事实」。
    // 那是恒假警报（prompt 一个字都不缺），但每次压缩之后每轮都叫一次，真缺口就淹在
    // 里面——本仓的判据一旦恒红就等于没有判据。
    let carried: std::collections::HashSet<String> = match snapshot.as_ref() {
        Some((_, msgs)) => super::transcript::snapshot_entries(msgs)
            .into_iter()
            .map(|(node, _)| node)
            .collect(),
        None => std::collections::HashSet::new(),
    };

    // 网格侧：投影范围内的事件，逐条找对应 entry。
    let mut used = vec![false; messages.len()];
    for (idx, e) in events.iter().enumerate() {
        // 被最近一次快照**取代**的事件不再对 prompt 有主张：它们承载的那段历史已被
        // 快照载荷换基。跳过它们，与投影 [`super::transcript`] 的换基同一口径——
        // 否则每次压缩之后判据都会把整段被压掉的历史报成「丢事实」（恒红 = 无用）。
        //
        // 这一步与 `window_by_turn` 的裁剪**不重复**：窗口裁的是「更早的轮次」，
        // 而 `keep == 0`（不截断）时窗口是全量，被取代的事件就在切片里。
        if snapshot.as_ref().is_some_and(|(si, _)| idx < *si) {
            continue;
        }
        // 变更格自己**不产 entry**：它们不是新的对话内容，也不是「按设计不进请求包」
        // 的界面文本。计数单列（`mutations`）——并进 `checked_events` 就看不出
        // 「折叠有没有真的发生」了。
        if is_mutation(e) {
            rep.checked_events += 1;
            rep.mutations += 1;
            continue;
        }
        // 取代格（压缩快照）：**认领**它载荷里那份列表（见模块文档「第四类」）。
        if is_snapshot(e) {
            rep.checked_events += 1;
            rep.consolidations += 1;
            if let Some((si, msgs)) = snapshot.as_ref() {
                if *si == idx {
                    for (node, entry) in super::transcript::snapshot_entries(msgs) {
                        // 与投影同一份折叠索引：被删的跳过（它本就不该在 prompt 里），
                        // 被编辑过的按新读法认领。
                        if mutations.is_removed(&node) {
                            continue;
                        }
                        let (role, text) = match mutations.edited(&node) {
                            Some(ed) => (
                                ed.role.clone().unwrap_or_else(|| entry.role.clone()),
                                ed.text.clone().unwrap_or_else(|| entry.text.clone()),
                            ),
                            None => (entry.role.clone(), entry.text.clone()),
                        };
                        // 「按设计不进请求包」的界面文本**不该**出现在 prompt 里：
                        // 不认领它（与事件侧 `excluded_from_prompt` 同一处置）。
                        if entry.exclude_from_context {
                            continue;
                        }
                        claim(
                            &mut rep,
                            &mut used,
                            messages,
                            &text,
                            &role,
                            e.kind.as_str(),
                            e.turn,
                        );
                    }
                }
            }
            continue;
        }
        // 按设计不进请求包的那一类：**方向反过来**——不是「网格有、prompt 没有」，
        // 而是「网格标了不该进、prompt 里却出现了」。同样按正文配对，找得到就报缺口。
        //
        // 这条判据的存在理由与 `declared` 同源：漏进请求包的界面文本会让线上出现
        // 连续两条 assistant，而**没有任何现有判据会红**——`expected_role` 对它返回
        // None（它在投影里，但它不是对话内容）。
        if excluded_from_prompt(e) {
            rep.checked_events += 1;
            let text = payload_text(e);
            if messages.iter().any(|m| m.text == text) {
                rep.gaps.push(PromptGap::ExcludedLeakedIntoPrompt {
                    kind: e.kind.as_str().to_string(),
                    turn: e.turn,
                });
            }
            continue;
        }
        let Some(expected) = expected_role(e) else {
            continue;
        };
        rep.checked_events += 1;
        // 生效读法：该节点被编辑过 ⇒ 用**新**正文 / 角色；被删掉 ⇒ **跳过**
        // （它本就不该在 prompt 里，若仍在，反方向的 `NoSourceInGrid` 会照出来）。
        let node = event_node_of(e);
        if let Some(node) = node.as_deref() {
            if mutations.is_removed(node) {
                continue;
            }
            // 换基边界：这条事实的节点身份已被快照基线认领过（上面 `carried`），不再
            // 对 prompt 提第二次主张。它与 `idx < *si` 那个跳过**不是**同一件事：那些是
            // 被整段取代的事件，而这一条是**取代之后**才落的格，只是载荷里已带了原文。
            if carried.contains(node) {
                continue;
            }
        }
        let edited = node.as_deref().and_then(|n| mutations.edited(n));
        let role: String = edited
            .and_then(|ed| ed.role.clone())
            .unwrap_or_else(|| expected.to_string());
        let text: String = match edited.and_then(|ed| ed.text.clone()) {
            Some(t) => t,
            None => payload_text(e).to_string(),
        };
        claim(
            &mut rep,
            &mut used,
            messages,
            &text,
            &role,
            e.kind.as_str(),
            e.turn,
        );
    }

    // prompt 侧：每条消息都得被网格认领过，或被**显式声明**为请求级内容。
    for (i, m) in messages.iter().enumerate() {
        rep.checked_entries += 1;
        if used[i] {
            continue;
        }
        // 声明过的例外（如 `prefix` 三段读视图）：放过，但**记一笔**——
        // 放过本身不可见，例外就会悄悄变多。写进 `declared` 让它每次都看得见。
        if declared.is_some_and(|d| d == m.text) {
            rep.declared += 1;
            continue;
        }
        rep.gaps.push(PromptGap::NoSourceInGrid {
            role: m.role.clone(),
            text: m.text.clone(),
        });
    }

    rep
}

#[cfg(test)]
#[path = "prompt_fidelity.test.rs"]
mod tests;
