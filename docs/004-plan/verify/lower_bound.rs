//! 职责合并检验 · 诚实版
//!
//! 论证纪律：
//!   1. 分组由**冲突矩阵计算**得出，不是硬编码的常量返回值。
//!   2. 提供两个**反事实实验**：改一个输入约束，结论必须改变。改了不变 = 判据没在算。
//!   3. 不假装“下界 = N”是证明结果。Act 计不计入原语是**记账立场**，程序把两种账都算出来。
//!
//! 编译运行：rustc --edition 2021 lower_bound.rs -o lb && ./lb

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Duty {
    Hold,   // 事实的持有 + 唯一写入口
    Derive, // 从事件序列纯派生视图
    Act,    // 读事实、产生新事实（调 LLM、有副作用）
}

impl fmt::Display for Duty {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            Duty::Hold => "Hold(持有+写门)",
            Duty::Derive => "Derive(纯派生)",
            Duty::Act => "Act(行为+副作用)",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Polarity {
    Requires, // 该职责要求此性质成立
    Forbids,  // 该职责要求此性质不成立
}

impl fmt::Display for Polarity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Polarity::Requires => "要求",
            Polarity::Forbids => "禁止",
        })
    }
}

/// 反事实开关：用来证明判据真的在参与计算，而不是常量表。
struct Model {
    /// Act 是否必须能直接写事实源。
    /// 关掉它 = 假设"行为单元不需要直连写口"（例如只允许经队列代理）。
    act_needs_direct_write: bool,
    /// Derive 是否禁止副作用。关掉它 = 假设"派生允许有副作用"。
    derive_forbids_side_effect: bool,
}

impl Model {
    fn standard() -> Self {
        Model { act_needs_direct_write: true, derive_forbids_side_effect: true }
    }
    fn counterfactual_a() -> Self {
        Model { act_needs_direct_write: false, derive_forbids_side_effect: true }
    }
    fn counterfactual_b() -> Self {
        Model { act_needs_direct_write: true, derive_forbids_side_effect: false }
    }
    /// 两个开关同时关掉：三者之间再无任何冲突 → 分组数应真的塌缩到 1。
    fn counterfactual_c() -> Self {
        Model { act_needs_direct_write: false, derive_forbids_side_effect: false }
    }
}

fn constraints(m: &Model, d: Duty) -> Vec<(&'static str, Polarity)> {
    match d {
        Duty::Hold => vec![
            ("write_gate_exclusive", Polarity::Requires),
            ("direct_write", Polarity::Forbids),
        ],
        Duty::Derive => {
            let mut v = vec![("nondeterminism", Polarity::Forbids)];
            if m.derive_forbids_side_effect {
                v.push(("side_effect", Polarity::Forbids));
            }
            v
        }
        Duty::Act => {
            let mut v = vec![("side_effect", Polarity::Requires)];
            if m.act_needs_direct_write {
                v.push(("direct_write", Polarity::Requires));
            }
            v
        }
    }
}

/// 冲突判据（唯一判据）：同一性质的极性相反 → 不可合并。
fn conflict(m: &Model, a: Duty, b: Duty) -> Option<String> {
    let ca = constraints(m, a);
    let cb = constraints(m, b);
    for (ka, pa) in &ca {
        for (kb, pb) in &cb {
            if ka == kb && pa != pb {
                return Some(format!("{}：{} {} / {} {}", ka, a, pa, b, pb));
            }
        }
    }
    None
}

/// 贪心分组：能放进已有组就放，放不进就新开一组。组数 = 不可再分的职责分组数。
fn group(m: &Model) -> Vec<Vec<Duty>> {
    let all = [Duty::Hold, Duty::Derive, Duty::Act];
    let mut groups: Vec<Vec<Duty>> = Vec::new();
    for d in all {
        let mut placed = false;
        for g in groups.iter_mut() {
            let ok = g.iter().all(|x| conflict(m, *x, d).is_none());
            if ok {
                g.push(d);
                placed = true;
                break;
            }
        }
        if !placed {
            groups.push(vec![d]);
        }
    }
    groups
}

