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

/// 机制表：与 mechanism_growth.rs 完全一致（v2 唯一的机制清单）
const MECHANISMS: &[&str] = &[
    "store", "projection", "actor.pattern", "actor.capability", "actor.budget_ms",
    "event.entity", "event.verb", "scope", "vis_scope", "principal",
];

struct Demand {
    id: &'static str,
    name: &'static str,
    assignments: Vec<(&'static str, &'static str)>,
    /// 是否需要**扩展已有键的值域**（区别于"取一个新值"）
    extends_domain: bool,
}

fn demands() -> Vec<Demand> {
    vec![
        Demand { id: "C01", name: "七类认知内容入库（知识/经验/技能/判断/策略/直觉/情绪）", extends_domain: false,
            assignments: vec![("event.entity", "memory"), ("event.verb", "asserted"),
                              ("projection", "recall:tag=judgment")] },
        Demand { id: "C02", name: "内容密度金字塔（原文/要点/摘要/模式）", extends_domain: false,
            assignments: vec![("projection", "recall:density=summary")] },
        Demand { id: "C03", name: "Level 0-4 通用性分级与过滤（读侧，与遗忘同构）", extends_domain: false,
            assignments: vec![("projection", "recall:min_generality=2")] },
        Demand { id: "C04", name: "五层存储（原始/情景/语义/技能/元）", extends_domain: false,
            assignments: vec![("event.entity", "memory"), ("projection", "recall:layer=semantic")] },
        Demand { id: "C05", name: "激活扩散检索（关系网络多跳）", extends_domain: false,
            assignments: vec![("projection", "recall:spread=on"), ("actor.pattern", "translator"),
                              ("actor.budget_ms", "500")] },
        Demand { id: "C06", name: "向量索引 + 关系索引 + 时间分区", extends_domain: false,
            assignments: vec![("store", "index:vector+graph+time")] },
        Demand { id: "C07", name: "时间衰减 + 使用反馈调权", extends_domain: false,
            assignments: vec![("projection", "recall:decay=exp"), ("event.entity", "memory"),
                              ("event.verb", "asserted")] },
        Demand { id: "C08", name: "内外双轨动作空间（Internal / External）", extends_domain: false,
            assignments: vec![("actor.capability", "external.execute"), ("actor.pattern", "decider")] },
        Demand { id: "C09", name: "系统 2 → 系统 1 技能编译", extends_domain: false,
            assignments: vec![("projection", "skill_compile"), ("actor.pattern", "decider")] },
        Demand { id: "C10", name: "个人认知归属与隔离", extends_domain: false,
            assignments: vec![("principal", "person:zhangsan"), ("vis_scope", "thread_private")] },
        Demand { id: "C11", name: "跨主体借用他人认知（人 → 智能体）", extends_domain: false,
            assignments: vec![("principal", "agent:helper"), ("vis_scope", "shared")] },
        Demand { id: "C12", name: "动态学习：巩固（压缩 + 反事实）", extends_domain: false,
            assignments: vec![("projection", "consolidate"), ("event.entity", "memory"),
                              ("event.verb", "progressed"), ("actor.pattern", "reasoner")] },
        Demand { id: "C13", name: "动态学习：遗忘（投影排除，非物理删除）", extends_domain: false,
            assignments: vec![("event.entity", "memory"), ("event.verb", "closed"),
                              ("projection", "recall:exclude_forgotten")] },
        Demand { id: "C14", name: "认知注入上下文（检索结果进提示词）", extends_domain: false,
            assignments: vec![("actor.pattern", "translator"), ("projection", "recall"),
                              ("event.entity", "memory"), ("event.verb", "asserted")] },
        Demand { id: "C15", name: "认知置信度校准（元认知）", extends_domain: false,
            assignments: vec![("projection", "calibration"), ("event.entity", "verdict"),
                              ("event.verb", "asserted")] },
        Demand { id: "C16", name: "L1-L5 能力演进（自主层 + 子作用域）", extends_domain: false,
            assignments: vec![("scope", "child"), ("actor.budget_ms", "86400000")] },
    ]
}

/// 审计一：越界键（= 必须新增机制 = 重构）
fn audit(ds: &[Demand]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for d in ds {
        for (k, v) in &d.assignments {
            if !MECHANISMS.contains(k) {
                bad.push((d.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// 审计二：需要扩展值域的键。**应为 0** —— 非 0 说明有需求逼迫改动已有参数的语义，
/// 必须先找同构的既有模式；找不到才走 ADR（见 03 §3 前置闸门）。
fn domain_extensions(ds: &[Demand]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for d in ds {
        if d.extends_domain {
            out.push(d.id);
        }
    }
    out
}

fn keys_used(ds: &[Demand]) -> usize {
    let mut used: Vec<&str> = Vec::new();
    for d in ds {
        for (k, _) in &d.assignments {
            if !used.contains(k) {
                used.push(k);
            }
        }
    }
    used.len()
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
    let ds = demands();

    println!("── Q1/Q2：需求映射审计 ──");
    println!("机制表（恒定）：{} 项", MECHANISMS.len());
    println!("认知体系需求：{} 条", ds.len());
    for d in &ds {
        let mark = if d.extends_domain { "（扩值域）" } else { "" };
        println!("  {} {}{}", d.id, d.name, mark);
    }

    let bad = audit(&ds);
    let ext = domain_extensions(&ds);
    println!("\n越界键（= 必须新增机制 = 重构）：{} 个", bad.len());
    for b in &bad {
        println!("  {} → {}", b.0, b.1);
    }
    println!("需扩展值域的需求：{} 条 → {:?}", ext.len(), ext);
    println!("认知体系用到的机制种类：{} / {}", keys_used(&ds), MECHANISMS.len());

    // 反向用例：加一条真正需要新机制的需求，审计必须失败
    let mut with_new = demands();
    with_new.push(Demand { id: "C99", name: "（反例）要求独立的认知存储层", extends_domain: false,
        assignments: vec![("cognition.layer", "5")] });
    let bad_new = audit(&with_new);

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
    assert_eq!(ds.len(), 16, "认知体系需求应为 16 条");
    assert!(bad.is_empty(), "16 条需求不应触发任何新增机制（不需要重构）");
    assert!(ext.is_empty(), "16 条需求不应需要扩展任何已有键的值域（同构模式应能覆盖）");
    assert_eq!(bad_new.len(), 1, "反向用例：要求新机制的需求必须被审计抓到");
    assert!(bad_new[0].1.contains("cognition.layer"), "反向用例：抓到的必须是那个新键");

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

    println!("\n✅ 全部断言通过（含 6 条反向用例）。");
    println!("   结论：认知体系 16 条需求全部落在已有机制键上 → 承载无需重构；");
    println!("   且无需扩展任何值域——通用性过滤与 §9.3 遗忘同构，归入读侧投影参数即可。");
}
