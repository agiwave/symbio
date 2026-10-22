//! `session/loop_registry.rs` 的单元测试 —— 表的**裁决顺序**与两条策略行的**逐条件**钉法。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 为什么这些用例必须存在
//!
//! 表错了不会有错误信号：它只表现为"某一步悄悄不发生"（判决不判了 / 汇报不说了）
//! 或"两步抢着发生"。前者是本项目最反对的失效形态（沉默），后者只在用户面前
//! 表现为"助手莫名其妙说了两遍"。两者都不会让任何 e2e 变红——所以判据必须钉在
//! 单测这一层。
//!
//! 每个条件都要有一条"**只差它不成立**"的用例：只验"全满足时命中"是不够的，
//! 那在"恒返回 `Some`"的实现下也通过。
//!
//! ## 三条结构判据（`Site` 引入后新增）
//!
//! 1. `every_site_is_manned`——每一格都至少有一行。空格的失效形态是「那一格
//!    永远不做事」，而调用点看到的是「不做事」，没有错误信号。
//! 2. `decide_only_scans_its_own_site`——别的格的行**不被调用**，不是"调用了
//!    但没命中"。这是 `decide` 从"扫全表"收窄到"扫本格"的可执行形态。
//! 3. `no_row_reads_across_sites`——行只读本格构造器填的字段。这条此前是
//!    「靠人读 5 个构造器确认」，现在是哨兵用例：把非本格字段全推到极端值，
//!    若某行跨格读字段，结果会与真实状态不同。

use super::*;

use crate::symbio_core::LatencyTier;

/// 四个旋钮都允许汇报的策略；各用例只改其中一项。
fn progress() -> ProgressPolicy {
    ProgressPolicy {
        enabled: true,
        interval_ms: 60_000,
        min_rounds: 2,
        max_per_turn: 5,
    }
}

/// 某一格「全部条件都满足」的问表状态 —— 逐条件用例与可达性用例的基准。
fn call_of(site: Site) -> LoopState {
    match site {
        Site::TurnHead => LoopState::turn_head(0, true, true, false),
        Site::RoundBoundary => LoopState::round_boundary(3, 90_000, 0, progress()),
        Site::Compress => LoopState::compress(true, false),
        Site::TurnPrepare => LoopState::turn_prepare(0, true, false),
        Site::Reflex => LoopState::reflex(true, true),
    }
}

// ── 表本身：顺序、冲突、先命中者胜 ────────────────────────────────────────

#[test]
fn builtin_rows_are_stable() {
    // 每格恰一行、行名固定。行名是日志与排查的取值点（「这一步是 policy.report
    // 决定的」），改名会让历史日志与文档里的指认失效。
    let mut seen = Vec::new();
    for site in Site::ALL {
        let rows = builtin().rows_of(*site);
        assert_eq!(rows.len(), 1, "{} 格应当恰有一行", site.name());
        seen.push(rows[0].name);
    }
    assert_eq!(
        seen,
        vec![
            "policy.classify",
            "policy.report",
            "policy.compress",
            "policy.recall",
            "policy.reflex",
        ],
        "行名与格的对应关系是契约（日志、文档、e2e 都按这些名字指认）"
    );
}

/// 内建常量表的守门人。
///
/// 生产侧注册失败只告警（见 `Registry::with_builtin_rows` 的文档：一个只可能因
/// 改错常量而触发的崩溃，不值得占两个 panic 面）。因此"表里到底有没有这几行、
/// 有没有撞 prio"这件事必须由**本用例**钉住——它是 CI 里红，不是用户机器上炸。
#[test]
fn builtin_rows_are_conflict_free_and_ordered() {
    let reg = Registry::with_builtin_rows();
    assert_eq!(
        reg.unmanned_sites(),
        Vec::new(),
        "每个决策点都必须有行——空格意味着那一格永远不做事，而调用点看不出区别"
    );
    for site in Site::ALL {
        let prios: Vec<u32> = reg.rows_of(*site).iter().map(|r| r.prio).collect();
        assert!(
            prios.windows(2).all(|w| w[0] < w[1]),
            "{} 格的 prio 必须严格升序（既保证顺序，也保证格内无重复）：{prios:?}",
            site.name()
        );
    }
}

