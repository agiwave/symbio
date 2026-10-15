//! 召回评测集 · recall@k（plan/13 批 P5）
//!
//! 本程序回答一个问题：
//!   **「存进了记忆」等于「用的时候找得回」吗？**
//!
//! 这是 G6「存了但找不到」的**可复现数字**。在它之前，召回通路只有口径判据
//! （排除式遗忘 / 放宽即召回 / 活参数三条），没有一个数字能回答
//! 「给定一个查询，前 k 条里有多少是用户当时想找的那条」。
//!
//! ## 本程序跑的是**生产排序内核**，不是它的临摹件
//!
//! 排序经 `#[path]` **包含生产文件**：[`recall_score`]（`symbio/src/symbio_core/
//! projection/recall_score.rs`——`recall` 投影的排序实现）。因此这里出的
//! recall@k 是**生产投影的口径**（T-085 批②③）：查询驱动的内容重叠分降序，
//! 平手 ts 降序、再平手 seq 降序。
//!
//! 在此之前，本程序测的是三个**假想召回器**（`TagThenTime` 临摹当时的现状、
//! `ContentMatch` / `TagAndContent` 是愿望）——数字很漂亮，但与生产没有因果
//! 关系：生产换一种排序，这套数字一个都不动。临摹件随生产漂移而**静默失效**，
//! 这正是 T-067 黑盒纪律点名要防的形态。现在换排序 = 两边同时换。
//!
//! 边界（诚实划界）：池成员资格（`tag` 过滤 / as-of / 排除式遗忘 / 预算）由
//! 投影的外壳负责，**不在本文件**——那些各有口径判据（`recall.test.rs` /
//! S06 §6 验收）。本程序钉的是**排序**：给定同一个池，生产把用户想找的那条
//! 排到多前。
//!
//! ## 评测集为什么是**数据**而不是断言
//!
//! 一个 recall@k 数字要可信，必须满足两条：
//!   1. 每个用例**独立可读**（查询是什么、期望命中哪一格、为什么是它）；
//!   2. **反向可改**——把某条的 query 改掉，那个用例的数字必须跟着掉
//!      （否则整套集合可能是恒真的）。
//!
//! 所以用例是显式表（见 `CASES`），主体只有三段：建记忆 → 召回 → 算 hit@k。
//!
//! 论证纪律：每条断言配反向用例（改输入，结论必须变）。
//!
//! 编译运行：rustc --edition 2021 recall_eval.rs -o re && ./re

// ── 生产排序内核（唯一共有实现，T-085 批③）─────────────────────────────────
//
// `#[path]` 指向仓库里的生产文件——**这一行就是「跑的是生产」的全部含义**。
// 它被 `recall` 投影与召回评测集共同编译：改排序只有一处可改，两边同时变。
#[path = "../../../symbio/src/symbio_core/projection/recall_score.rs"]
mod recall_score;

use recall_score::{rank, ScoreEntry};

// ───────────────────────── 领域类型 ─────────────────────────

/// 一条记忆（本程序的最小形状：内容 + 标签 + 时间戳）。
#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    /// 记忆正文（将来的 embedding 落在 content 上，01 §9.2）
    pub content: String,
    /// 标签（「记忆类型」：召回的唯一**读侧过滤**参数）
    pub tag: &'static str,
    /// 写入时刻（越新越靠前）
    pub ts: u64,
}

/// 一个评测用例。
#[derive(Debug, Clone)]
pub struct Case {
    /// 用例名（也是失败时的定位符）
    pub name: &'static str,
    /// 查询（用户「想找什么」的自然语言）
    pub query: &'static str,
    /// 记忆池（同一池内建多条，只有一条是用户想找的）
    pub memories: Vec<Memory>,
    /// 期望命中的格子：`memories` 里那条的**下标**
    pub expect: usize,
    /// k（前几条算命中）
    pub k: usize,
}

// ───────────────────────── 召回主体 ─────────────────────────

/// 把评测池喂给**生产排序内核**：`query` 驱动内容匹配（生产读侧
/// `v2_memory::recall_view` 的口径），seq 用池下标（全序 ⇒ 确定性）。
fn production_recall(pool: &[Memory], query: &str, k: usize) -> Vec<usize> {
    let entries: Vec<ScoreEntry> = pool
        .iter()
        .enumerate()
        .map(|(i, m)| ScoreEntry {
            content: m.content.clone(),
            tag: m.tag.to_string(),
            ts: m.ts as i64,
            seq: i as u64,
        })
        .collect();
    rank(entries, Some(query))
        .into_iter()
        .take(k)
        .map(|e| e.seq as usize)
        .collect()
}

