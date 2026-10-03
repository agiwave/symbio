//! 可执行不变量检查（v2 F4，[plan/01 §5](../../../../docs/plan/01-核心架构.md)、[plan/04 §4](../../../../docs/plan/04-工程落地.md)）。
//!
//! ## 形态：纯函数，不是 trait
//!
//! 每条检查 = 「事件切片 ⇒ 违规清单」。输入是数据，输出是数据——它可以被
//! 单测双跑、可以挂 CI、可以离线跑在导出的事件序列上，而不需要任何运行时。
//!
//! ## 覆盖的 CI 断言（S0 步骤 2 出口判据 + S1 步骤 4 故障注入，[roadmap/S01 §5](../../../../docs/plan/roadmap/S01-最小闭环.md)）
//!
//! | 断言 | 不变量 | 检查 | 函数 |
//! |---|---|---|---|
//! | C1 | I1 单通道 | `seq` 严格单调、无跳号 | [`seq_monotonic`] |
//! | C2 / N3 | I1 单通道 | 每 `turn` 至多 1 条 final | [`final_unique_per_turn`] |
//! | C3 / N5 | I2 无溯源不声明 | 断言类 + 收束类事件 `produced_by` 非空 | [`produced_by_coverage`] |
//! | C4 | I3 到点必答 | 开过的 turn 必须收束（final 或 fallback） | [`unresolved_turns`] |
//! | C5 | I3 到点必答 | 事件 `cost_ms` 不得超主体预算 | [`budget_exceeded`] |
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

/// C1（I1）：`seq` 严格单调且**无跳号**。
///
/// 已入库事件按存储顺序应得到 `0,1,2,…`——跳号意味着「有事件被静默丢弃或绕过
/// 唯一写入口」。`pending`（未入库）事件跳过不查（它们还没有 seq）。
pub fn seq_monotonic(events: &[Event]) -> Vec<Violation> {
    let mut bad = Vec::new();
    let mut expect = 0u64;
    for e in events {
        // pending（未入库）没有 seq，跳过——它们还不是事实。
        if let Some(s) = e.seq {
            if s.value() != expect {
                bad.push(Violation::at(
                    e,
                    format!("seq {} 处应为 {expect}（严格单调且无跳号）", s.value()),
                ));
                expect = s.value() + 1;
            } else {
                expect += 1;
            }
        }
    }
    bad
}

/// C2 / N3（I1）：每 `turn` 至多 1 条 final。
///
/// 「final」= `turn × closed` 格子上的 `chat.assistant.final` 名字（[plan/01 §6](../../../../docs/plan/01-核心架构.md)）。
/// 一轮对用户说两次「最终答复」，转写的收敛性就没了——这是 I1 在发言路径上的形状。
pub fn final_unique_per_turn(events: &[Event]) -> Vec<Violation> {
    let mut seen: Vec<(u64, &str)> = Vec::new();
    let mut bad = Vec::new();
    for e in events {
        let is_final =
            e.entity == Entity::Turn && e.verb == Verb::Closed && e.kind == "chat.assistant.final";
        if !is_final {
            continue;
        }
        if let Some((_, first_id)) = seen.iter().find(|(turn, _)| *turn == e.turn) {
            bad.push(Violation::at(
                e,
                format!(
                    "turn {} 已有 final（{}），又出现一条——每 turn 至多 1 条 final",
                    e.turn, first_id
                ),
            ));
        } else {
            seen.push((e.turn, e.event_id.as_str()));
        }
    }
    bad
}

/// C3 / N5（I2）：**声明类**事件必须带溯源。
///
/// 「声明」= 宣称某件事为真或已发生，三类（[plan/03 §2 I2](../../../../docs/plan/03-演进与验证.md)）：
/// - **断言类**（`verb == Asserted`）：`task.verified` / `artifact.added` / `classify.verdict`…
/// - **收束类**（`turn × closed` 的 `chat.assistant.final` / `chat.assistant.fallback`）：
///   宣称"已经答复"（[roadmap/S01 §5](../../../../docs/plan/roadmap/S01-最小闭环.md)）；
/// - **记忆类**（`entity == memory`，S5）：宣称"学到了什么"（[roadmap/S06](../../../../docs/plan/roadmap/S06-长期记忆与语义检索.md)
///   步骤 11 出口判据：溯源覆盖 100%——无溯源的记忆无法审计，也无法在遗忘时被追溯）。
///
/// 没有溯源的声明**无法审计也无法撤销**——静默失效的温床（J3）。
pub fn produced_by_coverage(events: &[Event]) -> Vec<Violation> {
    events
        .iter()
        .filter(|e| needs_provenance(e) && e.produced_by.is_none())
        .map(|e| {
            let why = if e.verb == Verb::Asserted {
                "断言类事件必须带 produced_by（I2 无溯源不声明）"
            } else if e.entity == Entity::Memory {
                "记忆事件必须带 produced_by（I2：溯源覆盖 100%）"
            } else {
                "收束类事件（答复）必须带 produced_by（I2：用户收到的话必须可追溯）"
            };
            Violation::at(e, why)
        })
        .collect()
}

