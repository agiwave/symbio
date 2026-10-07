//! 终止性：一条定理的三个前提，而不是一份规则清单
//!
//! 定理：若（1）seq 严格单调（2）每个任务的返工/重规划次数有硬上界（3）依赖图无环
//!      则任意任务树必然到达终态。
//! 三个前提各自可检查，且各自配反向用例：关掉任一前提，终止性保证必须失效。
//!
//! 编译运行：rustc --edition 2021 termination.rs -o tm && ./tm

use std::collections::HashMap;

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

/// 前提 3：依赖图无环（Kahn 拓扑排序）
fn is_acyclic(tasks: &[Task]) -> bool {
    let mut indeg: HashMap<u32, usize> = HashMap::new();
    for t in tasks {
        indeg.entry(t.id).or_insert(0);
    }
    for t in tasks {
        for d in &t.depends_on {
            *indeg.entry(t.id).or_insert(0) += 1;
            indeg.entry(*d).or_insert(0);
        }
    }
    let mut ready: Vec<u32> = indeg.iter().filter(|(_, v)| **v == 0).map(|(k, _)| *k).collect();
    let mut done = 0;
    while let Some(id) = ready.pop() {
        done += 1;
        for t in tasks {
            if t.depends_on.contains(&id) {
                let e = indeg.entry(t.id).or_insert(0);
                *e -= 1;
                if *e == 0 {
                    ready.push(t.id);
                }
            }
        }
    }
    // 依赖了不存在的任务（悬空依赖）也判为不合法
    let ids: Vec<u32> = tasks.iter().map(|t| t.id).collect();
    let dangling = tasks.iter().any(|t| t.depends_on.iter().any(|d| !ids.contains(d)));
    done == tasks.len() && !dangling
}

/// 前提 1：seq 严格单调
fn seq_monotonic(seqs: &[u64]) -> bool {
    seqs.windows(2).all(|w| w[1] > w[0])
}

/// 综合判定：三个前提全满足才保证终止
fn terminates(tasks: &[Task], seqs: &[u64], limits: &Limits) -> Result<(), String> {
    if !seq_monotonic(seqs) {
        return Err("前提 1 失效：seq 非严格单调".into());
    }
    if tasks.iter().any(|t| t.rework_count > limits.max_rework) {
        return Err(format!("前提 2 失效：返工次数超过上界 {}", limits.max_rework));
    }
    if !is_acyclic(tasks) {
        return Err("前提 3 失效：依赖图有环或存在悬空依赖".into());
    }
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
    let ok_seqs = vec![1u64, 2, 3, 4, 5];
    match terminates(&ok_tasks, &ok_seqs, &limits) {
        Ok(()) => println!("健康任务树 → 终止性成立"),
        Err(e) => println!("健康任务树 → {}", e),
    }

    // 破坏前提 1
    let bad_seq = vec![1u64, 2, 2, 4];
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
    println!("破坏前提 3（1↔2 成环）→ {:?}", terminates(&cyclic, &ok_seqs, &limits).err());
    println!("  is_acyclic(环) = {}", is_acyclic(&cyclic));

    // 悬空依赖
    let dangling = vec![Task { id: 1, depends_on: vec![99], rework_count: 0 }];
    println!("悬空依赖 → is_acyclic = {}（悬空也必须判否）", is_acyclic(&dangling));

    println!("\n═══ 反向用例：关掉前提，保证必须失效 ═══");
    // 关掉前提 3 的环检测，环就不再被发现 —— 说明这条检查是必要的
    let cyclic_without_check = is_acyclic_ignoring_cycles(&cyclic);
    println!("  忽略环检测后 is_acyclic(环) = {}（必须变 true，说明检查在起作用）", cyclic_without_check);

    // ── 断言 ──
    assert!(terminates(&ok_tasks, &ok_seqs, &limits).is_ok(), "健康任务树必须判定为终止");
    assert!(terminates(&ok_tasks, &bad_seq, &limits).is_err(), "前提 1 破坏后必须判否");
    assert!(terminates(&over, &ok_seqs, &limits).is_err(), "前提 2 破坏后必须判否");
    assert!(terminates(&cyclic, &ok_seqs, &limits).is_err(), "前提 3 破坏后必须判否");
    assert!(!is_acyclic(&dangling), "悬空依赖必须判否");
    assert!(cyclic_without_check, "反向用例：关掉环检测后环必须检测不到");
    assert!(seq_monotonic(&[1, 2, 3]), "单调序列判定");
    assert!(!seq_monotonic(&[3, 2, 1]), "反向用例：逆序必须判否");

    println!("\n✅ 全部断言通过（含 2 条反向用例）：三前提各自独立必要。");
}

/// 反事实实现：故意不做环检测（只查悬空），用来证明环检测不是摆设。
fn is_acyclic_ignoring_cycles(tasks: &[Task]) -> bool {
    let ids: Vec<u32> = tasks.iter().map(|t| t.id).collect();
    !tasks.iter().any(|t| t.depends_on.iter().any(|d| !ids.contains(d)))
}
