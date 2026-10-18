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

use super::*;

/// 四个旋钮都允许汇报的策略；各用例只改其中一项。
fn progress() -> ProgressPolicy {
    ProgressPolicy {
        enabled: true,
        interval_ms: 60_000,
        min_rounds: 2,
        max_per_turn: 5,
    }
}

/// 轮首状态：有发言、判决开着、工具轮次 0。
fn head() -> LoopState {
    LoopState::turn_head(0, true, true)
}

/// 轮边界状态：第 3 轮、静默 90s、还没汇报过。
fn boundary() -> LoopState {
    LoopState::round_boundary(3, 90_000, 0, progress())
}

// ── 表本身：顺序、冲突、先命中者胜 ────────────────────────────────────────

#[test]
fn builtin_rows_are_listed_in_prio_order() {
    let names: Vec<&str> = builtin().list().iter().map(|r| r.name).collect();
    assert_eq!(
        names,
        vec![
            "policy.classify",
            "policy.report",
            "policy.compress",
            "policy.recall",
            "policy.reflex",
        ],
        "内建行必须按 (prio, name) 升序——顺序是数据，不是插入顺序"
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
        reg.list().len(),
        5,
        "五行都得注册成功（撞名 / 撞 prio 会少一行）"
    );
    let prios: Vec<u32> = reg.list().iter().map(|r| r.prio).collect();
    // 严格升序 = 已排序且无重复，一条断言盖住两件事。
    assert!(
        prios.windows(2).all(|w| w[0] < w[1]),
        "prio 必须严格升序（既保证顺序，也保证没有撞 prio）：{prios:?}"
    );
}

#[test]
fn put_inserts_by_prio_not_by_call_order() {
    let mut reg = Registry::new();
    // 故意先注册 prio 大的：若实现按插入顺序排，本用例会得到相反的顺序。
    reg.put(Row {
        name: "policy.late",
        prio: 90,
        decide: |_| None,
    })
    .expect("首行注册");
    reg.put(Row {
        name: "policy.early",
        prio: 10,
        decide: |_| None,
    })
    .expect("次行注册");
    let names: Vec<&str> = reg.list().iter().map(|r| r.name).collect();
    assert_eq!(names, vec!["policy.early", "policy.late"]);
}

#[test]
fn duplicate_name_is_rejected() {
    let mut reg = Registry::new();
    let row = || Row {
        name: "policy.dup",
        prio: 10,
        decide: |_| None,
    };
    reg.put(row()).expect("首次注册");
    // 重名但换了 prio：仍然拒绝——重名会让日志与 `get` 指认不出是哪一行。
    let mut second = row();
    second.prio = 20;
    assert_eq!(
        reg.put(second),
        Err(RowConflict::DuplicateName { name: "policy.dup" })
    );
}

#[test]
fn prio_conflict_is_rejected_not_tie_broken_by_name() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.b",
        prio: 10,
        decide: |_| None,
    })
    .expect("首行注册");
    // 同 prio、不同名：**不**按字母序兜底，直接拒绝——字母序会让"谁先"取决于
    // 名字拼写，改个名字就改了行为。
    assert_eq!(
        reg.put(Row {
            name: "policy.a",
            prio: 10,
            decide: |_| None,
        }),
        Err(RowConflict::PrioConflict {
            prio: 10,
            kept: "policy.b",
            incoming: "policy.a",
        })
    );
    assert_eq!(reg.list().len(), 1, "被拒的行不得插入");
}

#[test]
fn decide_takes_the_first_hit_only() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.first",
        prio: 10,
        decide: |_| Some(Capsule::DECIDE),
    })
    .expect("注册");
    reg.put(Row {
        name: "policy.second",
        prio: 20,
        // 两行都认领同一格时，后者**不得**被问到（先命中者胜）。
        decide: |_| Some(Capsule::REPORT),
    })
    .expect("注册");
    let hit = reg.decide(&head()).expect("首行命中");
    assert_eq!(hit.row, "policy.first");
    assert_eq!(hit.capsule, Capsule::DECIDE);
}

#[test]
fn decide_returns_none_when_no_row_claims_the_state() {
    let mut reg = Registry::new();
    reg.put(Row {
        name: "policy.never",
        prio: 10,
        decide: |_| None,
    })
    .expect("注册");
    assert_eq!(reg.decide(&head()), None, "无行认领 = 不动点（环该退出）");
}

// ── policy.classify：三个条件逐个钉 ──────────────────────────────────────

#[test]
fn classify_fires_at_turn_head() {
    assert_eq!(
        builtin().decide(&head()).map(|h| h.capsule),
        Some(Capsule::DECIDE)
    );
}

#[test]
fn classify_does_not_fire_after_tool_rounds() {
    // 只差 `tool_rounds == 0`：工具轮次里循环回顶部时用户没再说话，
    // 再判一次就是拿同一句话重判。
    let state = LoopState::turn_head(1, true, true);
    assert_eq!(builtin().decide(&state), None);
}