/// 平凡值路径：同一内核在无查询时 = 纯 ts 降序（T-085 批②的 J2 判据）。
fn trivial_recall(pool: &[Memory], _query: &str, k: usize) -> Vec<usize> {
    let entries: Vec<ScoreEntry> = pool
        .iter()
        .enumerate()
        .map(|(i, m)| ScoreEntry {
            content: m.content.clone(),
            tag: m.tag.to_string(),
            ts: m.ts as i64,
            seq: i as u64,
        })
        .collect();
    rank(entries, None)
        .into_iter()
        .take(k)
        .map(|e| e.seq as usize)
        .collect()
}

// ───────────────────────── 评测集 ─────────────────────────

/// 用例集。**每条都要能独立读懂**——查询是什么、池里有哪几条、期望是哪条。
///
/// 设计取向：让「内容匹配」与「纯时间序」**恰好在这批用例上分胜负**。
/// 如果两者的数字处处相同，这个集合就证不了任何事。
pub fn cases() -> Vec<Case> {
    let mut v = Vec::new();

    // ① 经典「存了但找不到」：要找的那条**最旧**，另有更新的同 tag 记忆。
    //    纯时间序必然把它排到后面；内容匹配把它顶到第一。
    v.push(Case {
        name: "stale-but-relevant loses to fresh-but-irrelevant under time order",
        query: "部署时数据库连接池打满的处理",
        memories: vec![
            Memory { content: "部署时数据库连接池打满：把最大连接数从 20 调到 50，并给慢查询加索引".into(), tag: "经验", ts: 1_000 },
            Memory { content: "今天改了 README 的一段说明".into(), tag: "经验", ts: 9_000 },
            Memory { content: "调整了日志级别，减少噪声".into(), tag: "经验", ts: 8_000 },
        ],
        expect: 0,
        k: 2,
    });

    // ② 词部分重叠：期望那条共享两个查询词，另一条只共享一个。
    v.push(Case {
        name: "partial word overlap ranks the richer match first",
        query: "内存泄漏怎么定位",
        memories: vec![
            Memory { content: "用 heaptrack 定位内存泄漏，看谁的引用没释放".into(), tag: "经验", ts: 5_000 },
            Memory { content: "内存占用偏高时不一定是泄漏，先看缓存大小".into(), tag: "经验", ts: 9_000 },
        ],
        expect: 0,
        k: 1,
    });

    // ③ 完全无关的查询：任何实现都只能给"最近的那几条"——这一条钉的是
    //    **不许空手而归**（I3：静默超时 / 空且不标降级都算失败形态），
    //    同时 k=1 让「时间序兜底」成为**可分辨**的判据。
    //    ⚠️ k=2 是错的第一版：池里只有两条 ⇒ 任何排序都全命中，这条用例
    //    于是对"看内容 / 不看内容"零分辨力，整套集合的 content 与 time 分数
    //    就变得一模一样（评测跑出来正是 0.333 = 0.333）。**一条不分辨的用例
    //    比没有更坏**——它让集合看起来更大，却不增加任何判别力。
    v.push(Case {
        name: "an unrelated query falls back to recency, never to silence",
        query: "量子计算的纠错码",
        memories: vec![
            Memory { content: "前端构建慢：把 sourcemap 关掉".into(), tag: "经验", ts: 2_000 },
            Memory { content: "接口超时：把重试从 3 次改成 5 次".into(), tag: "经验", ts: 3_000 },
        ],
        // 词重叠全 0 ⇒ 内容口径也退回时间序 ⇒ 命中「最近的那条」
        expect: 1,
        k: 1,
    });

    v
}

// ───────────────────────── 评测主体 ─────────────────────────

/// 一条用例在一个召回口径下的 hit@k（命中 = 期望那条出现在前 k 个里）。
fn hit_at_k(got: &[usize], c: &Case) -> f64 {
    if got.contains(&c.expect) {
        1.0
    } else {
        0.0
    }
}

/// 整个用例集在一个召回口径下的 recall@k（平均值）。
fn recall_at_k(recall: impl Fn(&[Memory], &str, usize) -> Vec<usize>, cs: &[Case]) -> f64 {
    let sum: f64 = cs.iter().map(|c| hit_at_k(&recall(&c.memories, c.query, c.k), c)).sum();
    sum / cs.len() as f64
}

