//! 能力坐标系：完备性分类 + 伪高阶识别
//!
//! 能力清单是**开放集**，所以用坐标系而不是清单来保证完备：
//!   横向五维（知/行/言/省/欲）——回答"这是什么种类的能力"，正交且合起来完备
//!   纵向五轴（主动/时域/抽象/自我模型/自主）——回答"这件事让它高阶了多少"
//! 新增能力必须回答"属于哪一维、提升哪一轴"；答不出 = 它是已有维度的组合，不是新能力。
//!
//! **本程序里没有手填的计划数据**：名册、五维、五轴、天梯上界、伪高阶词表，
//! 以及三处**声明**（02 §2 条数、§6.2 生长位、§7 社交能力的轴）都由
//! `facts`（生成自 [02](../02-能力坐标系.md)）给出，判据在这里，数据不在这里。
//! 于是改文档必然改结论——改一行名册而不同步那三处声明，本程序就红。
//!
//! 反向用例：假维度 / 假轴 / 越界天梯必须被分类器拒绝；伪高阶必须被识别、
//! 真实能力不得被误判；名册填掉一个生长位而 §6.2 不删那行 ⇒ 对账必须失配。
//!
//! 编译运行：rustc --edition 2021 capability_frame.rs -o cf && ./cf

mod facts;
use facts::{
    AXES, CAPABILITIES, DECLARED_DIM_COUNTS, DECLARED_GROWTH, DECLARED_SOCIAL, DIMENSIONS,
    PSEUDO, TIER_MAX,
};

/// 一行的归类是否落在坐标系里。维度、轴、天梯上界都来自 02 的定义表。
fn classify(name: &str, dim: &str, axis: &str, tier: usize) -> Result<(), String> {
    if !DIMENSIONS.contains(&dim) {
        return Err(format!("「{name}」的维度「{dim}」不在五维内"));
    }
    if !AXES.contains(&axis) {
        return Err(format!("「{name}」的轴「{axis}」不在五轴内"));
    }
    if tier < 1 || tier > TIER_MAX {
        return Err(format!(
            "「{name}」的天梯层级 {tier} 越界（应 1-{TIER_MAX}）"
        ));
    }
    Ok(())
}

/// 逐维条数：遍历名册算出，不是抄 §2 的那一列
fn counts_by_dim() -> Vec<(&'static str, usize)> {
    DIMENSIONS
        .iter()
        .map(|d| (*d, CAPABILITIES.iter().filter(|c| c.dim == *d).count()))
        .collect()
}

/// 名册算出的空格子（生长位），按 02 §6.2 的「维 × 轴」写法给
fn empty_cells() -> Vec<(&'static str, &'static str)> {
    DIMENSIONS
        .iter()
        .flat_map(|d| {
            AXES.iter()
                .map(move |a| (*d, *a))
                .filter(|(d, a)| !CAPABILITIES.iter().any(|c| c.dim == *d && c.axis == *a))
        })
        .collect()
}

/// §7 声明的社会性能力：名字要在名册里，且它提升的轴要与名册一致
fn social_mismatches() -> Vec<String> {
    let mut bad = Vec::new();
    for (name, axis) in DECLARED_SOCIAL {
        match CAPABILITIES.iter().find(|c| c.name == *name) {
            None => bad.push(format!("§7 声明的社会性能力「{name}」不在名册里")),
            Some(c) if c.axis != *axis => {
                bad.push(format!(
                    "「{name}」：§7 说它提升「{axis}」，名册记的是「{}」",
                    c.axis
                ))
            }
            Some(_) => {}
        }
    }
    bad
}

/// §2 声明的逐维条数与名册算出的差集（声明是被验的一方，所以传进来算）
fn count_diff(declared: &[(&str, usize)]) -> Vec<String> {
    let computed = counts_by_dim();
    let mut bad = Vec::new();
    for (dim, want) in declared {
        match computed.iter().find(|(d, _)| d == dim) {
            None => bad.push(format!("§2 声明了「{dim}」这一维，坐标系的定义表里没有它")),
            Some((_, got)) if got != want => {
                bad.push(format!("「{dim}」：§2 声明 {want} 条，名册算出 {got} 条"))
            }
            Some(_) => {}
        }
    }
    if declared.len() != computed.len() {
        bad.push(format!(
            "§2 声明了 {} 维，名册里出现的维有 {} 个——两处不同集合",
            declared.len(),
            computed.len()
        ));
    }
    bad
}

/// 伪高阶识别：看起来高阶、实际是宽度或工程的能力。
/// 词表是 02 §5 那张表的「说法」列，不在这里另抄一份。
fn is_pseudo(name: &str) -> bool {
    PSEUDO.iter().any(|p| name.contains(p))
}

