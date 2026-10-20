//! 判据内核（std-only）——[`mod`](super) 各检查的**算法层**。
//!
//! ## 为什么有这个文件
//!
//! [`docs/004-plan/verify/`](../../../../docs/004-plan/verify/termination.rs) 的验证
//! 程序是**独立 rustc 编译**的（无 serde / tokio，见 gate 的 `55-verify.mjs`），
//! 而本模块的检查吃的是 `Event`（载荷是 `serde_json::Value`）——verify 程序
//! 包含不了它们。在 T-176 之前的结果是：verify/termination.rs 自己又写了一遍
//! Kahn、verify/invariants.rs 自己又写了一遍 seq 单调——**两套实现对同一性质
//! 给出两份结论**，谁也不欠谁红；生产改了判据，verify 仍全绿。
//!
//! 解法是这一层：把算法内核抽成**无依赖**的纯函数（输入是数与字符串，
//! 不含任何事件类型），生产从事件切片**解析出内核输入**、判定走这里；
//! verify 程序 `#[path]` 包含本文件、喂同一份内核。从此「判据只能与实现
//! 共处一处」——生产与验证的判定是同一份代码，改内核任何一处，两边的
//! 反向用例必须同时红（这正是 T-176 出口判据「每个新挂的都能红」）。
//!
//! ## 为什么本文件必须是 std-only
//!
//! 不是风格偏好：`#[path]` 包含是**逐文件**的，verify 程序的编译命令里
//! 没有 `--extern serde`。本文件引入任何一个外部 crate，四条内核就全部
//! 退回「两套实现」的旧病。CI 由 `55-verify.mjs` 兜底——它每次都真的
//! 编译本文件（经两个 verify 程序），std-only 一破当场红。

// ════════════════════════════════════════════════════════════════════
// C1 内核：seq 严格单调且无跳号
// ════════════════════════════════════════════════════════════════════

/// C1 的算法层：输入按存储顺序的 seq 序列，期望它恰好是 `0,1,2,…`。
///
/// 「pending 事件跳过不查」由调用方负责——调用方看得到某条事件有没有 seq，
/// 内核看不到；这层只判**已提交序列**的形状。
///
/// 返回违规三元组 `(序列内下标, 实际 seq, 期望 seq)`。空输入合法（空库无违规）。
pub fn seq_monotonic_core(seqs: &[u64]) -> Vec<(usize, u64, u64)> {
    let mut bad = Vec::new();
    let mut expect = 0u64;
    for (i, &s) in seqs.iter().enumerate() {
        if s != expect {
            bad.push((i, s, expect));
            expect = s + 1;
        } else {
            expect += 1;
        }
    }
    bad
}

// ════════════════════════════════════════════════════════════════════
// C2 / C2-对偶 内核：每 turn 至多一条某类事件
// ════════════════════════════════════════════════════════════════════

/// C2 / C2-对偶的算法层：输入 `(turn, 事件标识)` 对（调用方已把「final」
/// 「开轮」这类**哪些事件算数**的判据过滤完），判每 turn 至多一条。
///
/// 返回重复对的 `(重复事件标识, 首见事件标识, turn)`——与首见的冲突才算
/// 重复（第三条报两次，但都指回首见，读侧才有「是哪两条撞了」）。
pub fn unique_per_turn_core<'a>(
    items: impl Iterator<Item = (u64, &'a str)>,
) -> Vec<(&'a str, &'a str, u64)> {
    let mut seen: Vec<(u64, &str)> = Vec::new();
    let mut bad = Vec::new();
    for (turn, id) in items {
        if let Some((_, first)) = seen.iter().find(|(t, _)| *t == turn) {
            bad.push((id, *first, turn));
        } else {
            seen.push((turn, id));
        }
    }
    bad
}

// ════════════════════════════════════════════════════════════════════
// C14 内核：依赖图悬空 + 环检测（Kahn）
// ════════════════════════════════════════════════════════════════════

/// C14（终止性前提 3）的算法层：输入 `(任务 id, 它声明的依赖)` 列表
/// （**重复的 id 以最后一次为准**——与生产从事件切片构造时的语义一致：
/// 任务可以重开，新开格覆盖旧格）。
///
/// 返回 `(悬空依赖对, 环上节点)`：前者是依赖了不存在任务的 `(任务, 被依赖者)`，
/// 后者是 Kahn 剥离后剩余度 > 0 的节点（剥不完 = 都在环上或被环挡住）。
pub fn dep_graph_core<'a>(
    deps: &[(&'a str, Vec<&'a str>)],
) -> (Vec<(&'a str, &'a str)>, Vec<&'a str>) {
    use std::collections::{BTreeMap, BTreeSet};

    // 重复 id 后写覆盖先写（BTreeMap::insert 的语义，调用方依赖它）。
    let mut graph: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (id, ds) in deps {
        graph.insert(id, ds.iter().copied().collect());
    }
    let ids: BTreeSet<&str> = graph.keys().copied().collect();

    let mut dangling = Vec::new();
    for (id, ds) in &graph {
        for d in ds {
            if !ids.contains(d) {
                dangling.push((*id, *d));
            }
        }
    }

    // Kahn：能剥完 = 无环；剥不完 = 剩下的都在环上（或被环挡住）。
    let mut indeg: BTreeMap<&str, usize> = graph
        .iter()
        .map(|(k, v)| (*k, v.iter().filter(|d| ids.contains(**d)).count()))
        .collect();
    let mut ready: Vec<&str> = indeg
        .iter()
        .filter(|(_, v)| **v == 0)
        .map(|(k, _)| *k)
        .collect();
    while let Some(id) = ready.pop() {
        for (t, ds) in &graph {
            if ds.contains(&id) {
                let e = indeg.get_mut(t).expect("图节点必在 indeg 中");
                *e -= 1;
                if *e == 0 {
                    ready.push(t);
                }
            }
        }
    }
    let on_cycle: Vec<&str> = indeg
        .iter()
        .filter(|(_, deg)| **deg > 0)
        .map(|(id, _)| *id)
        .collect();
    (dangling, on_cycle)
}

// ════════════════════════════════════════════════════════════════════
// 终止性前提 2 内核：返工次数有硬上界
// ════════════════════════════════════════════════════════════════════

/// 前提 2 的算法层：输入按时间序的 `replaces` 值（每次返工声明它替换谁），
/// 判每个被替换者的返工轮数是否超过 `max_rework`。
///
/// 返回 `(被替换者, 第几轮, 上界)`——**每一轮**越界都报（第 4、5 轮各一条，
/// 与生产 `rework_bounded` 的历史行为一致：读侧要的是完整清单，截到首条
/// 会把「越了多远」藏起来）。
pub fn rework_core(replaces: &[&str], max_rework: u32) -> Vec<(String, u32, u32)> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, u32> = BTreeMap::new();
    let mut bad = Vec::new();
    for r in replaces {
        let c = counts.entry(r).or_insert(0);
        *c += 1;
        if *c > max_rework {
            bad.push(((*r).to_string(), *c, max_rework));
        }
    }
    bad
}

// ── panic 面登记（判据见 `scripts/panic-audit.mjs`）──────────────────
// panic-allow symbio/src/symbio_core/invariants/kernel.rs::dep_graph_core: 穷尽性不变量：indeg 与 graph 同构造（同一批键），走到 expect 说明本文件的判据漏了一个分支，属代码缺陷。
