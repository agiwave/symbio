//! 五轴的独立性：极性冲突矩阵（可分性）+ 共现测量（可登记的共线）
//!
//! [DE-008 §3](../008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") 给每条轴两列**极性**——它 `requires` 什么性质、
//! `forbids` 什么性质。这两条不是描述，是判据：
//!
//! | 情形 | 读法 | 处置 |
//! |---|---|---|
//! | A 的 requires == B 的 forbids（或反向） | 两条轴互相抵消，不能各自独立度量 | 冲突，必须合并或改写极性 |
//! | A 的 requires == B 的 requires 且 forbids 相同 | 它们其实是**同一条轴**登记了两次 | 重复，必须合并 |
//! | 其余 | 在声明的极性下可分 | ——
//!
//! 冲突矩阵由**程序算出**，分组数也是算出来的（做法与 `lower_bound.rs` 的职责冲突
//! 矩阵同源：极性是数据，结论是遍历的结果）。声明的五条轴必须在声明的极性下两两可分，
//! 否则「五轴足以度量高阶」这句话在声明的极性上就已经破了。
//!
//! **正交性证明不了，所以另一半是测量**：程序统计「一对轴在 T1–T8 里有几阶同时被抬起」，
//! 超过 [DE-008 §3.3](../008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系") 的上限即为**超限**；超限的轴对必须在该节逐条登记
//! 理由，声明的轴对集合与算出的超限集合必须**完全相同**，且每条声明的阶数必须等于算出的值。
//!
//! 两条纪律：输入不含结论（极性 / 上限 / 登记全来自 `facts`，即 02 的表）·
//! 每条断言配反向用例（改极性 / 降上限 / 塞一条假登记，结论都必须变）。
//!
//! 编译运行：rustc --edition 2021 axis_orthogonality.rs -o ao && ./ao

mod facts;
use facts::{AXES, AXIS_POLARITIES, CAPABILITIES, DECLARED_OVERLAP, OVERLAP_LIMIT, TIER_MAX};

type Pol = facts::AxisPolarity;

/// 两条轴是否**互相抵消**：A 要求的正是 B 禁止的（任一方向）。
fn cancels(a: &Pol, b: &Pol) -> bool {
    a.requires == b.forbids || b.requires == a.forbids
}

/// 两条轴是不是**同一条轴**登记了两次。
fn same_axis(a: &Pol, b: &Pol) -> bool {
    a.requires == b.requires && a.forbids == b.forbids
}

/// 冲突 / 重复对：`(i, j, 读法)`
fn clashes(pols: &[Pol]) -> Vec<(usize, usize, &'static str)> {
    let mut out = Vec::new();
    for i in 0..pols.len() {
        for j in (i + 1)..pols.len() {
            if same_axis(&pols[i], &pols[j]) {
                out.push((i, j, "同一条轴登记了两次"));
            } else if cancels(&pols[i], &pols[j]) {
                out.push((i, j, "要求命中对方的禁止：互相抵消"));
            }
        }
    }
    out
}