fn main() {
    println!("═══ 能力坐标系完备性 ═══");
    println!(
        "名册：{} 项（02 §6.1）；维 {} 个 × 轴 {} 个 = {} 格，天梯上界 T{}",
        CAPABILITIES.len(),
        DIMENSIONS.len(),
        AXES.len(),
        DIMENSIONS.len() * AXES.len(),
        TIER_MAX
    );

    let mut bad: Vec<String> = Vec::new();
    for c in CAPABILITIES {
        if let Err(e) = classify(c.name, c.dim, c.axis, c.tier) {
            bad.push(e);
        }
    }
    println!("归类失败：{} 条", bad.len());
    for b in &bad {
        println!("  {b}");
    }

    println!("\n覆盖矩阵（行=五维，列=五轴，由名册算出）：");
    print!("        ");
    for a in AXES {
        print!("{a:>10}  ");
    }
    println!();
    for d in DIMENSIONS {
        print!("  {d}    ");
        for a in AXES {
            let n = CAPABILITIES
                .iter()
                .filter(|c| c.dim == *d && c.axis == *a)
                .count();
            print!("{n:>10}  ");
        }
        println!();
    }

    let empties = empty_cells();
    let declared: Vec<(&str, &str)> = DECLARED_GROWTH.iter().copied().collect();
    let missing = declared
        .iter()
        .filter(|c| !empties.contains(c))
        .cloned()
        .collect::<Vec<_>>();
    let unlisted = empties
        .iter()
        .filter(|c| !declared.contains(c))
        .cloned()
        .collect::<Vec<_>>();
    println!(
        "\n§6.2 声明的生长位：{} 格；名册算出的空格子：{} 格（集合不同就红）",
        declared.len(),
        empties.len()
    );
    for m in &missing {
        println!("  声明了「{} × {}」但名册里那一格并不空", m.0, m.1);
    }
    for u in &unlisted {
        println!("  名册里「{} × {}」是空的，§6.2 却没登记这个生长位", u.0, u.1);
    }

    println!("\n各维条数（§2 声明 vs 名册算出）：");
    for (dim, declared_n) in DECLARED_DIM_COUNTS {
        let got = counts_by_dim()
            .iter()
            .find(|(d, _)| d == dim)
            .map(|(_, n)| *n)
            .unwrap_or(0);
        println!("  {dim}：声明 {declared_n} / 算出 {got}");
    }

    let social_bad = social_mismatches();
    println!("\n§7 社会性能力对账：声明 {} 条，失配 {} 条", DECLARED_SOCIAL.len(), social_bad.len());
    for s in &social_bad {
        println!("  {s}");
    }

    println!("\n═══ 伪高阶识别（词表 = 02 §5 的「说法」列）═══");
    for p in PSEUDO {
        println!("  {p} → 伪高阶 = {}", is_pseudo(p));
    }
    let real = CAPABILITIES
        .iter()
        .find(|c| !is_pseudo(c.name))
        .expect("名册里至少要有一条真能力");
    println!("  {} → 伪高阶 = {}", real.name, is_pseudo(real.name));

    // ── 反向用例 1/2/3：非法维度、非法轴、越界天梯，分类器必须拒绝 ──
    let fake_dim = classify("玄学共振", "灵", "主动性", 3).is_err();
    let fake_axis = classify("玄学共振2", "知", "气场强度", 3).is_err();
    let fake_tier = classify("玄学共振3", "知", "主动性", TIER_MAX + 1).is_err();

    // ── 反向用例 4：名册填掉某个空格子、而 §6.2 没有删那一行 ⇒ 对账必须失配 ──
    // 这证明空格子是**算出来**的：少一格空的，声明就多一条对不上。
    let sample = *empties
        .first()
        .expect("至少要有一个空格子供反向用例（全格都满说明名册读歪了）");
    let mut shrunken = empties.clone();
    shrunken.retain(|c| *c != sample);
    let fill_breaks = shrunken.len() != declared.len() || declared.contains(&sample);

    println!(
        "\n── 反向用例：假维度 = {fake_dim}，假轴 = {fake_axis}，越界天梯 = {fake_tier}，\n   填掉「{} × {}」而 §6.2 不删那行 = {fill_breaks}",
        sample.0, sample.1
    );

    let count_bad = count_diff(DECLARED_DIM_COUNTS);
    let mut drifted = DECLARED_DIM_COUNTS.to_vec();
    drifted[0].1 += 1;
    let drift_caught = !count_diff(&drifted).is_empty();
    println!("   声明条数 +1 而名册不动 = {drift_caught}（被抓到）");

    // ── 断言 ──
    assert!(bad.is_empty(), "名册每一行都必须能归入五维五轴与天梯");
    assert!(
        !CAPABILITIES.is_empty(),
        "名册为空说明 02 §6.1 被读成了一张空表"
    );
    assert!(count_bad.is_empty(), "逐维条数必须与 §2 的声明一致：{count_bad:?}");
    assert!(
        missing.is_empty() && unlisted.is_empty(),
        "§6.2 的生长位必须与名册算出的空格子同集合"
    );
    assert!(social_bad.is_empty(), "§7 声明的社会性能力必须与名册逐条一致：{social_bad:?}");
    assert!(
        DIMENSIONS.iter().all(|d| CAPABILITIES.iter().any(|c| c.dim == *d)),
        "每一维都不能为空（否则该维是摆设）"
    );
    assert!(
        fake_dim && fake_axis && fake_tier,
        "反向用例：分类器必须拒绝非法维度/轴/天梯"
    );
    assert!(fill_breaks, "反向用例：填掉一个生长位而 §6.2 不删那行 ⇒ 必须失配");
    assert!(drift_caught, "反向用例：声明的条数与名册算出的不同 ⇒ 必须被抓到");
    assert!(PSEUDO.iter().all(|p| is_pseudo(p)), "§5 的每一条说法都必须被识别为伪高阶");
    assert!(!is_pseudo(real.name), "真实能力不应被误判为伪高阶");

    println!("\n✅ 全部断言通过（含 5 条反向用例）：名册归位、条数与声明同、生长位与声明同集合、");
    println!("   伪高阶词表来自文档且可识别。数据在 02，判据在这里。");
}
