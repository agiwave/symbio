//! `recall` 的**排序内核**（纯函数，无 crate 依赖）。
//!
//! ## 为什么单独成一个文件
//!
//! 它是**生产读侧与召回评测集的唯一共有实现**：
//!
//! - 生产：`super::recall`（`recall` 投影）调 [`rank`] 排 `RecallEntry`；
//! - 评测：`docs/003-design/verify/recall_eval.rs` 用 `#[path]` **包含本文件**，对
//!   召回评测集跑同一份排序，出「生产投影的 recall@k」。
//!
//! 两边共用一份源码，而不是各写一份同构逻辑——临摹件会随生产漂移而**静默**失效
//! （要消灭的正是这个形态：评测集此前测的是三个假想召回器，其中一个
//! 「临摹现状」，数字很好看，但与生产没有因果关系）。
//!
//! 因此本文件**不许** import 任何 crate 内类型（不 import 才能被两边各自编译），
//! 入口只有 [`terms`] / [`overlap`] / [`rank`] 三个纯函数与 [`ScoreEntry`] 一个
//! 数据结构。算法判据（怎么分词、怎么算重叠、平手怎么断）只有这一份。
//!
//! ## 它与代码检索的关系（[D-109](../../plugins/memory/README.md#d-109-两套检索内核不合一打分各归各平手有稳定次级键截断在全序之后是全项目一份的排序口径 "crates/symbio-plugin-memory/src/README")）
//!
//! 本文件是**记忆侧**「查询 → 相关性」这条打分口径的唯一 owner。代码检索
//! （`plugins/local/codebase_search.rs` 的 `semantic_search`）是**另一份**打分实现
//! （嵌入余弦），两边**刻意不合一**：量纲不同（计数 vs [−1,1]）、候选来源不同
//! （事件 Log vs 文件系统）。共享的只有判据形状那一半——**分数降序之后必须有稳定
//! 次级键，top-k 截断在全序之后**；两侧各自的实例与这条的 owner 都登记在 D-109。
//! 想「把两套检索内核合成一套」之前先读那条：它已经被否决过一次，理由在账上。

/// 一条待排序的条目——生产 `RecallEntry` 与评测 `Memory` 的**最小公共形状**。
///
/// 字段与 `symbio_core::view::RecallEntry` 逐字对应（生产侧做一次显式转换）：
/// 带 `tag` 是给将来的标签维度留位（今天是内容维度，混合打分的其余维度落地时
/// 加在这个排序函数里，不动两边的调用形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoreEntry {
    /// 内容（召回正文；打分跑在它上面）。
    pub content: String,
    /// 认知内容标签（`tag` 过滤在投影层做完，排序层只看结果）。
    pub tag: String,
    /// 编码时间（Unix 毫秒）——内容打分的**第一平手**判据。
    pub ts: i64,
    /// 来源事件 seq——最终平手判据（全序 ⇒ 确定性，N1）。
    pub seq: u64,
}

/// 分词：**拉丁/数字词 + 中文字符二元组**。
///
/// ## 为什么不能只按空白切
///
/// 中文查询「部署时数据库连接池打满的处理」按空白切会整成一个词，而它作为整体
/// 出现在任何一条记忆里的概率接近零 ⇒ 每条记忆的重叠数都是 0 或 1，内容口径
/// **恒等于时间口径**，混合打分失去分辨力。这是「中文必须有字级切分」的实测学费
/// （recall_eval 第一版：content 0.333 = time 0.333）。
///
/// ## 为什么用二元组而不是单字
///
/// 单字太宽：「的」「时」「理」这类功能字会让无关记忆也拿到重叠分。二元组
/// （bigram）在「不引入分词依赖」前提下给出可用的区分度——与检索界对 CJK 的
/// 常规做法一致（character n-gram），也是本仓能落的**最小形状**。
/// 换成真 embedding 时，替换点只有这一个函数（`overlap` 与 `rank` 不动）。
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

/// CJK 统一表意文字的基本区（含扩展 A 与兼容表）——够覆盖中文日常文，
/// 不追求完整 Unicode 分类（那要引入 unicode 依赖，违背「最小形状」）。
fn is_cjk(c: char) -> bool {
    ('\u{4E00}'..='\u{9FFF}').contains(&c)
        || ('\u{3400}'..='\u{4DBF}').contains(&c)
        || ('\u{F900}'..='\u{FAFF}').contains(&c)
}

/// 查询词里有几个出现在内容中（词级包含，不做子串——子串会把「内存」算进
/// 「内存泄漏」两次，而那是不同的记忆条目）。
pub fn overlap(query_terms: &[String], content: &str) -> usize {
    let c = content.to_lowercase();
    query_terms
        .iter()
        .filter(|t| c.contains(t.as_str()))
        .count()
}

/// 排序（混合打分的第一维：**语义**）。
///
/// - `query` 为 `Some` 且非空 ⇒ **内容重叠分降序**，平手看 `ts` 降序，再平手看
///   `seq` 降序（全序 ⇒ 确定性）；
/// - `query` 为 `None` 或空 ⇒ **纯 `ts` 降序**（平手 `seq` 降序）——这是
///   **平凡值**（J2）：没有查询就是今天的行为，逐字不变。
///
/// 混合打分的其余维度（时序已经在平手判据里；因果 / 扩散 / 衰减）落地时加在
/// 这个函数里，两边的调用形态（`rank(entries, query)`）不动。
pub fn rank(mut entries: Vec<ScoreEntry>, query: Option<&str>) -> Vec<ScoreEntry> {
    let q = query.filter(|s| !s.is_empty()).map(terms);
    match q {
        Some(terms) if !terms.is_empty() => {
            // 先打分再排序：比较器里调 `overlap` 会对每对元素重算一遍
            // （O(n log n) 次全量扫描），先打分是 O(n) 次。
            let mut scored: Vec<(usize, ScoreEntry)> = entries
                .into_iter()
                .map(|e| (overlap(&terms, &e.content), e))
                .collect();
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then(b.1.ts.cmp(&a.1.ts))
                    .then(b.1.seq.cmp(&a.1.seq))
            });
            scored.into_iter().map(|(_, e)| e).collect()
        }
        // 平凡值：无查询 ⇒ 新近度优先（与混合打分落地前逐字相同）。
        _ => {
            entries.sort_by(|a, b| b.ts.cmp(&a.ts).then(b.seq.cmp(&a.seq)));
            entries
        }
    }
}
