//! 认知架构体系 → v2 骨架的承载压力测试
//!
//! 目的不是"把认知内容搬进架构"，而是回答三个问题：
//!   Q1 承载：认知体系的每条需求，能否表达为**已有机制键的新取值**？
//!   Q2 重构：有没有任何一条需求逼迫新增机制键（= 动原语 / 不变量 / 签名）？
//!   Q3 反噬：认知体系引入的后台巩固与反馈检索，会不会破坏 v2 已承诺的性质？
//!
//! 方法（沿用三条验证纪律）：
//!   · 需求 = 一组（机制键, 取值）赋值向量，审计"越界键"与"值域扩展"
//!   · 每条结论配反向用例：改输入，结论必须改变
//!   · 性质测试用真实的小日志计算，不打印常量冒充结论
//!
//! 编译运行：rustc --edition 2021 cognitive_fit.rs -o cf && ./cf

//! 认知架构体系 → v2 骨架的承载压力测试
//!
//! 目的不是"把认知内容搬进架构"，而是回答三个问题：
//!   Q1 承载：认知体系的每条需求，能否表达为**已有机制键**上的取值？（越界键应为 0）
//!   Q2 值域：那些取值是否落在 [01 §8](../01-核心架构.md) 声明的取值域内？（越界取值应为 0
//!      = 无需值域扩展。这一条此前由一个恒为 `false`、任何输入都翻不动的手填标志冒充，
//!      见 plan/13 §1 M9）
//!   Q3 反噬：认知体系引入的后台巩固与反馈检索，会不会破坏 v2 已承诺的性质？
//!
//! **本程序里没有手填的需求清单。** 16 条需求的 `(键, 取值)` 向量来自
//! [S10 §2](../roadmap/S10-个人认知体系注入.md) 那张映射表，经
//! `scripts/gen-verify-facts.mjs` 抽成 `facts::COGNITIVE_DEMANDS`；值域与匹配规则同源于
//! `facts::in_domain`。清单改了文档而程序照绿，就是这个批次要消灭的形态。
//!
//! 方法（沿用三条验证纪律）：
//!   · 需求 = 一组（机制键, 取值）赋值向量，审计"越界键"与"越界取值"
//!   · 每条结论配反向用例：改输入，结论必须改变
//!   · 性质测试用真实的小日志计算，不打印常量冒充结论
//!
//! 编译运行：rustc --edition 2021 cognitive_fit.rs -o cf && ./cf

mod facts;

use facts::{CognitiveDemand, COGNITIVE_DEMANDS, DOMAINS, MECHANISMS, domain_of, in_domain};