fn show(m: &Model, label: &str) -> usize {
    println!("\n── {} ──", label);
    let pairs = [(Duty::Hold, Duty::Derive), (Duty::Hold, Duty::Act), (Duty::Derive, Duty::Act)];
    for (a, b) in pairs {
        match conflict(m, a, b) {
            None => println!("  {} ⊕ {} → 可合并", a, b),
            Some(reason) => println!("  {} ⊕ {} → 不可合并（{}）", a, b, reason),
        }
    }
    let g = group(m);
    for (i, gg) in g.iter().enumerate() {
        let names: Vec<String> = gg.iter().map(|d| d.to_string()).collect();
        println!("  分组 {}：{{{}}}", i + 1, names.join(", "));
    }
    println!("  不可再分分组数 = {}", g.len());
    g.len()
}

fn main() {
    println!("═══ 职责合并检验（由冲突矩阵计算，非硬编码）═══");

    // 标准模型
    let n_standard = show(&Model::standard(), "标准模型");

    // 反事实 A：行为单元不需要直连写口 → Hold 与 Act 不再冲突 → 分组数应下降
    let n_a = show(&Model::counterfactual_a(), "反事实 A：Act 不需要 direct_write");

    // 反事实 B：派生允许副作用 → Derive 与 Act 不再冲突
    let n_b = show(&Model::counterfactual_b(), "反事实 B：Derive 不禁 side_effect");
    // 反事实 C：两者同时关掉 → 三者再无冲突，分组数应塌缩到 1
    let n_c = show(&Model::counterfactual_c(), "反事实 C：两者同时关掉");

    println!("\n═══ 反事实检验（判据是否真在参与计算）═══");
    let std_ha = conflict(&Model::standard(), Duty::Hold, Duty::Act).is_some();
    let a_ha = conflict(&Model::counterfactual_a(), Duty::Hold, Duty::Act).is_some();
    let std_da = conflict(&Model::standard(), Duty::Derive, Duty::Act).is_some();
    let b_da = conflict(&Model::counterfactual_b(), Duty::Derive, Duty::Act).is_some();
    println!("  标准 Hold⊕Act 冲突 = {}；反事实 A 下 = {}", std_ha, a_ha);
    println!("  标准 Derive⊕Act 冲突 = {}；反事实 B 下 = {}", std_da, b_da);

    println!("\n═══ 下界：两种记账立场 ═══");
    println!("  Act 计入原语        → 原语下界 = {}", n_standard);
    println!("  Act 计为一等概念    → 原语下界 = {}", n_standard - 1);
    println!("  （本方案立场：Act 是「使用者」不是「构成者」，故取 {}）", n_standard - 1);

    // ── 断言（每条都对应一个真实计算，且配了反向用例）──
    assert_eq!(n_standard, 2, "标准模型下不可再分分组应为 2");
    assert!(std_ha, "标准模型下 Hold⊕Act 必须冲突（否则 I1 单通道失效）");
    assert!(std_da, "标准模型下 Derive⊕Act 必须冲突（否则纯度失效）");
    assert!(!a_ha, "反向用例 A：关掉 direct_write 后 Hold⊕Act 冲突应消失");
    assert!(!b_da, "反向用例 B：关掉 side_effect 禁令后 Derive⊕Act 冲突应消失");
    // A 单独关只解除 Hold⊕Act，Act 仍与 Derive 冲突，故分组数不变；
    // 两个都关才解除全部冲突 —— 分组数必须真的塌缩。这一条是"判据在算"的硬证据。
    assert_eq!(n_a, 2, "反事实 A 单独作用时分组数不变（Act 仍与 Derive 冲突）");
    assert_eq!(n_b, 2, "反事实 B 单独作用时分组数不变（Act 仍与 Hold 冲突）");
    assert_eq!(n_c, 1, "反向用例 C：冲突全部解除后分组数必须从 2 塌缩到 1");

    println!("\n✅ 全部断言通过（含 4 条反向用例）：下界由计算得出，且判据对输入敏感。");
}
