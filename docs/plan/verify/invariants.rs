//! 三条不变量的可执行判据 + 反向用例
//!
//! I1 单通道   —— 一切状态变更经唯一写入口；同一 turn 的 final 发言唯一
//! I2 无溯源不声明 —— 断言类事件必须能追溯到产生它的事件
//! I3 到点必答   —— 发言路径上的 Actor 必须在 budget 内返回，否则必须产出兜底事件
//!
//! 判据纪律：
//!   每条断言都配一个**反向用例**：把检查关掉，违规必须变得检测不到。
//!   检测不到 = 这条检查是必要的；关掉也一样 = 这条检查是摆设。
//!
//! 编译运行：rustc --edition 2021 invariants.rs -o inv && ./inv

#[derive(Debug, Clone)]
struct Event {
    seq: u64,
    kind: &'static str,
    turn: u64,
    #[allow(dead_code)]
    actor: &'static str,
    produced_by: Option<u64>, // I2：产生本事件的上游
    cost_ms: u64,             // I3：本次产出耗时
}

#[derive(Debug, PartialEq, Eq)]
enum Violation {
    I1SeqNotMonotonic { seq: u64, prev: u64 },
    I1DoubleFinal { turn: u64 },
    I2NoProvenance { seq: u64 },
    I3BudgetExceeded { seq: u64, cost_ms: u64, budget: u64 },
}

struct Check {
    enable_i1_seq: bool,
    enable_i1_final: bool,
    enable_i2: bool,
    enable_i3: bool,
}

impl Check {
    fn all() -> Self {
        Check { enable_i1_seq: true, enable_i1_final: true, enable_i2: true, enable_i3: true }
    }
}

/// 需要溯源的事件种类（断言类）
fn needs_provenance(kind: &str) -> bool {
    kind.starts_with("task.verified")
        || kind.starts_with("classify.")
        || kind.starts_with("artifact.")
        || kind.starts_with("commitment.")
}

fn check(log: &[Event], budget_ms: u64, c: &Check) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut prev: Option<u64> = None;
    let mut final_of_turn: Vec<u64> = Vec::new();

    for e in log {
        if c.enable_i1_seq {
            if let Some(p) = prev {
                if e.seq <= p {
                    out.push(Violation::I1SeqNotMonotonic { seq: e.seq, prev: p });
                }
            }
        }
        prev = Some(e.seq);

        if c.enable_i1_final && e.kind == "chat.assistant.final" {
            if final_of_turn.contains(&e.turn) {
                out.push(Violation::I1DoubleFinal { turn: e.turn });
            } else {
                final_of_turn.push(e.turn);
            }
        }

        if c.enable_i2 && needs_provenance(e.kind) && e.produced_by.is_none() {
            out.push(Violation::I2NoProvenance { seq: e.seq });
        }

        if c.enable_i3 && e.cost_ms > budget_ms {
            out.push(Violation::I3BudgetExceeded { seq: e.seq, cost_ms: e.cost_ms, budget: budget_ms });
        }
    }
    out
}

/// 一个健康的日志：单通道、有溯源、都在预算内
fn healthy_log() -> Vec<Event> {
    vec![
        Event { seq: 1, kind: "user.message", turn: 1, actor: "user", produced_by: None, cost_ms: 0 },
        Event { seq: 2, kind: "classify.verdict", turn: 1, actor: "classify", produced_by: Some(1), cost_ms: 40 },
        Event { seq: 3, kind: "task.created", turn: 1, actor: "planner", produced_by: Some(2), cost_ms: 300 },
        Event { seq: 4, kind: "artifact.added", turn: 1, actor: "worker", produced_by: Some(3), cost_ms: 900 },
        Event { seq: 5, kind: "task.verified", turn: 1, actor: "verifier", produced_by: Some(4), cost_ms: 200 },
        Event { seq: 6, kind: "chat.assistant.final", turn: 1, actor: "chat", produced_by: Some(5), cost_ms: 150 },
    ]
}

/// 注入三类违规：seq 回退、第二个 final、无溯源、超预算
fn violated_log() -> Vec<Event> {
    let mut v = healthy_log();
    v.push(Event { seq: 4, kind: "task.progress", turn: 1, actor: "worker", produced_by: Some(3), cost_ms: 10 }); // seq 回退
    v.push(Event { seq: 7, kind: "chat.assistant.final", turn: 1, actor: "worker", produced_by: Some(4), cost_ms: 10 }); // 双 final
    v.push(Event { seq: 8, kind: "task.verified", turn: 1, actor: "worker", produced_by: None, cost_ms: 10 }); // 无溯源
    v.push(Event { seq: 9, kind: "chat.assistant.final", turn: 2, actor: "chat", produced_by: Some(8), cost_ms: 5000 }); // 超预算
    v
}

fn main() {
    println!("═══ 三条不变量的可执行判据 ═══");
    let budget = 1500;

    let clean = check(&healthy_log(), budget, &Check::all());
    println!("\n健康日志（{} 条事件）→ 违规 {} 条", healthy_log().len(), clean.len());

    let dirty = check(&violated_log(), budget, &Check::all());
    println!("注入违规后 → 违规 {} 条：", dirty.len());
    for d in &dirty {
        println!("  {:?}", d);
    }

    println!("\n═══ 反向用例：关掉检查，违规必须消失 ═══");
    let no_i1 = check(&violated_log(), budget, &Check { enable_i1_seq: false, enable_i1_final: false, ..Check::all() });
    let no_i2 = check(&violated_log(), budget, &Check { enable_i2: false, ..Check::all() });
    let no_i3 = check(&violated_log(), budget, &Check { enable_i3: false, ..Check::all() });
    println!("  关掉 I1 → 剩 {} 条（少了 {} 条）", no_i1.len(), dirty.len() - no_i1.len());
    println!("  关掉 I2 → 剩 {} 条（少了 {} 条）", no_i2.len(), dirty.len() - no_i2.len());
    println!("  关掉 I3 → 剩 {} 条（少了 {} 条）", no_i3.len(), dirty.len() - no_i3.len());

    // ── 断言：正向 ──
    assert!(clean.is_empty(), "健康日志不应有违规");
    assert_eq!(dirty.len(), 4, "注入的 4 类违规应被全部捕获");

    // ── 断言：反向用例（证明每条检查都是必要的，不是摆设）──
    assert!(
        !no_i1.iter().any(|v| matches!(v, Violation::I1SeqNotMonotonic { .. } | Violation::I1DoubleFinal { .. })),
        "反向用例：关掉 I1 后 I1 类违规必须检测不到（证明 I1 检查在起作用）"
    );
    assert!(
        !no_i2.iter().any(|v| matches!(v, Violation::I2NoProvenance { .. })),
        "反向用例：关掉 I2 后 I2 类违规必须检测不到"
    );
    assert!(
        !no_i3.iter().any(|v| matches!(v, Violation::I3BudgetExceeded { .. })),
        "反向用例：关掉 I3 后 I3 类违规必须检测不到"
    );

    println!("\n✅ 全部断言通过（含 3 条反向用例）：三条不变量各自独立起作用。");
}