#[test]
fn put_inserts_by_prio_not_by_call_order() {
    let mut reg = Registry::new();
    // 故意先注册 prio 大的：若实现按插入顺序排，本用例会得到相反的顺序。
    reg.put(Row {
        name: "policy.late",
        site: Site::TurnHead,
        prio: 90,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    })
    .expect("首行注册");
    reg.put(Row {
        name: "policy.early",
        site: Site::TurnHead,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    })
    .expect("次行注册");
    let names: Vec<&str> = reg.rows_of(Site::TurnHead).iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["policy.early", "policy.late"]);
}

#[test]
fn duplicate_name_is_rejected() {
    let mut reg = Registry::new();
    let row = || Row {
        name: "policy.dup",
        site: Site::TurnHead,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    };
    reg.put(row()).expect("首次注册");
    // 重名但换了 prio（甚至换了格）：仍然拒绝——重名会让日志与 `get`
    // 指认不出是哪一行，而"改一格就绕过去"正是重名检查要防的漏网。
    let mut second = row();
    second.prio = 20;
    second.site = Site::Reflex;
    assert_eq!(
        reg.put(second),
        Err(RowConflict::DuplicateName { name: "policy.dup" })
    );
}

#[test]
fn prio_conflict_is_rejected_within_a_site() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.b",
        site: Site::TurnPrepare,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    })
    .expect("首行注册");
    // 同格、同 prio、不同名：**不**按字母序兜底，直接拒绝——字母序会让"谁先"
    // 取决于名字拼写，改个名字就改了行为。
    assert_eq!(
        reg.put(Row {
            name: "policy.a",
            site: Site::TurnPrepare,
            prio: 10,
            effects: Effects {
                tier: LatencyTier::Reflex,
            },
            decide: |_| None,
        }),
        Err(RowConflict::PrioConflict {
            site: Site::TurnPrepare,
            prio: 10,
            kept: "policy.b",
            incoming: "policy.a",
        })
    );
    assert_eq!(reg.rows_of(Site::TurnPrepare).len(), 1, "被拒的行不得插入");
}

/// 跨格同 `prio` 不是冲突 —— 「同格加第二行」不再需要挤全局编号。
///
/// 这一条把「`prio` 的作用域 = 格」钉成判据。此前的实现里 `prio` 全局唯一，
/// 于是 PL-014 §5 步骤 3 要做的「`policy.recall` 拆成记忆 / 技能两行」必须先
/// 从全局编号里找一个没被占用的数——而「两行同格谁先」与「两格撞号」根本不是
/// 一回事，用同一个判据挡会把前者误判为冲突。
#[test]
fn same_prio_in_different_sites_is_not_a_conflict() {
    // 五个内建行的 prio 全是 10，分属五格——若判据是全局唯一，这张表自己就
    // 注册不进来（`with_builtin_rows` 里第一个之后全被拒，只留一行）。
    let mut reg = Registry::with_builtin_rows();
    for site in Site::ALL {
        assert_eq!(
            reg.rows_of(*site)[0].prio,
            10,
            "每格都从 10 起：跨格共用编号是正常的"
        );
    }
    reg.put(Row {
        name: "policy.extra",
        site: Site::TurnPrepare,
        prio: 20,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    })
    .expect("同格加第二行只需一个格内新序（20），不必从全局编号里挤位");
}

#[test]
fn decide_takes_the_first_hit_only() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.first",
        site: Site::TurnHead,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| Some(Capsule::Decide),
    })
    .expect("注册");
    reg.put(Row {
        name: "policy.second",
        site: Site::TurnHead,
        prio: 20,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        // 两行都认领同一格时，后者**不得**被问到（先命中者胜）。
        decide: |_| Some(Capsule::Report),
    })
    .expect("注册");
    let hit = reg
        .decide(&call_of(Site::TurnHead))
        .hit()
        .expect("首行命中");
    assert_eq!(hit.row, "policy.first");
    assert_eq!(hit.capsule, Capsule::Decide);
}

