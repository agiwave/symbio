//! 元素数守恒律：机制数不随场景数增长
//!
//! 这是"扩充不会重构"唯一可自动审计的形式：
//!   · 场景 = 一组（机制键, 取值）赋值向量
//!   · 合法性 = 场景用到的键 ⊆ 机制表
//!   · 守恒 = 机制表长度不随场景数变化
//!
//! 关键：这里**遍历场景计算键集合**，不是把“是否会增长”预先硬编码成 false 再断言 false。
//! 反向用例：加入一个需要新机制的场景，审计必须失败。
//!
//! 编译运行：rustc --edition 2021 mechanism_growth.rs -o mg && ./mg

/// 机制表：**这是唯一的机制清单**。新增能力只允许在已有键上取新值。
const MECHANISMS: &[&str] = &[
    "store",           // 事实源（唯一原语）
    "projection",      // 纯函数派生的视图名与参数
    "actor.pattern",   // 三种模式：decider / reasoner / translator
    "actor.capability",// 能力（7 个封顶）
    "actor.budget_ms", // I3 时延预算
    "event.entity",    // 事件语法网格：实体
    "event.verb",      // 事件语法网格：动词
    "scope",           // 递归：子作用域
    "vis_scope",       // 可见域
    "principal",       // 身份（数据，可无限增长）
];

struct Scenario {
    name: &'static str,
    assignments: Vec<(&'static str, &'static str)>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        Scenario { name: "S1 直接回复", assignments: vec![
            ("event.entity", "turn"), ("event.verb", "closed"),
            ("actor.pattern", "reasoner"), ("actor.capability", "reply.first"), ("actor.budget_ms", "300")] },
        Scenario { name: "S2 单步工具", assignments: vec![
            ("event.entity", "task"), ("event.verb", "progressed"),
            ("actor.pattern", "reasoner"), ("actor.capability", "produce.artifact"), ("actor.budget_ms", "30000")] },
        Scenario { name: "S3 多步任务", assignments: vec![
            ("event.entity", "task"), ("event.verb", "opened"),
            ("actor.pattern", "reasoner"), ("actor.capability", "define.work"), ("actor.budget_ms", "60000")] },
        Scenario { name: "S4 并发与租约", assignments: vec![
            ("event.entity", "task"), ("event.verb", "progressed"),
            ("actor.pattern", "decider"), ("actor.capability", "assign.work"), ("actor.budget_ms", "10")] },
        Scenario { name: "S5 独立验证返工", assignments: vec![
            ("event.entity", "task"), ("event.verb", "asserted"),
            ("actor.pattern", "decider"), ("actor.capability", "assert.verification"), ("actor.budget_ms", "5000")] },
        Scenario { name: "S6 定时触发", assignments: vec![
            ("event.entity", "system"), ("event.verb", "opened"),
            ("actor.pattern", "decider"), ("actor.capability", "assign.work"), ("actor.budget_ms", "10")] },
        Scenario { name: "S7 跨 turn 存续", assignments: vec![
            ("event.entity", "task"), ("event.verb", "held"),
            ("actor.pattern", "reasoner"), ("actor.capability", "define.work"), ("actor.budget_ms", "60000")] },
        Scenario { name: "S8 多主体隔离", assignments: vec![
            ("event.entity", "task"), ("event.verb", "progressed"),
            ("actor.pattern", "decider"), ("actor.capability", "assign.work"),
            ("vis_scope", "thread_private"), ("principal", "agent:researcher-01")] },
        Scenario { name: "S9 长期记忆检索", assignments: vec![
            ("event.entity", "memory"), ("event.verb", "asserted"),
            ("actor.pattern", "translator"), ("actor.capability", "produce.artifact"),
            ("projection", "recall"), ("actor.budget_ms", "500")] },
        Scenario { name: "S10 记忆巩固", assignments: vec![
            ("event.entity", "memory"), ("event.verb", "progressed"),
            ("actor.pattern", "reasoner"), ("actor.capability", "define.work"), ("projection", "consolidate")] },
        Scenario { name: "S11 反射级应答", assignments: vec![
            ("event.entity", "turn"), ("event.verb", "closed"),
            ("actor.pattern", "decider"), ("actor.capability", "reply.first"), ("actor.budget_ms", "80")] },
        Scenario { name: "S12 对等承诺", assignments: vec![
            ("event.entity", "commitment"), ("event.verb", "asserted"),
            ("actor.pattern", "translator"), ("actor.capability", "produce.artifact"), ("principal", "agent:peer-b")] },
        Scenario { name: "S13 声誉与信任", assignments: vec![
            ("event.entity", "commitment"), ("event.verb", "asserted"),
            ("actor.pattern", "translator"), ("projection", "reputation")] },
        Scenario { name: "S14 子智能体递归", assignments: vec![
            ("event.entity", "task"), ("event.verb", "opened"),
            ("actor.pattern", "reasoner"), ("scope", "child"), ("actor.capability", "define.work")] },
        Scenario { name: "S15 多模态输入", assignments: vec![
            ("event.entity", "turn"), ("event.verb", "opened"),
            ("actor.pattern", "translator"), ("actor.capability", "judge.intent")] },
        Scenario { name: "S16 元认知校准", assignments: vec![
            ("event.entity", "verdict"), ("event.verb", "asserted"),
            ("actor.pattern", "decider"), ("actor.capability", "assert.verification"), ("projection", "calibration")] },
        Scenario { name: "S17 技能编译", assignments: vec![
            ("event.entity", "memory"), ("event.verb", "progressed"),
            ("actor.pattern", "decider"), ("projection", "skill_compile")] },
        Scenario { name: "S18 硬质实时闭环", assignments: vec![
            ("event.entity", "system"), ("event.verb", "progressed"),
            ("actor.pattern", "decider"), ("actor.budget_ms", "20")] },
        Scenario { name: "S19 成本控制", assignments: vec![
            ("event.entity", "system"), ("event.verb", "asserted"),
            ("actor.pattern", "decider"), ("projection", "budget")] },
        Scenario { name: "S20 审计回放", assignments: vec![
            ("event.entity", "system"), ("event.verb", "asserted"),
            ("actor.pattern", "translator"), ("projection", "replay")] },
    ]
}