#[test]
fn classify_does_not_fire_without_utterance() {
    // 只差 `has_utterance`：resume / 心跳 / 无发言轮次没有可判的话。
    let state = LoopState::turn_head(0, false, true);
    assert_eq!(builtin().decide(&state), None);
}

#[test]
fn classify_does_not_fire_when_switch_is_off() {
    // 只差 `classify_enabled`：关掉判决 ⇒ 与未挂载 `classify` 逐字一致。
    let state = LoopState::turn_head(0, true, false);
    assert_eq!(builtin().decide(&state), None);
}

// ── policy.report：`tool_rounds > 0` 与四个旋钮 ───────────────────────────

#[test]
fn report_fires_at_round_boundary() {
    assert_eq!(
        builtin().decide(&boundary()).map(|h| h.capsule),
        Some(Capsule::REPORT)
    );
}

#[test]
fn report_never_fires_at_turn_head_even_if_min_rounds_is_zero() {
    // 这是本批最容易被"顺手优化掉"的一条：`min_rounds = 0` 时 `ProgressPolicy::due`
    // 会放行 `tool_rounds == 0`，而轮首汇报是语义错误——那时一轮工具都没跑。
    // 判据必须独立成立，不能并进 `due`。
    let permissive = ProgressPolicy {
        min_rounds: 0,
        ..progress()
    };
    let state = LoopState::round_boundary(0, 90_000, 0, permissive);
    assert_eq!(builtin().decide(&state), None);
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
        let state = LoopState::round_boundary(rounds, quiet_ms, reports, policy);
        assert_eq!(
            builtin().decide(&state),
            None,
            "quiet_ms={quiet_ms} rounds={rounds} reports={reports} 应当不汇报"
        );
    }
}

#[test]
fn report_interval_threshold_is_inclusive() {
    // 阈值是"到了就该说"，不是"超过才说"：`quiet_ms == interval_ms` 必须汇报。
    let state = LoopState::round_boundary(3, 60_000, 0, progress());
    assert_eq!(
        builtin().decide(&state).map(|h| h.capsule),
        Some(Capsule::REPORT)
    );
}

// ── policy.compress / policy.recall：逐条件钉 ──────────────

#[test]
fn compress_follows_the_request_switch_only() {
    assert_eq!(
        builtin()
            .decide(&LoopState::compress(true))
            .map(|h| h.capsule),
        Some(Capsule::COMPRESS)
    );
    // 关掉自动压缩 ⇒ 不进产线。压不压得动是内核的事，本行只管进不进。
    assert_eq!(builtin().decide(&LoopState::compress(false)), None);
}

#[test]
fn reflex_requires_a_fresh_turn_and_a_match() {
    assert_eq!(
        builtin()
            .decide(&LoopState::reflex(true, true))
            .map(|h| h.capsule),
        Some(Capsule::REFLEX)
    );
    // 续写轮不走快路：它会撞幂等键（复用 `resume.user_seq` 却新开一轮）。
    assert_eq!(builtin().decide(&LoopState::reflex(false, true)), None);
    // 没命中技能就没什么可快。
    assert_eq!(builtin().decide(&LoopState::reflex(true, false)), None);
}

#[test]
fn recall_requires_first_round_and_a_session_dir() {
    assert_eq!(
        builtin()
            .decide(&LoopState::turn_prepare(0, true))
            .map(|h| h.capsule),
        Some(Capsule::RECALL)
    );
    // 只差 `tool_rounds == 0`：同一轮内记忆不漂移，事实源只扫一遍。
    assert_eq!(builtin().decide(&LoopState::turn_prepare(1, true)), None);
    // 只差 `has_session_dir`：临时会话没有事实源，不读不写。
    assert_eq!(builtin().decide(&LoopState::turn_prepare(0, false)), None);
}

// ── 各格互不误命中（`prio` 目前不参与裁决的前提）─────────────────────────

#[test]
fn each_grid_point_hits_only_its_own_row() {
    // 每个构造器只填自己那几格，因此任何一格都不会命中别人的行。
    // 这条一旦破了，`decide` 的"先命中者胜"就会把另一格的动作顶掉。
    let cases: [(&str, LoopState, Capsule); 5] = [
        ("policy.classify", head(), Capsule::DECIDE),
        ("policy.report", boundary(), Capsule::REPORT),
        (
            "policy.compress",
            LoopState::compress(true),
            Capsule::COMPRESS,
        ),
        (
            "policy.recall",
            LoopState::turn_prepare(0, true),
            Capsule::RECALL,
        ),
        (
            "policy.reflex",
            LoopState::reflex(true, true),
            Capsule::REFLEX,
        ),
    ];
    for (row, state, capsule) in cases {
        let hit = builtin().decide(&state).expect("该格应当命中");
        assert_eq!(hit.row, row, "命中行不对：{hit:?}");
        assert_eq!(hit.capsule, capsule);
    }
}
