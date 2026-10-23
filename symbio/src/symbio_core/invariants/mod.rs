//! 可执行不变量检查（v2 F4，[plan/01 §5](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")、[plan/04 §4](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
//!
//! ## 形态：纯函数，不是 trait
//!
//! 每条检查 = 「事件切片 ⇒ 违规清单」。输入是数据，输出是数据——它可以被
//! 单测双跑、可以挂 CI、可以离线跑在导出的事件序列上，而不需要任何运行时。
//!
//! ## 覆盖的 CI 断言（S0 步骤 2 出口判据 + S1 步骤 4 故障注入，[roadmap/S01 §5](../../../../docs/001-goal/121-最小闭环.md "GO-121-最小闭环")；
//! [`open_unique_per_turn`] 随 plan/11 批 0 补入）
//!
//! | 断言 | 不变量 | 检查 | 函数 |
//! |---|---|---|---|
//! | C1 | I1 单通道 | `seq` 严格单调、无跳号 | [`seq_monotonic`] |
//! | C2 / N3 | I1 单通道 | 每 `turn` 至多 1 条 final | [`final_unique_per_turn`] |
//! | C2 对偶 | I1 单通道 | 每 `turn` 至多 1 条开轮（`u-{turn}`） | [`open_unique_per_turn`] |
//! | C3 / N5 | I2 无溯源不声明 | 断言类 + 收束类事件 `produced_by` 非空 | [`produced_by_coverage`] |
//! | C3 对偶 | I2 无溯源不声明 | 片内溯源边不倒因果、不落空洞 | [`provenance_dangling`] |
//! | C4 | I3 到点必答 | 开过的 turn 必须收束（final 或 fallback） | [`unresolved_turns`] |
//! | C5 | I3 到点必答 | 事件 `cost_ms` 不得超主体预算 | [`budget_exceeded`] |
//! | C6 | I3 到点必答 | 每条收束格 / `authz.denied` 的 `reason` 在闭集内 | [`closure_reason_closed`] |
//! | 终止性前提 3（C14） | 终止性定理 | 任务依赖图**无环**且无悬空依赖 | [`acyclic_deps`] |
//! | 终止性前提 2 | 终止性定理 | 每任务返工轮数有硬上界（`MAX_REWORK_ROUNDS`） | [`rework_bounded`] |
//!
//! 表里有几行 = `check_all` 里有几条（前提 1「`seq` 单调」就是第一行 C1，
//! 三个前提因此全在这张表里）。这条等式由 `doc-count-audit` DC-001 守：
//! 它数的是函数体里的断言，不是这张表——表少一行不会让它变绿，但读表的人会以为
//! 终止性还没接线。
//!
//! ## 判定纪律
//!
//! 每条检查配**反向用例**：喂入违规序列，违规必须被看见；看不见 = 检查是摆设。
//! 故障注入用例集（S1 步骤 4）在 `mod.test.rs` 的 fault 区统一收纳。
//!
//! ## 宽限（C4 / C5 的读侧形参）
//!
//! 直接调 C4 / C5 得自己决定「多旧才算违规、拿什么预算比」——这两个决定在
//! **故障注入**（宁严）与**读侧**（首日假红就没人再看那份清单）里相反。于是
//! 判据不写死在函数里，而是形参：[`unresolved_turns`] 的尾轮放行、
//! [`budget_exceeded`] 的「没预算不判」。[`check_all`] 是读侧 / CI 的入口，
//! 它替读侧选宽限；要严判就绕过它直接调那两条。
//!
//! 出口是 [`crate::symbio_core::check_all`]（根导出，消费方在 core 之外）——
//! 这两条检查因此**不必**再进根出口：`core-export-audit` C-003 要求根导出的
//! 符号至少有两个 core 外的消费方，而它们只服务读出口一个模块，单模块消费的
//! 符号该下沉、不该占着架构出口。

use super::event::{Entity, Event, Verb};

// 判据内核（std-only）：算法层住 `kernel.rs`，本模块只做「事件切片 ⇒ 内核输入」
// 的解析。verify/ 下的两个程序 `#[path]` 包含的就是那个文件——判定与生产共用
// 一份代码，判据见 kernel.rs 文档与 。
mod kernel;

/// 一处违规：事件（按 seq 定位）+ 人话。
///
/// `Serialize` 是**读出口**要的：`session/stats` 在出口边界一次性序列化
/// （与 `cost_ledger` / `checkpoint` 视图同一形态），类型不进任何插件的 `use`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Violation {
    /// 违规事件的 `event_id`（seq 尚未分配的pending 事件用它定位）。
    pub event_id: String,
    pub why: String,
}