/// I2 的覆盖判据（参照 [`docs/plan/verify/invariants.rs`](../../../../docs/plan/verify/invariants.rs)
/// 的 `needs_provenance`；S1 扩展收束类，S5 扩展记忆类）。
fn needs_provenance(e: &Event) -> bool {
    e.verb == Verb::Asserted
        || e.entity == Entity::Memory
        || (e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                    | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
            ))
}

/// 五条一起跑（S0/S1 的 CI 形态：任一违规 ⇒ 清单非空）。
///
/// 前三条是结构判定（C1 `seq` 单调 / C2 每轮一条 final / C3 断言带溯源）；
/// 后两条（C4 未收束 / C5 超预算）按**读侧口径**带宽限——本函数是
/// `session/stats` 读出口与 e2e 的入口，首日就跑真实流量，假红一次
/// 那份清单就再没人看：
///
/// - C4 放行**切片末尾**仍在途的一轮：读取瞬间它可能正在生成，而切片没有
///   wall-clock（`Event::pending` 的 `ts` 恒 0，写方从不填），「在途」与「卡死」
///   在纯函数里无从分辨。被**后续轮越过**的未收束轮照样报——那才确凿：
///   会话已经往前走了，旧轮却没收束。
/// - C5 只在切片**声明过档位**时判，预算取声明档位里**最宽**的一档（判定方向
///   宁可漏报、不可假红：拿最严档比，会把别的档位的正常流量整片报成违规）。
///
/// 要严判就绕过本函数直接调 [`unresolved_turns`]（`false`）/ [`budget_exceeded`]。
pub fn check_all(events: &[Event]) -> Vec<Violation> {
    let mut all = seq_monotonic(events);
    all.extend(final_unique_per_turn(events));
    all.extend(produced_by_coverage(events));
    all.extend(unresolved_turns(events, true));
    all.extend(budget_exceeded(events, declared_budget_ms(events)));
    all
}

/// 本切片**声明过**的档位里最宽的一档预算；一档都没声明 ⇒ `None`（C5 跳过）。
///
/// 档位只从开轮事件（`user.message`）的载荷读——与 `projection::fallback` 的
/// `declared_tier` 同一个字段、同一个 `LatencyTier::from_name`，只是问的问题
/// 不同：那边是「这一轮归哪一档」（按轮归位），这边是「这份切片最宽的预算
/// 是多少」（一个全局上界，因此不会误伤任何一档）。
fn declared_budget_ms(events: &[Event]) -> Option<u64> {
    events
        .iter()
        .filter(|e| {
            e.entity == Entity::Turn
                && e.verb == Verb::Opened
                && e.kind == crate::symbio_core::event::EVENT_USER_MESSAGE
        })
        .filter_map(|e| e.payload.get("tier").and_then(|v| v.as_str()))
        .filter_map(crate::symbio_core::adapters::LatencyTier::from_name)
        .map(|t| t.budget_ms())
        .max()
}

