//! 元素数守恒律：机制数不随场景数增长
//!
//! 这是「扩充不会重构」唯一可自动审计的形式：
//!   · 场景 = 一组（机制键, 取值）赋值向量
//!   · 合法性 = 场景用到的键 ⊆ 机制表
//!   · 守恒 = 机制表长度不随场景数变化
//!
//! 机制表**不在本程序里**：它由 `scripts/gen-verify-facts.mjs` 从
//! [01 §8 权威参数表](../01-核心架构.md) 生成进 `facts`（顶层键判据见该脚本注释——
//! `projection.param` 是 `projection` 的子键，不是第 11 个机制）。
//! 于是这里审的是「计划文档里登记的键 ⊆ 权威参数表」，而不是「程序抄的那一份自洽」。
//!
//! 两类场景，来源不同、判据相同：
//!   · `facts::STAGE_DOCS` —— 路线图 13 阶**真实声明**的赋值向量（各阶 §3 的
//!     `capability-assign` 块），这是计划本体；
//!   · `scenarios()` 的 20 个 —— 本程序**自造**的压力样本（不属于任何文档），
//!     它们问的是另一个问题：往「已有场景」这个方向再加到 20 种能力，会不会长出
//!     第 11 个键。数据是输入，结论仍由遍历算出。
//!
//! 反向用例：加入一个需要新机制的场景，审计必须失败。
//!
//! 编译运行：rustc --edition 2021 mechanism_growth.rs -o mg && ./mg

mod facts;

use facts::{MECHANISMS, PARAM_KEYS, STAGE_DOCS};

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

/// 同一判据用在计划本体上：路线图各阶 §3 声明的赋值。
///
/// 越界即「这一阶要长出新机制」——那是重构，不是排期，必须先过 ADR 前置闸门。
fn audit_stage_docs() -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for doc in STAGE_DOCS {
        for (k, v) in doc.assigns {
            if !MECHANISMS.contains(k) {
                bad.push((doc.id.to_string(), format!("{} = {}", k, v)));
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
    let stage_bad = audit_stage_docs();
    println!(
        "机制表（01 §8 生成，顶层键）：{} 项；§8 登记的键（含子键）：{} 项",
        MECHANISMS.len(),
        PARAM_KEYS.len()
    );
    println!("路线图各阶 §3 的赋值越界键：{} 个", stage_bad.len());
    println!("自造场景数：{}", scs.len());

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
    println!(
        "  全量场景 → 用到机制 {} 种（机制表 {} 项，未用 {} 项为延迟启用）",
        prev,
        MECHANISMS.len(),
        MECHANISMS.len() - prev
    );

    // ── 反向用例：加入一个需要新机制的场景，审计必须失败 ──
    let mut with_new = scenarios();
    with_new.push(Scenario {
        name: "S21 需要新机制的场景",
        assignments: vec![("event.entity", "turn"), ("wizard.mode", "on")],
    });
    let bad_new = audit(&with_new);
    println!("\n反向用例：加入含 'wizard.mode' 的场景 → 越界键 {} 个", bad_new.len());

    // ── 断言 ──
    assert!(bad.is_empty(), "自造场景不应需要任何新机制");
    assert_eq!(bad_new.len(), 1, "反向用例：含未知键的场景必须被审计抓到");
    assert!(
        bad_new[0].1.contains("wizard.mode"),
        "反向用例：抓到的必须是那个未知键"
    );
    assert!(
        stage_bad.is_empty(),
        "路线图某一阶 §3 声明的键不在机制表里——那一阶要长出新机制，先走 ADR"
    );
    assert!(prev <= MECHANISMS.len(), "用到的机制数不得超过机制表长度");
    // 「机制键是 10 个还是 11 个」的口径分歧在此一次清偿：判据不是名单，是形状。
    assert!(
        PARAM_KEYS
            .iter()
            .all(|k| MECHANISMS.contains(k) || (k.contains('.') && MECHANISMS.contains(&k.split('.').next().unwrap()))),
        "§8 里的键要么是顶层机制键，要么首段是——否则它就是被漏掉的第 11 个机制"
    );
    assert!(
        PARAM_KEYS.len() > MECHANISMS.len(),
        "§8 登记了子键，全键数应严格大于顶层键数（相等说明表读歪了）"
    );

    println!("\n✅ 全部断言通过（含 1 条反向用例）：机制数不随场景数增长。");
}