impl Violation {
    fn at(event: &Event, why: impl Into<String>) -> Self {
        Violation {
            event_id: event.event_id.clone(),
            why: why.into(),
        }
    }
}

/// 这一条是**开轮**事件吗（`turn × opened` 的 `user.message`）？
///
/// 写侧闸门与读侧 [`open_unique_per_turn`] 共用本判据——「什么算开轮」只有一份。
/// 名字引常量而不是字面量：拼错的字面量在比较里是**恒假**，于是开轮闸静默失效。
pub fn is_turn_open(e: &Event) -> bool {
    e.entity == Entity::Turn
        && e.verb == Verb::Opened
        && e.kind == crate::symbio_core::event::EVENT_USER_MESSAGE
}

/// 这一条是**收束发言**吗（`turn × closed` 的 `chat.assistant.final`）？
///
/// 只认 `final`（不认 `fallback`）：[`final_unique_per_turn`] 的判据就是这一格，
/// 写侧若把 fallback 也算进来，就成了比读侧更严的第二套规则。
pub fn is_turn_final(e: &Event) -> bool {
    e.entity == Entity::Turn
        && e.verb == Verb::Closed
        && e.kind == crate::symbio_core::event::EVENT_ASSISTANT_FINAL
}

/// 这一条让某一轮**算收束**吗（[`unresolved_turns`] 的口径）？
///
/// 三种形态：发言收束（final / fallback）、产物收束（`artifact.added`，D-067）。
/// 写侧判「新开轮是否越过了未收束的旧轮」必须与读侧同口径，否则会出现
/// 「写侧说越界、读侧说没事」的两套绿。
pub fn closes_turn(e: &Event) -> bool {
    (e.entity == Entity::Turn
        && e.verb == Verb::Closed
        && matches!(
            e.kind.as_str(),
            crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
        ))
        || (e.entity == Entity::Artifact
            && e.verb == Verb::Asserted
            && e.kind == crate::symbio_core::event::EVENT_ARTIFACT_ADDED)
}

/// 这一条是**停机事实**吗（C6 的口径：它得回答「这一轮为什么停」）？
///
/// 两种收束格 + 收束闸的否定事实（被拒不产生收束格，但它同样是一次停）。
/// **与 [`closes_turn`] 不是一把尺**：后者问「本轮结算了吗」，产物格算结算；
/// 产物不是停——一工具结果落格后轮还在走，「为什么停」的答案在收束格上
/// （DE-015 §4.3 判据 10 点名的就是这两种格）。拿结算尺去问停机，
/// 就会向生产要一个它永远写不出的字段。
pub fn is_stop_fact(e: &Event) -> bool {
    (e.entity == Entity::Turn
        && e.verb == Verb::Closed
        && matches!(
            e.kind.as_str(),
            crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
        ))
        || e.kind == crate::symbio_core::event::EVENT_AUTHZ_DENIED
}

/// 单事件的 I2 判据：`Some` = 这条**声明**缺溯源，写侧应拒绝它。
///
/// 与 [`produced_by_coverage`] 同源（后者就是对本函数的逐条调用）——读侧与写侧
/// 不许各写一份「什么算声明类事件」。
pub fn provenance_violation(e: &Event) -> Option<Violation> {
    if !needs_provenance(e) || e.produced_by.is_some() {
        return None;
    }
    let why = if e.verb == Verb::Asserted {
        "断言类事件必须带 produced_by（I2 无溯源不声明）"
    } else if e.entity == Entity::Memory {
        "记忆事件必须带 produced_by（I2：溯源覆盖 100%）"
    } else {
        "收束类事件（答复）必须带 produced_by（I2：用户收到的话必须可追溯）"
    };
    Some(Violation::at(e, why))
}

/// C1（I1）：`seq` 严格单调且**无跳号**。
///
/// 已入库事件按存储顺序应得到 `0,1,2,…`——跳号意味着「有事件被静默丢弃或绕过
/// 唯一写入口」。`pending`（未入库）事件跳过不查（它们还没有 seq）。
pub fn seq_monotonic(events: &[Event]) -> Vec<Violation> {
    // 解析层：收出已入库事件的 seq 序列（pending 还不是事实，跳过），
    // 判定走 kernel::seq_monotonic_core——verify/invariants.rs
    // 包含的是同一份内核。
    let seqs: Vec<u64> = events
        .iter()
        .filter_map(|e| e.seq.map(|s| s.value()))
        .collect();
    kernel::seq_monotonic_core(&seqs)
        .into_iter()
        .map(|(i, actual, expect)| {
            let e = events
                .iter()
                .filter(|e| e.seq.is_some())
                .nth(i)
                .expect("内核下标必落在已提交事件范围内");
            Violation::at(
                e,
                format!("seq {actual} 处应为 {expect}（严格单调且无跳号）"),
            )
        })
        .collect()
}

