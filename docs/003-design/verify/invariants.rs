//! 三条不变量的可执行判据 + 真实日志回放
//!
//! I1 单通道   —— 一切状态变更经唯一写入口；同一 turn 的 final 发言唯一
//! I2 无溯源不声明 —— 断言类事件必须能追溯到产生它的事件
//! I3 到点必答   —— 每一轮都必须有一条收束格（不许静默消失）；
//! 且每条停机格必须说得出「为什么停」（闭集）
//!
//! ## 判定走生产内核
//!
//! I1 的两个判定（seq 单调无跳号 / 每 turn 一条 final）**不是本文件实现的**——
//! 生产侧 [`symbio_core::invariants`](../../../crates/symbio-core/src/invariants/mod.rs)
//! 把算法层抽成了 std-only 内核（`invariants/kernel.rs`），本程序 `#[path]` 包含
//! **同一个文件**：改内核任何一处（比如把「无跳号」放宽），下面的反向用例必须
//! 同时红。此前本文件自己实现了一遍 seq 检查——生产改判据、这里仍全绿的
//! 「验的东西与跑的不是同一个」，正是那条要堵的失效。
//!
//! ## 两种数据，两种结论
//!
//! 1. **自造日志**（`healthy_log` / `violated_log`）证明「检查器会响」；
//! 2. **真实日志回放**（`fixtures/real-session.tsv`，一次真实运行的 37 条事件信封）
//!    证明「跑起来的系统遵守不变量」。
//!
//! 第二种才是 GO-000 要的证据。自造数据能证明检查不是摆设，但**证不了**系统遵守——
//! 那份数据是自己写的，写它的时候心里已经有答案了。
//!
//! ## I3 的边界（真实数据第一次回放就撞出来的）
//!
//! 真实日志里 `chat.assistant.final` 的 `cost_ms = 893184`，而同一条 `user.message`
//! 携带的 `budget_ms = 60000`——差 14.9 倍。这**不是违规**，是口径错位：
//! `turn_runner.rs` 写进收束格的是**整轮累计**（含全部工具往返），而 `budget_ms` 是
//! **单次模型调用**的事前闸门。用 `cost_ms > budget_ms` 判「到点必答」，量的是两把尺。
//!
//! 于是 I3 在**信封上**只能判结构面：每轮有一条收束格。时间面（这次调用有没有超时、
//! 超时了有没有走兜底）判不了——那需要「每次调用的耗时」，而日志里只有轮累计。
//! 这条缺口留在这里，不拿一条恒空的检查假装补上。
//!
//! 判据纪律：每条断言都配一个**反向用例**：把检查关掉，违规必须变得检测不到。
//!
//! 编译运行：rustc --edition 2021 invariants.rs -o inv && ./inv

// `#[path]` 指向仓库里的**生产文件**（与 recall_eval 挂 recall_score、
// consolidation_fidelity 挂 merge_kernel 同一形态）——这一行就是「判定与生产
// 共处一处」的全部含义。kernel 是 std-only（理由见它的文件头）；本程序只用它
// 四个内核里的两个，`allow(dead_code)` 只在 verify 侧生效（生产 crate 全用）。
#[allow(dead_code)]
#[path = "../../../crates/symbio-core/src/invariants/kernel.rs"]
mod kernel;

use kernel::{seq_monotonic_core, unique_per_turn_core};

#[derive(Debug, Clone)]
struct Event {
    seq: u64,
    kind: String,
    turn: u64,
    produced_by: Option<u64>, // I2：产生本事件的上游
    cost_ms: u64,             // 轮累计耗时（不是单次调用，见文件头）
    status: String,           // 收束格的状态（`completed` / `aborted` / `-`）
    fell_back: bool,          // 走了兜底
    budget_ms: Option<u64>,   // 该轮的单次调用预算（只有用户格带）
    reason: Option<String>,   // 停机原因（闭集；只有停机格带）
}

#[derive(Debug, PartialEq, Eq)]
enum Violation {
    /// seq 处**应为** `expect`（内核语义：已入库事件 seq 恰为 0..n-1，严格单调且无跳号）。
    I1SeqNotMonotonic { seq: u64, expect: u64 },
    I1DoubleFinal { turn: u64 },
    I2NoProvenance { seq: u64 },
    /// `produced_by` 指向的 seq 不存在或**不更早**（悬空 / 前向边）。
    I2DanglingProvenance { seq: u64, up: u64 },
    I3NoClosure { turn: u64 },
    /// 收束格 / `authz.denied` 的 `reason` 缺失或越界。
    I3NoClosureReason { seq: u64, got: String },
}

