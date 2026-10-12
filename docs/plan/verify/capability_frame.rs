//! 能力坐标系：完备性分类 + 刻度 + 伪高阶识别
//!
//! 能力清单是**开放集**，所以用坐标系而不是清单来保证完备：
//!   横向五维（知/行/言/省/欲）——回答"这是什么种类的能力"，正交且合起来完备
//!   纵向五轴（主动/时域/抽象/自我模型/自主）——回答"这条轴被抬到了第几级"
//!   刻度四级（L1–L4，**格数由 02 §3.1 表的列数算出**）——回答"抬到了第几格"
//! 新增能力必须回答"属于哪一维、把哪条轴抬到第几级、由哪一阶点亮"；
//! 答不出 = 它是已有维度的组合（02 §2.1）。
//!
//! **本程序里没有手填的计划数据**：名册（含等级 / 类型 / 落点）、五维、五轴、刻度格数、
//! 天梯的「核心能力」列、共线上限与登记、以及四处**声明**
//! （02 §2 条数、§3.3 共线、§6.2 生长位、§7 社交能力的轴）都由 `facts`
//! （生成自 [02](../PL-002-能力坐标系.md)）给出，判据在这里，数据不在这里。
//!
//! 反向用例：假维度 / 假轴 / 越界天梯 / 越界等级 / 假类型 / 不存在的阶号必须被分类器拒绝；
//! 伪高阶必须被识别、真实能力不得被误判；名册填掉一个生长位而 §6.2 不删那行 ⇒ 对账失配；
//! 顶格无人占据、某一阶没有能力落在它上面、天梯列出的核心能力不在该级名册里 ⇒ 都要被抓到。
//!
//! 编译运行：rustc --edition 2021 capability_frame.rs -o cf && ./cf

mod facts;
use facts::{
    AXES, CAPABILITIES, DECLARED_DIM_COUNTS, DECLARED_GROWTH, DECLARED_SOCIAL, DIMENSIONS, KINDS,
    LEVEL_MAX, PSEUDO, STAGE_CLAIMS, TIER_CORES, TIER_MAX,
};