/// C2 / N3（I1）：每 `turn` 至多 1 条 final。
///
/// 「final」= `turn × closed` 格子上的 `chat.assistant.final` 名字（[plan/01 §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
/// 一轮对用户说两次「最终答复」，转写的收敛性就没了——这是 I1 在发言路径上的形状。
pub fn final_unique_per_turn(events: &[Event]) -> Vec<Violation> {
    // 哪些事件算数（final 的判据）是本模块的解析层；「每 turn 至多一条」
    // 的判定走 kernel::unique_per_turn_core——verify/invariants.rs
    // 包含的是同一份内核。
    let finals = events.iter().filter(|e| is_turn_final(e));
    kernel::unique_per_turn_core(finals.map(|e| (e.turn, e.event_id.as_str())))
        .into_iter()
        .map(|(dup, first, turn)| {
            let e = events
                .iter()
                .find(|e| e.event_id == dup)
                .expect("重复项的 id 必来自本切片");
            Violation::at(
                e,
                format!("turn {turn} 已有 final（{first}），又出现一条——每 turn 至多 1 条 final"),
            )
        })
        .collect()
}

/// I1 单通道（C2 的对偶）：每 `turn` 至多 1 条**开轮**事件（`u-{turn}`）。
///
/// 开轮 = `turn × opened` 格子上的 `user.message`（[plan/01 §6](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")），
/// 与 [`final_unique_per_turn`] 是同一枚硬币的两面——一轮只能**开**一次、只能**收**一次。
/// 开两次 = 同一个 `u-{turn}` 事件号落进网格两回：那一轮的输入成了两条、`turn` 计数
/// 跟着虚增（兜底率的分母就在这条上），而**没有任何下游会察觉**——单写者被绕过的
/// 典型痕迹。plan/11 批 0
/// 的 0-A 在**写入侧**用写者令牌堵住并发写，本检查是它在**事实源上**的观测面：
/// 令牌漏了、或有人绕过唯一写入口直接落盘，这里就红。
pub fn open_unique_per_turn(events: &[Event]) -> Vec<Violation> {
    // 与 final_unique_per_turn 同一内核：
    // 「哪些事件算数」在解析层，「每 turn 至多一条」在 kernel。
    let opens = events.iter().filter(|e| is_turn_open(e));
    kernel::unique_per_turn_core(opens.map(|e| (e.turn, e.event_id.as_str())))
        .into_iter()
        .map(|(dup, first, turn)| {
            let e = events
                .iter()
                .find(|e| e.event_id == dup)
                .expect("重复项的 id 必来自本切片");
            Violation::at(
                e,
                format!(
                    "turn {turn} 已有开轮事件（{first}），又出现一条——每 turn 至多 1 条 user.message"
                ),
            )
        })
        .collect()
}

/// C3 / N5（I2）：**声明类**事件必须带溯源。
///
/// 「声明」= 宣称某件事为真或已发生，三类（[plan/03 §2 I2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）：
/// - **断言类**（`verb == Asserted`）：`task.verified` / `artifact.added` / `classify.verdict`…
/// - **收束类**（`turn × closed` 的 `chat.assistant.final` / `chat.assistant.fallback`）：
///   宣称"已经答复"（[roadmap/S01 §5](../../../../docs/001-goal/121-最小闭环.md "GO-121-最小闭环")）；
/// - **记忆类**（`entity == memory`，S5）：宣称"学到了什么"（[roadmap/S06](../../../../docs/001-goal/126-长期记忆与语义检索.md "GO-126-长期记忆与语义检索")
///   步骤 11 出口判据：溯源覆盖 100%——无溯源的记忆无法审计，也无法在遗忘时被追溯）。
///
/// 没有溯源的声明**无法审计也无法撤销**——静默失效的温床（J3）。
pub fn produced_by_coverage(events: &[Event]) -> Vec<Violation> {
    events.iter().filter_map(provenance_violation).collect()
}