struct Check {
    enable_i1_seq: bool,
    enable_i1_final: bool,
    enable_i2: bool,
    /// I2 的另一半：`produced_by` 指向的 seq 必须**更早且存在**。
    /// 与 `enable_i2`（只问「有没有」）分开，是为了让两条各有自己的反向用例。
    enable_i2_dangling: bool,
    enable_i3: bool,
    /// I3 的另一半：停机原因 `reason` 必填且在闭集内。与 `enable_i3`（只问
    /// 「有没有收束」）分开，是为了让两条各有自己的反向用例。
    enable_i3_reason: bool,
}

impl Check {
    fn all() -> Self {
        Check {
            enable_i1_seq: true,
            enable_i1_final: true,
            enable_i2: true,
            enable_i2_dangling: true,
            enable_i3: true,
            enable_i3_reason: true,
        }
    }
}

const FINAL: &str = "chat.assistant.final";

/// 停机原因的闭集——与生产侧 `ClosureReason::ALL` **同一份取值**。
/// 这里手写而不是引用：验证器被 `#[path]` 包含进来跑，不能拖进整个 core；
/// 两处各写一份的代价由 `closure_reason_matches_production` 这条用例钉住。
const REASONS: [&str; 5] = ["completed", "interrupted", "exhausted", "fallback", "denied"];

/// 这一条是**停机格**吗：`turn × closed` 的两种收束格 + `authz.denied`（拒绝不产生
/// 收束格，但它同样是一次"停"）。与生产侧 `closure_reason_closed` 同口径。
fn is_stop(e: &Event) -> bool {
    e.kind == FINAL
        || e.kind == "chat.assistant.fallback"
        || e.kind == "authz.denied"
}

/// 需要溯源的事件种类（断言类）
fn needs_provenance(kind: &str) -> bool {
    kind.starts_with("task.verified")
        || kind.starts_with("classify.")
        || kind.starts_with("artifact.")
        || kind.starts_with("commitment.")
        // `task.asserted` 是断言类：谁断言的、基于什么是它成立的前提。
        // 真实日志里这一格正是靠 `produced_by` 串起「巡检 → 结论」的。
        || kind.starts_with("task.asserted")
}