/// `Declined` 与 `Unmanned` 必须可分辨 —— 两者在调用点都是"不做事"。
///
/// 这是 PL-014 §3 规则 g「`None` 是缺口，不是出口」在直线代码上的落法：
/// 不需要 `Halt(NoRule)`，只需要返回值不再把「本格有行但都没认领」与
/// 「本格一行都没有」压成同一个 `None`。
#[test]
fn declined_and_unmanned_are_distinguishable() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.never",
        site: Site::TurnHead,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| None,
    })
    .expect("注册");
    assert_eq!(
        reg.decide(&call_of(Site::TurnHead)),
        Verdict::Declined {
            site: Site::TurnHead,
            rows: 1
        },
        "有行但都不认领 = Declined（正常）"
    );
    assert_eq!(
        reg.decide(&call_of(Site::Reflex)),
        Verdict::Unmanned { site: Site::Reflex },
        "一行都没有 = Unmanned（配置缺口）"
    );
    // 两者对调用点都是"不做事"，但只有后者该让 CI 红。
    assert_eq!(reg.decide(&call_of(Site::TurnHead)).capsule(), None);
    assert_eq!(reg.decide(&call_of(Site::Reflex)).capsule(), None);
}

/// `decide` 只问本格的行 —— 别的格的行**不被调用**，不是"调用了但没命中"。
///
/// 若实现扫全表，本用例会拿到 `Hit(policy.bomb)`（那一行恒返回 `Some`）。
#[test]
fn decide_only_scans_its_own_site() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.bomb",
        site: Site::Reflex,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| Some(Capsule::Reflex),
    })
    .expect("注册");
    assert_eq!(
        reg.decide(&call_of(Site::TurnHead)),
        Verdict::Unmanned {
            site: Site::TurnHead
        },
        "TurnHead 格问表时，Reflex 格的行不该被问到"
    );
    // 反证：问到 Reflex 格时它确实命中（不是"这行永远不响"）。
    assert_eq!(
        reg.decide(&call_of(Site::Reflex)).capsule(),
        Some(Capsule::Reflex)
    );
}

// ── policy.classify：三个条件逐个钉 ──────────────────────────────────────

#[test]
fn classify_fires_at_turn_head() {
    assert_eq!(
        builtin().decide(&call_of(Site::TurnHead)).capsule(),
        Some(Capsule::Decide)
    );
}

#[test]
fn classify_does_not_fire_after_tool_rounds() {
    // 只差 `round == 0`：工具轮次里循环回顶部时用户没再说话，
    // 再判一次就是拿同一句话重判。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_head(1, true, true, false))
            .capsule(),
        None
    );
}

#[test]
fn classify_does_not_fire_without_utterance() {
    // 只差 `has_utterance`：resume / 心跳 / 无发言轮次没有可判的话。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_head(0, false, true, false))
            .capsule(),
        None
    );
}

#[test]
fn classify_does_not_fire_when_switch_is_off() {
    // 只差 `classify_enabled`：关掉判决 ⇒ 与未挂载 `classify` 逐字一致。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_head(0, true, false, false))
            .capsule(),
        None
    );
}

#[test]
fn classify_does_not_fire_once_it_has_run() {
    // 只差 `done`：前三条在一轮内**恒真**（`request_round` 只在收轮后 +1），
    // 没有这一条，任何以「再问一次表」为形状的驱动都会拿同一句话重判——
    // ADR-107 判据 2 的第一条实例。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_head(0, true, true, true))
            .capsule(),
        None
    );
}

// ── policy.report：`round > 0` 与四个旋钮 ───────────────────────────

#[test]
fn report_fires_at_round_boundary() {
    assert_eq!(
        builtin().decide(&call_of(Site::RoundBoundary)).capsule(),
        Some(Capsule::Report)
    );
}

#[test]
fn report_never_fires_at_turn_head_even_if_min_rounds_is_zero() {
    // 这是本批最容易被"顺手优化掉"的一条：`min_rounds = 0` 时 `ProgressPolicy::due`
    // 会放行 `round == 0`，而轮首汇报是语义错误——那时一轮工具都没跑。
    // 判据必须独立成立，不能并进 `due`。
    let permissive = ProgressPolicy {
        min_rounds: 0,
        ..progress()
    };
    let call = LoopState::round_boundary(0, 90_000, 0, permissive);
    assert_eq!(builtin().decide(&call).capsule(), None);
}