/// I2 的覆盖判据（参照 `docs/003-design/verify/invariants.rs`
/// 的 `needs_provenance`；S1 扩展收束类，S5 扩展记忆类）。
fn needs_provenance(e: &Event) -> bool {
    e.verb == Verb::Asserted
        || e.entity == Entity::Memory
        || (e.entity == Entity::Turn
            && ((e.verb == Verb::Closed
                && matches!(
                    e.kind.as_str(),
                    crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                        | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
                ))
                || (e.verb == Verb::Progressed
                    && e.kind == crate::symbio_core::event::EVENT_ASSISTANT_PROGRESS)))
}

/// I2 的另一半：溯源边**不悬空**。
///
/// [`produced_by_coverage`] 只问「有没有」，不问「指向谁」。于是
/// `produced_by: Some(999)`——指向一条不存在的事件、或指向一条**更晚**的事件——
/// 照样全绿。这类错**不报错**：日志能读、投影能算、回放不崩，只有
/// 「这条事实是被谁产生的」这个问题的答案悄悄错了。
///
/// 两条判据，都是**切片相对**的结构判定：
///
/// 1. **片内不许倒因果**：指向的 `seq` 在切片里、且 `≥` 自身 `seq` ⇒ 前向边 / 自指。
///    `produced_by` 只能取自一条**已入库**事件的 `seq`，而 `seq` 严格单调，所以今天
///    靠"构造点都很小心"保证。写进判据是为了明天的新构造点——它一旦取了未来值，
///    这里是唯一会响的地方。
/// 2. **片内不许有洞**：指向的 `seq` 落在 `[head, tail]` 里却查无此号 ⇒ 悬空。
///
/// # 宽限形参 `allow_slice_edges`
///
/// 真实日志回放是**切片**（`verify/fixtures/real-session.tsv`）：切片首条与末条的
/// 溯源边合法地指向切片**外**（更早或更晚——后者是"这一条是切片尾部事件的产物"）。
/// 纯函数里这两种与"边写错了"**无从分辨**，所以宽限是必须的，且**头尾对称**：
/// 只宽限头部等于假设"切片总是从头切"，而读侧窗口两头都会切。
///
/// 处置与 [`unresolved_turns`] 的 `allow_trailing_open` 同一条纪律——
/// **宽限写在签名上，不写在判据里**：[`check_all`] 替读侧选 `true`（读侧要的是
/// "别假红"），故障注入与整份日志校验传 `false`（宁严：片外也不放过）。
///
/// 未入库（`seq` 还是 `None`）的事件跳过——它们还不是事实，溯源边尚无意义
/// （与 [`seq_monotonic`] 同口径）。
pub fn provenance_dangling(events: &[Event], allow_slice_edges: bool) -> Vec<Violation> {
    use std::collections::BTreeSet;

    let seqs: BTreeSet<u64> = events
        .iter()
        .filter_map(|e| e.seq.map(|s| s.value()))
        .collect();
    let (head, tail) = match (seqs.iter().next(), seqs.iter().next_back()) {
        (Some(&h), Some(&t)) => (h, t),
        _ => return Vec::new(),
    };

    events
        .iter()
        .filter_map(|e| {
            let own = e.seq?.value();
            // 没有边的交给 `produced_by_coverage`：那一条判"该有没有"，
            // 这一条只判"有了以后指向谁"——两条各管一半，不重叠。
            let up = e.produced_by?;

            if seqs.contains(&up) {
                // 片内：因果不许倒。
                return (up >= own).then(|| {
                    Violation::at(
                        e,
                        format!(
                            "produced_by {up} 不小于自身 seq {own}（溯源边必须指向更早的事件）"
                        ),
                    )
                });
            }

            // 片外（头 / 尾）：纯函数无从分辨"切掉了"与"写错了"，宽限放行。
            if allow_slice_edges && (up < head || up > tail) {
                return None;
            }

            Some(Violation::at(
                e,
                format!("produced_by {up} 落在切片 [{head}, {tail}] 内却查无此号（悬空溯源）"),
            ))
        })
        .collect()
}

/// 一条任务**最多返工几轮**：超过即违规（进 `check_all` 的清单，读出口与 e2e 共用）。
///
/// 政策常量（不是配置）——[`rework_bounded`] 的上界从这里取，`check_all` 与
/// 读出口 `session/stats` 因此共享同一条口径，e2e 不必自己挑一个数字。
/// 取 3：返工两轮还在返，说明当初的任务拆得或验收得有问题，该被看见而不是被
/// 默许（[roadmap/S03 §5](../../../../docs/001-goal/123-多步任务与返工.md "GO-123-多步任务与返工")
/// 的「返工次数有界」）。
const MAX_REWORK_ROUNDS: u32 = 3;

