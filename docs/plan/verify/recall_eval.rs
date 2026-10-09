//! 召回评测集 · recall@k（plan/13 批 P5）
//!
//! 本程序回答一个问题：
//!   **「存进了记忆」等于「用的时候找得回」吗？**
//!
//! 这是 G6「存了但找不到」的**可复现数字**。在它之前，召回通路只有口径判据
//! （排除式遗忘 / 放宽即召回 / 活参数三条），没有一个数字能回答
//! 「给定一个查询，前 k 条里有多少是用户当时想找的那条」。
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
//! ## 本程序刻意不做什么
//!
//! 它**不评测仓内召回实现的现状**，而是给定一个**召回器的接口**（`Recall` trait），
//! 对三个假想实现各跑一遍同一套用例：
//!   - `TagThenTime`：只按 tag 过滤 + 时间倒序（本仓 `recall` 投影的现状口径）；
//!   - `ContentMatch`：按查询与内容的词重叠排序（内容匹配的下界）；
//!   - `TagAndContent`：先按 tag 收窄、再按内容匹配排序。
//!
//! 这样这个程序给出的是**一组可比的数字**，而不是一句「现状不行」。
//! 「现状为何达不到」是产品判断；本程序只保证：**换一个召回器，数字会变**，
//! 且变的方向可预期。没有这一条，评测集本身会退化成又一份恒绿断言。
//!
//! 论证纪律：每条断言配反向用例（改输入，结论必须变）。
//!
//! 编译运行：rustc --edition 2021 recall_eval.rs -o re && ./re

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

// ───────────────────────── 召回器接口 ─────────────────────────

/// 召回器：给定池与查询，返回**排好序的下标**（前 k 个）。
///
/// 抽成 trait 而不是三个 if-else 分支：新增一种召回口径时只加一个 impl，
/// 评测主体（建池 → 召回 → 算 hit）不动——否则加第四种口径就要再抄一遍主体，
/// 而主体的口径（hit@k 怎么算）必须只有一处。
pub trait Recall {
    fn name(&self) -> &'static str;
    fn recall(&self, pool: &[Memory], query: &str, k: usize) -> Vec<usize>;
}

/// 现状口径：tag 过滤 + 时间倒序。**不看内容**。
pub struct TagThenTime;

impl Recall for TagThenTime {
    fn name(&self) -> &'static str {
        "tag+time"
    }
    fn recall(&self, pool: &[Memory], _query: &str, k: usize) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..pool.len()).collect();
        // 稳定排序 ⇒ 同 ts 时保留下标序（确定性，N1）
        idx.sort_by(|&a, &b| pool[b].ts.cmp(&pool[a].ts).then(a.cmp(&b)));
        idx.into_iter().take(k).collect()
    }
}

/// 内容匹配的下界：查询与记忆的词重叠数（越多越靠前），平手看时间。
pub struct ContentMatch;

impl Recall for ContentMatch {
    fn name(&self) -> &'static str {
        "content"
    }
    fn recall(&self, pool: &[Memory], query: &str, k: usize) -> Vec<usize> {
        let q = terms(query);
        let mut idx: Vec<usize> = (0..pool.len()).collect();
        idx.sort_by(|&a, &b| {
            let sa = overlap(&q, &pool[a].content);
            let sb = overlap(&q, &pool[b].content);
            sb.cmp(&sa)
                .then(pool[b].ts.cmp(&pool[a].ts))
                .then(a.cmp(&b))
        });
        idx.into_iter().take(k).collect()
    }
}

/// 先按 tag 收窄、再按内容匹配排序（产物口径应有的形状）。
pub struct TagAndContent;

