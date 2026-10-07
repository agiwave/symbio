//! 语义漂移审计 · 架构适应度函数（进 CI 的常驻闸门）
//!
//! ── 这个程序要解决的是"审计盲区"，不是"某个具体 bug" ──
//!
//! 已知事实（plan3 自己的复盘）：
//!   - `mechanism_growth.rs` 与 `scenario_ladder.rs` 审计了 33 个场景，
//!     结论是"0 新增机制键、0 值域扩展"。这是**真结论**，但它回答的问题很窄。
//!   - 它们查的是：**"有没有发明新机制键 / 有没有扩展值域"**。
//!   - 它们**查不出**：**"某个参数取值的语义，会不会在场景演化中悄悄滑走"**。
//!
//! G7 就是从这个盲区里长出来的：
//!   `consolidate:max_gen` 这个取值一直都在，值域也没变，
//!   但"巩固的巩固"让它承载的**语义**（一条记忆的可用性）持续劣化——
//!   两个审计器都报"合规"，因为它们压根不度量"语义"。
//!
//! ── 因此本程序是一类**适应度函数（architectural fitness function）** ──
//!
//! 它把架构期望编码为**可执行检查**，常驻 CI。它的性质：
//!   1. **原子而非整体**：一个断言只查一个可度量的漂移维度；
//!   2. **静态而非动态**：对**参数表 + 事件采样**做分析，不跑系统；
//!   3. **触发而非持续**：随每次 PR / 每日跑一遍，失败即阻断。
//!   4. **失败信息带 `.Because()`**：说清"为什么这条期望存在"，而不是只报"断言失败"。
//!
//! ── 它审计的四个漂移维度（每个都是可度量的，不靠感觉）──
//!
//!   D1 值域滑移      —— 某个参数键出现了**参数表里没有的取值**（新机制悄悄诞生）
//!   D2 语义边界漂移  —— 取值没变，但**该取值在事件流里实际代表的东西**变了
//!   D3 退化取值滥用  —— 平凡值被当成"正常档位"高频使用（等于该机制其实没启用）
//!   D4 参数表—代码失配 —— 文档声明的机制数 ≠ 代码里实际出现的机制键集合
//!
//! 论证纪律：每个维度配**反向用例**（构造一个合规样本，再构造一个漂移样本，
//!           断言检查器对前者放过、对后者报警——否则这个检查器是摆设）。
//!
//! 编译运行：rustc --edition 2021 semantic_drift_audit.rs -o sda && ./sda

use std::collections::{BTreeMap, BTreeSet};

// ═══════════════════════ 权威参数表（与 01 §8 同源）═══════════════════════

/// 01 §8 声明的参数键 + 各自的值域。**这是审计的"契约"侧。**
/// 注意：这张表必须与 01 §8 逐字对应；对不上，D4 会抓出来。
pub fn authority_table() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    let mut m: BTreeMap<&'static str, BTreeSet<&'static str>> = BTreeMap::new();
    m.insert("store", ["memory", "wal", "sharded", "distributed"].into_iter().collect());
    m.insert(
        "projection",
        [
            "snapshot", "display", "readyset", "turnstate", "checkpoint", "eval", "recall",
            "consolidate", "reputation", "calibration", "budget", "replay", "skill_compile",
        ]
        .into_iter()
        .collect(),
    );
    m.insert("projection.param", ["consolidate:max_gen", "consolidate:min_fidelity"].into_iter().collect());
    m.insert("actor.pattern", ["decider", "reasoner", "translator"].into_iter().collect());
    m.insert("actor.budget_ms", ["80", "300", "60000", "86400000"].into_iter().collect());
    m.insert("scope", ["root", "child"].into_iter().collect());
    m.insert("vis_scope", ["thread_private", "shared", "public"].into_iter().collect());
    m
}

/// 参数的**平凡值**（J2）。被当成"正常档位"高频使用 → D3 报警。
pub fn trivial_value(key: &str) -> Option<&'static str> {
    match key {
        "store" => Some("memory"),
        "projection" => Some("eval"),
        "projection.param" => Some("consolidate:min_fidelity=0.0"),
        "actor.pattern" => Some("decider"),
        "actor.budget_ms" => Some("60000"),
        "scope" => Some("root"),
        "vis_scope" => Some("thread_private"),
        _ => None,
    }
}

// ═══════════════════════ 被审计的样本 ═══════════════════════

/// 一条参数赋值：键 + 取值 + 该取值在事件流里**实际代表**的语义标签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assign {
    pub key: String,
    pub value: String,
    /// 语义标签：这个取值**在业务上真正意味着什么**。
    /// 关键：同一个 (key, value) 在不同场景里语义标签**应当一致**。
    pub semantics: String,
}