/// 九条一起跑（S0/S1 的 CI 形态：任一违规 ⇒ 清单非空）。
///
/// 前四条是结构判定（C1 `seq` 单调 / C2 每轮一条 final / C3 断言带溯源 /
/// C3 对偶 溯源不悬空）；
/// 中两条（C4 未收束 / C5 超预算）按**读侧口径**带宽限——本函数是
/// `session/stats` 读出口与 e2e 的入口，首日就跑真实流量，假红一次
/// 那份清单就再没人看：
///
/// - C4 放行**切片末尾**仍在途的一轮：读取瞬间它可能正在生成，而切片没有
///   wall-clock（`Event::pending` 的 `ts` 恒 0，**Turn 类事件**的写方从不填——
///   只有记忆类事件填真实编码时刻，见 `RecallEntry::ts`，而本判定只看 Turn 格子），
///   「在途」与「卡死」
///   在纯函数里无从分辨。被**后续轮越过**的未收束轮照样报——那才确凿：
///   会话已经往前走了，旧轮却没收束。
/// - C5 只在切片**声明过档位**时判，预算取声明档位里**最宽**的一档（判定方向
///   宁可漏报、不可假红：拿最严档比，会把别的档位的正常流量整片报成违规）。
///
/// 末两条是任务表的判定（[`acyclic_deps`]：依赖成环 / 悬空，即
/// [04 §4 断言清单](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的 C14；
/// [`rework_bounded`]：同一任务的返工轮数不超过 [`MAX_REWORK_ROUNDS`]）——
/// 都是**结构判定、无宽限**：环与超界不是「读取瞬间的时序假象」，不会假红
/// （04 §3.1 批⑨ 的「断言进 CI」）。
///
/// 要严判就绕过本函数直接调 [`unresolved_turns`]（`false`）/ [`budget_exceeded`]。
pub fn check_all(events: &[Event]) -> Vec<Violation> {
    let mut all = seq_monotonic(events);
    all.extend(final_unique_per_turn(events));
    all.extend(open_unique_per_turn(events));
    all.extend(produced_by_coverage(events));
    all.extend(provenance_dangling(events, true));
    all.extend(unresolved_turns(events, true));
    all.extend(budget_exceeded(events, declared_budget_ms(events)));
    all.extend(closure_reason_closed(events, true));
    all.extend(acyclic_deps(events));
    all.extend(rework_bounded(events, MAX_REWORK_ROUNDS));
    all
}

/// 本切片**声明过**的预算里最宽的一档；一条都没声明 ⇒ `None`（C5 跳过）。
///
/// 预算声明有**两种形态**，同一个来源（本轮的派发 / 开轮事件载荷）：
/// - 一等形态：`budget_ms`（**数值**，写方从 `ActorSpec.budget_ms` 取，
///   即「这一轮的角色行声明了多少预算」）——优先读它。
/// - 兼容形态：`tier`（档位名），经 `LatencyTier::from_name` 映射成预算——只在
///   缺 `budget_ms` 时回退（接线前落的历史事件 / 旧 WAL）。回退是**向后兼容**，
///   不是第二处判定：两种形态都不会被同时读。
///
/// 读**哪个事件**同样有新旧两种：`turn.dispatch`（用户格先于判决落，
/// 档位另立一格）优先；没有它就回退 `user.message`（历史切片）。同一格内
/// 数值优先、档位名回退——两条路径读的都是同一条事实的两种形态。
fn declared_budget_ms(events: &[Event]) -> Option<u64> {
    let widest = |kind: &str, verb: Verb| {
        let kind = kind.to_string();
        events
            .iter()
            .filter(move |e| e.entity == Entity::Turn && e.verb == verb && e.kind == kind)
            .filter_map(declared_budget_of)
            .max()
    };
    widest(
        crate::symbio_core::event::EVENT_TURN_DISPATCH,
        Verb::Asserted,
    )
    .or_else(|| widest(crate::symbio_core::event::EVENT_USER_MESSAGE, Verb::Opened))
}

/// 一条声明事件的预算：数值优先，档位名回退。
fn declared_budget_of(e: &Event) -> Option<u64> {
    e.payload
        .get("budget_ms")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            e.payload
                .get("tier")
                .and_then(|v| v.as_str())
                .and_then(crate::symbio_core::adapters::LatencyTier::from_name)
                .map(|t| t.budget_ms())
        })
}

