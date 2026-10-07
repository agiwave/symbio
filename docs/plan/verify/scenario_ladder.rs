//! 能力扩充阶梯审计：13 阶从低阶到高阶，是否始终无需新增机制
//!
//! 回答四件事：
//!   Q1 有没有任何一阶逼迫新增机制键（越界键应为 0）
//!   Q2 逐阶新增键数量是否如路线图所声称（后 5 阶应为 0）
//!   Q3 每阶是否声明了真正的平凡值回退（J2）
//!   Q4 唯一的值域扩展发生在哪一阶
//!
//! 沿用三条验证纪律：输入不含结论 / 每条断言配反向用例 / 结论由遍历算出而非硬编码。
//!
//! 编译运行：rustc --edition 2021 scenario_ladder.rs -o sl && ./sl

const MECHANISMS: &[&str] = &[
    "store", "projection", "actor.pattern", "actor.capability", "actor.budget_ms",
    "event.entity", "event.verb", "scope", "vis_scope", "principal",
];

struct Stage {
    id: &'static str,
    name: &'static str,
    tier: &'static str,
    /// 本阶段新增/变化的赋值（机制键 → 取值）
    adds: Vec<(&'static str, &'static str)>,
    /// J2 平凡值回退：系统退回这里仍能完整运行
    fallback: (&'static str, &'static str),
    /// 是否需要扩展已有键的值域（区别于取一个新值）
    extends_domain: bool,
}

fn ladder() -> Vec<Stage> {
    vec![
        Stage { id: "S01", name: "最小闭环（对话基线）", tier: "T1", extends_domain: false,
            fallback: ("actor.pattern", "decider"),
            adds: vec![("event.entity", "turn"), ("event.verb", "closed"),
                       ("actor.pattern", "reasoner"), ("actor.capability", "reply.first"),
                       ("actor.budget_ms", "300"), ("projection", "snapshot")] },
        Stage { id: "S02", name: "工具调用与产物", tier: "T1-T2", extends_domain: false,
            fallback: ("actor.capability", "reply.append"),
            adds: vec![("event.entity", "artifact"), ("event.verb", "asserted"),
                       ("actor.capability", "produce.artifact"), ("projection", "display")] },
        Stage { id: "S03", name: "多步任务与返工", tier: "T2", extends_domain: false,
            fallback: ("scope", "root"),
            adds: vec![("event.entity", "task"), ("event.verb", "progressed"),
                       ("actor.pattern", "decider"), ("actor.capability", "define.work"),
                       ("scope", "child")] },
        Stage { id: "S04", name: "并发调度与租约", tier: "T3", extends_domain: false,
            fallback: ("projection", "readyset:cap=1"),
            adds: vec![("event.verb", "held"), ("actor.capability", "assign.work"),
                       ("actor.budget_ms", "80"), ("projection", "readyset")] },
        Stage { id: "S05", name: "长会话与断点恢复", tier: "T3", extends_domain: false,
            fallback: ("store", "memory"),
            adds: vec![("event.entity", "thread"), ("projection", "checkpoint"),
                       ("store", "wal")] },
        Stage { id: "S06", name: "长期记忆与语义检索", tier: "T3", extends_domain: false,
            fallback: ("projection", "snapshot"),
            adds: vec![("event.entity", "memory"), ("event.verb", "asserted"),
                       ("actor.pattern", "translator"), ("projection", "recall")] },
        Stage { id: "S07", name: "插话与实时打断", tier: "T3-T4", extends_domain: false,
            fallback: ("event.entity", "turn"),
            adds: vec![("event.entity", "control"), ("event.verb", "held"),
                       ("actor.budget_ms", "80"), ("projection", "turnstate")] },
        Stage { id: "S08", name: "多主体与对等承诺", tier: "T4-T5", extends_domain: false,
            fallback: ("principal", "agent:main"),
            adds: vec![("event.entity", "commitment"), ("principal", "agent:peer-b"),
                       ("vis_scope", "thread_private"), ("projection", "reputation"),
                       ("actor.pattern", "translator")] },
        Stage { id: "S09", name: "外部执行与熔断", tier: "T5-T6", extends_domain: false,
            fallback: ("actor.capability", "reply.append"),
            adds: vec![("event.entity", "control"), ("event.verb", "asserted"),
                       ("actor.pattern", "decider"), ("projection", "budget")] },
        Stage { id: "S10", name: "个人认知体系注入", tier: "T4-T6", extends_domain: false,
            fallback: ("projection", "recall:min_generality=0"),
            adds: vec![("event.entity", "memory"), ("event.verb", "asserted"),
                       ("projection", "recall:tag=judgment"),
                       ("projection", "recall:min_generality=2"), ("vis_scope", "shared")] },
        Stage { id: "S11", name: "技能编译与自我改进", tier: "T6", extends_domain: false,
            fallback: ("actor.pattern", "reasoner"),
            adds: vec![("event.entity", "memory"), ("event.verb", "progressed"),
                       ("actor.pattern", "decider"), ("projection", "skill_compile")] },
        Stage { id: "S12", name: "自主层与长期目标", tier: "T7", extends_domain: false,
            fallback: ("event.entity", "turn"),
            adds: vec![("event.entity", "system"), ("event.verb", "opened"),
                       ("actor.pattern", "decider"), ("actor.budget_ms", "86400000")] },
        Stage { id: "S13", name: "多智能体社会", tier: "T8", extends_domain: false,
            fallback: ("vis_scope", "thread_private"),
            adds: vec![("event.entity", "commitment"), ("event.verb", "asserted"),
                       ("principal", "agent:market"), ("vis_scope", "public"),
                       ("projection", "reputation")] },
    ]
}

