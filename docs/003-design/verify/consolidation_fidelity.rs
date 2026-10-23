//! 巩固保真度 · 质量约束（取代纯"代数上界"形态约束）
//!
//! 本程序回答一个问题：
//!   **`consolidate:max_gen = 3` 能防住"巩固的巩固无限推进"吗？**
//!
//! 现状（plan3 v2 的 G7）：`max_gen` 是一个**形态约束** —— 它只说"最多巩固 3 代"。
//!   它不回答一个更要紧的问题：**跑到第 3 代，还剩多少信息？**
//!
//!   原始事件 (gen 0)    信息量 1.00
//!     → consolidated 1  信息量 0.6?
//!       → consolidated 2  0.35?
//!         → consolidated 3  0.2?   ← 代数合规（≤3），但这条"记忆"已经没用了
//!
//! 业界的对应事实（见 REF）：
//!   - 摘要是**有损压缩**，没有解压步骤能恢复原文（不像 gzip）；
//!   - 多跳派生会**累积漂移**，三到四跳后输出偏离源的程度**没有任何单独一步能察觉**；
//!   - 业界的经验阈值不是相似度数字，而是**跳数**：1 跳可缓存、2 跳需周期重算、**≥3 跳精度任务必须从源重推**。
//!
//! 因此本方案在 `max_gen` 之外补一个**质量约束**：
//!   `consolidate:min_fidelity = 0.7`
//!   —— 若某次巩固产出的条目与**其源事件**的相似度低于 0.7，则**拒绝产出该条事件**。
//!   加固后，"最多巩固 3 次"变成"**巩固到信息失真为止**"—— 后者更接近生物记忆的真实机制。
//!
//! 论证纪律：每条断言配反向用例（改输入，结论必须变）。
//!
//! ## 两段式：概念链 + 生产内核（ 批①）
//!
//! 本程序分两段，各答各的问题，**不互相冒充**：
//!
//! - **概念链**（本文件原有）：特征向量域上的抽象模型，回答「min_fidelity 这个
//!   机制在不在算、阈值敏不敏感、拒绝理由可不可区分」——那是**设计论证**；
//! - **生产内核段**（`main` 末尾，`#[path]` 包含生产
//!   [`symbio/src/symbio_core/text/merge_kernel.rs`](../../../symbio/src/symbio_core/text/merge_kernel.rs)）：
//!   跑的是生产这条保真度算法本身——**字符域的保留率**，与概念链的向量域读数
//!   互相印证（同一现象：逐代下降、阈值前停）。没有这一段，本程序论证的机制
//!   与生产实现的闸门（`memory_events::consolidate` → `accept`）之间就只剩一句
//! 修辞——那正是 [] 黑盒纪律点名要防的「测自己 mock 的想象行为」。
//!
//! 编译运行：rustc --edition 2021 consolidation_fidelity.rs -o cf && ./cf

use std::collections::HashMap;

// 生产内核（ 批①）：`#[path]` 指向仓库里的生产文件——**这一行就是「跑的是
// 生产」的全部含义**（与 `recall_eval.rs` 同一条纪律）。它是 `memory_events::consolidate`
// 调用的同一份源码：分句 / 合并 / 保真度。
#[path = "../../../symbio/src/symbio_core/text/merge_kernel.rs"]
mod merge_kernel;

// ───────────────────────── 领域类型 ─────────────────────────

pub type Seq = u64;

/// 一条记忆条目（简化为：id + 特征向量 + 代数 + 溯源到源事件）
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryItem {
    pub seq: Seq,
    /// 语义特征（这里用低维整数向量代替 embedding，便于确定性验证）
    pub features: Vec<i32>,
    /// 派生代数：原始事件为 0，每巩固一次 +1
    pub gen: u32,
    /// 溯源：本条是从哪条事件派生的（指向更早的 seq，保证无环）
    pub derived_from: Option<Seq>,
    pub tag: &'static str,
}

