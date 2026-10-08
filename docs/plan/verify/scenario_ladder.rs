//! 能力扩充阶梯审计：13 阶从低阶到高阶，是否始终无需新增机制
//!
//! **本程序里没有手填的计划数字。** 数据全部来自 `facts`（生成物，由
//! `scripts/gen-verify-facts.mjs` 从 `docs/plan` 抽取）：
//!   · 机制表与其**取值域** ← [01 §8 权威参数表](../01-核心架构.md)
//!   · 名册与**声明**（逐阶新增键数、参数变化数）← [路线图总览 §1](../roadmap/00-路线图总览.md)
//!   · 每阶点亮的机制键 ← 各阶文档 §3 的 `capability-assign` 块
//!   · 每阶的退路口 ← 各阶文档 §4 里平凡值**加粗**的那一行
//!
//! 「声明」是被验的结论、不是输入：逐阶新增键数由赋值向量**算出**再与总表那一列比。
//! 文档两处各说各话从此是一次红灯，而不是两句话各自成立。
//!
//! 回答五件事：
//!   Q1 有没有任何一阶用到机制表之外的键（越界键应为 0）
//!   Q2 逐阶新增键数量是否如总表所声明（末 5 阶应为 0）
//!   Q3 每阶的退路口是否是个**真的**平凡值（J2：平凡值 ≠ 本阶取值）
//!   Q4 名册是否只有一份：总表行数与 roadmap 的阶文件数相等、且按 id 逐一对应
//!   Q5 有没有任何一阶取了一个**值域之外**的值（越界取值应为 0——这一条此前由一个
//!      恒为 `false`、任何输入都翻不动的标志冒充，见 plan/13 §1 M8）
//!
//! 沿用三条验证纪律：输入不含结论 / 每条断言配反向用例 / 结论由遍历算出而非硬编码。
//!
//! 编译运行：rustc --edition 2021 scenario_ladder.rs -o sl && ./sl

mod facts;

use facts::{DOMAINS, MECHANISMS, STAGE_CLAIMS, STAGE_DOCS, domain_of, in_domain};

#[derive(Clone)]
struct Stage {
    id: &'static str,
    /// roadmap 目录里的那篇（名册与文档一一对应的凭据）
    file: &'static str,
    name: &'static str,
    tier: &'static str,
    /// 本阶点亮的机制键 → 取值（§3 的赋值块）
    assigns: &'static [(&'static str, &'static str)],
    /// J2 退路口：机制键 → 平凡值（§4 的加粗行）
    fallback: (&'static str, &'static str),
    /// 总表**声明**的本阶新增机制键数——被验对象
    claimed_new_keys: usize,
    /// 总表**声明**的本阶参数变化数——应等于赋值块行数
    claimed_param_changes: usize,
}

/// 名册：总表的每一行配一篇阶文档，**按 id 配对**。
///
/// 配不上就 panic，而不是断言失败——程序拒绝带着残缺名册给出绿灯。
/// 「少一阶」在输出表里与「全绿」长得一模一样，必须在这里响。
fn stages() -> Vec<Stage> {
    STAGE_CLAIMS
        .iter()
        .map(|c| {
            let doc = STAGE_DOCS.iter().find(|d| d.id == c.id).unwrap_or_else(|| {
                panic!("总表声明了 {}，roadmap 目录里却没有对应的阶文档", c.id)
            });
            Stage {
                id: c.id,
                file: doc.file,
                name: c.name,
                tier: c.tier,
                assigns: doc.assigns,
                fallback: doc.fallback,
                claimed_new_keys: c.claimed_new_keys,
                claimed_param_changes: c.claimed_param_changes,
            }
        })
        .collect()
}

