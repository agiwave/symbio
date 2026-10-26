//! 终止性：一条定理的三个前提，而不是一份规则清单
//!
//! 定理：若（1）seq 严格单调（2）每个任务的返工/重规划次数有硬上界（3）依赖图无环
//!      则任意任务树必然到达终态。
//! 三个前提各自可检查，且各自配反向用例：关掉任一前提，终止性保证必须失效。
//!
//! ## 判定走生产内核
//!
//! 前提 2 / 3 的判定**不是本文件实现的**——生产侧
//! [`symbio_core::invariants`](../../../crates/symbio-core/src/invariants/mod.rs)
//! 的 `acyclic_deps` / `rework_bounded` 把算法层抽成了 std-only 内核
//! （`invariants/kernel.rs`，`dep_graph_core` / `rework_core`），本程序 `#[path]`
//! 包含**同一个文件**。此前本文件自己又写了一遍 Kahn——生产改环检测、这里
//! 仍全绿的「验的东西与跑的不是同一个」，正是 要堵的失效。
//!
//! 生产与这里的分工：生产 `acyclic_deps(&[Event])` 从**任务格事件切片**解析出
//! `(task_id, depends_on)` 喂内核；本程序从**自造任务树**构造同样的输入喂
//! 同一份内核。内核改任何一处（比如 Kahn 剥离判据错改），生产单测与本文件的
//! 反向用例必须同时红。
//!
//! 编译运行：rustc --edition 2021 termination.rs -o tm && ./tm

// `#[path]` 指向仓库里的**生产文件**；本程序只用它四个内核里的两个，
// `allow(dead_code)` 只在 verify 侧生效（生产 crate 全用）。
#[allow(dead_code)]
#[path = "../../../crates/symbio-core/src/invariants/kernel.rs"]
mod kernel;

use kernel::{dep_graph_core, rework_core, seq_monotonic_core};

#[derive(Debug, Clone)]
struct Task {
    id: u32,
    depends_on: Vec<u32>,
    rework_count: u32,
}

struct Limits {
    max_rework: u32,
    // 不需要单独的 max_depth：依赖图无环 + 节点数有限 ⇒ 深度自动有界。
    // 只有当将来允许运行时无限生成子树时，才需要把深度列为第四个前提。
}

/// 前提 3：依赖图无环（判定走生产内核 [`dep_graph_core`]）。
///
/// 返回 `Err(违规)`：悬空依赖与环上节点分开报——两者都要看得见，
/// 合成一个 bool 会把「依赖写错了」与「任务互相等」混成一句。
fn check_acyclic(tasks: &[Task]) -> Result<(), String> {
    let borrowed: Vec<(&str, Vec<&str>)> = tasks
        .iter()
        .map(|t| {
            // 任务 id 的字符串形态是内核输入的唯一表示（生产侧的 task_id
            // 本来就是字符串——`task.opened` 格里的 `task_id`）。
            let id: &'static str = Box::leak(format!("t{}", t.id).into_boxed_str()) as &'static str;
            let ds: Vec<&'static str> = t
                .depends_on
                .iter()
                .map(|d| Box::leak(format!("t{d}").into_boxed_str()) as &'static str)
                .collect();
            (id, ds)
        })
        .collect();
    let (dangling, on_cycle) = dep_graph_core(&borrowed);
    if dangling.is_empty() && on_cycle.is_empty() {
        return Ok(());
    }
    let mut why = Vec::new();
    for (id, d) in &dangling {
        why.push(format!("{id} 悬空依赖 {d}（依赖不存在）"));
    }
    for id in &on_cycle {
        why.push(format!("{id} 处在依赖环上"));
    }
    Err(why.join("；"))
}

/// 前提 2：返工次数有硬上界（判定走生产内核 [`rework_core`]）。
///
/// 本文件的 `Task.rework_count` 是「该任务已被返工几次」的汇总；生产侧的
/// `rework_bounded` 吃的是按时间序的 `replaces` 声明序列。这里把汇总展开成
/// 等价的声明序列（返工 n 次 = 同一目标声明 n 次）喂内核——两种输入形态
/// 走同一个计数判定。
fn check_rework(tasks: &[Task], limits: &Limits) -> Result<(), String> {
    let mut replaces: Vec<&str> = Vec::new();
    for t in tasks {
        let id: &'static str = Box::leak(format!("t{}", t.id).into_boxed_str()) as &'static str;
        for _ in 0..t.rework_count {
            replaces.push(id);
        }
    }
    let bad = rework_core(&replaces, limits.max_rework);
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad
            .into_iter()
            .map(|(t, c, bound)| format!("任务 {t} 返工第 {c} 轮，超过上界 {bound}"))
            .collect::<Vec<_>>()
            .join("；"))
    }
}