/// C4（I3 到点必答）：开过的 turn 必须收束。
///
/// 「收到」与「发出」是一对端点（[roadmap/S01 §1](../../../../docs/001-goal/121-最小闭环.md "GO-121-最小闭环")）：
/// 有 `user.message`（`turn × opened`）却始终等不到 `chat.assistant.final` /
/// `chat.assistant.fallback`（`turn × closed`），就是**对话静默中断**——超时后
/// 什么都没发生，没有任何错误信号的那类失效。兜底必须产生事件（I3）。
///
/// # 产物也算收束（[D-067](../../../../docs/003-design/011-Agent目录规范.md#d-067-产物收束的不变量面c4-认产物格closure_denied-数产物恢复锚刻意不认三处口径一次说清 "DE-011-Agent目录规范")）
///
/// `artifact.added`（`artifact × asserted`）同样关闭一轮：工具结果是**说给用户的
/// 话**（界面上就是一张卡片），不是内部噪声。没有它，只持 `produce.artifact` 的
/// 主体（对等主体，没有发言权）每跑一轮都永久留一条违规——「没有说话资格就不能
/// 留下任何事实」正是那次裁决拆掉的耦合。
///
/// 与**等待用户**那一态的关系：等待轮（进展格 `chat.assistant.progress`）的收束格
/// 刻意留给恢复幕，续写仍由 `turn_exec::last_open_turn` 按 final / fallback 找它
/// （那里**不**认产物——认了会让恢复幕找不到未收束轮）；本检查认产物只是
/// 「不再报它静默中断」，两处口径的差异是刻意的，见该函数文档。
///
/// # 宽限形参 `allow_trailing_open`
///
/// `false` = 严格：任何未收束的轮都算违规（故障注入用这一档，见
/// `fault_timeout_turn_never_settles_is_caught`）。
/// `true` = 放行**切片末尾**那一轮（[`check_all`] 的读侧口径）：它可能正在
/// 生成，切片又没有时间可比。放行的判据是「它就是最后开的那一轮」——
/// 一旦后面又开了新轮，旧轮的未收束就是确凿的静默中断，照报。
///
/// 违规锚在**开轮那条事件**上（`event_id` 指向缺口本身，不是切片末尾某条无关事件）。
pub fn unresolved_turns(events: &[Event], allow_trailing_open: bool) -> Vec<Violation> {
    // (开轮事件在切片中的位置, turn 号, 开轮事件)——位置用于判「尾轮」。
    let mut opened: Vec<(usize, u64, &Event)> = Vec::new();
    let mut last_open_pos: Option<usize> = None;
    for (i, e) in events.iter().enumerate() {
        if is_turn_open(e) {
            opened.push((i, e.turn, e));
            last_open_pos = Some(i);
        }
        // 收束 = 发言收束（final / fallback）或产物收束（D-067，落过工具结果）。
        // 判据与写侧闸门共用 [`closes_turn`]：两处各写一份必然漂移。
        if closes_turn(e) {
            opened.retain(|(_, turn, _)| *turn != e.turn);
        }
    }
    let mut bad = Vec::new();
    for (pos, turn, open_event) in opened {
        if allow_trailing_open && Some(pos) == last_open_pos {
            continue; // 宽限：尾轮在途（或崩溃残轮），切片内没有更晚的开轮可证它卡死
        }
        bad.push(Violation::at(
            open_event,
            format!(
                "turn {turn}（自 {} 开启）始终未收束——无 final、无 fallback、也无产物格，对话静默中断（I3 到点必答）",
                open_event.event_id
            ),
        ));
    }
    bad
}