/// Q1：越界键（= 必须新增机制 = 重构）
fn out_of_bounds(roster: &[Stage]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for s in roster {
        for (k, v) in s.assigns {
            if !MECHANISMS.contains(k) {
                bad.push((s.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// 截至第 upto 阶（含）累计用过的机制键
fn cumulative_keys(roster: &[Stage], upto: usize) -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::new();
    for s in &roster[..upto] {
        for (k, _) in s.assigns {
            if !v.contains(k) {
                v.push(*k);
            }
        }
    }
    v
}

/// 逐阶新增键数量：由累计差算出，不是写死的常量表
fn new_keys_per_stage(roster: &[Stage]) -> Vec<usize> {
    (0..roster.len())
        .map(|i| cumulative_keys(roster, i + 1).len() - cumulative_keys(roster, i).len())
        .collect()
}

/// Q3：J2 违例 —— 退路口给的平凡值就是本阶的运行值，等于没有回退
fn j2_violations(roster: &[Stage]) -> Vec<&'static str> {
    roster
        .iter()
        .filter(|s| {
            s.assigns
                .iter()
                .any(|(k, v)| s.fallback.0 == *k && s.fallback.1 == *v)
        })
        .map(|s| s.id)
        .collect()
}

/// Q5：越界取值 = 需要一次值域扩展。值域与匹配规则都在 `facts`（[01 §8](../01-核心架构.md)
/// 那张表的生成物，判据见 `facts::in_domain`）。没有值域行的键不在这里报——那正是 Q1 的
/// 越界键，同一处红不该有两种说法。
fn out_of_domain(roster: &[Stage]) -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for s in roster {
        for &(k, v) in s.assigns.iter().chain(std::iter::once(&s.fallback)) {
            if domain_of(k).is_some() && !in_domain(k, v) {
                bad.push((s.id.to_string(), format!("{} = {}", k, v)));
            }
        }
    }
    bad
}

/// Q2：总表声明与算出不符的阶——「同一事实写两处」的可执行形式
fn claim_mismatches(roster: &[Stage], computed: &[usize]) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, s) in roster.iter().enumerate() {
        if computed[i] != s.claimed_new_keys {
            bad.push(format!(
                "{} 新增键：总表声明 {}，由赋值块算出 {}",
                s.id, s.claimed_new_keys, computed[i]
            ));
        }
        if s.assigns.len() != s.claimed_param_changes {
            bad.push(format!(
                "{} 参数变化：总表声明 {}，§3 赋值块 {} 行",
                s.id,
                s.claimed_param_changes,
                s.assigns.len()
            ));
        }
    }
    bad
}

fn main() {
    println!("═══ 能力扩充阶梯审计 ═══\n");
    let roster = stages();
    println!(
        "阶数：{}（名册 = 总表行数 ∩ roadmap 阶文件数，二者按 id 配对）\n",
        roster.len()
    );

    let nk = new_keys_per_stage(&roster);
    let cum_all = cumulative_keys(&roster, roster.len()).len();
    println!("| 阶 | 场景 | 天梯 | 新增键（算出） | 总表声明 | 累计 |");
    println!("|---|---|:-:|:-:|:-:|:-:|");
    for (i, s) in roster.iter().enumerate() {
        let cum = cumulative_keys(&roster, i + 1).len();
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            s.id, s.name, s.tier, nk[i], s.claimed_new_keys, cum
        );
    }

    let bad = out_of_bounds(&roster);
    let odv = out_of_domain(&roster);
    let mism = claim_mismatches(&roster, &nk);
    let tail_zero = nk[nk.len() - 5..].iter().all(|n| *n == 0);
    let zero_stages = nk.iter().filter(|n| **n == 0).count();

    println!("\n── Q1 越界键（= 需新增机制 = 重构）：{} 个", bad.len());
    for b in &bad {
        println!("   {} → {}", b.0, b.1);
    }
    println!("── Q2 声明与算出的差集：{} 处", mism.len());
    for m in &mism {
        println!("   {}", m);
    }
    println!("── Q2 累计机制键：{} / 机制表 {} 项", cum_all, MECHANISMS.len());
    println!("── 末 5 阶新增键全为 0：{}", tail_zero);
    println!("── 零新增键的阶数：{} / {}", zero_stages, roster.len());
    println!("\n── Q5 越界取值（= 值域扩展）：{} 个", odv.len());
    for (id, kv) in &odv {
        println!("   {} → {}", id, kv);
    }
    // 开放值域如实报出来：读者要能看出「这一格没判」，而不是以为整张表都判了。
    let open: Vec<&str> = DOMAINS.iter().filter(|d| d.open).map(|d| d.key).collect();
    println!(
        "── Q5 判据覆盖 {} 个键，其中开放值域（§8 写 `*`，不判取值）：{}",
        DOMAINS.len(),
        if open.is_empty() {
            "无".to_string()
        } else {
            open.join(", ")
        }
    );

    // ── 反向用例 1：加一阶需要新机制 → 越界键必须抓到 ──
    let fake_assigns: &[(&str, &str)] = &[("meta.architecture", "self-modifying")];
    let mut with_new = roster.clone();
    with_new.push(Stage {
        id: "S98",
        file: "（反例）不在名册里",
        name: "（反例）主体自改架构",
        tier: "T9",
        assigns: fake_assigns,
        fallback: ("scope", "root"),
        claimed_new_keys: 1,
        claimed_param_changes: 1,
    });
    let bad_new = out_of_bounds(&with_new);

    // ── 反向用例 2：退路口给的就是运行值 → J2 必须抓到 ──
    let fake_assigns2: &[(&str, &str)] = &[("actor.pattern", "reasoner")];
    let mut with_fake = roster.clone();
    with_fake.push(Stage {
        id: "S99",
        file: "（反例）不在名册里",
        name: "（反例）假回退",
        tier: "T1",
        assigns: fake_assigns2,
        fallback: ("actor.pattern", "reasoner"),
        claimed_new_keys: 0,
        claimed_param_changes: 1,
    });
    let j2_new = j2_violations(&with_fake);

    // ── 反向用例 3：删掉点亮 `scope` 的那一阶 → 累计键必须掉下来 ──
    //    这一条钉的是「覆盖由遍历算出」：名册少一阶而结论不变，就说明结论是抄来的。
    let scope_at = roster
        .iter()
        .position(|s| s.assigns.iter().any(|(k, _)| *k == "scope"))
        .expect("反例前提：名册里得有一阶点亮 scope");
    let mut minus = roster.clone();
    minus.remove(scope_at);
    let cum_minus = cumulative_keys(&minus, minus.len()).len();

    // ── 反向用例 4：总表那一列虚增一格 → 声明差集必须非空 ──
    let mut drifted = roster.clone();
    drifted[0].claimed_new_keys += 1;
    let drift = claim_mismatches(&drifted, &nk);

    // ── 反向用例 5：注入一个越界取值 → Q5 必须抓到，且只抓它 ──
    //    同一阶里放两个**合法**形态做对照：通配段（`child:<id>`）与子键参数
    //    （`recall:tag=judgment`）都得放行——只测「抓得住」的守卫可以靠
    //    「逢取值即判越界」拿到绿灯，那和当初那个永真标志一样是假证据。
    let fake_assigns5: &[(&str, &str)] = &[
        ("store", "sqlite"),
        ("scope", "child:task-7"),
        ("projection", "recall:tag=judgment"),
    ];
    let mut with_domain = roster.clone();
    with_domain.push(Stage {
        id: "S97",
        file: "（反例）不在名册里",
        name: "（反例）换了个存储后端",
        tier: "T2",
        assigns: fake_assigns5,
        fallback: ("actor.pattern", "decider"),
        claimed_new_keys: 0,
        claimed_param_changes: 3,
    });
    let odv_new = out_of_domain(&with_domain);

    println!("\n── 反向用例 ──");
    println!("  加一阶需要新机制 → 越界键 {} 个", bad_new.len());
    println!("  加一阶假回退 → J2 违例 {} 个", j2_new.len());
    println!(
        "  删掉点亮 scope 的 {}（{}）→ 累计键 {} → {}",
        roster[scope_at].id, roster[scope_at].file, cum_all, cum_minus
    );
    println!("  总表声明虚增一格 → 声明差集 {} 处", drift.len());
    println!(
        "  注入 `store = sqlite`（同阶另带两个合法取值）→ 越界取值 {} 个",
        odv_new.len()
    );

    // ── 断言 ──
    assert!(
        !roster.is_empty(),
        "名册为空：总表或 roadmap 目录读不出来，绿灯是假的"
    );
    assert_eq!(
        STAGE_CLAIMS.len(),
        STAGE_DOCS.len(),
        "总表行数与 roadmap 阶文件数不等——有一阶只存在于一处"
    );
    assert!(bad.is_empty(), "有任何一阶需要新增机制键");
    assert!(
        mism.is_empty(),
        "总表声明的逐阶新增键数 / 参数变化数与算出的值不符"
    );
    assert_eq!(
        cum_all,
        MECHANISMS.len(),
        "全梯累计必须点亮整张机制表——少一阶就少一个键，这张阶梯没铺完"
    );
    assert!(tail_zero, "末 5 阶新增键必须全为 0");
    assert!(
        j2_violations(&roster).is_empty(),
        "每一阶的退路口都必须是个不同于运行值的平凡值"
    );
    assert!(
        MECHANISMS.iter().all(|k| DOMAINS.iter().any(|d| d.key == *k)),
        "机制表里每个键都要有取值域行——漏一行的键等于没判"
    );
    assert!(
        odv.is_empty(),
        "有任何一阶取了 01 §8 值域之外的值——那是**值域扩展**，得先改那张表并走 ADR"
    );

    assert_eq!(bad_new.len(), 1, "反向用例：需要新机制的那一阶必须被抓到");
    assert!(
        bad_new[0].1.contains("meta.architecture"),
        "反向用例：抓到的必须是那个新键"
    );
    assert_eq!(j2_new.len(), 1, "反向用例：假回退必须被 J2 抓到");
    assert_eq!(j2_new[0], "S99", "反向用例：抓到的必须是那一阶");
    assert!(
        cum_minus < cum_all,
        "反向用例：删掉点亮 scope 的一阶后，累计键数必须下降"
    );
    assert_eq!(drift.len(), 1, "反向用例：总表与算出值差一格必须被报出来");
    assert_eq!(
        odv_new.len(),
        1,
        "反向用例：越界取值必须被抓到，且同阶的通配段与子键参数取值不得被误伤"
    );
    assert_eq!(odv_new[0].0, "S97", "反向用例：抓到的必须是那一阶");
    assert!(
        odv_new[0].1.contains("store = sqlite"),
        "反向用例：报出的必须是那个越界取值"
    );

    println!("\n✅ 全部断言通过（含 5 条反向用例）。");
    println!(
        "   全梯 {} 阶 = 0 新增机制键 + 0 越界取值 + 0 声明不符；末 5 阶只取新值，不引入新键。",
        roster.len()
    );
}
