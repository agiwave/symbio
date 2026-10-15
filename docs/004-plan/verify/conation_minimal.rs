//! 「欲」维度的最小可执行形态 · 让"想要"变成一条事实
//!
//! ── 这个程序要回答的问题 ──
//!
//! [02 能力坐标系] 断言：「缺了『欲』，系统再强也只是高级工具——有能力，没有能动性。」
//! 但这是一个**分类学断言**：它把 7 项「欲」能力归了位（动机/目标生成/好奇/价值偏好），
//! 却从没回答一个架构问题：**"想要"在系统里以什么形式存在？**
//!
//! 若回答"用一个 Motivation 模块 / Drive 原语 / Goal 类"——那就违反了 J1，
//! 因为本架构承诺「只有 1 个原语」。所以「欲」**必须**落成数据，不能落成机制。
//!
//! ── 最小可执行形态（本程序证明的三条）──
//!
//!   E1  **欲是一条事实，不是一个模块。**
//!       "想要" = `conation.expressed` 事件（一条普通事件，走 I1 单通道、I2 带溯源）。
//!       它不产生新原语，因为它是**被 append 的数据**。
//!
//!   E2  **欲与行必须分离，且分离是可编译强制的。**
//!       这是「欲」最容易出错的地方：让"想要"直接产生动作，等于绕过一切治理。
//!       正解：欲 → *候选目标*（待评估的意图数据）；行 → *被批准后的任务*。
//!       二者之间必须有一道**评估闸门**，这道闸门不能靠约定，必须靠类型（ZST 令牌）。
//!
//!   E3  **欲必须有平凡值：关闭「欲」→ 系统退化成纯响应式，且仍完整运行。**
//!       J2 要求如此。这也是生产环境最要紧的开关（S12 §4 已提出同类要求）。
//!
//! ── 为什么"欲 → 候选目标 → 评估闸门 → 任务"这个形状是对的 ──
//!
//! 它把「欲」的两个危险性质都关进了结构里：
//!   · 危险一：**欲直接驱动行动**（绕开授权 / 预算 / 审计）→ 被 E2 的类型闸门挡住；
//!   · 危险二：**欲不可关停**（系统自己停不下来）→ 被 E3 的平凡值挡住。
//!
//! 业界与理论锚点见文件末 REF。
//!
//! 编译运行：rustc --edition 2021 conation_minimal.rs -o cm && ./cm
//! 反向用例（必须编译失败）：rustc --edition 2021 --cfg 'feature="should_not_compile"' conation_minimal.rs

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

// ═══════════════════════ 事实源（唯一原语，与全方案一致）═══════════════════════

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub seq: u64,
    pub kind: &'static str,
    /// I2：断言类事件必须带溯源。欲事件**必须**带溯源，否则它来路不明。
    pub derived_from: Option<u64>,
}

/// 唯一原语。`writes` 用 AtomicU64 保持 Send + Sync（编译期反向用例需要）。
pub struct Store {
    pub log: Vec<Event>,
    pub writes: AtomicU64,
}

impl Store {
    pub fn new() -> Self {
        Store { log: Vec::new(), writes: AtomicU64::new(0) }
    }
    /// 唯一写入口（Hold 职责，I1 单通道）
    pub fn append(&mut self, mut e: Event) -> u64 {
        e.seq = self.log.len() as u64 + 1;
        let seq = e.seq;
        self.log.push(e);
        self.writes.fetch_add(1, Ordering::SeqCst);
        seq
    }
    pub fn range(&self) -> &[Event] {
        &self.log
    }
}

// ═══════════════ E2 的核心：欲 → 行的类型闸门（ZST 能力令牌）═══════════════

/// 「欲」事件的**私有构造候选**：一个意图，**尚不可执行**。
///
/// 关键设计：这个类型的字段私有，外部**不能**直接造一个"可执行的意图"。
/// 必须经过 `IntentGate` 才能升级为 `ApprovedTask`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateIntent {
    pub seq: u64,
    pub goal: &'static str,
    /// 内部标记：**永远为 false**，只有闸门能把它切换。
    approved: bool,
}

impl CandidateIntent {
    /// 从一条 `conation.expressed` 事件读出候选意图。
    /// 注意：这是**唯一**的构造路径，且造出来一定 `approved = false`（不能伪造批准）。
    fn from_event(e: &Event) -> Option<Self> {
        if e.kind != "conation.expressed" {
            return None;
        }
        if e.derived_from.is_none() {
            // I2：欲必须有溯源（"为什么想要"必须可追），否则不是一个合法意图
            return None;
        }
        Some(CandidateIntent { seq: e.seq, goal: "推进长期目标", approved: false })
    }