/// 一个"场景"= 一组参数赋值。对应 33 个场景中的一个。
#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: String,
    pub assigns: Vec<Assign>,
}

fn a(key: &str, value: &str, semantics: &str) -> Assign {
    Assign { key: key.into(), value: value.into(), semantics: semantics.into() }
}

// ═══════════════════════ 四个漂移检查器 ═══════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum Finding {
    /// D1：出现了参数表里没有的取值
    ValueDomainSlip { scenario: String, key: String, value: String },
    /// D1 变体：出现了参数表里都没有的**键**
    NewMechanismKey { scenario: String, key: String },
    /// D2：同一个 (key, value) 在不同场景里语义标签不一致
    SemanticBoundaryDrift { key: String, value: String, semantics: Vec<(String, String)> },
    /// D3：平凡值当正常档位用（占比超过阈值）
    TrivialValueAbuse { key: String, value: String, ratio: f64, threshold: f64 },
    /// D4：代码里出现的键集合 ≠ 参数表的键集合
    TableCodeMismatch { only_in_code: Vec<String>, only_in_table: Vec<String> },
}

impl Finding {
    /// 适应度函数的 `.Because()` —— 失败信息必须说清"为什么这条期望存在"。
    pub fn because(&self) -> String {
        match self {
            Finding::ValueDomainSlip { scenario, key, value } => format!(
                "[D1 值域滑移] 场景「{}」用了 `{}={}`，但权威参数表没有这个取值。\n    \
                 Because：参数表是『系统存在什么』的唯一清单。表外的取值 = 存在一个没登记的机制——\n    \
                 它可能是合理的（那就该走 ADR 补进表），但绝不能**悄悄出现**。"
                , scenario, key, value),
            Finding::NewMechanismKey { scenario, key } => format!(
                "[D1 新机制键] 场景「{}」用了参数表里没有的键 `{}`。\n    \
                 Because：新机制键意味着系统多了一个『构成者』。J1 要求先证明它只是取值、不是机制。"
                , scenario, key),
            Finding::SemanticBoundaryDrift { key, value, semantics } => format!(
                "[D2 语义边界漂移] `{}={}` 在不同场景里代表了不同东西：{:?}\n    \
                 Because：这是审计盲区的核心形态——**取值没变、值域也没变**，\n    \
                 但『这个取值意味着什么』已经滑走了。G7 正是这么长出来的：\n    \
                 `consolidate:max_gen` 一直在表里，值域也没扩展，\n    \
                 可它的语义从『最多巩固几代』滑成了『记忆还剩多少可用信息』。"
                , key, value, semantics),
            Finding::TrivialValueAbuse { key, value, ratio, threshold } => format!(
                "[D3 平凡值滥用] `{}={}` 出现占比 {:.0}% > 阈值 {:.0}%。\n    \
                 Because：J2 要求每个参数有平凡值（不启用该机制时系统仍完整运行）。\n    \
                 但若平凡值成了**最常用档位**，说明这个机制在名义上启用、实际上没起作用——\n    \
                 这是另一种静默失效。"
                , key, value, ratio * 100.0, threshold * 100.0),
            Finding::TableCodeMismatch { only_in_code, only_in_table } => format!(
                "[D4 表—码失配] 代码里独有的键：{:?}；表里独有的键：{:?}\n    \
                 Because：参数表称『唯一一份』。若代码用了表外的键、或表里的键从未被用，\n    \
                 两者之一必然过期——而『唯一一份』这句话就不再成立。"
                , only_in_code, only_in_table),
        }
    }
}

/// D1：值域滑移 + 新机制键
pub fn check_d1_value_domain(table: &BTreeMap<&str, BTreeSet<&str>>, sc: &Scenario) -> Vec<Finding> {
    let mut out = vec![];
    for asg in &sc.assigns {
        match table.get(asg.key.as_str()) {
            None => out.push(Finding::NewMechanismKey {
                scenario: sc.name.clone(),
                key: asg.key.clone(),
            }),
            Some(domain) => {
                if !domain.contains(asg.value.as_str()) {
                    out.push(Finding::ValueDomainSlip {
                        scenario: sc.name.clone(),
                        key: asg.key.clone(),
                        value: asg.value.clone(),
                    });
                }
            }
        }
    }
    out
}