/// 不可分的轴聚成一组（并查集）——组数是**算出来**的，不是写死的。
fn groups_of(pols: &[Pol]) -> Vec<Vec<usize>> {
    let n = pols.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(p: &mut Vec<usize>, mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for (i, j, _) in clashes(pols) {
        let (a, b) = (root(&mut parent, i), root(&mut parent, j));
        if a != b {
            parent[a] = b;
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        match groups.iter_mut().find(|g| g[0] == r) {
            Some(g) => g.push(i),
            // 组用**代表元**开头（不是用首个成员 i）：否则并查集里被合并到别人的
            // 那个成员会另起一组，分组数就永远等于轴数——这条判据会静默地变成永真。
            None => groups.push(vec![r, i]),
        }
    }
    groups.sort_by_key(|g| g[0]);
    groups
}

/// 一对轴在 T1–T{n} 里有几阶**同时**被抬起（两条轴在这一阶各有至少一条名册行）。
fn co_occurrence(a: &'static str, b: &'static str) -> usize {
    (1..=TIER_MAX)
        .filter(|t| {
            CAPABILITIES.iter().any(|c| c.tier == *t && c.axis == a)
                && CAPABILITIES.iter().any(|c| c.tier == *t && c.axis == b)
        })
        .count()
}

/// 算出「超过上限的轴对」：`(轴, 轴, 阶数)`
fn over_limit(limit: usize) -> Vec<(&'static str, &'static str, usize)> {
    let mut out = Vec::new();
    for (i, a) in AXES.iter().enumerate() {
        for b in AXES.iter().skip(i + 1) {
            let n = co_occurrence(a, b);
            if n > limit {
                out.push((*a, *b, n));
            }
        }
    }
    out
}

/// §3.3 登记的超限轴对与算出的**差集**（两个方向都要报——多登记与漏登记同样是说假话）。
fn overlap_diff(limit: usize) -> (Vec<String>, Vec<String>) {
    let computed = over_limit(limit);
    let has = |a: &str, b: &str| computed.iter().any(|(x, y, _)| (*x == a && *y == b) || (*x == b && *y == a));
    let mut undeclared = computed
        .iter()
        .filter(|(a, b, _)| !DECLARED_OVERLAP.iter().any(|(x, y, _)| (*x == *a && *y == *b) || (*x == *b && *y == *a)))
        .map(|(a, b, n)| format!("{a} × {b} 同时抬起 {n} 阶（>{limit}）但 §3.3 没登记"))
        .collect::<Vec<_>>();
    let mut stale = DECLARED_OVERLAP
        .iter()
        .filter(|(a, b, _)| !has(a, b))
        .map(|(a, b, n)| format!("§3.3 登记了 {a} × {b}（声明 {n} 阶），但算出来并没有超限"))
        .collect::<Vec<_>>();
    for (a, b, n) in DECLARED_OVERLAP {
        let real = co_occurrence(a, b);
        if real != *n {
            undeclared.push(format!("§3.3 声明 {a} × {b} 是 {n} 阶，算出来是 {real} 阶"));
        }
    }
    undeclared.extend(stale.drain(..));
    (undeclared, stale)
}

fn main() {
    let pols: Vec<Pol> = AXIS_POLARITIES.iter().copied().collect();

    println!("═══ 五轴独立性 · 极性冲突矩阵 ═══");
    println!("声明的轴：{}", pols.iter().map(|p| p.axis).collect::<Vec<_>>().join(" · "));
    for p in &pols {
        println!("  {:<12} requires `{}` · forbids `{}`", p.axis, p.requires, p.forbids);
    }

    let bad_pairs = clashes(&pols);
    let groups = groups_of(&pols);
    println!("\n两两检查：{} 对，冲突 / 重复 {} 对", pols.len() * (pols.len() - 1) / 2, bad_pairs.len());
    for (i, j, why) in &bad_pairs {
        println!("  {} × {} —— {why}", pols[*i].axis, pols[*j].axis);
    }
    println!("不可分的组：{} 组", groups.len());
    for g in &groups {
        println!(
            "  组 {}：{}",
            g.iter().map(|i| pols[*i].axis).collect::<Vec<_>>().join(" + "),
            if g.len() == 1 { "（独立）" } else { "（不可分）" }
        );
    }

    println!("\n═══ 五轴独立性 · 共现测量（T1–T{TIER_MAX}，同一对轴被抬起的阶数）═══");
    print!("  {:<14}", "");
    for b in AXES {
        print!("{b:>14}  ");
    }
    println!();
    for a in AXES {
        print!("  {a:<14}");
        for b in AXES {
            if a == b {
                print!("{:>14}  ", "—");
            } else {
                let n = co_occurrence(a, b);
                print!("{n:>14}  ");
            }
        }
        println!();
    }
    let computed_over = over_limit(OVERLAP_LIMIT);
    println!(
        "\n上限 {OVERLAP_LIMIT} 阶 ⇒ 超限轴对 {} 对（声明 {} 对）",
        computed_over.len(),
        DECLARED_OVERLAP.len()
    );
    for (a, b, n) in &computed_over {
        println!("  {a} × {b}：{n} 阶");
    }
    let (mismatch, _) = overlap_diff(OVERLAP_LIMIT);

    // ── 反向用例 1：把某条轴的禁止改成另一条轴的要求 ⇒ 必须出现抵消 ──
    let mut flipped: Vec<Pol> = pols.clone();
    let victim = flipped.len() - 1;
    flipped[victim].forbids = flipped[0].requires;
    let flip_caught = clashes(&flipped).len() > bad_pairs.len()
        && groups_of(&flipped).len() < groups_of(&pols).len();

    // ── 反向用例 2：把某条轴的极性抄成另一条 ⇒ 必须判为同一条轴 ──
    let mut dup: Vec<Pol> = pols.clone();
    let last = dup.len() - 1;
    let (r0, f0) = (dup[0].requires, dup[0].forbids);
    dup[last].requires = r0;
    dup[last].forbids = f0;
    let dup_caught = clashes(&dup).iter().any(|(_, _, why)| *why == "同一条轴登记了两次");

    // ── 反向用例 3：把上限调低一阶 ⇒ 算出的超限对变多，声明必然对不上 ──
    let (strictly_worse, _) = overlap_diff(OVERLAP_LIMIT.saturating_sub(1));
    let stricter_caught = strictly_worse.len() > mismatch.len();

    // ── 反向用例 4：登记一对并不超限的轴 ⇒ 多登记必须被抓到 ──
    let fake_pair = AXES
        .iter()
        .enumerate()
        .flat_map(|(i, a)| AXES.iter().skip(i + 1).map(move |b| (*a, *b)))
        .find(|(a, b)| {
            co_occurrence(a, b) <= OVERLAP_LIMIT
                && !DECLARED_OVERLAP.iter().any(|(x, y, _)| *x == *a && *y == *b)
        })
        .expect("至少有一对轴没超限且没登记（否则这条判据测不到）");
    let fake_n = co_occurrence(fake_pair.0, fake_pair.1);
    let with_fake: Vec<(&str, &str, usize)> = DECLARED_OVERLAP
        .iter()
        .copied()
        .chain(std::iter::once((fake_pair.0, fake_pair.1, fake_n)))
        .collect();
    let stale_pairs: Vec<(&str, &str, usize)> = with_fake
        .iter()
        .copied()
        .filter(|(a, b, _)| {
            !over_limit(OVERLAP_LIMIT)
                .iter()
                .any(|(x, y, _)| (*x == *a && *y == *b) || (*x == *b && *y == *a))
        })
        .collect();
    let bogus_caught = !stale_pairs.is_empty();

    println!(
        "\n── 反向用例：极性被改成互相抵消 = {flip_caught}，极性抄成重复 = {dup_caught}，\n   上限调低一阶 ⇒ 漏登记被抓到 = {stricter_caught}，登记一对不超限的轴 = {bogus_caught}"
    );
    for m in &mismatch {
        println!("   对账失配：{m}");
    }

    assert!(
        !AXIS_POLARITIES.is_empty(),
        "DE-008 §3 的极性列为空——没有极性就无从判断可分性"
    );
    assert!(
        AXIS_POLARITIES.len() == AXES.len(),
        "每条轴都要有极性：轴 {} 条、极性 {} 条",
        AXES.len(),
        AXIS_POLARITIES.len()
    );
    assert!(
        bad_pairs.is_empty(),
        "声明的五轴必须在声明的极性下两两可分：{bad_pairs:?}"
    );
    assert_eq!(
        groups.len(),
        AXES.len(),
        "不可分的组数必须等于轴数（{}）——少即有两条轴其实是一条",
        AXES.len()
    );
    assert!(
        mismatch.is_empty(),
        "§3.3 的超限登记必须与算出的超限轴对逐条相等：{mismatch:?}"
    );
    assert!(flip_caught, "反向用例：把禁止改成对方的要求 ⇒ 必须出现抵消");
    assert!(dup_caught, "反向用例：把极性抄成与另一条相同 ⇒ 必须判为同一条轴");
    assert!(stricter_caught, "反向用例：上限调低一阶 ⇒ 漏登记必须被抓到");
    assert!(bogus_caught, "反向用例：登记一对并不超限的轴 ⇒ 必须被抓到");

    println!("\n✅ 全部断言通过（含 4 条反向用例）：极性下五轴两两可分、组数 = 轴数、");
    println!("   共现测量与 §3.3 的登记逐条相等。极性与上限在 02，判据在这里。");
}