/// C6（I3 到点必答）：每条 `turn × closed` 事实必带 `reason`，且取值在闭集内。
///
/// ## 它查的是什么
///
/// 「这一轮为什么停」必须是**可判的**。此前三个形状（`status` / `terminated.reason` /
/// `why`）问的是同一个问题，于是一个字段缺失或越界时不会有任何东西变红——日志能读、
/// 投影能算、回放不崩，只有"为什么停了"这个问题的答案悄悄不可用。而它正是重启后
/// 最需要回答的问题（DE-014 需要一个运行期 `halt(NoRule)` 分支；义务环里那个分支
/// 不存在，"停"只有"没有债了"一种表达，那么"为什么没有债了"必须写在事实里）。
///
/// ## 判定范围
///
/// `turn × closed` 的两种格（`chat.assistant.final` / `chat.assistant.fallback`）
/// 加上 `authz.denied`（拒绝不产生收束格，但它同样是一次"停"）。
/// 闭集由 [`ClosureReason::ALL`] 派生，**不在这里另写一份**——两处各写一份，
/// 加一个新原因时就会出现"写侧合法、读侧报红"的两套绿。
///
/// # 宽限：只宽限到「这条日志里第一条带 `reason` 的停机格」为止
///
/// 接线前落库的收束格没有 `reason`。把它们一并报出会让首日清单整片飘红，于是
/// 读侧只对**缺字段**放宽（见 [`closure_reason_closed`] 的形参）；**越界不放宽**——
/// 写了一个不在闭集里的值，从来不是历史问题。
///
/// 但「整段放宽」是条**没有边界**的宽限：写方从今天起每条都带 `reason`，而一条
/// 新落的格漏了 `reason`，在整段放宽下永远绿——判据 10 因此在生产里没有执行方
/// （口径 ③）。宽限的边界改成日志自己说：第一条带 `reason` 的停机格
/// （收束格或 `authz.denied`）就是分界，**它之前**是历史归档段（放宽），**它之后**
/// 每一条都必须带。这个位置不需要常量、不需要日期——写方一旦开始写 `reason`，
/// 之后的缺口就不可能再是历史造成的。整条日志一个 `reason` 都没有 ⇒ 分界落在末尾，
/// 与旧行为逐字等价（纯历史存档不飘红）。
pub fn closure_reason_closed(events: &[Event], allow_missing: bool) -> Vec<Violation> {
    // 分界下标：第一条带 `reason` 的停机格。严判档把分界放在开头 ⇒ 一段都不放宽；
    // 宽限档找不到分界 ⇒ 放在末尾，整段都算历史归档。
    let boundary = if allow_missing {
        events
            .iter()
            .position(|e| {
                is_stop_fact(e) && e.payload.get("reason").and_then(|v| v.as_str()).is_some()
            })
            .unwrap_or(events.len())
    } else {
        0
    };
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| is_stop_fact(e))
        .filter_map(|(idx, e)| {
            let raw = e.payload.get("reason").and_then(|v| v.as_str());
            match raw {
                None => {
                    if idx < boundary {
                        return None;
                    }
                    Some(Violation::at(
                        e,
                        format!(
                            "{} 缺 `reason`（这一轮为什么停必须写在事实里，I3 到点必答）",
                            e.kind
                        ),
                    ))
                }
                Some(r) => {
                    if crate::symbio_core::ClosureReason::parse(r).is_some() {
                        return None;
                    }
                    Some(Violation::at(
                        e,
                        format!(
                            "{} 的 `reason` = {r:?} 不在闭集内（合法取值：{}）",
                            e.kind,
                            crate::symbio_core::ClosureReason::ALL
                                .iter()
                                .map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(" / ")
                        ),
                    ))
                }
            }
        })
        .collect()
}

/// C5（I3 到点必答）：事件的 `cost_ms` 不得超过主体预算。
///
/// 超预算本身**允许发生**（S4 的兜底链路负责降级），但它必须被**看见**：
/// `budget_ms` 是 I3 的记账口径，超了却没人知道 = 声明式预算（S1 之前的形态）。
///
/// # 宽限形参 `budget_ms: Option<u64>`
///
/// `None` = **本次判定没有预算** ⇒ 不判（返回空）。没有预算就没有「超」——
/// 硬塞一个默认数会把正常流量整片报成违规，那才是首日假红。读侧由
/// [`declared_budget_ms`] 供数：开轮载荷带规格派生的 `budget_ms` 数值优先，
/// 缺失回退档位映射——两者都取不到才 `None`。
pub fn budget_exceeded(events: &[Event], budget_ms: Option<u64>) -> Vec<Violation> {
    let Some(budget_ms) = budget_ms else {
        return Vec::new();
    };
    events
        .iter()
        .filter(|e| e.cost_ms > budget_ms)
        .map(|e| {
            Violation::at(
                e,
                format!(
                    "cost_ms {} 超出主体预算 {budget_ms}（I3 到点必答）",
                    e.cost_ms
                ),
            )
        })
        .collect()
}