impl Recall for TagAndContent {
    fn name(&self) -> &'static str {
        "tag+content"
    }
    fn recall(&self, pool: &[Memory], query: &str, k: usize) -> Vec<usize> {
        // tag 过滤是**读侧**参数（recall 投影的唯一活参数）：本程序的用例
        // 全用同一 tag，故过滤在这里是恒真的——刻意如此，本程序测的是
        // 「排序」而不是「过滤」（过滤另有口径判据）。
        let q = terms(query);
        let mut idx: Vec<usize> = (0..pool.len()).collect();
        idx.sort_by(|&a, &b| {
            let sa = overlap(&q, &pool[a].content);
            let sb = overlap(&q, &pool[b].content);
            sb.cmp(&sa)
                .then(pool[b].ts.cmp(&pool[a].ts))
                .then(a.cmp(&b))
        });
        idx.into_iter().take(k).collect()
    }
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

/// 分词：**拉丁词 + 中文字符二元组**。
///
/// ## 为什么不能只按空白切
///
/// 第一版按空白切，中文查询「部署时数据库连接池打满的处理」会被整成一个词，
/// 而它**作为一个整体**出现在任何一条记忆里的概率接近零 ⇒ 每条记忆的重叠数
/// 都是 1（或 0），内容口径于是**恒等于时间口径**，评测集立刻失去分辨力
/// （实测 content 与 time 都是 0.333）。这是"中文必须有字级切分"的实测学费。
///
/// ## 为什么用二元组而不是单字
///
/// 单字太宽：「的」「时」「理」这类功能字会让无关记忆也拿到重叠分。二元组
/// （bigram）在"不引入分词依赖"前提下给出可用的区分度——这与检索界对 CJK
/// 的常规做法一致（character n-gram），也是本仓能落的**最小形状**。
/// 换成真 embedding 时，替换点只有这一个函数（overlap 与三个召回器不动）。
pub fn terms(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // ① 拉丁/数字词：按非字母数字边界切（CJK 不算——它们走二元组）
    let mut buf = String::new();
    for ch in s.chars() {
        if ch.is_alphanumeric() && !is_cjk(ch) {
            buf.push(ch);
        } else if !buf.is_empty() {
            out.push(std::mem::take(&mut buf).to_lowercase());
        }
    }
    if !buf.is_empty() {
        out.push(buf.to_lowercase());
    }
    // ② CJK 连续段 → 字符二元组（段长为 1 时退化为单字）
    let run: Vec<char> = s.chars().filter(|c| is_cjk(*c)).collect();
    if run.len() == 1 {
        out.push(run[0].to_string());
    } else {
        for w in run.windows(2) {
            out.push(w.iter().collect());
        }
    }
    out
}

/// CJK 统一表意文字的基本区（含扩展 A 与兼容表）——够覆盖中日常文，
/// 不追求完整 Unicode 分类（那要引入 unicode 依赖，违背"最小形状"）。
fn is_cjk(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
        || ('\u{3400}'..='\u{4DBF}').contains(&c)
        || ('\u{F900}'..='\u{FAFF}').contains(&c)
}

/// 查询词里有几个出现在内容中（词级包含，不做子串——子串会把「内存」算进
/// 「内存泄漏」两次，而那是不同的记忆条目）。
fn overlap(query_terms: &[String], content: &str) -> usize {
    let c = content.to_lowercase();
    query_terms.iter().filter(|t| c.contains(t.as_str())).count()
}

/// 一条用例在一个召回器下的 hit@k（命中 = 期望那条出现在前 k 个里）。
fn hit_at_k(r: &dyn Recall, c: &Case) -> f64 {
    let got = r.recall(&c.memories, c.query, c.k);
    if got.contains(&c.expect) {
        1.0
    } else {
        0.0
    }
}

/// 整个用例集在一个召回器下的 recall@k（平均值）。
fn recall_at_k(r: &dyn Recall, cs: &[Case]) -> f64 {
    let sum: f64 = cs.iter().map(|c| hit_at_k(r, c)).sum();
    sum / cs.len() as f64
}