/// 审计：场景用到的所有键是否都在机制表内。返回越界键。
fn audit(scs: &[Scenario]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for s in scs {
        for (k, v) in &s.assignments {
            if !MECHANISMS.contains(k) {
                bad.push((s.name.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// 场景覆盖到的机制种类数（只统计被用到的键）
fn mechanisms_used(scs: &[Scenario]) -> usize {
    let mut used: Vec<&str> = Vec::new();
    for s in scs {
        for (k, _) in &s.assignments {
            if !used.contains(k) {
                used.push(k);
            }
        }
    }
    used.len()
}

fn main() {
    println!("═══ 元素数守恒审计 ═══");
    let scs = scenarios();
    println!("机制表（恒定）：{} 项", MECHANISMS.len());
    println!("场景数：{}", scs.len());

    let bad = audit(&scs);
    println!("越界键（需要新机制的）：{} 个", bad.len());
    for b in &bad {
        println!("  {} → {}", b.0, b.1);
    }

    // 守恒律：逐步增加场景，机制数不应随之增长
    println!("\n机制种类数随场景数的变化：");
    let mut prev = 0;
    for n in [1usize, 5, 10, 15, 20] {
        let used = mechanisms_used(&scs[..n.min(scs.len())]);
        println!("  场景 {} 个 → 用到机制 {} 种", n, used);
        prev = used;
    }
    println!("  全量场景 → 用到机制 {} 种（机制表 {} 项，未用 {} 项为延迟启用）",
        prev, MECHANISMS.len(), MECHANISMS.len() - prev);

    // ── 反向用例：加入一个需要新机制的场景，审计必须失败 ──
    let mut with_new = scenarios();
    with_new.push(Scenario { name: "S21 需要新机制的场景", assignments: vec![
        ("event.entity", "turn"), ("wizard.mode", "on")] });
    let bad_new = audit(&with_new);
    println!("\n反向用例：加入含 'wizard.mode' 的场景 → 越界键 {} 个", bad_new.len());

    // ── 断言 ──
    assert!(bad.is_empty(), "20 个场景不应需要任何新机制");
    assert_eq!(bad_new.len(), 1, "反向用例：含未知键的场景必须被审计抓到");
    assert!(bad_new[0].1.contains("wizard.mode"), "反向用例：抓到的必须是那个未知键");
    assert!(prev <= MECHANISMS.len(), "用到的机制数不得超过机制表长度");

    println!("\n✅ 全部断言通过（含 1 条反向用例）：机制数不随场景数增长。");
}