/// C4（I3 到点必答）：开过的 turn 必须收束。
///
/// 「收到」与「发出」是一对端点（[roadmap/S01 §1](../../../../docs/plan/roadmap/S01-最小闭环.md)）：
/// 有 `user.message`（`turn × opened`）却始终等不到 `chat.assistant.final` /
/// `chat.assistant.fallback`（`turn × closed`），就是**对话静默中断**——超时后
/// 什么都没发生，没有任何错误信号的那类失效。兜底必须产生事件（I3）。
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
        if e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == "user.message" {
            opened.push((i, e.turn, e));
            last_open_pos = Some(i);
        }
        if e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                crate::symbio_core::event::EVENT_ASSISTANT_FINAL
                    | crate::symbio_core::event::EVENT_ASSISTANT_FALLBACK
            )
        {
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
                "turn {turn}（自 {} 开启）始终未收束——无 final 也无 fallback，对话静默中断（I3 到点必答）",
                open_event.event_id
            ),
        ));
    }
    bad
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
/// [`declared_budget_ms`] 供数：切片声明过档位才 `Some`。
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
/// 有环的依赖图违反时**没有任何信号**——可能只是永远跑不完（[roadmap/S03 §5](../../../../docs/plan/roadmap/S03-多步任务与返工.md)），
/// 所以必须是 CI 断言。任务图从 `task.opened` 事件的载荷提取：
/// `{ task_id, depends_on }`——**图是数据，不是机制**。
/// 悬空依赖（依赖不存在的任务）同样判违规。反向用例见测试区（验收 1 / 4）。
#[allow(dead_code)] // dead-code-allow R-002: 不变量可执行名（README §1.2 invariants 行）；04 §3.1 批⑨ 接线后摘除
pub fn acyclic_deps(events: &[Event]) -> Vec<Violation> {
    use std::collections::{BTreeMap, BTreeSet};
    let mut deps: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut opened_at: BTreeMap<&str, &Event> = BTreeMap::new();
    for e in events {
        if e.entity != Entity::Task || e.verb != Verb::Opened {
            continue;
        }
        let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let ds: BTreeSet<&str> = e
            .payload
            .get("depends_on")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        opened_at.insert(id, e);
        deps.insert(id, ds);
    }
    let ids: BTreeSet<&str> = deps.keys().copied().collect();
    let mut bad = Vec::new();
    // 悬空依赖：依赖不存在的任务。
    for (id, ds) in &deps {
        for d in ds {
            if !ids.contains(d) {
                if let Some(e) = opened_at.get(id) {
                    bad.push(Violation::at(
                        e,
                        format!("任务 {id} 悬空依赖 {d}（依赖不存在）"),
                    ));
                }
            }
        }
    }
    // Kahn：能剥完 = 无环；剥不完 = 剩下的都在环上。
    let mut indeg: BTreeMap<&str, usize> = deps
        .iter()
        .map(|(k, v)| (*k, v.iter().filter(|d| ids.contains(**d)).count()))
        .collect();
    let mut ready: Vec<&str> = indeg
        .iter()
        .filter(|(_, v)| **v == 0)
        .map(|(k, _)| *k)
        .collect();
    let mut done = 0usize;
    while let Some(id) = ready.pop() {
        done += 1;
        for (t, ds) in &deps {
            if ds.contains(&id) {
                let e = indeg.get_mut(t).expect("图节点必在");
                *e -= 1;
                if *e == 0 {
                    ready.push(t);
                }
            }
        }
    }
    if done < deps.len() {
        for (id, deg) in &indeg {
            if *deg > 0 {
                if let Some(e) = opened_at.get(id) {
                    bad.push(Violation::at(
                        e,
                        format!("任务 {id} 处在依赖环上（C14：依赖图必须无环）"),
                    ));
                }
            }
        }
    }
    bad
}

/// 终止性前提 2：每个任务的**返工次数有硬上界**。
///
/// 返工 = `task.rework_created` 事件（新增一条事实，不是修改历史）；同一被返工
/// 节点的返工轮数超过 `max_rework` ⇒ 违规——无上界的返工可能永不终止
/// （[docs/plan/verify/termination.rs](../../../../docs/plan/verify/termination.rs) 前提 2）。
#[allow(dead_code)] // dead-code-allow R-002: 不变量可执行名（README §1.2 invariants 行）；04 §3.1 批⑨ 接线后摘除
pub fn rework_bounded(events: &[Event], max_rework: u32) -> Vec<Violation> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let mut bad = Vec::new();
    for e in events {
        if e.entity != Entity::Task
            || e.kind != crate::symbio_core::event::EVENT_TASK_REWORK_CREATED
        {
            continue;
        }
        let Some(replaces) = e.payload.get("replaces").and_then(|v| v.as_str()) else {
            continue;
        };
        let c = counts.entry(replaces.to_string()).or_insert(0);
        *c += 1;
        if *c > max_rework {
            bad.push(Violation::at(
                e,
                format!("任务 {replaces} 返工第 {c} 轮，超过上界 {max_rework}（终止性前提 2）"),
            ));
        }
    }
    bad
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