/// D2：语义边界漂移 —— 同一 (key, value) 的语义标签必须全场景一致
pub fn check_d2_semantic_drift(scenarios: &[Scenario]) -> Vec<Finding> {
    // (key, value) -> [(scenario, semantics)]
    let mut seen: BTreeMap<(String, String), Vec<(String, String)>> = BTreeMap::new();
    for sc in scenarios {
        for asg in &sc.assigns {
            seen.entry((asg.key.clone(), asg.value.clone()))
                .or_default()
                .push((sc.name.clone(), asg.semantics.clone()));
        }
    }
    let mut out = vec![];
    for ((key, value), entries) in seen {
        let distinct: BTreeSet<&String> = entries.iter().map(|(_, s)| s).collect();
        if distinct.len() > 1 {
            out.push(Finding::SemanticBoundaryDrift { key, value, semantics: entries });
        }
    }
    out
}

/// D3：平凡值滥用
///
/// ⚠️ 这里有一个**必须讲清的判断**：不是所有参数都应当"非平凡取值占多数"。
/// 例如 `scope=root`（非递归子智能体）在大多数场景里就是正常形态；
/// 要求它有充分变异性，等于要求"为了显得启用而故意递归"——那是荒谬的。
///
/// 因此 D3 **按参数分别配置阈值**：只对"架构上承诺会分化"的参数检查。
/// `D3_ENFORCED` 里没有的键 = 该参数允许长期取平凡值。
pub const D3_ENFORCED: &[(&str, f64)] = &[
    // 投影：13 个视图，若 90% 场景都用平凡值 `eval`，说明"视图体系"名存实亡
    ("projection", 0.9),
    // 行为模式：若 90% 都是 `decider`，说明 reasoner / translator 没真正启用
    ("actor.pattern", 0.9),
    // 时延档位：若 90% 都是 60000（深度），说明分层没有落地
    ("actor.budget_ms", 0.9),
];

pub fn check_d3_trivial_abuse(scenarios: &[Scenario], _unused: f64) -> Vec<Finding> {
    let mut total: BTreeMap<String, usize> = BTreeMap::new();
    let mut trivial_hits: BTreeMap<(String, String), usize> = BTreeMap::new();
    for sc in scenarios {
        for asg in &sc.assigns {
            *total.entry(asg.key.clone()).or_insert(0) += 1;
            if trivial_value(&asg.key).is_some_and(|t| t == asg.value.as_str()) {
                *trivial_hits.entry((asg.key.clone(), asg.value.clone())).or_insert(0) += 1;
            }
        }
    }
    let mut out = vec![];
    for ((key, value), hits) in trivial_hits {
        // 只对"承诺会分化"的参数执行——其余参数允许长期取平凡值
        let Some((_, threshold)) = D3_ENFORCED.iter().find(|(k, _)| *k == key) else {
            continue;
        };
        let t = *total.get(&key).unwrap_or(&1) as f64;
        let ratio = hits as f64 / t;
        if ratio > *threshold {
            out.push(Finding::TrivialValueAbuse {
                key,
                value,
                ratio,
                threshold: *threshold,
            });
        }
    }
    out
}

/// D4：参数表 — 代码 失配
pub fn check_d4_table_code(
    table: &BTreeMap<&str, BTreeSet<&str>>,
    scenarios: &[Scenario],
) -> Vec<Finding> {
    let in_code: BTreeSet<String> = scenarios
        .iter()
        .flat_map(|s| s.assigns.iter().map(|a| a.key.clone()))
        .collect();
    let in_table: BTreeSet<String> = table.keys().map(|k| (*k).to_string()).collect();

    let only_in_code: Vec<String> = in_code.difference(&in_table).cloned().collect();
    let only_in_table: Vec<String> = in_table.difference(&in_code).cloned().collect();
    if only_in_code.is_empty() && only_in_table.is_empty() {
        vec![]
    } else {
        vec![Finding::TableCodeMismatch { only_in_code, only_in_table }]
    }
}

// ═══════════════════════ 主程序 ═══════════════════════

/// 角色切换：把机器可用性纳入的辅助打印
fn print_findings(title: &str, findings: &[Finding]) {
    println!("\n── {} ──", title);
    if findings.is_empty() {
        println!("  无漂移 ✓");
    } else {
        for f in findings {
            println!("  ✗ {}", f.because());
        }
    }
}