fn check(log: &[Event], c: &Check) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut closed: Vec<u64> = Vec::new();

    // ── I1-seq：判定走生产内核 ──
    // 内核语义 = 生产语义：已入库事件 seq 恰为 0..n-1（严格单调且**无跳号**——
    // 比「非降序」严一档：跳号意味着有事件被静默丢弃或绕过唯一写入口）。
    // 自造日志因此也从 0 起（`healthy_log`），与 `WalStore::append` 的分配一致。
    if c.enable_i1_seq {
        let seqs: Vec<u64> = log.iter().map(|e| e.seq).collect();
        for (_, actual, expect) in seq_monotonic_core(&seqs) {
            out.push(Violation::I1SeqNotMonotonic { seq: actual, expect });
        }
    }

    // ── I1-final：判定走生产内核 ──
    // 哪些事件算数（FINAL 判据）是本文件的解析层；「每 turn 至多一条」在内核。
    // 本文件的事件没有 event_id 列（信封只留七列），以 seq 的字符串形态当标识。
    if c.enable_i1_final {
        let final_events: Vec<&Event> = log.iter().filter(|e| e.kind == FINAL).collect();
        let ids: Vec<String> = final_events.iter().map(|e| e.seq.to_string()).collect();
        let finals = final_events
            .iter()
            .map(|e| e.turn)
            .zip(ids.iter().map(String::as_str));
        for (_, _, turn) in unique_per_turn_core(finals) {
            out.push(Violation::I1DoubleFinal { turn });
        }
    }

    // I2 的另一半：有了边以后**指向谁**。切片两头的跨片边放行（指向片外更早/更晚的
    // 事件）——与生产侧 `provenance_dangling(events, allow_slice_edges)` 同一条宽限纪律：
    // 宽限写在形参上、头尾对称，判据本身不放松。
    let seqs: Vec<u64> = log.iter().map(|e| e.seq).collect();
    let head = seqs.iter().copied().min();
    let tail = seqs.iter().copied().max();

    for e in log {
        if e.kind == FINAL && !closed.contains(&e.turn) {
            closed.push(e.turn);
        }

        if c.enable_i2 && needs_provenance(&e.kind) && e.produced_by.is_none() {
            out.push(Violation::I2NoProvenance { seq: e.seq });
        }

        if c.enable_i2_dangling {
            if let Some(up) = e.produced_by {
                let bad = if seqs.contains(&up) {
                    // 片内：因果不许倒（前向边 / 自指）。
                    up >= e.seq
                } else {
                    // 片外（头 / 尾）：纯函数无从分辨"切掉了"与"写错了"，头尾对称宽限。
                    let outside = head.is_some_and(|h| up < h) || tail.is_some_and(|t| up > t);
                    !outside
                };
                if bad {
                    out.push(Violation::I2DanglingProvenance { seq: e.seq, up });
                }
            }
        }
    }

    // I3 的另一半：停机原因必须在闭集内。
    if c.enable_i3_reason {
        for e in log.iter().filter(|e| is_stop(e)) {
            let got = e.reason.as_deref().unwrap_or("");
            if REASONS.iter().all(|r| *r != got) {
                out.push(Violation::I3NoClosureReason {
                    seq: e.seq,
                    got: got.to_string(),
                });
            }
        }
    }

    // I3：出现过事件的每一轮都必须有收束格——「静默消失」正是这条要防的形态
    if c.enable_i3 {
        let mut turns: Vec<u64> = log.iter().map(|e| e.turn).collect();
        turns.sort_unstable();
        turns.dedup();
        for t in turns {
            if !closed.contains(&t) {
                out.push(Violation::I3NoClosure { turn: t });
            }
        }
    }

    out
}

/// 一个健康的日志：单通道、有溯源、每轮有收束格
/// 一个健康的日志：单通道、有溯源、每轮有收束格。
///
/// seq 从 0 起严格连续——这是**生产语义**（`WalStore::append` 分配 0..n-1），
/// 也是生产内核判的形状（严格单调且无跳号）。旧版自造日志从 1 起、只判非降序，
/// 那是「仿判据」时代的残留；挂上生产内核后自造日志也按生产的形状造。
fn healthy_log() -> Vec<Event> {
    // 溯源一律指向**根事件**（seq 0 的用户格）——生产里就是这个形状：
    // 这一轮的一切都是"被那句话引出来的"。此前每条指向自己（`Some(i)` 在 seq i 上），
    // 那是自指边，`produced_by_coverage` 只查非空所以一直是绿的。
    vec![
        ev(0, "user.message", 1, None, 0, None),
        ev(1, "classify.verdict", 1, Some(0), 40, None),
        ev(2, "task.created", 1, Some(0), 300, None),
        ev(3, "artifact.added", 1, Some(0), 900, None),
        ev(4, "task.verified", 1, Some(0), 200, None),
        // 收束格必带停机原因——`completed` = 自然说完。
        ev(5, FINAL, 1, Some(0), 150, Some("completed")),
    ]
}

#[allow(clippy::too_many_arguments)]
fn ev(
    seq: u64,
    kind: &str,
    turn: u64,
    produced_by: Option<u64>,
    cost_ms: u64,
    reason: Option<&str>,
) -> Event {
    Event {
        seq,
        kind: kind.to_string(),
        turn,
        produced_by,
        cost_ms,
        status: if kind == FINAL { "completed".to_string() } else { "-".to_string() },
        fell_back: false,
        budget_ms: None,
        reason: reason.map(str::to_string),
    }
}