#[test]
fn report_respects_the_four_knobs() {
    // 每个旋钮各一条"只差它"：多一条、少一条、边界取 `>` 还是 `>=` 都不会报错，
    // 只表现为用户被打扰或收不到消息。
    let cases = [
        (
            ProgressPolicy {
                enabled: false,
                ..progress()
            },
            90_000,
            3,
            0,
        ),
        (
            ProgressPolicy {
                interval_ms: 90_001,
                ..progress()
            },
            90_000,
            3,
            0,
        ),
        (progress(), 90_000, 1, 0),
        (
            ProgressPolicy {
                max_per_turn: 2,
                ..progress()
            },
            90_000,
            3,
            2,
        ),
    ];
    for (policy, quiet_ms, rounds, reports) in cases {
        let call = LoopState::round_boundary(rounds, quiet_ms, reports, policy);
        assert_eq!(
            builtin().decide(&call).capsule(),
            None,
            "quiet_ms={quiet_ms} rounds={rounds} reports={reports} 应当不汇报"
        );
    }
}

#[test]
fn report_interval_threshold_is_inclusive() {
    // 阈值是"到了就该说"，不是"超过才说"：`quiet_ms == interval_ms` 必须汇报。
    let call = LoopState::round_boundary(3, 60_000, 0, progress());
    assert_eq!(builtin().decide(&call).capsule(), Some(Capsule::Report));
}

// ── policy.compress / policy.recall / policy.reflex：逐条件钉 ──────────

#[test]
fn compress_requires_the_switch_and_an_unused_round() {
    assert_eq!(
        builtin()
            .decide(&LoopState::compress(true, false))
            .capsule(),
        Some(Capsule::Compress)
    );
    // 关掉自动压缩 ⇒ 不进产线。压不压得动是内核的事，本行只管进不进。
    assert_eq!(
        builtin()
            .decide(&LoopState::compress(false, false))
            .capsule(),
        None
    );
    // 只差 `done`：本请求轮已进过产线 ⇒ 不再进。口径是**请求轮**而不是整轮——
    // 内核每轮重新取水位，扩到整轮会让后段压缩永远不发生。
    assert_eq!(
        builtin().decide(&LoopState::compress(true, true)).capsule(),
        None
    );
}

#[test]
fn reflex_requires_a_fresh_turn_and_a_match() {
    assert_eq!(
        builtin().decide(&LoopState::reflex(true, true)).capsule(),
        Some(Capsule::Reflex)
    );
    // 续写轮不走快路：它会撞幂等键（复用原轮用户格却新开一轮）。
    assert_eq!(
        builtin().decide(&LoopState::reflex(false, true)).capsule(),
        None
    );
    // 没命中技能就没什么可快。
    assert_eq!(
        builtin().decide(&LoopState::reflex(true, false)).capsule(),
        None
    );
}

#[test]
fn recall_requires_first_round_and_a_session_dir() {
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_prepare(0, true, false))
            .capsule(),
        Some(Capsule::Recall)
    );
    // 只差 `round == 0`：同一轮内记忆不漂移，事实源只扫一遍。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_prepare(1, true, false))
            .capsule(),
        None
    );
    // 只差 `has_facts`：临时会话没有事实源，不读不写。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_prepare(0, false, false))
            .capsule(),
        None
    );
    // 只差 `done`：本轮已召回过 ⇒ 不再认领（同一轮内视图不漂移的判据由此可执行）。
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_prepare(0, true, true))
            .capsule(),
        None
    );
}

// ── 行的可达性与格的对应 ────────────────────────────────────────────────

/// 每一行都必须从**它声明的格**可达。
///
/// 这条替代了此前的哨兵用例。哨兵守的是「行不跨格读字段」——`LoopState` 改成
/// 按格分形的枚举之后，那件事已经是编译错误（`TurnHead` 变体里没有 `quiet_ms`，
/// 别的行写不出来）。但枚举化留下一个**新的**漂移面：`Row.site` 是手写字段，
/// 它可能与判据实际接受的变体对不上（`site: TurnHead` 配一个只 match
/// `RoundBoundary` 的函数）。那样的一行永远不命中——死代码，且没有任何错误信号。
///
/// `call_of(site)` 是该格全部条件都满足的状态，所以「本格有行」必须表现为
/// `Hit`。`Unmanned` 说明这一格没挂行，`Declined` 说明挂了的行走不到——
/// 两种都是配置错，报错文案按变体区分（`Verdict` 三值的用处就在这里）。
#[test]
fn every_row_is_reachable_from_its_declared_site() {
    for site in Site::ALL {
        let verdict = builtin().decide(&call_of(*site));
        assert!(
            matches!(verdict, Verdict::Hit(_)),
            "{} 格没有行认领「全条件满足」的状态：{verdict:?}——\
             Unmanned = 本格没挂行；Declined = 挂了的行的 site 声明与判据接受的\
             变体对不上（或判据被改成了恒假）",
            site.name()
        );
    }
}