fn main() {
    let cs = cases();
    let impls: Vec<Box<dyn Recall>> = vec![
        Box::new(TagThenTime),
        Box::new(ContentMatch),
        Box::new(TagAndContent),
    ];

    println!("召回评测集：{} 条用例\n", cs.len());
    for c in &cs {
        println!(
            "  · {}\n      查询：{}\n      期望：第 {} 条（k={}）",
            c.name, c.query, c.expect, c.k
        );
    }

    println!("\n各召回口径的 recall@k：");
    let mut scores: Vec<(&'static str, f64)> = Vec::new();
    for r in &impls {
        let s = recall_at_k(r.as_ref(), &cs);
        scores.push((r.name(), s));
        println!("  {:<14} {:.3}", r.name(), s);
    }

    // ── 判据 ────────────────────────────────────────────────────────────
    //
    // 1. **评测集可分**：内容匹配必须严格优于纯时间序。否则这个集合证不了
    //    「内容匹配有用」，它就只是一组断言。
    let by_name = |n: &str| scores.iter().find(|(k, _)| *k == n).map(|(_, v)| *v).unwrap_or(-1.0);
    let time_score = by_name("tag+time");
    let content_score = by_name("content");
    let both_score = by_name("tag+content");

    assert!(
        content_score > time_score,
        "评测集必须能分辨「看内容」与「不看内容」：content={content_score} time={time_score}"
    );

    // 2. **两种内容口径同结果**：本批用例的 tag 全部相同，故 tag 过滤是恒真的；
    //    若两者分数不同，说明「先按 tag 收窄」改变了排序——那不是本程序要测的
    //    （过滤另有口径判据），故此处同分。
    assert!(
        (both_score - content_score).abs() < 1e-9,
        "tag 全同时两个内容口径应同分：content={content_score} both={both_score}"
    );

    // 3. **每一条用例至少被一个口径命中**：`expect` 下标若写错（越界 / 指向
    //    错的格子），这里会立刻暴露——全套命中数是 0 就意味着集合本身失效。
    let any_hit = cs.iter().any(|c| impls.iter().any(|r| hit_at_k(r.as_ref(), c) > 0.0));
    assert!(any_hit, "用例集必须可命中：expect 下标全错时此断言红");

    // 4. **反向**：把第一条用例的 expect 改成**一个明确排不进前 k 的下标**，
    //    content 口径的命中数必须下降。这一条防的是「整套集合恒真」。
    //
    //    ⚠️ 第一版写的是 expect = 1（指向「今天改了 README」），分数没掉——
    //    因为那条用例的 k=2，而池里只有 3 条，改后的下标照样落在前 2 里
    //    （实测 1.0 → 1.0）。**反向用例自己也要能反向**，否则它只是一条
    //    额外的恒真断言。正确做法：把 k 一并收紧到 1，让"被改指的那条"
    //    必然落在前 k 之外。
    let mut cs2 = cs.clone();
    cs2[0].expect = 1; // 「今天改了 README」——与查询零重叠
    cs2[0].k = 1; // 收紧窗口：内容口径此时只会把第 0 条放进来
    let rev = recall_at_k(&ContentMatch, &cs2);
    assert!(
        rev < content_score,
        "改错 expect 必须让分数下降：原 {content_score} → 改后 {rev}"
    );
    // 且下降的量正好是 1/|用例集|（只动了一条）——多降少降都说明改动不止一处生效
    let one_step = 1.0 / cs.len() as f64;
    assert!(
        (content_score - rev - one_step).abs() < 1e-9,
        "只改了一条 ⇒ 只该掉一格分数：应掉 {one_step}，实掉 {}",
        content_score - rev
    );

    println!("\n通过：评测集能分辨口径（content {content_score:.3} > time {time_score:.3}），");
    println!("      且 expect 下标可被反向证伪（改错后 content 掉到 {rev:.3}）。");
    println!("      recall@k 从此是一个**可复现的数字**，不再是修辞。");
}
