//! I3「到点必答」：发言路径的时延契约
//!
//! 对话是基础能力：没有发言路径预算就没有“快”。
//! 本程序把 I3 做成可执行的：
//!   · 四层时延 = 同一个 Actor 的四组 budget 取值，不是四套架构
//!   · 到点必须产出：要么正常事件，要么兜底事件（fallback）
//!   · 静默超时（什么都不产出）是违规，必须被检测到
//!
//! 编译运行：rustc --edition 2021 budget.rs -o bg && ./bg

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tier {
    Reflex,   // 反射：< 80ms
    Fast,     // 快速：~300ms
    Deep,     // 深度：秒–分钟
    Autonomic // 自主：小时–天
}

impl Tier {
    fn budget_ms(&self) -> u64 {
        match self {
            Tier::Reflex => 80,
            Tier::Fast => 300,
            Tier::Deep => 60_000,
            Tier::Autonomic => 86_400_000,
        }
    }
    fn label(&self) -> &'static str {
        match self {
            Tier::Reflex => "反射 (<80ms)",
            Tier::Fast => "快速 (~300ms)" ,
            Tier::Deep => "深度 (秒–分钟)",
            Tier::Autonomic => "自主 (小时–天)",
        }
    }
    /// 该层是否允许引入模型调用（判据是时延，不是"工具"这个形式）
    fn may_call_model(&self) -> bool {
        match self {
            Tier::Reflex => false,
            Tier::Fast => true, // 只允许分类，不允许生成
            Tier::Deep => true,
            Tier::Autonomic => true,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Yield {
    Event(&'static str),
    Fallback(&'static str),
    Silent, // 违规：静默超时
}

/// Actor 的运行时契约：耗时超预算 → 必须兜底，绝不静默。
fn run(tier: Tier, actual_ms: u64, enforce: bool) -> Yield {
    if actual_ms <= tier.budget_ms() {
        return Yield::Event("chat.assistant.final");
    }
    if enforce {
        Yield::Fallback("chat.assistant.fallback")
    } else {
        Yield::Silent
    }
}

fn violates_i3(_tier: Tier, y: &Yield) -> bool {
    // 唯一违规 = 静默超时。兜底是合法产出（它本身也是一条事件，可被元认知审计）。
    matches!(y, Yield::Silent)
}

fn main() {
    println!("═══ 四层时延 = 同一 Actor 的四组 budget 取值 ═══");
    let tiers = [Tier::Reflex, Tier::Fast, Tier::Deep, Tier::Autonomic];
    for t in tiers {
        println!("  {:>18}  budget = {:>9} ms   允许模型调用 = {}",
            t.label(), t.budget_ms(), t.may_call_model());
    }

    println!("\n═══ I3 到点必答 ═══");
    let reflex = Tier::Reflex;
    println!("  反射层耗时 20ms  → {:?}", run(reflex, 20, true));
    println!("  反射层耗时 500ms → {:?}（超预算，走兜底）", run(reflex, 500, true));
    println!("  反射层耗时 500ms 且不强制 → {:?}（静默超时，违规）", run(reflex, 500, false));

    println!("\n  违规判定：");
    println!("    正常   → 违规 = {}", violates_i3(reflex, &run(reflex, 20, true)));
    println!("    兜底   → 违规 = {}（兜底是合法的，兜底话术本身也是事件，可审计）",
        violates_i3(reflex, &run(reflex, 500, true)));
    println!("    静默   → 违规 = {}", violates_i3(reflex, &run(reflex, 500, false)));

    println!("\n═══ 反向用例 ═══");
    // 若把预算设得极大（等于取消契约），则超时应不再被捕获 —— 证明契约在起作用
    let no_budget = Tier::Autonomic;
    let y = run(no_budget, 500, true);
    println!("  把预算放宽到自主层（{}ms）后，500ms 的耗时 → {:?}", no_budget.budget_ms(), y);
    println!("  即：预算取值决定了是否被判定超时，契约对参数敏感");

    // ── 断言 ──
    assert!(Tier::Reflex.budget_ms() < Tier::Fast.budget_ms(), "反射层预算必须小于快速层");
    assert!(Tier::Fast.budget_ms() < Tier::Deep.budget_ms(), "快速层预算必须小于深度层");
    assert!(Tier::Deep.budget_ms() < Tier::Autonomic.budget_ms(), "深度层预算必须小于自主层");
    assert!(Tier::Reflex.budget_ms() <= 80, "反射层必须 ≤80ms（脊髓反射 20-50ms 的量级）");
    assert!(!Tier::Reflex.may_call_model(), "反射层不得引入模型调用");

    assert_eq!(run(reflex, 20, true), Yield::Event("chat.assistant.final"), "预算内应正常产出");
    assert_eq!(run(reflex, 500, true), Yield::Fallback("chat.assistant.fallback"), "超预算必须兜底");
    assert_eq!(run(reflex, 500, false), Yield::Silent, "不强制时会出现静默超时（这正是要禁止的）");
    assert!(violates_i3(reflex, &Yield::Silent), "静默超时必须判为违规");
    assert!(!violates_i3(reflex, &Yield::Fallback("chat.assistant.fallback")), "兜底不应判为违规");
    assert_eq!(y, Yield::Event("chat.assistant.final"), "反向用例：放宽预算后同一耗时应不再超时（证明契约对参数敏感）");

    println!("\n✅ 全部断言通过（含 1 条反向用例）：I3 到点必答成立。");
}