/// 注入五类违规：seq 撞号、第二个 final、无溯源、某轮没有收束格、停机原因越界。
///
/// 「seq 回退」的形状按内核语义造：追加的序列 6,7,8 本身连续，第 1 条 seq=3
/// 是**重号**——内核报「seq 3 处应为 6」。
fn violated_log() -> Vec<Event> {
    let mut v = healthy_log();
    v.push(ev(3, "task.progress", 1, Some(0), 10, None)); // seq 撞号（3 已存在）
    v.push(ev(6, FINAL, 1, Some(0), 10, Some("completed"))); // 双 final
    v.push(ev(7, "task.verified", 1, None, 10, None)); // 无溯源
    v.push(ev(8, "task.progress", 2, Some(7), 10, None)); // 第 2 轮没有收束格
    // 停机原因越界：写了一个不在闭集里的值。用 `authz.denied` 而不是
    // 再塞一条 final——后者会把第 2 轮收束掉，于是「无收束」那条判据反而消失了
    // （注入一条违规弄丢另一条，是这类自造假绿里最常见的一种）。
    v.push(ev(9, "authz.denied", 2, Some(0), 10, Some("timeout")));
    v
}

/// 真实日志信封（`fixtures/README.md` 说明来历与只留信封的理由）。
///
/// 用 `include_str!` 而不是运行时读文件：验证器会被门禁从固定目录跑，
/// 运行时的相对路径换个 cwd 就静默读到空——那会让「零违规」变成「零事件」的假绿。
const REAL_LOG_TSV: &str = include_str!("fixtures/real-session.tsv");

fn parse_envelopes(tsv: &str) -> Vec<Event> {
    let mut out = Vec::new();
    for line in tsv.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() >= 7, "真实日志的行必须是 7 列以上：{line:?}");
        out.push(Event {
            seq: f[0].parse().expect("seq 必须是数字"),
            kind: f[1].to_string(),
            turn: f[2].parse().expect("turn 必须是数字"),
            produced_by: if f[4] == "-" { None } else { f[4].parse().ok() },
            cost_ms: f[5].parse().expect("cost_ms 必须是数字"),
            status: f[6].to_string(),
            fell_back: f.get(7).is_some_and(|s| *s == "1"),
            budget_ms: f.get(8).and_then(|s| s.parse().ok()),
            reason: f.get(9).filter(|s| **s != "-").map(|s| s.to_string()),
        });
    }
    out
}