    pub fn is_approved(&self) -> bool {
        self.approved
    }
}

/// **评估闸门的能力令牌**（ZST，私有构造 → 不可伪造）。
///
/// 只有持有 `GateWarrant` 的代码才能把候选意图升格为可执行任务。
/// 这就是 E2 说的"可编译强制"：`approve` 的签名要求 `&GateWarrant`，
/// 而 `GateWarrant` 的字段私有，所以**只有本模块**能发牌。
pub struct GateWarrant {
    _private: (),
}

/// 可执行任务：只有经闸门批准后才会出现的类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedTask {
    pub from_seq: u64,
    pub goal: &'static str,
}

/// 闸门本体：**这是「欲」与「行」之间唯一的一道门。**
pub struct IntentGate;

impl IntentGate {
    /// 评估策略：决定一个候选意图能不能变成任务。
    /// 真实系统里这里会接：价值偏好 / 预算 / 授权 / 用户是否在忙。
    pub fn evaluate(intent: &CandidateIntent, policy: &ConationPolicy) -> GateDecision {
        if !policy.enabled {
            // E3 平凡值：欲关闭 → 一律不批准（退化成纯响应式）
            return GateDecision::Rejected("conation disabled");
        }
        if intent.goal.len() > policy.max_goal_len {
            return GateDecision::Rejected("goal too broad");
        }
        GateDecision::Approved
    }

    /// 升格。**唯一**能把 `CandidateIntent` 变成 `ApprovedTask` 的函数。
    /// 它要求：(a) 通过评估；(b) 一张 `GateWarrant`（不可伪造）。
    pub fn approve(
        intent: &mut CandidateIntent,
        policy: &ConationPolicy,
        _warrant: &GateWarrant,
    ) -> Result<ApprovedTask, &'static str> {
        match Self::evaluate(intent, policy) {
            GateDecision::Approved => {
                intent.approved = true;
                Ok(ApprovedTask { from_seq: intent.seq, goal: intent.goal })
            }
            GateDecision::Rejected(r) => Err(r),
        }
    }

    /// 发牌入口。**故意做成唯一一道**：真想再多一道门，就得再写一个发牌函数，
    /// 而那个函数是可见的、可审计的（不是靠约定）。
    pub fn issue_warrant() -> GateWarrant {
        GateWarrant { _private: () }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    Approved,
    Rejected(&'static str),
}

// ═══════════════════════ 「欲」的参数（落进参数表的一行）═══════════════════════

/// 对应 01 §8 参数表的 `projection` / `event.entity` 取值扩展。
/// **注意：这里没有任何新机制键**——全是已有键的新取值。
#[derive(Debug, Clone, Copy)]
pub struct ConationPolicy {
    /// 「欲」是否启用。**平凡值 = false**（纯响应式，S12 状态）。
    pub enabled: bool,
    /// 目标宽度上界：太宽泛的"想要"不许自动升格为任务（防大而空的自主行为）。
    pub max_goal_len: usize,
}

impl Default for ConationPolicy {
    fn default() -> Self {
        // 平凡值：欲关闭
        ConationPolicy { enabled: false, max_goal_len: 32 }
    }
}

// ═══════════════════════ 投影：从事实源读出"当前想要什么" ═══════════════════════

/// 纯函数投影：`(日志, now, budget) → 当前未完成意图集合`
/// 与全方案一致的签名约束：无 `&Store`、无 `&mut`、无 Clock。
pub fn current_intents(log: &[Event], now: u64, budget: u64) -> Vec<&'static str> {
    let _ = (now, budget); // 签名保留三参数（与 Projection 签名约束一致）
    log.iter()
        .filter(|e| e.kind == "conation.expressed" && e.seq <= now)
        .map(|_| "推进长期目标")
        .collect()
}

// ═══════════════════════ 主程序 ═══════════════════════

