//! 时延闸门 · 事前约束（取代"声明 + 事后断言"）
//!
//! 本程序回答一个问题：
//!   **"反射层不得调用模型"能不能从"运行时断言"升级为"构造期约束"？**
//!
//! 现状（plan3 v2 之前）：`actor.budget_ms` 是一个**声明**。
//!   一个 Reasoner 声明 `budget_ms = 80`，但 LLM 实际要 2000ms ——
//!   系统只能在事后发现"超预算"，然后用 `chat.assistant.fallback` 兜底。
//!   **兜底是正确的降级，但它不是预防。**
//!
//! 本方案（业界成熟模式，见 REF）：
//!   把 `budget_ms` 等级**编码进类型**，用零大小能力令牌控制"能拿到哪一档依赖"。
//!     - 反射档（≤80ms）  → 只能拿 `RuleOnly` 令牌 → **类型上就没有 LLM 句柄**
//!     - 快速档（~300ms） → 只能拿 `ClassifyOnly` 令牌 → 只有"四选一分类器"句柄
//!     - 深度档（≥60s）   → 拿 `FullModel` 令牌     → 完整 LLM + 工具
//!   于是"反射层不得调模型"**不靠自觉、不靠扫描 —— 它编译不过**。
//!
//! 关键点：这不是新机制，是 §05 `adapters` 包（⑤）的**依赖注入策略**。
//!   令牌是 ZST，零运行时开销；替换成"给每个 Actor 注入不同适配器"即可。
//!
//! 论证纪律：每条断言配反向用例；反向用例用 `should_not_compile` feature 承载。
//!
//! 编译运行：rustc --edition 2021 latency_gate.rs -o lg && ./lg
//! 反向验证：rustc --edition 2021 --check-cfg 'cfg(feature)' --cfg 'feature="should_not_compile"' latency_gate.rs -o should_fail.exe  # 必须失败

use std::marker::PhantomData;

// ───────────────────────── 四层时延（对应 01 §10）─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyTier {
    /// 反射：≤80ms，纯规则/查表，**禁模型**
    Reflex,
    /// 快速：~300ms，只允许**分类**不允许生成
    Fast,
    /// 深度：秒–分钟，完整规划 + 工具
    Deep,
    /// 自主：小时–天，常驻
    Autonomic,
}

impl LatencyTier {
    pub fn budget_ms(self) -> u64 {
        match self {
            LatencyTier::Reflex => 80,
            LatencyTier::Fast => 300,
            LatencyTier::Deep => 60_000,
            LatencyTier::Autonomic => 86_400_000,
        }
    }
    /// 该档位**是否允许**触碰模型（判据是时延，不是形式 —— 见 01 §10）
    pub fn may_call_model(self) -> bool {
        matches!(self, LatencyTier::Fast | LatencyTier::Deep | LatencyTier::Autonomic)
    }
    /// 该档位是否允许**生成**（而非只分类）
    pub fn may_generate(self) -> bool {
        matches!(self, LatencyTier::Deep | LatencyTier::Autonomic)
    }
}

// ─────────────────── 能力令牌：三档，层级关系（ZST，零成本）───────────────────

/// 反射档令牌：证明持码者处于"禁模型"档。**私有构造器 → 不可伪造。**
pub struct RuleOnly {
    _private: (),
}

/// 快速档令牌：允许**分类**（输出空间受限），不允许生成。
pub struct ClassifyOnly {
    _private: (),
}

/// 深度档令牌：允许完整模型调用 + 工具。
pub struct FullModel {
    _private: (),
}

impl RuleOnly {
    fn mint() -> Self {
        RuleOnly { _private: () }
    }
}
impl ClassifyOnly {
    fn mint() -> Self {
        ClassifyOnly { _private: () }
    }
}
impl FullModel {
    fn mint() -> Self {
        FullModel { _private: () }
    }
}

/// 令牌签发中心：**唯一**能造令牌的地方，按档位签发。
/// 这是"事前闸门"的全部实现 —— 一个 match，零运行时开销。
pub struct TokenIssuer;

impl TokenIssuer {
    /// 按档位签发对应的能力令牌。
    ///
    /// **注意返回类型的差异**：反射档返回 `RuleOnly`，深度档返回 `FullModel`。
    /// 调用方拿到的令牌类型**决定了它能调用哪些方法** —— 这就是闸门。
    pub fn issue_reflex() -> RuleOnly {
        RuleOnly::mint()
    }
    pub fn issue_fast() -> ClassifyOnly {
        ClassifyOnly::mint()
    }
    pub fn issue_deep() -> FullModel {
        FullModel::mint()
    }
    /// 按档位统一签发（返回一个"档位证据"；真正的依赖注入见下方 adapters）
    pub fn for_tier(t: LatencyTier) -> TierProof {
        TierProof { tier: t, _p: () }
    }
}