fn main() {
    println!("═══ 语义漂移审计 · 架构适应度函数 ═══");
    println!("（四个维度：D1 值域滑移 / D2 语义边界漂移 / D3 平凡值滥用 / D4 表—码失配）");

    let table = authority_table();

    // ══════════ 反向用例前置：先证明每个检查器"会响" ══════════
    // 纪律：一个从不报警的检查器是没有价值的。所以先喂它一个**已知漂移**的样本。

    println!("\n########## 第一步：负样本（必须全部报警）##########");

    // —— D1 负样本：用了表外的取值 ——
    let slip = Scenario {
        name: "漂移样本-值域滑移".into(),
        assigns: vec![
            a("projection", "consolidate", "巩固：压缩记忆"),
            a("projection.param", "consolidate:min_fidelity=0.8", "保真度下界"),
            // ↓ 表里没有 `semantic_dedup` 这个投影
            a("projection", "semantic_dedup", "去重：合并相似记忆"),
        ],
    };
    let f_d1 = check_d1_value_domain(&table, &slip);
    print_findings("D1 · 值域滑移（负样本）", &f_d1);
    assert!(
        f_d1.iter().any(|f| matches!(f, Finding::ValueDomainSlip { value, .. } if value == "semantic_dedup")),
        "D1 检查器必须抓出表外取值 `semantic_dedup`"
    );

    // —— D1 负样本 2：新机制键 ——
    let new_key = Scenario {
        name: "漂移样本-新机制键".into(),
        assigns: vec![a("memory.compress_algo", "zstd", "压缩算法选择")],
    };
    let f_d1b = check_d1_value_domain(&table, &new_key);
    print_findings("D1 · 新机制键（负样本）", &f_d1b);
    assert!(matches!(f_d1b.first(), Some(Finding::NewMechanismKey { .. })),
            "D1 检查器必须抓出表外键 `memory.compress_algo`");

    // —— D2 负样本：一模一样的 (key,value)，语义标签却不同 ——
    let d2_bad = vec![
        Scenario {
            name: "场景 S06".into(),
            assigns: vec![a("projection.param", "consolidate:max_gen", "最多巩固几代")],
        },
        Scenario {
            name: "场景 S10".into(),
            assigns: vec![a("projection.param", "consolidate:max_gen", "记忆还剩多少可用信息")],
        },
    ];
    let f_d2 = check_d2_semantic_drift(&d2_bad);
    print_findings("D2 · 语义边界漂移（负样本）", &f_d2);
    assert!(
        f_d2.iter().any(|f| matches!(f, Finding::SemanticBoundaryDrift { key, .. } if key == "projection.param")),
        "D2 检查器必须抓出同一个取值语义不一致"
    );

    // —— D3 负样本：平凡值被当成主档位 ——
    let mut d3_bad = vec![];
    for i in 0..10 {
        d3_bad.push(Scenario {
            name: format!("场景 {}", i),
            // 10 次全部用平凡值 `eval` → 100% > 阈值 90%
            // 即：13 个投影视图里，实际只启用了 1 个，且是平凡值——"视图体系"名存实亡
            assigns: vec![a("projection", "eval", "视图定义")],
        });
    }
    let f_d3 = check_d3_trivial_abuse(&d3_bad, 0.0);
    print_findings("D3 · 平凡值滥用（负样本）", &f_d3);
    assert!(matches!(f_d3.first(), Some(Finding::TrivialValueAbuse { .. })),
            "D3 检查器必须抓出平凡值占比过高");

    println!("\n  → 四个检查器全部会响（不是摆设）✓");

    // ══════════ 正样本：33 个场景的真实形态 ══════════
    // 用"符合纪律"的样本跑一遍，必须**零报警**——否则检查器自身有假阳性。

    println!("\n########## 第二步：正样本（必须零报警）##########");

    let healthy: Vec<Scenario> = (1..=13)
        .map(|i| Scenario {
            name: format!("S{:02}", i),
            assigns: vec![
                a("actor.pattern", "reasoner", "行为模式"),
                a("actor.budget_ms", if i % 2 == 0 { "60000" } else { "300" }, "时延档位"),
                a("vis_scope", "thread_private", "可见域"),
                a("projection", if i <= 5 { "eval" } else { "recall" }, "视图定义"),
                a(
                    "projection.param",
                    if i <= 8 { "consolidate:max_gen" } else { "consolidate:min_fidelity" },
                    "巩固参数",
                ),
                a("scope", "root", "作用域"),
                a("store", "memory", "存储后端"),
            ],
        })
        .collect();

    // 语义标签一致性：同一个 (key,value) 在所有场景里标签相同（这正是"健康"的条件）
    let f_d1h = healthy.iter().flat_map(|s| check_d1_value_domain(&table, s)).collect::<Vec<_>>();
    let f_d2h = check_d2_semantic_drift(&healthy);
    let f_d3h = check_d3_trivial_abuse(&healthy, 0.0);
    let f_d4h = check_d4_table_code(&table, &healthy);

    print_findings("D1 · 值域（正样本）", &f_d1h);
    print_findings("D2 · 语义（正样本）", &f_d2h);
    print_findings("D3 · 平凡值（正样本，按参数配置阈值）", &f_d3h);
    print_findings("D4 · 表—码（正样本）", &f_d4h);

    assert!(f_d1h.is_empty(), "正样本不得有 D1 报警（否则检查器假阳性）");
    assert!(f_d2h.is_empty(), "正样本不得有 D2 报警（否则检查器假阳性）");
    assert!(f_d3h.is_empty(), "正样本不得有 D3 报警（承诺分化的参数已充分分化）");
    println!("\n  → 正样本零报警（无假阳性）✓");

    // ══════════ D4 的负样本：表比代码多一个键（表过期了）══════════

    println!("\n########## 第三步：D4 负样本（表—码失配）##########");
    // 构造一张"多了一个键"的表，模拟文档写了、代码没实现
    let mut stale_table = table.clone();
    stale_table.insert("legacy_bus", ["kafka"].into_iter().collect());
    let f_d4b = check_d4_table_code(&stale_table, &healthy);
    print_findings("D4 · 表—码失配（负样本）", &f_d4b);
    assert!(
        f_d4b.iter().any(|f| matches!(f, Finding::TableCodeMismatch { only_in_table, .. }
                                    if only_in_table.contains(&"legacy_bus".to_string()))),
        "D4 检查器必须抓出'表里有、代码里没有'的过期键"
    );

    // ══════════ 汇总：作为 CI 闸门的契约 ══════════

    println!("\n########## 第四步：CI 闸门契约 ##########");
    let all_bad: Vec<Finding> = {
        let mut v = vec![];
        v.extend(check_d1_value_domain(&table, &slip));
        v.extend(check_d1_value_domain(&table, &new_key));
        v.extend(check_d2_semantic_drift(&d2_bad));
        v.extend(check_d3_trivial_abuse(&d3_bad, 0.0));
        v.extend(check_d4_table_code(&stale_table, &healthy));
        v
    };
    println!("  合成漂移样本共产生 {} 条 Finding，全部**阻断发布**", all_bad.len());
    println!("  接入位置：`04 §2 CI 清单` 的 N7 之后，作为常驻适配度闸门。");
    println!("  运行频率：每个 PR（D1/D4 静态可查）+ 每日（D2/D3 需事件采样）。");

    println!("\n✅ 全部断言通过（4 个检查器 × 各自的负样本 + 正样本无假阳性 + D4 负样本）");
    println!("   结论：这个审计器补上的，是 `mechanism_growth.rs` / `scenario_ladder.rs` 的**盲区**。");
    println!("         那两个审计器问的是「有没有发明新机制」；");
    println!("         本审计器问的是「**已有机制的语义有没有滑走**」。");
    println!("         G7（巩固代数发散）正是后者——两个旧审计器都报合规，因为语义不在它们的度量里。");
    println!("         这就是把'架构期望'编码成'可执行检查'的意义：");
    println!("         凡是靠人复核对不出来的东西，必须有一个程序每天替你问一遍。");
}