fn main() {
    println!("═══ 三条不变量的可执行判据 ═══");
    let all = Check::all();

    // ══════════ 第一部分：自造日志（证明检查器会响）══════════
    let clean = check(&healthy_log(), &all);
    println!("\n健康日志（{} 条事件）→ 违规 {} 条", healthy_log().len(), clean.len());

    let dirty = check(&violated_log(), &all);
    println!("注入违规后 → 违规 {} 条：", dirty.len());
    for d in &dirty {
        println!("  {:?}", d);
    }

    println!("\n═══ 反向用例：关掉检查，违规必须消失 ═══");
    let no_i1 = check(&violated_log(), &Check { enable_i1_seq: false, enable_i1_final: false, ..Check::all() });
    let no_i2 = check(
        &violated_log(),
        &Check { enable_i2: false, enable_i2_dangling: false, ..Check::all() },
    );
    let no_i3 = check(
        &violated_log(),
        &Check { enable_i3: false, enable_i3_reason: false, ..Check::all() },
    );
    println!("  关掉 I1 → 剩 {} 条（少了 {} 条）", no_i1.len(), dirty.len() - no_i1.len());
    println!("  关掉 I2 → 剩 {} 条（少了 {} 条）", no_i2.len(), dirty.len() - no_i2.len());
    println!("  关掉 I3 → 剩 {} 条（少了 {} 条）", no_i3.len(), dirty.len() - no_i3.len());

    assert!(clean.is_empty(), "健康日志不应有违规");
    // 注入 5 类违规捕到 **6** 条：seq 重号在内核语义下报两条——重号位「处应为 6」
    // 与其后一位「处应为 4」。这不是过杀：生产语义里 seq 撞车之后**全部错位**，
    // 只报第一条反而会把「后面还有多少条对不上」藏起来。
    assert_eq!(dirty.len(), 6, "注入的 5 类违规应被全部捕获（seq 重号算两条）");
    assert!(
        !no_i1.iter().any(|v| matches!(v, Violation::I1SeqNotMonotonic { .. } | Violation::I1DoubleFinal { .. })),
        "反向用例：关掉 I1 后 I1 类违规必须检测不到"
    );
    assert!(
        !no_i2
            .iter()
            .any(|v| matches!(v, Violation::I2NoProvenance { .. } | Violation::I2DanglingProvenance { .. })),
        "反向用例：关掉 I2 后两条 I2 判据（有没有 / 指向谁）都必须检测不到"
    );
    assert!(
        !no_i3.iter().any(|v| matches!(v, Violation::I3NoClosure { .. })),
        "反向用例：关掉 I3 后 I3 类违规必须检测不到"
    );

    // ══════════ 第二部分：真实日志回放（证明系统遵守）══════════
    println!("\n═══ 真实日志回放（fixtures/real-session.tsv）═══");
    let real = parse_envelopes(REAL_LOG_TSV);
    assert!(real.len() >= 30, "真实日志至少 30 条事件（读到 {} 条 ⇒ 夹具被截断或解析失败）", real.len());
    let turns: usize = {
        let mut t: Vec<u64> = real.iter().map(|e| e.turn).collect();
        t.sort_unstable();
        t.dedup();
        t.len()
    };
    // 回放**不跑** C6：这份夹具只留信封（没有载荷），而 `reason` 在载荷里，
    // 且它采于 C6 上线之前。硬跑只会得到「每条收束格都缺 reason」——那不是判据
    // 错了，是夹具没这个字段。C6 对真实数据形状的证明在下方单独做（取真实那条
    // 收束格，喂合法值 / 越界值各一次）。
    let real_v = check(&real, &Check { enable_i3_reason: false, ..Check::all() });
    println!("  {} 条事件 · {} 轮 · 违规 {} 条", real.len(), turns, real_v.len());
    for v in &real_v {
        println!("  ✗ {:?}", v);
    }
    assert!(
        real_v.is_empty(),
        "真实日志上 I1/I2/I3 必须零违规（有违规 = 跑起来的系统破了不变量，或判据本身错了）：{real_v:?}"
    );

    // ── 把「已知的口径错位」钉成事实，而不是留给人去猜 ──
    //
    // 真实日志里收束格的轮累计耗时远超该轮的单次预算。这不是 I3 违规（见文件头），
    // 但如果没人把它写死，后来的人两种误读都会发生：把它当违规去"修"，
    // 或把它当正常而顺手用 cost_ms 去做时延判据。所以在这里断言它**存在**。
    let overrun: Vec<&Event> = real
        .iter()
        .filter(|e| {
            e.kind == FINAL
                && real
                    .iter()
                    .find(|u| u.turn == e.turn && u.budget_ms.is_some())
                    .and_then(|u| u.budget_ms)
                    .is_some_and(|b| e.cost_ms > b)
        })
        .collect();
    assert!(
        !overrun.is_empty(),
        "这条断言钉的是「轮累计 ≠ 单次预算」这个口径错位仍然存在；它消失了说明 \
         cost_ms 或 budget_ms 的口径变了——先改文件头的边界说明，再删这条断言"
    );
    let worst = overrun.iter().map(|e| e.cost_ms).max().unwrap_or(0);
    let budget = overrun
        .first()
        .and_then(|e| real.iter().find(|u| u.turn == e.turn && u.budget_ms.is_some()))
        .and_then(|u| u.budget_ms)
        .unwrap_or(0);
    let fell_back = real.iter().filter(|e| e.fell_back).count();
    let closed_status: Vec<&str> = real.iter().filter(|e| e.kind == FINAL).map(|e| e.status.as_str()).collect();
    assert!(
        closed_status.iter().all(|s| !s.is_empty() && *s != "-"),
        "每条收束格都必须带状态（`completed` / `aborted` / …）——空状态等于「这条收束没说自己怎么回事」"
    );
    println!(
        "  ⚠ 口径错位（已知）：收束格 cost_ms 最大 {worst}，该轮 budget_ms {budget}——\
         轮累计 vs 单次闸门，I3 的时间面在信封上判不了"
    );
    println!("  兜底收束 {} 条 · 收束状态 {:?}", fell_back, closed_status);

    // ── 反向用例：真实日志上同样能红（判据对真数据不是恒绿）──
    println!("\n═══ 反向用例：真实日志上注入违规 ═══");
    let mut bad_seq = real.clone();
    bad_seq[10].seq = bad_seq[5].seq; // 序号撞车
    let mut double_final = real.clone();
    double_final.push(ev(999, FINAL, 0, Some(24), 1, Some("completed"))); // 第 0 轮再收一次
    let mut no_prov = real.clone();
    let at = no_prov.iter().position(|e| e.kind == "artifact.added").unwrap();
    no_prov[at].produced_by = None;
    let no_closure: Vec<Event> = real.iter().filter(|e| e.kind != FINAL).cloned().collect();
    // 悬空边：把一条事件的溯源指到一个**不存在**的 seq（比它自己小，所以不是前向边，
    // 只是片内查无此号——真实系统里这只会来自「边写错了」或「日志被截断」）。
    let mut dangling = real.clone();
    let at_d = dangling.iter().position(|e| e.seq > 3).unwrap();
    dangling[at_d].produced_by = Some(2);
    dangling.retain(|e| e.seq != 2);

    let n1 = check(&bad_seq, &all);
    let n2 = check(&double_final, &all);
    let n3 = check(&no_prov, &all);
    let n4 = check(&no_closure, &all);
    let n5 = check(&dangling, &all);
    println!(
        "  seq 撞车 → {} 条 · 双收束 → {} 条 · 去溯源 → {} 条 · 无收束 → {} 条 · 悬空溯源 → {} 条",
        n1.len(),
        n2.len(),
        n3.len(),
        n4.len(),
        n5.len()
    );
    assert!(n1.iter().any(|v| matches!(v, Violation::I1SeqNotMonotonic { .. })), "真实日志上 seq 撞车必须红");
    assert!(n2.iter().any(|v| matches!(v, Violation::I1DoubleFinal { .. })), "真实日志上双收束必须红");
    assert!(n3.iter().any(|v| matches!(v, Violation::I2NoProvenance { .. })), "真实日志上去溯源必须红");
    assert!(n4.iter().any(|v| matches!(v, Violation::I3NoClosure { .. })), "真实日志上无收束必须红");
    assert!(
        n5.iter().any(|v| matches!(v, Violation::I2DanglingProvenance { .. })),
        "真实日志上悬空溯源必须红——`produced_by` 只查「有没有」时这一条是绿的"
    );

    // ── C6：停机原因闭集 ──
    //
    // 真实日志的回放**不跑**这条：夹具只留信封（没有载荷），而 `reason` 在载荷里，
    // 且这份夹具采于 C6 上线之前。把它并进 `check(&real, &all)` 只会得到一条
    // 「每条收束格都缺 reason」的假红——那不是判据错了，是夹具没这个字段。
    // 所以改成**在真实数据形状上注入**：取真实那条收束格，分别喂合法值与越界值。
    let at_stop = real.iter().position(|e| is_stop(e)).expect("夹具里有一条收束格");
    let mut good_reason = real.clone();
    good_reason[at_stop].reason = Some("interrupted".to_string());
    let mut bad_reason = real.clone();
    bad_reason[at_stop].reason = Some("timeout".to_string());
    let mut no_reason = real.clone();
    no_reason[at_stop].reason = None;

    let r_ok = check(&good_reason, &all);
    let r_bad = check(&bad_reason, &all);
    let r_none = check(&no_reason, &Check { enable_i3_reason: false, ..Check::all() });
    println!(
        "  停机原因：合法值 → {} 条 · 越界值 → {} 条 · 缺值（关掉判据）→ {} 条",
        r_ok.len(),
        r_bad.len(),
        r_none.len()
    );
    assert!(
        r_ok.is_empty(),
        "真实数据形状上喂一个合法 `reason` 必须全绿（否则判据在误杀）：{r_ok:?}"
    );
    assert!(
        r_bad
            .iter()
            .any(|v| matches!(v, Violation::I3NoClosureReason { .. })),
        "真实数据形状上喂一个越界 `reason` 必须红"
    );
    assert!(
        !r_none
            .iter()
            .any(|v| matches!(v, Violation::I3NoClosureReason { .. })),
        "反向用例：关掉 C6 后越界必须检测不到"
    );

    println!("\n✅ 全部断言通过：自造日志 + 真实日志（{} 条 / {} 轮）上三条不变量各自独立起作用。", real.len(), turns);
    println!("   边界：I3 只判得了结构面（每轮有收束格 / 停机原因在闭集内）——时间面需要单次调用耗时，日志里只有轮累计。");
}