fn main() {
    let cs = cases();
    println!("召回评测集：{} 条用例\n", cs.len());
    for c in &cs {
        println!(
            "  · {}\n      查询：{}\n      期望：第 {} 条（k={}）",
            c.name, c.query, c.expect, c.k
        );
    }

    // 两个口径跑的是**同一份生产排序内核**（recall_score）：
    //   production = `rank(entries, Some(query))` —— 有查询（生产读侧口径）；
    //   trivial    = `rank(entries, None)`       —— 无查询（J2 平凡值，ts 序）。
    let prod = recall_at_k(production_recall, &cs);
    let triv = recall_at_k(trivial_recall, &cs);
    println!("\n生产投影（recall_score）的 recall@k：");
    println!("  {:<14} {:.3}  ← 内容匹配（有查询，生产读侧口径）", "production", prod);
    println!("  {:<14} {:.3}  ← 纯 ts 降序（无查询，J2 平凡值）", "trivial", triv);

    // ── 判据 ────────────────────────────────────────────────────────────
    //
    // 1. **语义维必须真的在算**：生产口径（有查询）严格优于平凡值（无查询）。
    //    否则 recall_score 里的内容分支是死的——「换汤不换药」的静默失效。
    assert!(
        prod > triv,
        "内容匹配必须严格优于纯时间序：production={prod} trivial={triv}"
    );

    // 2. **每条用例至少被一个口径命中**：`expect` 下标若写错（越界 / 指向
    //    错的格子），这里会立刻暴露——全套命中数是 0 就意味着集合本身失效。
    let any_hit = cs.iter().any(|c| {
        hit_at_k(&production_recall(&c.memories, c.query, c.k), c) > 0.0
            || hit_at_k(&trivial_recall(&c.memories, c.query, c.k), c) > 0.0
    });
    assert!(any_hit, "用例集必须可命中：expect 下标全错时此断言红");

    // 3. **反例**：把第一条用例的 expect 改成**一个明确排不进前 k 的下标**，
    //    生产口径的命中数必须下降。这一条防的是「整套集合恒真」。
    //
    //    ⚠️ 第一版写的是 expect = 1（指向「今天改了 README」），分数没掉——
    //    因为那条用例的 k=2，而池里只有 3 条，改后的下标照样落在前 2 里
    //    （实测 1.0 → 1.0）。**反向用例自己也要能反向**，否则它只是一条
    //    额外的恒真断言。正确做法：把 k 一并收紧到 1，让"被改指的那条"
    //    必然落在前 k 之外。
    let mut cs2 = cs.clone();
    cs2[0].expect = 1; // 「今天改了 README」——与查询零重叠
    cs2[0].k = 1; // 收紧窗口：内容口径此时只会把第 0 条放进来
    let rev = recall_at_k(production_recall, &cs2);
    assert!(
        rev < prod,
        "改错 expect 必须让分数下降：原 {prod} → 改后 {rev}"
    );
    // 且下降的量正好是 1/|用例集|（只动了一条）——多降少降都说明改动不止一处生效
    let one_step = 1.0 / cs.len() as f64;
    assert!(
        (prod - rev - one_step).abs() < 1e-9,
        "只改了一条 ⇒ 只该掉一格分数：应掉 {one_step}，实掉 {}",
        prod - rev
    );

    // 4. **分词在真的切**：中文查询按字级二元组切分（拉丁词整词）——这是
    //    内容维度的地基。第一版按空白切，中文查询整成一个词，重叠数恒 0/1，
    //    内容口径**恒等于时间口径**（实测 0.333 = 0.333）。
    let t = recall_score::terms("用 heaptrack 定位内存泄漏");
    assert!(
        t.contains(&"heaptrack".to_string()) && t.contains(&"定位".to_string()),
        "拉丁词整词 + CJK 二元组：{t:?}"
    );
    assert!(
        !t.contains(&"用 heaptrack 定位内存泄漏".to_string()),
        "不许整句当一个词（那正是第一版恒等的死因）"
    );
    let t2 = recall_score::terms("内存泄漏怎么定位");
    assert!(
        t2.contains(&"内存".to_string()) && t2.contains(&"泄漏".to_string()),
        "纯中文段全部走二元组：{t2:?}"
    );

    // 5. **反向**：清空查询词 ⇒ 内核退化为 ts 降序（与平凡值同序）。
    let empty_terms = recall_score::terms("");
    assert!(empty_terms.is_empty(), "空查询无词可切");
    let tied: Vec<ScoreEntry> = (0..3)
        .map(|i| ScoreEntry {
            content: format!("同文 {i}"),
            tag: "经验".to_string(),
            ts: 100 + i as i64,
            seq: i,
        })
        .collect();
    let by_empty = rank(tied.clone(), Some(""));
    assert_eq!(
        by_empty.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![2, 1, 0],
        "空查询 ⇒ ts 降序（seq 无同 ts 冲突时即输入序的逆）"
    );

    println!("\n通过：生产排序内核 recall@k = {prod:.3} > 平凡值 {triv:.3}，");
    println!("      且 expect 下标可被反向证伪（改错后 production 掉到 {rev:.3}）。");
    println!("      recall@k 从此是**生产投影的可复现数字**，不再是临摹件的修辞。");
}