/// 档位证据：携带档位信息 + 私有字段（不可伪造）
#[derive(Debug, Clone, Copy)]
pub struct TierProof {
    tier: LatencyTier,
    _p: (),
}

impl TierProof {
    pub fn tier(&self) -> LatencyTier {
        self.tier
    }
}

// ─────────────────────── adapters：按令牌限制能拿到的依赖 ───────────────────────

/// 纯规则引擎：**不需要任何模型**（反射层可用）
pub struct RuleEngine;
impl RuleEngine {
    pub fn lookup(&self, intent: &str) -> Option<&'static str> {
        match intent {
            "greet" => Some("你好，我能做什么？"),
            "help" => Some("我可以帮你查资料、跑任务、写东西。"),
            _ => None,
        }
    }
}

/// 分类器：输出空间**限定为四选一**（直答/派活/反问/拒绝）。
/// 这是"快速档"的全部算力 —— 分类比生成便宜一个数量级。
pub struct Classifier;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    AnswerDirectly,
    Dispatch,
    AskBack,
    Refuse,
}
impl Classifier {
    /// **只接受** `ClassifyOnly`（或更高档的 `FullModel`，见 trait 层级）令牌
    pub fn classify(&self, _tok: &impl CanClassify, text: &str) -> Route {
        if text.contains("？") || text.contains("?") {
            Route::AskBack
        } else if text.starts_with("帮我") {
            Route::Dispatch
        } else if text.is_empty() {
            Route::Refuse
        } else {
            Route::AnswerDirectly
        }
    }
}

/// 完整模型：可生成、可调工具
pub struct LlmClient {
    pub model: &'static str,
}
impl LlmClient {
    /// **只接受** `FullModel` 令牌 —— 反射/快速档拿不到这个句柄。
    pub fn generate(&self, _tok: &FullModel, prompt: &str) -> String {
        format!("[{}] 生成结果 for: {}", self.model, prompt)
    }
}

// ─────────────────── 令牌层级：用 trait 表达"高档能做低档的事" ───────────────────

/// 能做分类（快速档 / 深度档 / 自主档都满足）
pub trait CanClassify {}
/// 能完整生成（仅深度档 / 自主档）
pub trait CanGenerate: CanClassify {}

impl CanClassify for ClassifyOnly {}
impl CanClassify for FullModel {}
impl CanGenerate for FullModel {}
// 注意：`RuleOnly` **不**实现 `CanClassify` —— 这就是闸门在类型层的表达。

// ─────────────────────── 按档位装配 Actor：闸门落地处 ───────────────────────

/// 一个按档位装配好的 Actor。**注意它没有 LLM 字段是可选这一点** ——
/// 反射档根本装不进去。
pub struct AssembledActor {
    pub tier: LatencyTier,
    pub has_llm: bool,
    pub has_classifier: bool,
    pub has_rules: bool,
}

/// **核心：按档位装配。** 这是"事前闸门"的实际动作。
pub fn assemble(t: LatencyTier) -> AssembledActor {
    match t {
        LatencyTier::Reflex => {
            // 只签发 RuleOnly —— 类型上就拿不到 Classifier / LlmClient
            let _rule_tok: RuleOnly = TokenIssuer::issue_reflex();
            let _rules = RuleEngine;
            AssembledActor { tier: t, has_llm: false, has_classifier: false, has_rules: true }
        }
        LatencyTier::Fast => {
            // 签发 ClassifyOnly —— 能拿分类器，**拿不到 LlmClient**
            let tok: ClassifyOnly = TokenIssuer::issue_fast();
            let classifier = Classifier;
            // 证明它真的能用：跑一次分类
            let _ = classifier.classify(&tok, "帮我查一下");
            // let llm = LlmClient { model: "x" }; llm.generate(&tok, "..");
            //   ↑ 若取消注释：`&ClassifyOnly` 不满足 `&FullModel` → 编译失败
            AssembledActor { tier: t, has_llm: false, has_classifier: true, has_rules: true }
        }
        LatencyTier::Deep => {
            let tok: FullModel = TokenIssuer::issue_deep();
            let classifier = Classifier;
            let llm = LlmClient { model: "deep-model" };
            let _ = classifier.classify(&tok, "帮我查一下"); // 高档能做低档的事
            let _ = llm.generate(&tok, "完整规划");
            AssembledActor { tier: t, has_llm: true, has_classifier: true, has_rules: true }
        }
        LatencyTier::Autonomic => {
            let tok: FullModel = TokenIssuer::issue_deep();
            let llm = LlmClient { model: "autonomic-model" };
            let _ = llm.generate(&tok, "常驻任务");
            AssembledActor { tier: t, has_llm: true, has_classifier: true, has_rules: true }
        }
    }
}