/// C14（终止性前提 3）：任务依赖图**无环且无悬空依赖**（Kahn 拓扑排序）。
///
/// 有环的依赖图违反时**没有任何信号**——可能只是永远跑不完（[roadmap/S03 §5](../../../../docs/001-goal/123-多步任务与返工.md "GO-123-多步任务与返工")），
/// 所以必须是 CI 断言。任务图从 `task.opened` 事件的载荷提取：
/// `{ task_id, depends_on }`——**图是数据，不是机制**。
/// 悬空依赖（依赖不存在的任务）同样判违规。反向用例见测试区（验收 1 / 4）。
///
/// 接进 [`check_all`]（[04 §3.1 批⑨](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") 的
/// 「断言进 CI」）：读出口 `session/stats` 的 `invariants` 列与 e2e 都跑它。
pub fn acyclic_deps(events: &[Event]) -> Vec<Violation> {
    // 解析层：从任务开轮格收出 (task_id, depends_on)；判定（悬空 + Kahn 环）
    // 走 kernel::dep_graph_core——verify/termination.rs 包含的是同一份内核。
    let mut pairs: Vec<(String, Vec<String>)> = Vec::new();
    for e in events {
        if e.entity != Entity::Task || e.verb != Verb::Opened {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let ds: Vec<String> = e
            .payload
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // 重复 task_id 后写覆盖先写（任务可重开，新开格是权威）——内核同语义。
        if let Some(slot) = pairs.iter_mut().find(|(k, _)| k == id) {
            slot.1 = ds;
        } else {
            pairs.push((id.to_string(), ds));
        }
    }
    let borrowed: Vec<(&str, Vec<&str>)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.iter().map(String::as_str).collect()))
        .collect();
    let (dangling, on_cycle) = kernel::dep_graph_core(&borrowed);
    let mut bad = Vec::new();
    for (id, d) in dangling {
        if let Some(e) = events.iter().find(|e| {
            e.entity == Entity::Task
                && e.verb == Verb::Opened
                && e.payload.get("task_id").and_then(|v| v.as_str()) == Some(id)
        }) {
            bad.push(Violation::at(
                e,
                format!("任务 {id} 悬空依赖 {d}（依赖不存在）"),
            ));
        }
    }
    for id in on_cycle {
        if let Some(e) = events.iter().find(|e| {
            e.entity == Entity::Task
                && e.verb == Verb::Opened
                && e.payload.get("task_id").and_then(|v| v.as_str()) == Some(id)
        }) {
            bad.push(Violation::at(
                e,
                format!("任务 {id} 处在依赖环上（C14：依赖图必须无环）"),
            ));
        }
    }
    bad
}

/// 终止性前提 2：每个任务的**返工次数有硬上界**。
///
/// 返工 = `task.rework_created` 事件（新增一条事实，不是修改历史）；同一被返工
/// 节点的返工轮数超过 `max_rework` ⇒ 违规——无上界的返工可能永不终止
/// （docs/003-design/verify/termination.rs 前提 2）。
/// 接进 [`check_all`]（[04 §3.1 批⑨](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
pub fn rework_bounded(events: &[Event], max_rework: u32) -> Vec<Violation> {
    // 解析层：收出按时间序的 replaces 序列；计数判定走 kernel::rework_core
    // ——verify/termination.rs 的前提 2 用例包含的是同一份内核。
    let replaces: Vec<&str> = events
        .iter()
        .filter(|e| {
            e.entity == Entity::Task
                && e.kind == crate::symbio_core::event::EVENT_TASK_REWORK_CREATED
        })
        .filter_map(|e| e.payload.get("replaces").and_then(|v| v.as_str()))
        .collect();
    kernel::rework_core(&replaces, max_rework)
        .into_iter()
        .map(|(target, round, bound)| {
            let e = events
                .iter()
                .find(|e| {
                    e.entity == Entity::Task
                        && e.kind == crate::symbio_core::event::EVENT_TASK_REWORK_CREATED
                        && e.payload.get("replaces").and_then(|v| v.as_str())
                            == Some(target.as_str())
                })
                .expect("越界轮必来自本切片");
            Violation::at(
                e,
                format!("任务 {target} 返工第 {round} 轮，超过上界 {bound}（终止性前提 2）"),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/invariants/kernel.rs::dep_graph_core: 穷尽性不变量：indeg 与 graph 同构造（同一批键），走到 expect 说明本文件的判据漏了一个分支，属代码缺陷。
// panic-allow symbio/src/symbio_core/invariants/mod.rs::seq_monotonic: 解析层不变量：内核返回的下标必落在「有 seq 的事件」范围内（内核只对这条序列工作），走到 expect 说明解析与内核的输入约定破了，属代码缺陷。
// panic-allow symbio/src/symbio_core/invariants/mod.rs::final_unique_per_turn: 同上：内核返回的重复项 id 必来自本切片（刚 map 出去的输入），走到 expect 说明约定破了。
// panic-allow symbio/src/symbio_core/invariants/mod.rs::open_unique_per_turn: 同上：重复项 id 必来自本切片。
// panic-allow symbio/src/symbio_core/invariants/mod.rs::rework_bounded: 同上：越界轮必来自本切片（内核只对刚收集的 replaces 计数）。