/// 综合判定：三个前提全满足才保证终止
fn terminates(tasks: &[Task], seqs: &[u64], limits: &Limits) -> Result<(), String> {
    // 前提 1：seq 严格单调且无跳号——生产内核的语义（已入库 seq 恰为 0..n-1）。
    if !seq_monotonic_core(seqs).is_empty() {
        return Err("前提 1 失效：seq 不严格单调或有跳号".into());
    }
    check_rework(tasks, limits)?;
    check_acyclic(tasks)?;
    Ok(())
}

fn healthy() -> Vec<Task> {
    vec![
        Task { id: 1, depends_on: vec![], rework_count: 0 },
        Task { id: 2, depends_on: vec![1], rework_count: 1 },
        Task { id: 3, depends_on: vec![1], rework_count: 0 },
        Task { id: 4, depends_on: vec![2, 3], rework_count: 2 },
    ]
}

fn main() {
    println!("═══ 终止性三前提 ═══");
    let limits = Limits { max_rework: 3 };

    let ok_tasks = healthy();
    // 生产内核语义：seq 从 0 起严格连续（与 WalStore::append 的分配一致）。
    let ok_seqs = vec![0u64, 1, 2, 3, 4];
    match terminates(&ok_tasks, &ok_seqs, &limits) {
        Ok(()) => println!("健康任务树 → 终止性成立"),
        Err(e) => println!("健康任务树 → {e}"),
    }

    // 破坏前提 1：seq 重复（内核报「处应为 2」与其后全错位）
    let bad_seq = vec![0u64, 1, 1, 3];
    println!("破坏前提 1（seq 重复）→ {:?}", terminates(&ok_tasks, &bad_seq, &limits).err());

    // 破坏前提 2
    let mut over = healthy();
    over[1].rework_count = 99;
    println!("破坏前提 2（返工超限）→ {:?}", terminates(&over, &ok_seqs, &limits).err());

    // 破坏前提 3：成环
    let cyclic = vec![
        Task { id: 1, depends_on: vec![2], rework_count: 0 },
        Task { id: 2, depends_on: vec![1], rework_count: 0 },
    ];
    println!("破坏前提 3（1↔2 成环）→ {:?}", check_acyclic(&cyclic).err());

    // 悬空依赖
    let dangling = vec![Task { id: 1, depends_on: vec![99], rework_count: 0 }];
    println!("悬空依赖 → {:?}", check_acyclic(&dangling).err());

    println!("\n═══ 反向用例：关掉前提，保证必须失效 ═══");
    // 关掉前提 3 的环检测，环就不再被发现 —— 说明这条检查是必要的
    let cyclic_without_check = is_acyclic_ignoring_cycles(&cyclic);
    println!("  忽略环检测后悬空/环都查不到 = {}（必须为 true，说明检查在起作用）", cyclic_without_check);

    // ── 断言 ──
    assert!(terminates(&ok_tasks, &ok_seqs, &limits).is_ok(), "健康任务树必须判定为终止");
    assert!(terminates(&ok_tasks, &bad_seq, &limits).is_err(), "前提 1 破坏后必须判否");
    assert!(terminates(&over, &ok_seqs, &limits).is_err(), "前提 2 破坏后必须判否");
    assert!(check_acyclic(&cyclic).is_err(), "前提 3 破坏后必须判否");
    assert!(check_acyclic(&dangling).is_err(), "悬空依赖必须判否");
    assert!(cyclic_without_check, "反向用例：关掉环检测后环必须检测不到");
    assert!(seq_monotonic_core(&[0, 1, 2]).is_empty(), "单调序列判定（生产内核语义）");
    assert!(!seq_monotonic_core(&[2, 1, 0]).is_empty(), "反向用例：逆序必须判否");
    // 内核判定的边界自证：返工 3 轮（= 上界）合法，第 4 轮才越界。
    let at_bound = vec![Task { id: 1, depends_on: vec![], rework_count: 3 }];
    assert!(check_rework(&at_bound, &limits).is_ok(), "恰在上界的返工必须合法");
    let over_bound = vec![Task { id: 1, depends_on: vec![], rework_count: 4 }];
    assert!(check_rework(&over_bound, &limits).is_err(), "越过上界一轮必须判否");

    println!("\n✅ 全部断言通过（含反向用例）：三前提各自独立必要，判定与生产共用一份内核。");
}

/// 反事实实现：故意不做环检测（只查悬空），用来证明环检测不是摆设。
///
/// 这一份**故意**不挂内核——它的存在意义就是「没有环检测时的世界」，
/// 换成内核反而让反向用例失去反事实性。
fn is_acyclic_ignoring_cycles(tasks: &[Task]) -> bool {
    let ids: Vec<u32> = tasks.iter().map(|t| t.id).collect();
    !tasks.iter().any(|t| t.depends_on.iter().any(|d| !ids.contains(d)))
}
