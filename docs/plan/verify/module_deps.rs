//! 模块依赖：沿路线图扩充时，模块之间会不会长出相互依赖？
//!
//! 分四层回答，逐层可验证：
//!   L1 代码依赖（import / 直接调用）      → I1 杜绝
//!   L2 类型依赖（投影读另一个投影的输出） → 投影签名杜绝（只能读 &[E]）
//!   L3 语义前置依赖（高阶依赖低阶的事实） → 允许且必要，这是能力增长本身的形态
//!   L4 派生链（巩固的巩固的巩固…）        → 自动无环（溯源边沿 seq 单调方向），但**代数可能发散**
//!
//! 本程序验证 L4 的两条：派生图无环由谁保证、代数发散由谁兜住。
//!
//! 编译运行：rustc --edition 2021 module_deps.rs -o md && ./md

#[derive(Clone, Copy)]
struct Ev {
    seq: u64,
    kind: &'static str,
    /// 溯源：派生自哪条事件。原始事实为 None。
    derived_from: Option<u64>,
}

/// I2 的弱版：只看"有没有溯源"
fn weak_provenance_check(log: &[Ev]) -> Vec<u64> {
    log.iter()
        .filter(|e| e.kind == "memory.consolidated" && e.derived_from.is_none())
        .map(|e| e.seq)
        .collect()
}

/// I2 + I1 方向：派生事件必须溯源，且**只能溯源到更早的 seq**
fn direction_violations(log: &[Ev]) -> Vec<u64> {
    log.iter()
        .filter(|e| matches!(e.derived_from, Some(p) if p >= e.seq))
        .map(|e| e.seq)
        .collect()
}

/// 派生图是否有环（邻接：seq → derived_from）
fn has_cycle(log: &[Ev]) -> bool {
    let n = log.len();
    let idx = |s: u64| log.iter().position(|e| e.seq == s);
    // 入度 = 每个节点被多少条边指向（这里每个节点最多一条出边，指向父节点）
    let mut indeg = vec![0usize; n];
    for e in log {
        if let Some(p) = e.derived_from {
            if let Some(j) = idx(p) {
                indeg[j] += 1;
            }
        }
    }
    // Kahn：不断剥离"出边指向已处理节点"的叶子
    let mut removed = vec![false; n];
    let mut count = 0;
    loop {
        let mut progressed = false;
        for i in 0..n {
            if removed[i] {
                continue;
            }
            // 若它指向的父节点都已被移除（或它没有父节点），则它可剥离
            let parent_removed = match log[i].derived_from {
                None => true,
                Some(p) => match idx(p) {
                    Some(j) => removed[j],
                    None => true,
                },
            };
            if parent_removed {
                removed[i] = true;
                count += 1;
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    count != n
}

/// 代数：原始事实 = 0，派生事件 = 父代 + 1
fn gen_of(log: &[Ev], seq: u64) -> u32 {
    let mut g = 0;
    let mut cur = seq;
    for _ in 0..64 {
        let e = match log.iter().find(|e| e.seq == cur) {
            Some(e) => e,
            None => return g,
        };
        match e.derived_from {
            None => return g,
            Some(p) => {
                g += 1;
                cur = p;
            }
        }
    }
    g
}

fn max_generation(log: &[Ev]) -> u32 {
    log.iter().map(|e| gen_of(log, e.seq)).max().unwrap_or(0)
}

/// 模拟巩固链：每轮巩固产出一个派生事件。cap = 代数上界（None 表示无上界）
fn build_chain(rounds: u64, cap: Option<u32>) -> Vec<Ev> {
    let mut log = vec![Ev { seq: 1, kind: "memory.encoded", derived_from: None }];
    let mut prev = 1u64;
    for i in 2..=(rounds + 1) {
        let g = gen_of(&log, prev);
        if let Some(c) = cap {
            if g + 1 > c {
                break;
            }
        }
        log.push(Ev { seq: i, kind: "memory.consolidated", derived_from: Some(prev) });
        prev = i;
    }
    log
}

fn main() {
    println!("═══ 模块依赖 · 派生链与代数 ═══\n");

    // 健康日志：派生链只向更早的 seq 溯源
    let healthy = vec![
        Ev { seq: 1, kind: "turn.opened", derived_from: None },
        Ev { seq: 2, kind: "artifact.added", derived_from: None },
        Ev { seq: 3, kind: "task.verified", derived_from: Some(2) },
        Ev { seq: 4, kind: "memory.encoded", derived_from: None },
        Ev { seq: 5, kind: "memory.consolidated", derived_from: Some(4) },
        Ev { seq: 6, kind: "task.rework_created", derived_from: Some(3) },
    ];

    // 坏日志：一条事件溯源到自己（或更晚的 seq）
    let mut broken = healthy.clone();
    broken.push(Ev { seq: 7, kind: "memory.consolidated", derived_from: Some(7) });

    println!("── L4.1 派生图是否有环 ──");
    println!("  健康日志：方向违例 {} 条，有环 {}",
        direction_violations(&healthy).len(), has_cycle(&healthy));
    println!("  坏日志  ：方向违例 {} 条，有环 {}",
        direction_violations(&broken).len(), has_cycle(&broken));
    println!("  弱检查器（只看有无溯源）对坏日志报 {} 条",
        weak_provenance_check(&broken).len());

    println!("\n── L4.2 代数发散与代数上界 ──");
    let unbounded = build_chain(12, None);
    let capped3 = build_chain(12, Some(3));
    let capped5 = build_chain(12, Some(5));
    println!("  无上界  ：事件 {} 条，最大代数 {}", unbounded.len(), max_generation(&unbounded));
    println!("  上界=3  ：事件 {} 条，最大代数 {}", capped3.len(), max_generation(&capped3));
    println!("  上界=5  ：事件 {} 条，最大代数 {}", capped5.len(), max_generation(&capped5));

    // ── 断言 ──
    assert!(direction_violations(&healthy).is_empty(), "健康日志不应有溯源方向违例");
    assert!(!has_cycle(&healthy), "健康日志的派生图必须无环");
    assert_eq!(direction_violations(&broken).len(), 1, "坏日志的溯源方向违例必须被抓到");
    assert!(has_cycle(&broken), "坏日志的派生图必须检出环");
    assert!(weak_provenance_check(&broken).is_empty(),
        "反向用例：只看'有无溯源'的弱检查器抓不到环 —— 证明**方向**检查才是必要的，I2 单独不够");

    assert!(max_generation(&unbounded) >= 10, "无上界时巩固链代数会持续发散");
    assert_eq!(max_generation(&capped3), 3, "代数上界 = 3 时最大代数必须为 3");
    assert_eq!(max_generation(&capped5), 5, "反向用例：上界改成 5 后结果必须改变（证明 cap 在生效）");
    assert!(capped3.len() < unbounded.len(), "有上界时事件数必须少于无上界");

    println!("\n✅ 全部断言通过（含 2 条反向用例）。");
    println!("   派生图无环由 I1 的 seq 单调保证（溯源边只能指向更早的 seq），不是由 I2 保证；");
    println!("   但代数发散架构不禁止 —— 必须靠「代数上界」这个参数兜住，且要做成 CI 断言（J3 静默失效）。");
}