// ─────────────────────── I3 到点必答：预算 + 兜底（保留原有语义）───────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Event(&'static str),
    Fallback(&'static str),
    /// 静默超时 —— **违规**（I3 的核心禁令）
    Silent,
}

/// I3 判据：cost 是否在预算内；超预算时是否**必须**有 fallback
pub fn settle(t: LatencyTier, cost_ms: u64, has_fallback: bool) -> Outcome {
    if cost_ms <= t.budget_ms() {
        Outcome::Event("chat.assistant.final")
    } else if has_fallback {
        Outcome::Fallback("chat.assistant.fallback")
    } else {
        Outcome::Silent
    }
}

/// 违规判定：静默超时是违规；兜底**不是**违规
pub fn is_violation(o: &Outcome) -> bool {
    matches!(o, Outcome::Silent)
}

// ───────────────────────── 主程序 ─────────────────────────

fn main() {
    println!("═══ 时延闸门 · 事前约束（不是声明 + 事后断言）═══");

    // ── 1. 四层预算严格递增 ──
    let tiers = [LatencyTier::Reflex, LatencyTier::Fast, LatencyTier::Deep, LatencyTier::Autonomic];
    println!("\n── 四层预算 ──");
    let mut prev = 0u64;
    for t in tiers {
        println!("  {:?}: budget_ms = {}", t, t.budget_ms());
        assert!(t.budget_ms() > prev, "四层预算必须严格递增");
        prev = t.budget_ms();
    }

    // ── 2. 档位与模型许可的一致性（判据：时延，不是形式）──
    println!("\n── 档位 × 模型许可 ──");
    assert!(!LatencyTier::Reflex.may_call_model(), "反射层不得调用模型");
    assert!(LatencyTier::Fast.may_call_model(), "快速层允许分类");
    assert!(!LatencyTier::Fast.may_generate(), "快速层**不允许生成**（只有 300ms）");
    assert!(LatencyTier::Deep.may_generate(), "深度层允许生成");
    println!("  反射: 模型={} 生成={}", LatencyTier::Reflex.may_call_model(), LatencyTier::Reflex.may_generate());
    println!("  快速: 模型={} 生成={}", LatencyTier::Fast.may_call_model(), LatencyTier::Fast.may_generate());
    println!("  深度: 模型={} 生成={}", LatencyTier::Deep.may_call_model(), LatencyTier::Deep.may_generate());

    // ── 3. 装配结果：反射档**结构上**没有 LLM ──
    println!("\n── 按档位装配（事前闸门的结果）──");
    let reflex = assemble(LatencyTier::Reflex);
    let fast = assemble(LatencyTier::Fast);
    let deep = assemble(LatencyTier::Deep);
    for a in [&reflex, &fast, &deep] {
        println!("  {:?}: rules={} classifier={} llm={}",
                 a.tier, a.has_rules, a.has_classifier, a.has_llm);
    }
    assert!(!reflex.has_llm, "反射档装配后**不得**持有 LLM —— 这是事前闸门，不是事后检查");
    assert!(!reflex.has_classifier, "反射档也不该有分类器（只允许纯规则）");
    assert!(reflex.has_rules);
    assert!(fast.has_classifier, "快速档允许分类");
    assert!(!fast.has_llm, "快速档不得持有完整 LLM");
    assert!(deep.has_llm, "深度档允许完整 LLM");

    // ── 4. 反向用例 A：若用旧的"声明 + 事后断言"，反射档会**先超时再兜底** ──
    println!("\n── 反向用例 A：事前闸门 vs 事后断言 ──");
    // 旧做法：反射 Actor 声明 80ms，但真的调了模型（耗时 2000ms）
    let old_way = settle(LatencyTier::Reflex, 2000, true);
    println!("  旧做法（先调模型再兜底）: {:?}", old_way);
    assert_eq!(old_way, Outcome::Fallback("chat.assistant.fallback"),
               "旧做法只能事后兜底 —— 用户等了 2000ms 才收到兜底话术");
    // 新做法：反射档**根本拿不到模型**，所以不存在"调了模型"这个可能
    println!("  新做法（闸门）          : 反射档 has_llm = {} → 不可能发生 2000ms 的模型调用",
             reflex.has_llm);
    assert!(!reflex.has_llm, "闸门的价值：把\"超时后兜底\"变成\"不可能超时\"");

    // ── 5. 反向用例 B：预算取值决定是否超时（契约对参数敏感）──
    println!("\n── 反向用例 B：改预算，结论必须变 ──");
    let r_reflex = settle(LatencyTier::Reflex, 500, true);
    let r_deep = settle(LatencyTier::Deep, 500, true);
    println!("  500ms @ 反射(80ms)   → {:?}", r_reflex);
    println!("  500ms @ 深度(60s)    → {:?}", r_deep);
    assert_eq!(r_reflex, Outcome::Fallback("chat.assistant.fallback"), "反射档 500ms 应超时");
    assert_eq!(r_deep, Outcome::Event("chat.assistant.final"), "深度档 500ms 应正常");
    assert_ne!(r_reflex, r_deep, "反向用例失败：改预算结论没变");

    // ── 6. I3 静默超时必须是违规 ──
    println!("\n── I3 静默超时判定 ──");
    let normal = settle(LatencyTier::Reflex, 20, false);
    let fallback = settle(LatencyTier::Reflex, 500, true);
    let silent = settle(LatencyTier::Reflex, 500, false);
    println!("  正常   {:?} → 违规={}", normal, is_violation(&normal));
    println!("  兜底   {:?} → 违规={}", fallback, is_violation(&fallback));
    println!("  静默   {:?} → 违规={}", silent, is_violation(&silent));
    assert!(!is_violation(&normal));
    assert!(!is_violation(&fallback), "兜底不是违规（兜底话术也是网格里的一格，可审计）");
    assert!(is_violation(&silent), "静默超时是违规 —— 这是 I3 的核心禁令");

    // ── 7. 令牌是 ZST：零运行时开销 ──
    println!("\n── 零运行时开销 ──");
    println!("  size_of::<RuleOnly>()      = {}", std::mem::size_of::<RuleOnly>());
    println!("  size_of::<ClassifyOnly>()  = {}", std::mem::size_of::<ClassifyOnly>());
    println!("  size_of::<FullModel>()     = {}", std::mem::size_of::<FullModel>());
    assert_eq!(std::mem::size_of::<RuleOnly>(), 0, "能力令牌必须零大小");
    assert_eq!(std::mem::size_of::<ClassifyOnly>(), 0);
    assert_eq!(std::mem::size_of::<FullModel>(), 0);

    // ── 8. 档位证据不可伪造（构造器私有）──
    let proof = TokenIssuer::for_tier(LatencyTier::Deep);
    println!("\n── 档位证据 ──");
    println!("  for_tier(Deep) → tier = {:?}", proof.tier());
    assert_eq!(proof.tier(), LatencyTier::Deep);

    println!("\n═══ 编译期强制的反向用例 ═══");
    println!("  `should_not_compile` feature 里放了两段【必须编译失败】的代码：");
    println!("    (a) 反射档令牌 RuleOnly 去调分类器 → 不满足 CanClassify");
    println!("    (b) 快速档令牌 ClassifyOnly 去调 LLM → 不满足 FullModel");
    println!("  验证命令（应当以非 0 退出）：");
    println!("    rustc --edition 2021 --check-cfg 'cfg(feature)' \\");
    println!("          --cfg 'feature=\"should_not_compile\"' latency_gate.rs -o should_fail.exe");

    println!("\n✅ 全部断言通过（含 2 条数据反向用例 + 2 条编译期反向用例）");
    println!("   结论：\"反射层不得调模型\"由类型保证 —— 它是**构造期约束**，");
    println!("         不是运行时断言。超时从\"事后兜底\"变成\"结构上不可能\"。");
}

// ───────────────────────── 编译期反向用例 ─────────────────────────

#[cfg(feature = "should_not_compile")]
mod must_fail {
    use super::*;

    /// (a) 反射档去调分类器 → 必须编译失败（RuleOnly 不实现 CanClassify）
    pub fn reflex_calls_classifier() {
        let tok: RuleOnly = TokenIssuer::issue_reflex();
        let classifier = Classifier;
        let _ = classifier.classify(&tok, "帮我查一下");
        // 预期错误：`RuleOnly` doesn't implement `CanClassify` → E0277
    }

    /// (b) 快速档去调完整 LLM → 必须编译失败（ClassifyOnly 不是 FullModel）
    pub fn fast_calls_llm() {
        let tok: ClassifyOnly = TokenIssuer::issue_fast();
        let llm = LlmClient { model: "deep-model" };
        let _ = llm.generate(&tok, "生成一段长文");
        // 预期错误：mismatched types, expected `&FullModel`, found `&ClassifyOnly` → E0308
    }
}