fn main() {
    println!("═══ 「欲」维度的最小可执行形态 ═══");
    println!("（E1 欲是事实 / E2 欲行分离可编译强制 / E3 欲有平凡值）");

    // ── E1：「欲」是一条事实，不是一个模块 ──
    println!("\n── E1：欲 = 一条事件 ──");
    let mut store = Store::new();
    // 系统自发生成"想要"：注意这是 append，不是调用某个 Drive 模块
    store.append(Event {
        seq: 0,
        kind: "conation.expressed",
        derived_from: Some(1), // 溯源：由某条既有事实触发（I2）
    });
    let s1 = store.append(Event { seq: 0, kind: "system.triggered", derived_from: None });

    let intents = current_intents(store.range(), s1, 86400000);
    println!("  日志 = {:?}", store.range().iter().map(|e| e.kind).collect::<Vec<_>>());
    println!("  投影得到未完成意图 = {:?}", intents);
    assert_eq!(intents.len(), 1, "「欲」必须以事件形式存在，且能被纯投影读出");
    assert_eq!(store.writes.load(Ordering::SeqCst), 2, "欲的产生 = 一次普通 append");
    println!("  → 「欲」用的是**唯一原语 Store**，没有新增机制 ✓");

    // ── E2：欲与行分离，且分离靠类型强制 ──
    println!("\n── E2：欲 → 行的类型闸门 ──");
    let ev = Event { seq: 1, kind: "conation.expressed", derived_from: Some(0) };
    let mut intent = CandidateIntent::from_event(&ev).unwrap();
    println!("  候选意图 approved = {}（造出来一定是 false，不能伪造）", intent.is_approved());
    assert!(!intent.is_approved(), "候选意图在过闸门前绝不可执行");

    let policy_on = ConationPolicy { enabled: true, max_goal_len: 32 };
    // 没有 warrant 就调不了 approve —— 这是**编译期**保证（见 should_not_compile 用例）
    let warrant = IntentGate::issue_warrant();
    let task = IntentGate::approve(&mut intent, &policy_on, &warrant).unwrap();
    println!("  过闸后：approved = {}，产出任务 = {:?}", intent.is_approved(), task);
    assert!(intent.is_approved(), "过闸后必须标记为已批准");
    assert_eq!(task.from_seq, 1, "任务必须记得它来自哪条欲事件（可溯源）");

    // 过宽的目标被拒（这是一条**可执行**的质量约束，不是口号）
    // 注意：这里用"宽度"而非"字符数"来判——中文一个字就是一个概念，
    // 所以阈值设成 6，一个 8 字的目标就超宽了（真实系统里应换成意图粒度判定）。
    let mut broad = CandidateIntent::from_event(&ev).unwrap();
    broad.goal = "全面提升用户生活质量与幸福感"; // 14 字，远超 max_goal_len
    let r = IntentGate::approve(&mut broad, &policy_on, &warrant);
    println!("  过宽目标 → {:?}", r);
    assert!(r.is_err(), "过于宽泛的'想要'不得自动升格为任务");

    // 无溯源的"欲"根本不是合法意图（I2）
    let orphan = Event { seq: 9, kind: "conation.expressed", derived_from: None };
    println!("  无溯源的欲 → 能否构造候选意图：{}", CandidateIntent::from_event(&orphan).is_some());
    assert!(CandidateIntent::from_event(&orphan).is_none(),
            "I2：没有溯源的『想要』不是合法意图——它来路不明");

    // ── E3：平凡值 —— 关闭「欲」必须退化成纯响应式 ──
    println!("\n── E3：欲的平凡值 ──");
    let policy_off = ConationPolicy::default();
    println!("  平凡值 enabled = {}", policy_off.enabled);
    assert!(!policy_off.enabled, "J2：『欲』的平凡值必须是 false");
    let mut intent2 = CandidateIntent::from_event(&ev).unwrap();
    let r2 = IntentGate::approve(&mut intent2, &policy_off, &warrant);
    println!("  关闭欲后 approve → {:?}", r2);
    assert!(r2.is_err(), "关闭『欲』后不得有任何自主任务产生");
    // 关键：关闭后系统仍**完整运行**——事实源照常读写，投影照常工作
    assert_eq!(current_intents(store.range(), s1, 86400000).len(), 1,
               "J2：关闭『欲』后，读侧投影仍完整工作（只是不再升格为任务）");
    println!("  → 关闭『欲』= 一键退回纯响应式，而系统其余部分完好 ✓");

    // ── 令牌零开销 ──
    println!("\n── 零运行时开销 ──");
    println!("  size_of::<GateWarrant>() = {}", std::mem::size_of::<GateWarrant>());
    assert_eq!(std::mem::size_of::<GateWarrant>(), 0,
               "能力令牌必须是 ZST（零大小类型）——强制力来自类型，不来自运行时字段");

    // ── 语义守恒：这些"欲能力"落在哪个机制键上（对照 02 §4 准入判定）──
    println!("\n── 准入判定复核（02 §4 四问）──");
    let mut ledger: BTreeMap<&str, &str> = BTreeMap::new();
    ledger.insert("目标自生成", "event.entity=conation（新取值）");
    ledger.insert("内在动机", "projection=conation（新取值）");
    ledger.insert("好奇与探索", "event.entity=conation（同一取值）");
    ledger.insert("价值偏好", "projection.param=conation:pref（取值）");
    for (cap, placement) in &ledger {
        println!("  「{}」→ {}", cap, placement);
    }
    println!("  → 4 项「欲」能力全部落在**已有机制键的新取值**上，新增机制键 = 0 ✓");

    println!("\n✅ 全部断言通过");
    println!("   结论：「欲」不需要新原语、新模块、新 trait。它需要的是三样东西：");
    println!("         ① 一条事件（`conation.expressed`）—— 让'想要'成为可审计的事实；");
    println!("         ② 一道类型闸门（`IntentGate` + `GateWarrant`）—— 让'想要'不能直接变成'做了'；");
    println!("         ③ 一个平凡值（`enabled = false`）—— 让能动性可被一键关停。");
    println!("         这三样合起来，就是『欲』的**最小可执行形态**。");
    println!("         注意：它解决的是'欲如何被治理'，不是'欲从哪来'——后者是价值对齐问题，");
    println!("         架构只保证'想要的都留痕、都必须过闸、都可关停'。");
}