/// 审计一：越界键（= 必须新增机制 = 重构）
fn audit(ds: &[CognitiveDemand]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for d in ds {
        for (k, v) in d.assignments {
            if !MECHANISMS.contains(k) {
                bad.push((d.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// 审计二：越界取值（= 需要一次值域扩展）。没有值域行的键不在这里报——那正是 Q1 的
/// 越界键，同一处红不该有两种说法。**应为 0**：非 0 说明有需求逼迫改动已有参数的语义，
/// 必须先找同构的既有模式（[S10 §5.3](../roadmap/S10-个人认知体系注入.md) 是范例），
/// 找不到才走 [03 §3](../03-演进与验证.md) 的前置闸门。
fn out_of_domain(ds: &[CognitiveDemand]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for d in ds {
        for &(k, v) in d.assignments {
            if domain_of(k).is_some() && !in_domain(k, v) {
                bad.push((d.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

fn keys_used(ds: &[CognitiveDemand]) -> Vec<&'static str> {
    let mut used: Vec<&'static str> = Vec::new();
    for d in ds {
        for (k, _) in d.assignments {
            if !used.contains(k) {
                used.push(k);
            }
        }
    }
    used
}

/// 没被任何一条需求取用的机制键。S10 §1 那句「用满 10 个里的 9 个」由它算，不靠人记得。
fn keys_untouched(ds: &[CognitiveDemand]) -> Vec<&'static str> {
    MECHANISMS
        .iter()
        .filter(|k| !ds.iter().any(|d| d.assignments.iter().any(|(dk, _)| dk == *k)))
        .copied()
        .collect()
}

/// 落点覆盖的阶段——「认知体系是跨阶的」这句话的可执行形式
fn stages_covered(ds: &[CognitiveDemand]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for d in ds {
        if !v.contains(&d.stage) {
            v.push(d.stage);
        }
    }
    v.sort();
    v
}

/// 取过开放值域（§8 写 `*`，**不判取值**）的键：把"这一处没判"报出来，
/// 而不是让「0 越界取值」读起来像"全都判过"。
fn open_keys_used(ds: &[CognitiveDemand]) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for d in ds {
        for (k, _) in d.assignments {
            if DOMAINS.iter().any(|x| x.key == *k && x.open) && !v.contains(k) {
                v.push(k);
            }
        }
    }
    v.sort();
    v
}

// ─────────────────────────────────────────────────────────────
// 性质测试一：后台巩固不得污染 as-of 视图
// ─────────────────────────────────────────────────────────────

struct Ev { ts: u64, tag: &'static str }

/// 正确的 Recall 视图：只含 as_of 之前的事件，取最近 n 条
fn view_recent(log: &[Ev], as_of: u64, n: usize) -> Vec<&'static str> {
    log.iter().filter(|e| e.ts <= as_of).rev().take(n).map(|e| e.tag).collect()
}

/// 反向用例实现：忽略 as-of，直接取全表最近 n 条
fn view_recent_ignore_asof(log: &[Ev], _as_of: u64, n: usize) -> Vec<&'static str> {
    log.iter().rev().take(n).map(|e| e.tag).collect()
}

// ─────────────────────────────────────────────────────────────
// 性质测试二：扩散检索必须预算有界且可降级（I3 对 Recall 同样成立）
// ─────────────────────────────────────────────────────────────

struct Diffusion { nodes: usize, degraded: bool, used_ms: u64 }

/// 正确的实现：逐跳扩散，超预算就停在已扩散的部分并标 degraded
fn diffuse(seeds: usize, fanout: usize, budget_ms: u64, ms_per_hop: u64) -> Diffusion {
    let mut visited = seeds;
    let mut used = 0u64;
    let mut degraded = false;
    for _ in 0..8 {
        if used + ms_per_hop > budget_ms {
            degraded = true;
            break;
        }
        used += ms_per_hop;
        visited += visited * fanout;
        if visited > 100_000 { visited = 100_000; }
    }
    Diffusion { nodes: visited, degraded, used_ms: used }
}

/// 反向用例实现：超预算时返回空结果且不标 degraded（= 静默超时）
fn diffuse_silent_timeout(seeds: usize, fanout: usize, budget_ms: u64, ms_per_hop: u64) -> Diffusion {
    let d = diffuse(seeds, fanout, budget_ms, ms_per_hop);
    if d.degraded {
        Diffusion { nodes: 0, degraded: false, used_ms: d.used_ms }
    } else {
        d
    }
}

/// I3 违例判据：什么都没产出，且没标降级 —— 这就是静默超时
fn violates_i3(d: &Diffusion) -> bool {
    d.nodes == 0 && !d.degraded
}

// ─────────────────────────────────────────────────────────────
// 性质测试三：通用性过滤的可逆性（读侧过滤 vs 写侧过滤）
//
// 认知文档说"Level 0-1 无需存储"，直觉做法是在 append 时拒绝（写侧）。
// 但 v2 已有同构模式：§9.3 的遗忘 = 投影排除，不是物理删除。
// 本测试证明两者在"过滤效果"上等价，但只有读侧可逆。
// ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct Mem { gen: u8, text: &'static str }

/// 读侧过滤：日志完整保留，投影按阈值排除
fn recall_read_side(log: &[Mem], min_gen: u8) -> Vec<&'static str> {
    log.iter().filter(|m| m.gen >= min_gen).map(|m| m.text).collect()
}

/// 写侧过滤：入库时低于阈值的直接不落盘
fn ingest_write_side(all: &[Mem], threshold: u8) -> Vec<Mem> {
    all.iter().filter(|m| m.gen >= threshold).copied().collect()
}

fn recall_from(stored: &[Mem], min_gen: u8) -> Vec<&'static str> {
    stored.iter().filter(|m| m.gen >= min_gen).map(|m| m.text).collect()
}

fn main() {
    println!("═══ 认知架构体系 → v2 承载压力测试 ═══\n");
    let ds = COGNITIVE_DEMANDS;

    println!("── Q1 承载 / Q2 值域：需求映射审计 ──");
    println!("机制表（01 §8 生成物）：{} 项", MECHANISMS.len());
    println!("认知体系需求（S10 §2 生成物）：{} 条", ds.len());
    for d in ds {
        let n = d.assignments.len();
        let mark = if n == 0 { "（不占参数面）" } else { "" };
        println!("  {} {}{} → {} 项赋值，落在 {}", d.id, d.name, mark, n, d.stage);
    }

    let bad = audit(ds);
    let odv = out_of_domain(ds);
    let used = keys_used(ds);
    let untouched = keys_untouched(ds);
    let open_keys = open_keys_used(ds);
    println!("\n越界键（= 必须新增机制 = 重构）：{} 个", bad.len());
    for b in &bad {
        println!("  {} → {}", b.0, b.1);
    }
    println!("越界取值（= 需要一次值域扩展）：{} 个", odv.len());
    for o in &odv {
        println!("  {} → {}", o.0, o.1);
    }
    println!("用到的机制种类：{} / {}（未被取用：{:?}）", used.len(), MECHANISMS.len(), untouched);
    println!("落点覆盖的阶段：{:?}", stages_covered(ds));
    println!(
        "其中取过**开放值域**（§8 写 `*`，不判取值）的键：{:?}——这些赋值只判了键，没判值",
        open_keys
    );

    // 反向用例一：加一条真正需要新机制的需求，Q1 的审计必须失败
    let new_mech = [CognitiveDemand {
        id: "C99",
        name: "（反例）要求独立的认知存储层",
        assignments: &[("cognition.layer", "5")],
        stage: "S10",
    }];
    let with_new = ds.iter().copied().chain(new_mech).collect::<Vec<_>>();
    let bad_new = audit(&with_new);

    // 反向用例二：把一条需求的取值改成 §8 之外的值，Q2 必须失败；同一条注入里另带
    // 两个**合法**取值（通配段、子键参数）作放行对照——只报越界的判据若连合法值也报，
    // 它一样不值钱。
    let bad_value = [CognitiveDemand {
        id: "C98",
        name: "（反例）取值越出 §8",
        assignments: &[("store", "sqlite"), ("scope", "child:task-7"), ("projection", "recall:tag=judgment")],
        stage: "S10",
    }];
    let with_bad_value = ds.iter().copied().chain(bad_value).collect::<Vec<_>>();
    let odv_new = out_of_domain(&with_bad_value);

    // ─────────────────────────────────────────────────────────
    println!("\n── Q3.1：后台巩固是否污染在途视图 ──");
    let mut log = vec![
        Ev { ts: 1, tag: "raw-1" }, Ev { ts: 2, tag: "raw-2" },
        Ev { ts: 3, tag: "raw-3" }, Ev { ts: 4, tag: "raw-4" },
    ];
    let before = view_recent(&log, 4, 3);
    log.push(Ev { ts: 5, tag: "consolidated-a" });
    log.push(Ev { ts: 6, tag: "consolidated-b" });
    let after = view_recent(&log, 4, 3);
    let wrong = view_recent_ignore_asof(&log, 4, 3);
    let moved = view_recent(&log, 6, 3);
    println!("  as-of=4 巩固前：{:?}", before);
    println!("  as-of=4 巩固后：{:?}", after);
    println!("  （错实现，忽略 as-of）：{:?}", wrong);
    println!("  as-of=6：{:?}", moved);

    // ─────────────────────────────────────────────────────────
    println!("\n── Q3.2：扩散检索的预算有界性（I3）──");
    let small = diffuse(3, 2, 10_000, 40);
    let large = diffuse(3, 6, 100, 40);
    let silent = diffuse_silent_timeout(3, 6, 100, 40);
    let bigger_budget = diffuse(3, 6, 10_000, 40);
    println!("  预算充足：nodes={} degraded={} used_ms={}", small.nodes, small.degraded, small.used_ms);
    println!("  预算不足：nodes={} degraded={} used_ms={}", large.nodes, large.degraded, large.used_ms);
    println!("  （错实现，静默超时）：nodes={} degraded={}", silent.nodes, silent.degraded);
    println!("  放大预算后：degraded={}", bigger_budget.degraded);

    // ─────────────────────────────────────────────────────────
    println!("\n── Q3.3：通用性过滤的可逆性（读侧 vs 写侧）──");
    let all = [
        Mem { gen: 0, text: "L0-通用常识" }, Mem { gen: 1, text: "L1-通用术语" },
        Mem { gen: 2, text: "L2-个人经验" }, Mem { gen: 3, text: "L3-个人判断" },
    ];
    let read_2 = recall_read_side(&all, 2);
    let stored = ingest_write_side(&all, 2);
    let write_2 = recall_from(&stored, 2);
    let read_0 = recall_read_side(&all, 0);
    let write_0 = recall_from(&stored, 0);
    println!("  阈值=2        读侧 {:?}", read_2);
    println!("  阈值=2        写侧 {:?}", write_2);
    println!("  阈值放宽到 0  读侧 {:?}", read_0);
    println!("  阈值放宽到 0  写侧 {:?}", write_0);

    // ── 断言 ──
    assert_eq!(ds.len(), 16, "S10 §2 应列出 16 条认知体系需求（S10 §2 与 03 §7.1 的那两句话都数它）");
    assert!(bad.is_empty(), "16 条需求不应触发任何新增机制（不需要重构）");
    assert!(odv.is_empty(), "16 条需求的取值不应越出 01 §8 声明的取值域（越界即一次值域扩展）");
    assert_eq!(bad_new.len(), 1, "反向用例：要求新机制的需求必须被 Q1 的审计抓到");
    assert!(bad_new[0].1.contains("cognition.layer"), "反向用例：抓到的必须是那个新键");
    assert_eq!(odv_new.len(), 1, "反向用例：越界取值必须被 Q2 抓到，而两个合法取值（通配段、子键参数）必须放行");
    assert!(odv_new[0].1.contains("store = sqlite"), "反向用例：抓到的必须是那条越界取值");
    assert_eq!(
        untouched, ["store"],
        "S10 §1 声明「用满 10 个里的 9 个」：没被取用的应当只有 `store`（索引是 Store 的实现细节，见 §2 的 C06）"
    );
    assert!(stages_covered(ds).contains(&"S10"), "落点应覆盖本阶——通用性分级与内容密度就住在 S10");

    assert_eq!(before, after, "后台追加巩固事件后，as-of 视图必须逐字节不变");
    assert_ne!(before, wrong, "反向用例：忽略 as-of 的投影必须被这条断言抓到");
    assert_ne!(before, moved, "反向用例：as-of 前移后视图必须改变（否则 as-of 是死参数）");

    assert!(!small.degraded, "预算充足时不应降级");
    assert!(large.degraded, "预算不足时必须降级");
    assert!(large.nodes > 0, "降级必须带部分结果，不能返回空");
    assert!(!violates_i3(&large), "降级返回不违反 I3");
    assert!(violates_i3(&silent), "反向用例：返回空且不标降级 = 静默超时，必须被抓到");
    assert!(!bigger_budget.degraded, "反向用例：预算放大后降级标志必须翻转");

    assert_eq!(read_2, write_2, "同一阈值下，读侧过滤与写侧过滤的召回结果必须等价");
    assert_eq!(read_0.len(), 4, "读侧过滤放宽阈值后应能召回全部内容");
    assert_eq!(write_0.len(), 2, "写侧过滤放宽阈值也无法召回——内容已永久丢失");
    assert!(read_0.len() > write_0.len(), "读侧过滤可逆：放宽阈值必须能召回此前被排除的内容");
    assert_ne!(read_2.len(), read_0.len(), "反向用例：阈值改变后召回数必须改变（否则 min_generality 是死参数）");

    println!("\n✅ 全部断言通过（含 7 条反向用例）。");
    println!(
        "   结论：{} 条需求全部落在已有机制键上、且取值全部落在 01 §8 声明的值域内 → 承载无需重构、无需值域扩展；",
        ds.len()
    );
    println!("   {} 项不占参数面（索引是 Store 的实现细节），其余落在 {} / {} 个机制键上；", ds.iter().filter(|d| d.assignments.is_empty()).count(), used.len(), MECHANISMS.len());
    println!("   通用性过滤与 §9.3 遗忘同构 → 读侧投影参数，不是写侧机制（S10 §5）。");
}