// ═══════════════════════ 参考文献（写入程序，便于核对）═══════════════════════
//
// [REF-1] "Architectural Fitness Functions"（developersvoice.com / 多来源）
//   - 定义：把架构期望编码为**可执行检查**，使"架构有没有按预期进化"可自动判定；
//   - 三组正交分类：
//       · 原子 vs 整体（atomic / holistic）
//       · 静态 vs 动态（static / dynamic）
//       · 触发 vs 持续（triggered / continuous）
//   - 本审计器的定位：**原子（每断言只查一个维度）× 静态（分析参数表与采样）× 触发（每 PR / 每日）**；
//   - 关键纪律：断言失败信息必须带 `.Because()` —— 说清"为什么这条期望存在"，
//     否则未来的维护者只会删掉这个"碍事的检查"。
//
// [REF-2] "Architectural Drift" / "Evolutionary Architecture"（Neal Ford 等）
//   - 架构不会突然坏掉，它**漂移**：每一步都合理，合起来就偏了；
//   - 漂移的有效对策不是"更仔细"，而是**把不变量变成测试**；
//   - "last responsible moment"：能在 CI 里自动查的，绝不留给人复核。
//
// [REF-3] 本方案自身复盘（03 §7.3 G7）
//   - "13 阶审计没有覆盖到 G7，因为审计只查『用不用新机制键』，
//     **查不出『值的语义边界』**——与 S10 那处复核暴露的是同一类盲区。"
//   - → 这段自我诊断，正是本审计器存在的**直接理由**。
//     盲区被识别了，但当时没有把对策程序化；本程序把它做成常驻闸门。