/// 构造器与格的对应关系：`call_of` 自己不能错。
#[test]
fn call_of_matches_its_site() {
    for site in Site::ALL {
        assert_eq!(call_of(*site).site(), *site, "构造器与格错位");
    }
}

/// 每条臂都必须有**生产者**（PL-015 §4.2；ADR-107 判据 1）。
///
/// `every_row_is_reachable_from_its_declared_site` 钉的是「格 → 行可达」，
/// 本条钉它的对偶：「臂 → 行有产出」。加一个 `Capsule` 变体、写好 `name()`，
/// 却没挂任何行时，编译与既有用例都不会红——那条臂永远不会被派发。
/// `Capsule::ALL` 是遍历的唯一取值点（与 `Site::ALL` 同规矩）。
#[test]
fn every_capsule_has_a_producing_row() {
    let produced: Vec<Capsule> = Site::ALL
        .iter()
        .filter_map(|site| builtin().decide(&call_of(*site)).capsule())
        .collect();
    for capsule in Capsule::ALL {
        assert!(
            produced.contains(capsule),
            "臂 {} 没有任何内建行产出——它永远不会被派发",
            capsule.name()
        );
    }
    assert_eq!(
        Capsule::ALL.len(),
        produced.len(),
        "内建行产出了 `ALL` 之外的臂，或两条行争同一臂"
    );
}

/// 臂的归属格与「产出它的那一格」必须是同一个 —— `Capsule::owner` 的落点。
///
/// `every_capsule_has_a_producing_row` 只钉「有行产出」，不钉「产出行挂的格
/// 与臂声明的归属格一致」。行挂错格时：调用点按 owner 分派，臂会在错误的格
/// 被执行（或永远不执行）——两者都不崩、都不报。
#[test]
fn every_capsule_is_owned_by_the_site_that_produces_it() {
    for site in Site::ALL {
        let hit = builtin()
            .decide(&call_of(*site))
            .hit()
            .unwrap_or_else(|| panic!("{} 格没有命中", site.name()));
        assert_eq!(
            hit.capsule.owner(),
            *site,
            "臂 {} 产出于 {} 格，却声明归属 {} 格",
            hit.capsule.name(),
            site.name(),
            hit.capsule.owner().name()
        );
    }
}

/// `owner` 是**满射且单值**：五格恰被五臂各声明一次。
///
/// `owner` 的 `match` 已经是穷尽的（加臂必须给格），但「两条臂都声明同一格」
/// 不会编译错误——它会让某一格执行另一格的臂（或另一格无臂可执行）。
#[test]
fn capsule_owner_is_total_and_injective() {
    let owners: Vec<Site> = Capsule::ALL.iter().map(|c| c.owner()).collect();
    for site in Site::ALL {
        assert_eq!(
            owners.iter().filter(|o| *o == site).count(),
            1,
            "{} 格应恰被一条臂声明归属：{owners:?}",
            site.name()
        );
    }
}

/// 内建行的档位表：档位随**行**携带，不是调用点反推的。
///
/// 档位错了不会有错误信号：它只表现为「本该走反射的动作多付一次模型往返」
/// 或「该调模型的动作拿不到 `FullModel` 令牌」。因此逐行钉住，改档位时
/// 必须回来改这张表。
#[test]
fn builtin_rows_carry_their_declared_tier() {
    let cases = [
        // classify：可落到模型（`VerdictSource::Model`），但不生成。
        (Site::TurnHead, LatencyTier::Fast),
        // 汇报：模板产线，零模型往返。
        (Site::RoundBoundary, LatencyTier::Reflex),
        // 语义压缩调模型。
        (Site::Compress, LatencyTier::Deep),
        // 召回：事实投影的纯读 + 技能路由。
        (Site::TurnPrepare, LatencyTier::Reflex),
        // 快路就是「不调模型」。
        (Site::Reflex, LatencyTier::Reflex),
    ];
    for (site, tier) in cases {
        let hit = builtin()
            .decide(&call_of(site))
            .hit()
            .unwrap_or_else(|| panic!("{} 格没有命中", site.name()));
        assert_eq!(hit.effects.tier, tier, "{} 格的档位", site.name());
        assert_eq!(
            hit.effects.budget_ms(),
            tier.budget_ms(),
            "预算必须由档位派生（J1：不存第二份）"
        );
    }
}