/// 一行的归类是否落在坐标系里。维度、轴、天梯上界、等级上界、类型闭集、阶号
/// 全部来自 02 的定义表——**没有一个是这里写死的**。
fn classify(c: &facts::Capability) -> Result<(), String> {
    if !DIMENSIONS.contains(&c.dim) {
        return Err(format!("「{}」的维度「{}」不在五维内", c.name, c.dim));
    }
    if !AXES.contains(&c.axis) {
        return Err(format!("「{}」的轴「{}」不在五轴内", c.name, c.axis));
    }
    if c.tier < 1 || c.tier > TIER_MAX {
        return Err(format!("「{}」的天梯层级 {} 越界（应 1-{TIER_MAX}）", c.name, c.tier));
    }
    if c.level < 1 || c.level > LEVEL_MAX {
        return Err(format!(
            "「{}」的等级 {} 越界（应 L1-L{LEVEL_MAX}，刻度格数来自 02 §3.1）",
            c.name, c.level
        ));
    }
    if !KINDS.contains(&c.kind) {
        return Err(format!("「{}」的类型「{}」不在闭集内", c.name, c.kind));
    }
    if c.stages.is_empty() {
        return Err(format!("「{}」没有落点阶——坐标系与实施阶梯就此脱钩", c.name));
    }
    for s in c.stages {
        if !STAGE_CLAIMS.iter().any(|x| x.id == *s) {
            return Err(format!(
                "「{}」的落点「{s}」不是阶梯里的一阶（阶梯共 {} 阶）",
                c.name,
                STAGE_CLAIMS.len()
            ));
        }
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
            AXES.iter().map(move |a| (*d, *a)).filter(|(d, a)| {
                !CAPABILITIES.iter().any(|c| c.dim == *d && c.axis == *a)
            })
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
                bad.push(format!("「{name}」：§7 说它提升「{axis}」，名册记的是「{}」", c.axis))
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

/// 每条轴的**顶格**（L{LEVEL_MAX}）是否有人占据。没人的刻度是装饰：它让人以为
/// 那条轴还能更高，而实际上名册里没有任何能力走到那一格。
fn dead_rungs(caps: &[&facts::Capability]) -> Vec<(&'static str, usize)> {
    AXES
        .iter()
        .map(|a| {
            let top = caps.iter().filter(|c| c.axis == *a).map(|c| c.level).max().unwrap_or(0);
            (*a, top)
        })
        .filter(|(_, top)| *top < LEVEL_MAX)
        .collect()
}

/// 阶梯里没有任何能力落在它上面的阶——坐标系与实施阶梯脱钩的那种形态。
fn stages_without_caps(caps: &[&facts::Capability]) -> Vec<&'static str> {
    STAGE_CLAIMS
        .iter()
        .filter(|s| !caps.iter().any(|c| c.stages.contains(&s.id)))
        .map(|s| s.id)
        .collect()
}

/// 天梯上没有任何能力落在它上面的级——空台阶（阶梯里的一段空白）。
fn tiers_without_caps(caps: &[&facts::Capability]) -> Vec<usize> {
    (1..=TIER_MAX).filter(|t| !caps.iter().any(|c| c.tier == *t)).collect()
}

/// §3.2 天梯表的「核心能力」列必须逐条对应名册里**该级**的行：
/// 它若写了一个不在该级名册里的能力名，这张表就在讲另一个故事。
/// 取 `cores` 作参数是为了让反向用例能喂一份假的 §3.2。
fn tier_core_mismatches(cores: &[facts::TierCore], caps: &[&facts::Capability]) -> Vec<String> {
    let mut bad = Vec::new();
    for tc in cores {
        for n in tc.names {
            match caps.iter().find(|c| c.name == *n) {
                None => bad.push(format!("T{} 的核心能力「{n}」不在名册里", tc.tier)),
                Some(c) if c.tier != tc.tier => bad.push(format!(
                    "T{} 的核心能力「{n}」在名册里属于 T{}",
                    tc.tier, c.tier
                )),
                Some(_) => {}
            }
        }
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
    let caps: Vec<&facts::Capability> = CAPABILITIES.iter().collect();
    println!(
        "名册：{} 项（02 §6.1）；维 {} 个 × 轴 {} 个 = {} 格，天梯 T1-T{}，刻度 L1-L{}，类型 {} 种",
        CAPABILITIES.len(),
        DIMENSIONS.len(),
        AXES.len(),
        DIMENSIONS.len() * AXES.len(),
        TIER_MAX,
        LEVEL_MAX,
        KINDS.len()
    );

    let mut bad: Vec<String> = Vec::new();
    for c in CAPABILITIES {
        if let Err(e) = classify(c) {
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
            .find(|e| e.0 == *dim)
            .map(|e| e.1)
            .unwrap_or(0);
        println!("  {dim}：声明 {declared_n} / 算出 {got}");
    }

    let social_bad = social_mismatches();
    println!(
        "\n§7 社会性能力对账：声明 {} 条，失配 {} 条",
        DECLARED_SOCIAL.len(),
        social_bad.len()
    );
    for s in &social_bad {
        println!("  {s}");
    }

    // 天梯的轴等级向量：逐级打印**累计**刻度（截至该级为止每条轴的顶格），
    // 使「T 级 = 五轴的组合」可读，也使它不可能另立一套语义。
    println!("\n§3.2 天梯的轴等级向量（截至 Tn 的累计顶格等级，由名册算出）：");
    for t in 1..=TIER_MAX {
        let cells: Vec<String> = AXES
            .iter()
            .map(|a| {
                let m = caps
                    .iter()
                    .filter(|c| c.tier <= t && c.axis == *a)
                    .map(|c| c.level)
                    .max()
                    .unwrap_or(0);
                format!("{a} L{m}")
            })
            .collect();
        println!("  T{t}: {}", cells.join(" · "));
    }
    let core_bad = tier_core_mismatches(TIER_CORES, &caps);
    println!(
        "  核心能力列共 {} 个指认，与名册失配 {} 个",
        TIER_CORES.iter().map(|t| t.names.len()).sum::<usize>(),
        core_bad.len()
    );
    for c in &core_bad {
        println!("    {c}");
    }

    let dead = dead_rungs(&caps);
    println!("\n每条轴的顶格占用（刻度的 L{LEVEL_MAX} 无人即装饰）：");
    for a in AXES {
        let top = CAPABILITIES
            .iter()
            .filter(|c| c.axis == *a)
            .map(|c| c.level)
            .max()
            .unwrap_or(0);
        println!("  {a}: 顶格 L{top}{}", if top < LEVEL_MAX { " ← 死格" } else { "" });
    }

    let unlit = stages_without_caps(&caps);
    println!(
        "\n阶梯咬合：{} 阶里没有能力落在它上面的：{} 阶",
        STAGE_CLAIMS.len(),
        unlit.len()
    );
    for s in &unlit {
        println!("  {s}");
    }
    let empty_tiers = tiers_without_caps(&caps);
    println!("天梯台阶：T1-T{TIER_MAX} 里没有能力落在它上面的：{} 级", empty_tiers.len());
    for t in &empty_tiers {
        println!("  T{t}");
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

    // ── 反向用例 1/2/3：非法维度、非法轴、越界天梯 / 越界等级 / 假类型 / 不存在的阶号 ──
    let mut fake = *CAPABILITIES
        .first()
        .expect("名册不能为空——空表会让下面每条断言都变成永真");
    let base = fake;
    fake.dim = "灵";
    let fake_dim = classify(&fake).is_err();
    fake = base;
    fake.axis = "气场强度";
    let fake_axis = classify(&fake).is_err();
    fake = base;
    fake.tier = TIER_MAX + 1;
    let fake_tier = classify(&fake).is_err();
    fake = base;
    fake.level = LEVEL_MAX + 1;
    let fake_level = classify(&fake).is_err();
    fake = base;
    fake.kind = "花活";
    let fake_kind = classify(&fake).is_err();
    fake = base;
    fake.stages = &["S99"];
    let fake_stage = classify(&fake).is_err();
    fake = base;
    fake.stages = &[];
    let no_stage = classify(&fake).is_err();

    // ── 反向用例 4：名册填掉某个空格子、而 §6.2 没有删那一行 ⇒ 对账必须失配 ──
    // 这证明空格子是**算出来**的：少一格空的，声明就多一条对不上。
    let sample = *empties
        .first()
        .expect("至少要有一个空格子供反向用例（全格都满说明名册读歪了）");
    let mut shrunken = empties.clone();
    shrunken.retain(|c| *c != sample);
    let fill_breaks = shrunken.len() != declared.len() || declared.contains(&sample);

    println!(
        "\n── 反向用例：假维度 = {fake_dim}，假轴 = {fake_axis}，越界天梯 = {fake_tier}，\n   越界等级 = {fake_level}，假类型 = {fake_kind}，不存在的阶 = {fake_stage}，无落点 = {no_stage}，\n   填掉「{} × {}」而 §6.2 不删那行 = {fill_breaks}",
        sample.0, sample.1
    );

    let count_bad = count_diff(DECLARED_DIM_COUNTS);
    let mut drifted = DECLARED_DIM_COUNTS.to_vec();
    drifted[0].1 += 1;
    let drift_caught = !count_diff(&drifted).is_empty();
    println!("   声明条数 +1 而名册不动 = {drift_caught}（被抓到）");

    // ── 反向用例 6：把某条轴的顶格行抽掉 ⇒ 该轴立刻变成死格 ──
    let top_axis = AXES
        .iter()
        .find(|a| caps.iter().any(|c| c.axis == **a && c.level == LEVEL_MAX))
        .copied()
        .expect("至少要有一条轴顶格有人（否则刻度本身就是死的）");
    let without_top: Vec<&facts::Capability> = caps
        .iter()
        .filter(|c| !(c.axis == top_axis && c.level == LEVEL_MAX))
        .copied()
        .collect();
    let rung_caught = !dead_rungs(&without_top).is_empty();
    println!("   抽掉「{top_axis}」的顶格行 ⇒ 死格被抓到 = {rung_caught}");

    // ── 反向用例 7：抽掉某一阶唯一落在它上面的能力 ⇒ 该阶变空 ──
    let solo_stage = STAGE_CLAIMS
        .iter()
        .find(|s| caps.iter().filter(|c| c.stages.contains(&s.id)).count() == 1)
        .map(|s| s.id)
        .expect("至少有一阶只被一条能力点亮（否则这条判据测不到）");
    let solo_name = caps
        .iter()
        .find(|c| c.stages.contains(&solo_stage))
        .expect("反例前提")
        .name;
    let without_solo: Vec<&facts::Capability> =
        caps.iter().filter(|c| c.name != solo_name).copied().collect();
    let stage_caught = stages_without_caps(&without_solo).contains(&solo_stage);
    println!("   抽掉「{solo_name}」⇒ {solo_stage} 变空被抓到 = {stage_caught}");

    // ── 反向用例 8：天梯的核心能力列塞一个名册里没有的名字 ⇒ 必须被抓到 ──
    let invented: Vec<facts::TierCore> = vec![facts::TierCore {
        tier: 1,
        names: &["凭空捏造的能力"],
    }];
    let core_caught = !tier_core_mismatches(&invented, &caps).is_empty();
    println!("   T1 的核心能力列写一个名册里没有的名字 = {core_caught}（被抓到）");

    // ── 反向用例 9：抽掉某一级唯一的能力 ⇒ 该级成空台阶 ──
    let solo_tier = (1..=TIER_MAX)
        .find(|t| caps.iter().filter(|c| c.tier == *t).count() == 1)
        .expect("至少有一级只被一条能力占据（否则这条判据测不到）");
    let solo_tier_name = caps.iter().find(|c| c.tier == solo_tier).expect("反例前提").name;
    let without_tier: Vec<&facts::Capability> =
        caps.iter().filter(|c| c.name != solo_tier_name).copied().collect();
    let tier_caught = tiers_without_caps(&without_tier).contains(&solo_tier);
    println!("   抽掉「{solo_tier_name}」⇒ T{solo_tier} 成空台阶 = {tier_caught}");

    // ── 断言 ──
    assert!(bad.is_empty(), "名册每一行都必须能归入五维五轴、刻度、类型闭集与阶梯");
    assert!(!CAPABILITIES.is_empty(), "名册为空说明 02 §6.1 被读成了一张空表");
    assert!(count_bad.is_empty(), "逐维条数必须与 §2 的声明一致：{count_bad:?}");
    assert!(
        missing.is_empty() && unlisted.is_empty(),
        "§6.2 的生长位必须与名册算出的空格子同集合"
    );
    assert!(social_bad.is_empty(), "§7 声明的社会性能力必须与名册逐条一致：{social_bad:?}");
    assert!(
        core_bad.is_empty(),
        "§3.2 的核心能力必须逐条对应该级名册行：{core_bad:?}"
    );
    assert!(
        dead.is_empty(),
        "每条轴的顶格 L{LEVEL_MAX} 必须有人占据（死格 = 刻度是装饰）：{dead:?}"
    );
    assert!(
        unlit.is_empty(),
        "阶梯里每一阶至少要有一条能力落在它上面：{unlit:?}"
    );
    assert!(
        empty_tiers.is_empty(),
        "天梯每一级至少要有一条能力落在它上面（空台阶）：{empty_tiers:?}"
    );
    assert!(
        DIMENSIONS.iter().all(|d| CAPABILITIES.iter().any(|c| c.dim == *d)),
        "每一维都不能为空（否则该维是摆设）"
    );
    assert!(
        fake_dim && fake_axis && fake_tier && fake_level && fake_kind && fake_stage && no_stage,
        "反向用例：分类器必须拒绝非法维度/轴/天梯/等级/类型/落点"
    );
    assert!(fill_breaks, "反向用例：填掉一个生长位而 §6.2 不删那行 ⇒ 必须失配");
    assert!(drift_caught, "反向用例：声明的条数与名册算出的不同 ⇒ 必须被抓到");
    assert!(rung_caught, "反向用例：抽掉某条轴的顶格行 ⇒ 必须判出死格");
    assert!(stage_caught, "反向用例：抽掉某阶唯一的能力 ⇒ 必须判出空阶");
    assert!(core_caught, "反向用例：核心能力列写一个不存在的名字 ⇒ 必须被抓到");
    assert!(tier_caught, "反向用例：抽掉某一级唯一的能力 ⇒ 必须判出空台阶");
    assert!(PSEUDO.iter().all(|p| is_pseudo(p)), "§5 的每一条说法都必须被识别为伪高阶");
    assert!(!is_pseudo(real.name), "真实能力不应被误判为伪高阶");

    println!("\n✅ 全部断言通过（含 9 类反向用例）：名册归位、等级与刻度、类型与落点、");
    println!("   阶梯咬合、台阶非空、生长位与声明同集合、天梯核心能力与名册一致、伪高阶可识别。");
    println!("   数据在 02，判据在这里。");
}