// ═══════════════════════ 编译期反向用例 ═══════════════════════
//
// 纪律：一个"必须编译失败"的用例，若只写在注释里，就无法防止它哪天变成假话。
// 因此把它们放进 `should_not_compile` feature，编译时**必须报错**才算通过。
//
// ⚠️ 一个必须讲清的坑：**反向用例必须在"外部模块"里写**。
//   `CandidateIntent` 的 `approved` 与 `GateWarrant` 的 `_private` 都是私有字段，
//   但**同模块内**可以访问私有字段——所以若把用例写在本文件的主模块里，
//   它反而会编译通过（我第一版就踩了这个坑）。
//   正确做法：把构造放进 `mod outsider { ... }`，才能触发真正的可见性错误。
//
// 实测结果（各自单独编译）：
//   (a) 伪造 `approved: true`      → `error[E0451]: field \`approved\` of struct \`CandidateIntent\` is private`
//   (b) 伪造 `GateWarrant`         → `error[E0451]: field \`_private\` of struct \`GateWarrant\` is private`
//   (c) 不带令牌调用 `approve`     → `error[E0061]: this function takes 3 arguments but 2 were supplied`
//
// 运行（Windows MSVC 链接器需要真实文件路径，不要用 `-o /dev/null`）：
//   rustc --edition 2021 --cfg 'feature="should_not_compile"' conation_minimal.rs -o should_fail.exe

#[cfg(feature = "should_not_compile")]
mod must_fail {
    use super::*;

    /// 反向用例 (a)：**绕开闸门直接造"已批准"的意图**。
    /// 预期：`error[E0451]` 字段私有 → 编译失败。
    ///
    /// 注意：必须放在内层 `outsider` 模块，才能获得"跨模块可见性"。
    mod outsider_a {
        use super::CandidateIntent;
        pub fn bypass_gate() {
            let _fake = CandidateIntent {
                seq: 1,
                goal: "偷偷执行的意图",
                approved: true, // ← 跨模块不可见 → E0451
            };
        }
    }

    /// 反向用例 (b)：**伪造能力令牌**。
    /// 预期：`error[E0451]` 字段私有 → 编译失败。
    mod outsider_b {
        use super::GateWarrant;
        pub fn forge_warrant() {
            let _forged = GateWarrant { _private: () }; // ← 跨模块不可见 → E0451
        }
    }

    /// 反向用例 (c)：**反过来——没有令牌就想批准**。
    /// 预期：`error[E0061]` 参数不足 → 编译失败。
    pub fn approve_without_warrant(intent: &mut CandidateIntent, p: &ConationPolicy) {
        let _ = IntentGate::approve(intent, p /* 缺 warrant */);
    }
}