// ── 裁决顺序：`prio` 说话，不是插入顺序 ─────────────────────────────────

#[test]
fn lower_prio_wins_regardless_of_insertion_order() {
    // 两行**同一格都命中**（都返回 `Some`），只有 `prio` 能决定谁先。
    // 先注册 prio 30、再注册 prio 10：如果实现按插入顺序扫描，胜者会是先注册
    // 那一行；按 `prio` 排序则胜者是 10。这一条把两种实现分开。
    //
    // 内建表今天没有同格竞争，所以「prio 生效」只能靠这张合成表来证。
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.later",
        site: Site::TurnHead,
        prio: 30,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| Some(Capsule::Report),
    })
    .expect("注册 later");
    reg.put(Row {
        name: "policy.earlier",
        site: Site::TurnHead,
        prio: 10,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |_| Some(Capsule::Decide),
    })
    .expect("注册 earlier");

    let hit = reg
        .decide(&call_of(Site::TurnHead))
        .hit()
        .expect("两行都命中");
    assert_eq!(
        hit.row, "policy.earlier",
        "prio 小的先命中（不是先注册的先）"
    );
    assert_eq!(hit.capsule, Capsule::Decide);
    assert_eq!(
        reg.rows_of(Site::TurnHead)
            .iter()
            .map(|r| r.name)
            .collect::<Vec<_>>(),
        vec!["policy.earlier", "policy.later"],
        "rows_of 必须按裁决顺序给出——它是「顺序是数据」的可观测面"
    );
}

/// 「加一条策略 = 一个函数 + 一行注册，环不改」的**可执行形态**，且落在
/// **同一格加第二行**这个真实场景上（PL-014 §5 步骤 3 的「记忆 / 技能拆两行」）。
#[test]
fn adding_a_second_row_to_a_site_is_pure_registration() {
    let mut reg = Registry::with_builtin_rows();
    // 新行复用一个已有臂（`Recall`），并在 `turn_prepare` 格里抢在 `policy.recall`
    // **之前**（prio 5 < 10）——环那 5 处分派点一个字符都没动。
    reg.put(Row {
        name: "policy.skill",
        site: Site::TurnPrepare,
        prio: 5,
        effects: Effects {
            tier: LatencyTier::Reflex,
        },
        decide: |s| {
            let LoopState::TurnPrepare {
                round, has_facts, ..
            } = s
            else {
                return None;
            };
            (*round == 0 && *has_facts).then_some(Capsule::Recall)
        },
    })
    .expect("同格加第二行只需格内新序");

    let hit = reg
        .decide(&call_of(Site::TurnPrepare))
        .hit()
        .expect("新行认领");
    assert_eq!(hit.row, "policy.skill");
    assert_eq!(hit.capsule, Capsule::Recall);
    assert_eq!(
        reg.rows_of(Site::TurnPrepare).len(),
        2,
        "同格两行都在表里，`policy.recall` 只是被先命中者挡住"
    );
    // 反过来：若新策略需要**新行为**（环不认识的动作），它必须加一个 `Capsule`
    // 变体——届时 `name()` 的 `match` 与每个分派点都会编译错误。那是想要的
    // 「必须显式处理」，不是遗漏。
}

#[test]
fn capsule_names_are_unique_and_stable() {
    // 臂名是日志与冲突报错的取值点：重名会让「哪一行/哪条臂」指认不出。
    // 遍历 `Capsule::ALL` 而不是在测试里另写一份变体清单——两份清单必然漂移，
    // 而「哪条臂有行产出」由 `every_capsule_has_a_producing_row` 钉住。
    let names: Vec<&'static str> = Capsule::ALL.iter().map(|c| c.name()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "臂名重复：{names:?}");
    assert_eq!(names[0], "decide", "臂名是跨进程日志词，改动要同步文档");
    assert_eq!(names.len(), 5, "臂的数目变了要同步 PL-014 与本文档");
}

#[test]
fn site_names_are_unique_and_stable() {
    // 格名进日志（「哪一格没行」）与冲突报错，同样是跨进程词。
    let names: Vec<&str> = Site::ALL.iter().map(|s| s.name()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "格名重复：{names:?}");
    assert_eq!(names.len(), 5, "格的数目变了要同步 PL-014 与本文档");
}