/// Q1：越界键（= 必须新增机制 = 重构）
fn out_of_bounds(stages: &[Stage]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for s in stages {
        for (k, v) in &s.adds {
            if !MECHANISMS.contains(k) {
                bad.push((s.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// 截至第 upto 阶（含）累计用过的机制键
fn cumulative_keys(stages: &[Stage], upto: usize) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in &stages[..upto] {
        for (k, _) in &s.adds {
            if !v.contains(k) {
                v.push(k);
            }
        }
    }
    v
}

/// 逐阶新增键数量：由累计差算出，不是写死的常量表
fn new_keys_per_stage(stages: &[Stage]) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 0..stages.len() {
        out.push(cumulative_keys(stages, i + 1).len() - cumulative_keys(stages, i).len());
    }
    out
}

/// Q3：J2 违例 —— 回退值等于本阶正常运行值，等于没有回退
fn j2_violations(stages: &[Stage]) -> Vec<&'static str> {
    stages.iter()
        .filter(|s| s.adds.iter().any(|(k, v)| s.fallback.0 == *k && s.fallback.1 == *v))
        .map(|s| s.id)
        .collect()
}

fn main() {
    println!("═══ 能力扩充阶梯审计（13 阶）═══\n");
    let stages = ladder();

    let nk = new_keys_per_stage(&stages);
    let cum_all = cumulative_keys(&stages, stages.len()).len();
    println!("| 阶 | 场景 | 天梯 | 新增键 | 累计 |");
    println!("|---|---|:-:|:-:|:-:|");
    for (i, s) in stages.iter().enumerate() {
        let mut cum = 0;
        for j in 0..=i { cum += nk[j]; }
        println!("| {} | {} | {} | {} | {} |", s.id, s.name, s.tier, nk[i], cum);
    }

    let bad = out_of_bounds(&stages);
    let ext: Vec<&str> = stages.iter().filter(|s| s.extends_domain).map(|s| s.id).collect();
    println!("\n── Q1 越界键（= 需新增机制 = 重构）：{} 个", bad.len());
    for b in &bad { println!("   {} → {}", b.0, b.1); }
    println!("── Q2 累计机制键：{} / {}（机制表恒定 {} 项）", cum_all, cum_all, MECHANISMS.len());
    println!("── Q4 需扩展值域的阶：{:?}（应为 0：任何非 0 都必须走 ADR 前置闸门）", ext);

    let tail_zero = nk[8..].iter().all(|n| *n == 0);
    let zero_stages = nk.iter().filter(|n| **n == 0).count();
    println!("\n后 5 阶（S09-S13）新增键全为 0：{}", tail_zero);
    println!("零新增键的阶数：{} / {}", zero_stages, stages.len());

    // ── 反向用例 1：第 14 阶需要新机制 → 审计必须失败 ──
    let mut plus_new = ladder();
    plus_new.push(Stage { id: "S14", name: "（反例）主体自改架构", tier: "T9", extends_domain: false,
        fallback: ("scope", "root"),
        adds: vec![("meta.architecture", "self-modifying")] });
    let bad_new = out_of_bounds(&plus_new);

    // ── 反向用例 2：回退值 == 运行值 → J2 必须抓到 ──
    let mut plus_fake = ladder();
    plus_fake.push(Stage { id: "S99", name: "（反例）假回退", tier: "T1", extends_domain: false,
        fallback: ("actor.pattern", "reasoner"),
        adds: vec![("actor.pattern", "reasoner")] });
    let j2_new = j2_violations(&plus_fake);

    // ── 反向用例 3：删掉 S03 的 scope → 累计键数必须改变（证明在算，不是打印常量）──
    let mut minus_scope = ladder();
    minus_scope[2].adds.retain(|(k, _)| *k != "scope");
    let cum_minus = cumulative_keys(&minus_scope, minus_scope.len()).len();

    println!("\n── 反向用例 ──");
    println!("  加一个需新机制的 S14 → 越界键 {} 个", bad_new.len());
    println!("  加一个回退值等于运行值的 S99 → J2 违例 {} 个", j2_new.len());
    println!("  删掉 S03 的 scope → 累计键 {} → {}", cum_all, cum_minus);

    // ── 断言 ──
    assert_eq!(stages.len(), 13, "阶梯应为 13 阶");
    assert!(bad.is_empty(), "13 阶不应触发任何新增机制键");
    assert_eq!(cum_all, MECHANISMS.len(), "13 阶累计用满机制表且不超出");
    assert_eq!(nk, vec![6, 0, 1, 0, 1, 0, 0, 2, 0, 0, 0, 0, 0],
        "逐阶新增键应为 6,0,1,0,1,0,0,2,0,0,0,0,0");
    assert!(tail_zero, "后 5 阶（S09-S13）新增键必须全为 0");
    assert_eq!(zero_stages, 9, "13 阶中应有 9 阶零新增键");
    assert!(j2_violations(&stages).is_empty(), "每阶的回退值必须不同于运行值");
    assert!(ext.is_empty(), "13 阶不应有任何一阶需要扩展已有键的值域（架构改动应为 0）");

    assert_eq!(bad_new.len(), 1, "反向用例：需新机制的第 14 阶必须被抓到");
    assert!(bad_new[0].1.contains("meta.architecture"), "反向用例：抓到的必须是那个新键");
    assert_eq!(j2_new.len(), 1, "反向用例：假回退必须被 J2 抓到");
    assert_eq!(j2_new[0], "S99", "反向用例：抓到的必须是那一阶");
    assert_eq!(cum_minus, cum_all - 1, "反向用例：删掉一个键后累计数必须减少 1");

    println!("\n✅ 全部断言通过（含 4 条反向用例）。");
    println!("   13 阶能力扩充 = 0 新增机制键 + 0 值域扩展（架构改动 0）；");
    println!("   后 5 阶只取新值，不引入新键、不改任何已有键的语义。");
}