/// 保真度：**信息保留率**（不是余弦相似度）。
///
/// 为什么不能用余弦相似度：它**尺度不变**——向量整体除以 2，余弦相似度仍是 1.0。
/// 但"数值精度从 97 掉到 48"恰恰是 [REF-1] 说的第一种失效模式（数值精度损失），
/// 用余弦度会把它**完全掩掉**，于是"只靠代数上界会发生塌陷"这个现象就复现不出来。
///
/// 正确做法：保真度衡量"**原始信号的多少比例还在**"，包含两个分量：
///   1. **精度分量**：逐维度的相对误差（数值精度损失）
///   2. **区分度分量**：与全局均值的差距是否还在（条件性细节崩塌 / 归因抹除）
pub fn similarity(a: &[i32], b: &[i32]) -> f64 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let n = a.len() as f64;
    let mean_b: f64 = b.iter().map(|v| *v as f64).sum::<f64>() / n;

    // 分量 1：精度 —— 维度 \(i\) 的相对误差（以源的量纲为分母）
    let mut precision = 0.0f64;
    // 分量 2：区分度 —— 该维度"偏离均值的程度"保留了多少
    let mut distinct = 0.0f64;

    for i in 0..a.len() {
        let (av, bv) = (a[i] as f64, b[i] as f64);
        let denom = bv.abs().max(1.0);
        let rel_err = (av - bv).abs() / denom;
        precision += (1.0 - rel_err).max(0.0);

        let b_dev = (bv - mean_b).abs();
        let a_dev = (av - mean_b).abs();
        distinct += if b_dev < 1e-9 {
            1.0
        } else {
            (a_dev / b_dev).min(1.0)
        };
    }
    let p = precision / n;
    let d = distinct / n;
    // 两个分量都重要：精度掉了或区分度掉了，保真度都掉
    0.5 * p + 0.5 * d
}

// ─────────────────── 巩固策略：max_gen + min_fidelity 双约束 ───────────────────

/// 巩固参数（对应 01 §8 权威参数表的 `projection` 行取值扩展）
#[derive(Debug, Clone, Copy)]
pub struct ConsolidatePolicy {
    /// 形态约束：代数上界（plan3 已有）
    pub max_gen: u32,
    /// **质量约束：保真度下界（本方案新增）**
    pub min_fidelity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConsolidateOutcome {
    /// 产出巩固条目
    Produced(MemoryItem),
    /// 拒绝：超过代数上界
    RejectedMaxGen { gen: u32, max_gen: u32 },
    /// 拒绝：保真度不足（**新增的拒绝理由**）
    RejectedLowFidelity { fidelity: f64, min_fidelity: f64 },
    /// 拒绝：源不存在
    RejectedNoSource { seq: Seq },
}

/// 巩固：把一条记忆"压一压"（模拟摘要：丢掉部分细节维度 + 其余维度粗量化）
///
/// 关键：压缩是**有损且不可逆**的 —— 每次巩固都会让特征偏离源，这正是要治理的现象。
///
/// 为什么不能写成"取整到 3 的倍数"：那个函数是**幂等**的（`(x/3*3)/3*3 == x/3*3`），
/// 跑第二遍不再损失任何信息，于是保真度永远停在 0.9999 —— 断言会立刻抓到这个错误。
/// 真实的摘要是**不可逆**的（gzip 可以解压，摘要不能），核心危害是**归因抹除**：
/// 细节维度被删掉后，余弦相似度的**方向**就变了，而不是简单地整体缩小。
///
/// 建模原则：**保真度必须随代数单调、渐进地下降**，这样"阈值"才有分辨力
/// （若一代就掉到 0，任何阈值都退化成同一个行为，反向用例无意义）。
/// 这里用"每代抹掉一个维度、并把特征朝均值收缩"来模拟：
///   - 抹维度 → 归因抹除（REF-1 失效模式之二）
///   - 朝均值收缩 → 数值精度损失（失效模式之一）
fn compress(item: &MemoryItem, src: &MemoryItem) -> Vec<i32> {
    let n = src.features.len();
    let alive = n.saturating_sub(item.gen as usize).max(1); // 每代少一个"活着的"维度
    let src_mean = (src.features.iter().map(|v| *v as i64).sum::<i64>() / n as i64) as i32;

    (0..n)
        .map(|i| {
            if i < alive {
                // 活着的维度：向源均值轻微靠拢一档（3:1 保留 → 渐进量化，非一步归零）
                let v = *item.features.get(i).unwrap_or(&0);
                (v * 3 + src_mean) / 4
            } else {
                // 抹掉的维度：只剩均值（该细节的归因已丢失）
                src_mean
            }
        })
        .collect()
}

/// 执行一次巩固，带**双约束**判定。
///
/// 判定顺序（有意为之）：
///   1. 代数上界（形态）—— 先挡掉显然越界的
///   2. 保真度下界（质量）—— **再挡掉"代数合规但已经失真"的**
pub fn consolidate(
    item: &MemoryItem,
    source: Option<&MemoryItem>,
    p: &ConsolidatePolicy,
) -> ConsolidateOutcome {
    let next_gen = item.gen + 1;

    // 约束 1：代数上界
    if next_gen > p.max_gen {
        return ConsolidateOutcome::RejectedMaxGen { gen: next_gen, max_gen: p.max_gen };
    }

    // 约束 2：保真度下界 —— 与**源**（gen 0 的原始事件）比，而不是与上一代比
    let src = match source {
        Some(s) => s,
        None => return ConsolidateOutcome::RejectedNoSource { seq: item.seq },
    };
    let new_features = compress(item, src);
    let fidelity = similarity(&new_features, &src.features);
    if fidelity < p.min_fidelity {
        return ConsolidateOutcome::RejectedLowFidelity {
            fidelity,
            min_fidelity: p.min_fidelity,
        };
    }

    ConsolidateOutcome::Produced(MemoryItem {
        seq: item.seq + 1,
        features: new_features,
        gen: next_gen,
        derived_from: Some(item.seq),
        tag: "consolidated",
    })
}

/// 模拟"无上界的巩固链"：反复巩固直到被拒绝或达到硬性迭代上限。
/// 返回 (最终条目, 实际代数, 每一代的保真度序列)
pub fn run_chain(
    origin: &MemoryItem,
    p: &ConsolidatePolicy,
    hard_limit: u32,
) -> (MemoryItem, Vec<f64>) {
    let mut cur = origin.clone();
    let mut fidelities = vec![1.0]; // gen 0 的保真度定义为 1.0（它就是源）
    for _ in 0..hard_limit {
        match consolidate(&cur, Some(origin), p) {
            ConsolidateOutcome::Produced(next) => {
                let f = similarity(&next.features, &origin.features);
                fidelities.push(f);
                cur = next;
            }
            _ => break,
        }
    }
    (cur, fidelities)
}

// ───────────────────────── 主程序 ─────────────────────────

fn origin_item() -> MemoryItem {
    MemoryItem {
        seq: 100,
        // 一条信息量较高的原始事件（多维度、数值带精度）
        features: vec![97, 43, 88, 12, 65, 31, 74, 56],
        gen: 0,
        derived_from: None,
        tag: "raw",
    }
}

fn main() {
    println!("═══ 巩固保真度 · 质量约束（max_gen 之外补 min_fidelity）═══");

    let origin = origin_item();

    // ── 1. 现象：只有代数上界时，代数合规但信息已失真 ──
    println!("\n── 现象：只有 max_gen 时会发生什么 ──");
    let only_maxgen = ConsolidatePolicy { max_gen: 5, min_fidelity: 0.0 };
    let (final_item, fids) = run_chain(&origin, &only_maxgen, 10);
    for (g, f) in fids.iter().enumerate() {
        println!("  gen {}: 保真度 = {:.4}", g, f);
    }
    println!("  最终代数 = {}（≤ max_gen=5，形态合规）", final_item.gen);
    println!("  最终保真度 = {:.4}  ← 代数合规，但这条记忆还准吗？", fids.last().unwrap());
    assert_eq!(final_item.gen, 5, "无质量约束时应跑满代数上界");
    assert!(*fids.last().unwrap() < 0.5,
            "只靠代数上界时，最后一代的保真度必然塌陷（这是要被治理的现象）");

    // ── 2. 加固：min_fidelity 让它"巩固到失真为止" ──
    println!("\n── 加固：加 min_fidelity = 0.7 ──");
    let with_fidelity = ConsolidatePolicy { max_gen: 5, min_fidelity: 0.7 };
    let (f2_item, f2) = run_chain(&origin, &with_fidelity, 10);
    for (g, f) in f2.iter().enumerate() {
        println!("  gen {}: 保真度 = {:.4}", g, f);
    }
    println!("  最终代数 = {}（**小于** max_gen=5 —— 质量约束先于形态约束生效）", f2_item.gen);
    println!("  最终保真度 = {:.4} ≥ 0.7", f2.last().unwrap());
    assert!(f2_item.gen < 5, "质量约束必须先于形态约束生效（代数提前停止）");
    assert!(*f2.last().unwrap() >= 0.7, "停下的那一代保真度必须仍达标");
    assert!(f2_item.gen < final_item.gen,
            "反向用例：加质量约束后，代数必须比只有形态约束时**更少**");

    // ── 3. 反向用例 A：放宽阈值，必须能多走一代（阈值真的在算）──
    println!("\n── 反向用例 A：改阈值，结论必须变 ──");
    for thr in [0.9, 0.7, 0.5, 0.3] {
        let p = ConsolidatePolicy { max_gen: 10, min_fidelity: thr };
        let (it, _) = run_chain(&origin, &p, 10);
        println!("  min_fidelity={:.1} → 实际代数 = {}", thr, it.gen);
    }
    let strict = run_chain(&origin, &ConsolidatePolicy { max_gen: 10, min_fidelity: 0.9 }, 10).0.gen;
    let loose = run_chain(&origin, &ConsolidatePolicy { max_gen: 10, min_fidelity: 0.3 }, 10).0.gen;
    assert!(loose > strict, "反向用例失败：放宽阈值却没有多走代数 = 阈值没在算");
    println!("  严格(0.9) = {} 代 < 宽松(0.3) = {} 代 → 阈值确实在参与计算", strict, loose);

    // ── 4. 反向用例 B：min_fidelity = 0 必须等价于"只有 max_gen" ──
    println!("\n── 反向用例 B：min_fidelity 的平凡值 ──");
    let degenerate = ConsolidatePolicy { max_gen: 5, min_fidelity: 0.0 };
    let (d_item, _) = run_chain(&origin, &degenerate, 10);
    let (m_item, _) = run_chain(&origin, &only_maxgen, 10);
    println!("  min_fidelity=0.0 → 代数 = {}（应等于纯 max_gen 的 {}）", d_item.gen, m_item.gen);
    assert_eq!(d_item.gen, m_item.gen,
               "J2 平凡值：min_fidelity=0.0 必须退化为\"只有代数上界\"，行为完全一致");

    // ── 5. 拒绝理由可区分（这是 J3 的关键：拒绝必须留痕）──
    println!("\n── 拒绝理由可区分（不静默）──");
    let strict_p = ConsolidatePolicy { max_gen: 10, min_fidelity: 0.95 };
    let mut cur = origin.clone();
    let mut reasons: HashMap<&str, u32> = HashMap::new();
    for _ in 0..6 {
        match consolidate(&cur, Some(&origin), &strict_p) {
            ConsolidateOutcome::Produced(n) => {
                *reasons.entry("produced").or_insert(0) += 1;
                cur = n;
            }
            ConsolidateOutcome::RejectedLowFidelity { fidelity, min_fidelity } => {
                println!("  拒绝：保真度 {:.4} < 下界 {:.4}", fidelity, min_fidelity);
                *reasons.entry("low_fidelity").or_insert(0) += 1;
                break;
            }
            ConsolidateOutcome::RejectedMaxGen { gen, max_gen } => {
                println!("  拒绝：代数 {} > 上界 {}", gen, max_gen);
                *reasons.entry("max_gen").or_insert(0) += 1;
                break;
            }
            ConsolidateOutcome::RejectedNoSource { seq } => {
                println!("  拒绝：源缺失 seq={}", seq);
                *reasons.entry("no_source").or_insert(0) += 1;
                break;
            }
        }
    }
    assert!(reasons.contains_key("low_fidelity"),
            "保真度拒绝必须产生**可区分的理由**（J3：不能静默丢弃）");

    // ── 6. 溯源方向：派生只能指向更早的 seq（与 module_deps.rs 一致）──
    println!("\n── 溯源方向 ──");
    let (chain_item, _) = run_chain(&origin, &ConsolidatePolicy { max_gen: 3, min_fidelity: 0.0 }, 10);
    println!("  gen0 seq={} ← gen{} seq={}（溯源必须指向更早）", origin.seq, chain_item.gen, chain_item.seq);
    let mut walk = Some(&chain_item);
    let mut last_seq = chain_item.seq;
    while let Some(it) = walk {
        if let Some(d) = it.derived_from {
            assert!(d < it.seq || d == origin.seq,
                    "溯源必须指向更早的 seq（派生图无环的前提）");
            last_seq = d;
        }
        walk = None;
    }
    println!("  溯源链末端 seq = {} ≤ 起点 = {} ✓", last_seq, chain_item.seq);

    // ── 7. 生产内核段（ 批①）：跑的是生产这条保真度算法本身 ──
    println!("\n── 生产内核（#[path] 包含生产 merge_kernel.rs） ──");

    // 分句防护：裸 `.` 不当界（`1.5` 切开会把数字改坏）；换行是界；标点留在句内。
    let units = merge_kernel::split_sentences("版本 1.5 发布\n修复了内存泄漏。下一个版本");
    assert_eq!(units, vec!["版本 1.5 发布", "修复了内存泄漏。", "下一个版本"],
               "裸点不当界（1.5 不被切开）、换行断句、标点留在句内");
    // 英文句号只在后随空白或结尾时算界。
    assert_eq!(merge_kernel::split_sentences("done.Next"), vec!["done.Next"],
               "英文句点后无空白 ⇒ 不当界（域名 / 版本号同理）");

    // 装得下 ⇒ 1.0；同一件事两份都写 ⇒ 仍 1.0（去重省位置，不省信息）。
    let two = ["起床先喝水。", "周三不排会。"];
    let (merged, f) = merge_kernel::merge_with_fidelity(&two, &two, 100);
    assert_eq!(f, 1.0, "两源都装得下 ⇒ 保真度 1.0");
    assert_eq!(merged, "起床先喝水。\n周三不排会。", "按句合并，换行分隔");
    let dup = ["同一句话。", "同一句话。"];
    let (_, f_dup) = merge_kernel::merge_with_fidelity(&dup, &dup, 100);
    assert_eq!(f_dup, 1.0, "同文两份 ⇒ 句子都算保留（去重省的是位置不是信息）");

    // 装不下 ⇒ < 1.0，且保留靠前（与 recall 新近在前的口径一致）、丢的是尾部。
    let one = ["第一句。第二句。第三句。第四句。"];
    let (merged2, f2) = merge_kernel::merge_with_fidelity(&one, &one, 12);
    assert!(f2 < 1.0, "装不下必有失真：{f2}");
    assert_eq!(merged2, "第一句。\n第二句。", "保留靠前的句子，尾部被丢");

    // 空源 ⇒ 1.0（没有信息可丢，不是「丢光了」）。
    let empty = [""];
    let (empty_merged, f_empty) = merge_kernel::merge_with_fidelity(&empty, &empty, 100);
    assert_eq!(f_empty, 1.0, "空源 ⇒ 1.0");
    assert_eq!(empty_merged, "", "空源合成为空串");

    // ── 跨代判据：basis 恒为最初源，连续三代后产物与源可分 ──
    // 每源 15 句 × 4 字符；上限 130 只能装下 26 句（第一代就丢掉尾部）。
    // 之后每代再并一条新的短记忆，上限装不下它 ⇒ 当次读数仍很高而累积读数在跌。
    let text = |prefix: char, n: usize| -> String {
        (0..n).map(|i| format!("{prefix}{i:02}。")).collect()
    };
    let (a, b) = (text('甲', 15), text('乙', 15));
    let (c, d) = (text('丙', 1), text('丁', 1));
    let (g1, f1) = merge_kernel::merge_with_fidelity(&[&a, &b], &[&a, &b], 130);
    let (g2, f2x) = merge_kernel::merge_with_fidelity(&[&g1, &c], &[&a, &b, &c], 130);
    let (g3, f3) = merge_kernel::merge_with_fidelity(&[&g2, &d], &[&a, &b, &c, &d], 130);
    println!("  跨代保真度：gen1 {f1:.3} → gen2 {f2x:.3} → gen3 {f3:.3}");
    assert!(f3 < f2x && f2x < f1, "与最初源比 ⇒ 累积保真度必须逐代下降");
    assert!(f3 >= 0.7, "下界之上仍允许巩固（三代都过了门）");
    assert!(
        g3.contains("甲00。") && !g3.contains("乙14。") && !g3.contains("丙00。"),
        "第三代产物只保留最初源的前缀，与最初源可分"
    );
    // 同一产物换坐标：只与当次两条源比，累积漂移会被读没。
    let (_, local3) = merge_kernel::merge_with_fidelity(&[&g2, &d], &[&g2, &d], 130);
    assert!(
        local3 > f3 + 0.1,
        "当次口径与最初源口径必须可区分：local={local3:.3} origin={f3:.3}"
    );

    // 压缩幂等 + 阈值敏感性（同坐标下的生产读数）。
    let three = ["甲。乙。丙。", "丁。戊。", "己。"];
    let (loose, f_loose) = merge_kernel::merge_with_fidelity(&three, &three, 6);
    let (again, _) = merge_kernel::merge_with_fidelity(&[&loose, "丁。戊。", "己。"], &three, 6);
    assert_eq!(again, loose, "同一上限下压缩幂等（产物再压不缩水）");
    let (_, f_tight) = merge_kernel::merge_with_fidelity(&three, &three, 3);
    assert!(f_tight < f_loose, "上限收紧 ⇒ 当次留存率必须下降（阈值真的在算）");

    // 截断按字符数（中文一字一字符），不切 UTF-8 字节。
    let long = "问".repeat(500);
    let cut = merge_kernel::truncate_chars(&long, 10);
    assert_eq!(cut.chars().count(), 10, "按字符截断（不切汉字字节）");
    assert_eq!(merge_kernel::truncate_chars("短", 10), "短", "短于上限原样返回");

    println!("  生产保真度读数：足量 {f:.1} · 截断 {f2:.4} · 收紧 {f_tight:.4} → {f_loose:.4} · 跨代 gen3 {f3:.4}");

    println!("\n✅ 全部断言通过（含 3 条反向用例 + 1 条平凡值用例 + 生产内核段）");
    println!("   结论：`max_gen` 只保证\"不无限跑\"；`min_fidelity` 保证\"跑到还有用为止\"。");
    println!("   生产闸门 `consolidate → accept` 用的正是本文件生产内核段这条保真度算法。");
    println!("         两者是**不同性质的约束**：前者是形态，后者是质量。只有前者时，");
    println!("         系统会产出\"代数合规但已经失真\"的记忆 —— 这正是 G7 的真实危害。");
}

// ───────────────────────── 参考文献（写入程序，便于核对）─────────────────────────
//
// [REF-1] pjama, "Fidelity in LLM Information Processing"
//   - 摘要是**有损压缩**，没有解压步骤能恢复原文；
//   - 三种可预测失效模式：数值精度损失 / 归因抹除 / 条件性细节崩塌，
//     且**在所产生摘要被再次摘要时都会复合叠加**；
//   - 多跳链路：三到四跳后累积成漂移，"没有任何单独一步能察觉"；
//   - 保真度测量是**不对称的**：检测"幻觉新增"比检测"关键遗漏"更容易；
//   - 用**跳数**作累积漂移的代理变量：1 跳可缓存 / 2 跳周期重算 / ≥3 跳精度任务从源重推；
//   - 对策：结构化源归因 / 分层摘要 / 检索优于压缩 / 显式保留契约。
//   → 本方案的对应：`min_fidelity` 就是把"保留契约"变成可执行的数值约束；
//     与源（gen 0）比较而非与上一代比较，正是"从源重推"的落地形式。
//
// [REF-2] LLM Agent 记忆系统综述（lin-guanguo/llm-memory-research）
//   - "压缩质量是共有的未解问题……**没有系统能可靠地知道丢失了什么**"；
//   - Gemini CLI 的 two-pass probe（摘要后再探一次找遗漏）是唯一的验证尝试，代价翻倍；
//   - Context Rot 四类退化：poisoning / distraction / confusion / clash，
//     多数 Agent 只处理 distraction。
//   → 本方案的对应：`min_fidelity` 是比 two-pass probe 更便宜的**结构化**验证 ——
//     不比文本、比特征，因此可在每次巩固时全量执行。
//
// [REF-3] LongMemEval (ICLR 2025)
//   - 五大长期记忆能力：Information Extraction / Multi-Session Reasoning /
//     Knowledge Updates / Temporal Reasoning / **Abstention**；
//   - 有 turn-level 与 session-level 两级召回评测；500 题。
//   → 本方案的对应：§4.3 的"检索评测集"直接以这五项能力为骨架设计 gold query。
