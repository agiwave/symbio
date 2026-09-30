//! `actors` 单测 —— 含 **S01 闭环彩排**（[plan/04 §3](../../../../docs/plan/04-工程落地.md)
//! 第 3 步：用 `Decider` 跑通完整闭环，零 LLM）。
//!
//! 彩排把 S0 的地基（store / projection / invariants）与 S1 的 ② `actors`
//! 拧成一条完整链路，验收断言逐条对应
//! [roadmap/S01 §6](../../../../docs/plan/roadmap/S01-最小闭环.md)。

use super::*;
use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use crate::symbio_core::invariants::check_all;
use crate::symbio_core::projection::turnstate::turnstate;
use crate::symbio_core::store::{EventStore, Store};
use crate::symbio_core::view::Budget;

// ── ActorSpec / Pattern（01 §4 契约形状） ──────────────────────────────

#[test]
fn trivial_spec_is_the_s01_baseline() {
    let spec = ActorSpec::trivial("agent:main");
    assert_eq!(
        spec.pattern,
        Pattern::Decider,
        "平凡值 = decider（零 LLM 全链路）"
    );
    assert_eq!(spec.budget_ms, 60_000, "平凡值预算：慢但正确");
    assert_eq!(
        spec.scope,
        Scope::Root,
        "平凡值 scope = root（root 是一等输入）"
    );
}

// ── S01 闭环彩排 ────────────────────────────────────────────────────────

/// 一轮完整对话：用户消息入库 → Decider 应答 → final/fallback 入库 → 三查。
///
/// 返回 (store, 收束事件 seq)；`miss` 模拟生成失败（Decider 查空规则表）。
fn rehearse_turn(utterance: &str, decider: &Decider) -> (EventStore, Result<u64, u64>) {
    let store = EventStore::new();
    // 「收到」：用户消息成为一条事件（turn × opened）。
    let seq = store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": utterance })),
        )
        .expect("用户消息必入库");
    // 「思考 + 发出」：actor 只看事件切片（View 的原始形态）。
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let outcome = match decider.respond(&snapshot) {
        Ok(reply) => {
            let s = store
                .append(
                    Event::pending(
                        "f0",
                        EVENT_ASSISTANT_FINAL,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(seq.value())
                    .with_payload(serde_json::json!({ "text": reply })),
                )
                .expect("final 必入库");
            Ok(s.value())
        }
        Err(_) => {
            // 「兜底」：也是一条普通事件，落同一个 turn × closed 格子，同样带溯源。
            let s = store
                .append(
                    Event::pending(
                        "fb0",
                        EVENT_ASSISTANT_FALLBACK,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(seq.value())
                    .with_payload(serde_json::json!({ "text": "抱歉，我暂时答不上来。" })),
                )
                .expect("fallback 必入库");
            Err(s.value())
        }
    };
    (store, outcome)
}

/// 验收 1：一条 `user.message` → 恰好一条 `turn/closed`。
#[test]
fn one_user_message_yields_exactly_one_final() {
    let (store, _) = rehearse_turn("你好，帮我看看", &Decider::rehearsal());
    let finals: Vec<_> = store
        .range(crate::symbio_core::event::Seq::new(0))
        .into_iter()
        .filter(|e| e.kind == EVENT_ASSISTANT_FINAL)
        .collect();
    assert_eq!(finals.len(), 1, "发出端点：发言只有一条 final");
    // 三条不变量在整条链路上全绿（含 I2：final 带溯源）。
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// 验收 2：强制生成失败 → 必须存在 fallback，且它的 `produced_by` 非空。
#[test]
fn forced_failure_produces_fallback_with_provenance() {
    let (store, outcome) = rehearse_turn("这是规则表外的一句话", &Decider::new(vec![]));
    assert!(
        outcome.is_err(),
        "规则未命中 ⇒ 走兜底（Miss 不是错误，是触发条件）"
    );
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let fallbacks: Vec<_> = snapshot
        .iter()
        .filter(|e| e.kind == EVENT_ASSISTANT_FALLBACK)
        .collect();
    assert_eq!(fallbacks.len(), 1, "兜底必须产生事件——静默中断 = 违例");
    assert!(fallbacks[0].produced_by.is_some(), "兜底同样可被 I2 审计");
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// 验收 3：投影两次运行结果逐字节相同。
#[test]
fn turnstate_projection_is_deterministic() {
    let (store, _) = rehearse_turn("你好", &Decider::rehearsal());
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let p = turnstate();
    let v1 = p.apply(&snapshot, 1_000, Budget::generous());
    let v2 = p.apply(&snapshot, 1_000, Budget::generous());
    assert_eq!(v1, v2, "N1：同一事件序列 + 同一参数 ⇒ 逐字节相同");
    assert!(
        v1.value.opened && v1.value.settled(),
        "当前 turn 已开且已收束"
    );
    assert!(v1.value.final_text.is_some(), "规则命中 ⇒ final 态");
}

/// 验收 4（反向）：注入两条 final → N3 必须报警。
#[test]
fn two_finals_are_caught_by_n3() {
    let store = EventStore::new();
    store
        .append(Event::pending(
            "u0",
            EVENT_USER_MESSAGE,
            Entity::Turn,
            Verb::Opened,
            0,
            "user",
        ))
        .expect("首插必成");
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    store
        .append(
            Event::pending(
                "f1",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0),
        )
        .expect("首插必成");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let violations = check_all(&snapshot);
    assert!(
        violations.iter().any(|v| v.why.contains("final")),
        "同一 turn 两条 final 必须被看见：{:?}",
        violations
    );
    // 投影侧的孪生判定也必须不认这个 turn 为「已合法收束」。
    let v = turnstate().apply(&snapshot, 0, Budget::generous());
    assert!(
        !v.value.settled(),
        "settled 的 xor 语义：两条 final ≠ 合法收束"
    );
}

// ── Decider 的输入契约 ─────────────────────────────────────────────────

#[test]
fn decider_reads_the_last_user_message_from_events() {
    // Decider 自己从事件切片里找 user.message——「收到」是它的输入契约。
    let events = vec![Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "你好" }))];
    assert_eq!(
        Decider::rehearsal().respond(&events).unwrap(),
        "你好，我能做什么？"
    );
}

#[test]
fn decider_miss_reports_the_utterance() {
    let events = vec![Event::pending(
        "u0",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "text": "规则表外" }))];
    let miss = Decider::rehearsal().respond(&events).unwrap_err();
    assert_eq!(
        miss.utterance, "规则表外",
        "Miss 携带输入摘要（可观测，入兜底载荷）"
    );
}

// ── S2 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 5–6 步）：真实执行形状 ──
//
// 步骤 5：接入真实模型（pattern = reasoner）→ 真实对话 → N1/N3/N5 仍全绿；
// 步骤 6：兜底埋点 + 四层 budget_ms 就位 → 时延埋点可测。
// 这里用 ⑤ adapters 的确定性桩（StubLlmAdapter）代替真实模型——闭环形状与
// 真实接线完全一致，只换适配器。

use crate::symbio_core::adapters::{LatencyTier, StubLlmAdapter, TokenIssuer};
use crate::symbio_core::invariants::{budget_exceeded, unresolved_turns};
use std::time::Instant;

/// 一轮**真实执行形状**的对话：Decider 换成 Reasoner + 适配器，带 cost_ms 埋点。
///
/// 返回 (store, 收束事件 id)；失败路径产出 fallback（I3 到点必答）。
async fn rehearse_reasoner_turn(
    utterance: &str,
    llm: &dyn crate::symbio_core::adapters::LlmAdapter,
) -> (EventStore, Result<String, String>) {
    let tok = TokenIssuer::issue_deep();
    let store = EventStore::new();
    let seq = store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": utterance })),
        )
        .expect("用户消息必入库");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let started = Instant::now();
    let outcome = Reasoner.reply(llm, &tok, &snapshot).await;
    let cost_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(reply) => {
            store
                .append(
                    Event::pending(
                        "f0",
                        EVENT_ASSISTANT_FINAL,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(seq.value())
                    .with_cost_ms(cost_ms)
                    .with_payload(serde_json::json!({ "text": reply, "model": llm.model_id() })),
                )
                .expect("final 必入库");
            (store, Ok(reply))
        }
        Err(e) => {
            store
                .append(
                    Event::pending("fb0", EVENT_ASSISTANT_FALLBACK, Entity::Turn, Verb::Closed, 0, "agent:main")
                        .with_produced_by(seq.value())
                        .with_cost_ms(cost_ms)
                        .with_payload(serde_json::json!({ "text": "抱歉，我暂时答不上来。", "error": format!("{e:?}") })),
                )
                .expect("fallback 必入库");
            (store, Err(format!("{e:?}")))
        }
    }
}

/// 验收（步骤 5）：接入「真实模型」→ 真实对话 → N1/N3/N5 仍全绿。
#[tokio::test]
async fn reasoner_turn_keeps_all_invariants_green() {
    let llm = StubLlmAdapter::succeed("stub-model");
    let (store, outcome) = rehearse_reasoner_turn("写一首关于秋天的诗", &llm).await;
    let reply = outcome.expect("成功桩必答");
    assert!(reply.contains("stub-model"));
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
    assert!(unresolved_turns(&snapshot).is_empty());
    // turnstate 投影照常工作（N1：双跑一致）。
    let p = turnstate();
    let v1 = p.apply(&snapshot, 0, Budget::generous());
    let v2 = p.apply(&snapshot, 0, Budget::generous());
    assert_eq!(v1, v2);
    assert!(v1.value.settled() && v1.value.final_text.is_some());
}

/// 验收（步骤 6 上半）：生成失败 → 兜底埋点（cost_ms / produced_by / error 载荷）。
#[tokio::test]
async fn reasoner_failure_produces_fallback_with_telemetry() {
    let llm = StubLlmAdapter::always_fail("模型不可用");
    let (store, outcome) = rehearse_reasoner_turn("任何话", &llm).await;
    assert!(outcome.is_err());
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let fb = snapshot
        .iter()
        .find(|e| e.kind == EVENT_ASSISTANT_FALLBACK)
        .expect("兜底必在");
    assert!(fb.produced_by.is_some(), "兜底可被 I2 审计");
    let payload: serde_json::Value = fb.payload.clone();
    assert!(
        payload.get("error").is_some(),
        "失败原因入载荷（可观测）：{payload}"
    );
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
    assert!(
        unresolved_turns(&snapshot).is_empty(),
        "写了 fallback ⇒ turn 已收束"
    );
}

/// 验收（步骤 6 下半）：四层 budget_ms 就位——同一时延在不同档位下结论不同。
#[tokio::test]
async fn four_tier_budgets_are_measurable() {
    // 注入 120ms 时延：反射档（80ms）超预算，深度档（60s）预算内。
    let llm = StubLlmAdapter::with_delay("stub-model", 120);
    let (store, _) = rehearse_reasoner_turn("你好", &llm).await;
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let cost_ms = snapshot.iter().find(|e| e.cost_ms > 0).map(|e| e.cost_ms);
    assert!(cost_ms.is_some(), "cost_ms 埋点必非零（兜底埋点就位）");
    let cost = cost_ms.unwrap();
    assert!(
        !budget_exceeded(&snapshot, LatencyTier::Reflex.budget_ms()).is_empty(),
        "反射档 {}/80ms 必须被看见超预算",
        cost
    );
    assert!(
        budget_exceeded(&snapshot, LatencyTier::Deep.budget_ms()).is_empty(),
        "同一时延在深度档预算内"
    );
    // 反向锚：预算表口径（01 §10）在 adapters 域的单测里逐值钉死。
    assert_eq!(LatencyTier::Reflex.budget_ms(), 80);
}

// ── S5 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 11–13 步）：记忆链路 ──
//
// 编码（memory.encoded）→ 检索（recall 投影 + RecallTranslator → memory.recalled）
// → 巩固 / 遗忘（排除式 + 保真度下界）。S06 §6 四条验收逐条落地。

use crate::symbio_core::event::EVENT_MEMORY_RECALLED;
use crate::symbio_core::projection::consolidate::{accept, ConsolidateParams};
use crate::symbio_core::projection::recall::recall;

/// 编码一条记忆（带溯源与 embedding 载荷）。
fn encode(store: &EventStore, id: &str, ts: i64, content: &str, tag: &str, source: u64) {
    store
        .append(
            Event::pending(
                id,
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                "agent:main",
            )
            .with_produced_by(source)
            .with_ts(ts)
            .with_payload(serde_json::json!({ "content": content, "tag": tag, "vec": [0.1, 0.2] })),
        )
        .expect("编码必入库");
}

/// 验收 1 / C12：后台追加 memory.consolidated 后，as-of=T 的视图**逐字节不变**。
#[test]
fn consolidation_does_not_pollute_as_of_view() {
    let store = EventStore::new();
    encode(&store, "m0", 1_000, "用户偏好简洁答复", "semantic", 0);
    encode(&store, "m1", 2_000, "项目用 Rust 2021", "semantic", 0);
    let p = recall("agent:main", None);
    let before = p.apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        2_500,
        Budget::generous(),
    );
    assert_eq!(before.value.entries.len(), 2);

    // 后台巩固（ts 在 as-of 之后）：压缩 m0（seq 0）+ 遗忘它。
    store
        .append(
            Event::pending(
                "c0",
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                0,
                "agent:main",
            )
            .with_produced_by(0)
            .with_ts(5_000)
            .with_payload(
                serde_json::json!({ "content": "压缩摘要", "generation": 1, "fidelity": 0.9 }),
            ),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "g0",
                EVENT_MEMORY_FORGOTTEN,
                Entity::Memory,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0) // 遗忘目标 = m0 的 seq
            .with_ts(5_000),
        )
        .unwrap();

    let after = p.apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        2_500,
        Budget::generous(),
    );
    assert_eq!(
        before, after,
        "C12：as-of=T 的视图不受后台巩固污染（逐字节）"
    );
    // as-of 挪到巩固之后：新记忆可见、旧记忆已被排除式遗忘（Log 未删，投影不再包含）。
    let future = p.apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        9_999,
        Budget::generous(),
    );
    assert!(future.value.contains_content("压缩摘要"));
    assert!(
        !future.value.contains_content("用户偏好简洁答复"),
        "遗忘 = 投影不再包含"
    );
}
use crate::symbio_core::event::{
    EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED, EVENT_MEMORY_FORGOTTEN,
};

/// 验收 2：检索预算耗尽 → degraded + 部分结果；**不允许空且不标降级**。
#[test]
fn recall_degrades_with_partial_results_when_budget_exhausted() {
    let store = EventStore::new();
    for i in 0..5 {
        encode(
            &store,
            &format!("m{i}"),
            1_000 + i,
            &format!("记忆{i}"),
            "semantic",
            0,
        );
    }
    let p = recall("agent:main", None);
    let tight = p.apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        9_999,
        Budget::new(0, 2), // 配额 2 次扫描
    );
    assert!(tight.degraded, "预算耗尽必须标降级");
    assert_eq!(tight.value.entries.len(), 2, "带部分结果，不是空");
    // 反向：预算充足 → 不降级、全量。
    let ok = p.apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        9_999,
        Budget::generous(),
    );
    assert!(!ok.degraded && ok.value.entries.len() == 5);
}

/// 验收 3 / C13：A 的记忆在 B 的 recall 视图中不出现（记忆默认 thread_private）。
#[test]
fn memory_of_a_never_appears_in_b_recall() {
    let store = EventStore::new();
    encode(&store, "ma", 1_000, "A 的私有记忆", "semantic", 0);
    store
        .append(
            Event::pending(
                "mb",
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                "agent:b",
            )
            .with_produced_by(0)
            .with_ts(1_100)
            .with_payload(
                serde_json::json!({ "content": "B 的记忆", "tag": "semantic", "vec": [] }),
            ),
        )
        .unwrap();
    let events = || store.range(crate::symbio_core::event::Seq::new(0));
    let b_view = recall("agent:b", None).apply(&events(), 9_999, Budget::generous());
    assert!(
        !b_view.value.contains_content("A 的私有记忆"),
        "跨主体默认不可见（C13）"
    );
    assert!(b_view.value.contains_content("B 的记忆"));
    let a_view = recall("agent:main", None).apply(&events(), 9_999, Budget::generous());
    assert!(a_view.value.contains_content("A 的私有记忆"));
    assert!(!a_view.value.contains_content("B 的记忆"));
}

/// 验收 4（反向）：忽略 as-of 的投影实现 → 断言 1 必须失败（可捕获）。
#[test]
fn projection_that_ignores_as_of_would_be_caught() {
    let store = EventStore::new();
    encode(&store, "m0", 1_000, "旧记忆", "semantic", 0);
    let before_view = recall("agent:main", None).apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        2_000,
        Budget::generous(),
    );
    // 后台追加 ts=5000 的巩固（压缩 m0 并遗忘之）。
    store
        .append(
            Event::pending(
                "c0",
                EVENT_MEMORY_CONSOLIDATED,
                Entity::Memory,
                Verb::Progressed,
                0,
                "agent:main",
            )
            .with_produced_by(0)
            .with_ts(5_000)
            .with_payload(
                serde_json::json!({ "content": "新摘要", "generation": 1, "fidelity": 0.9 }),
            ),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "g0",
                EVENT_MEMORY_FORGOTTEN,
                Entity::Memory,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0)
            .with_ts(5_000),
        )
        .unwrap();
    let events = store.range(crate::symbio_core::event::Seq::new(0));
    // 一个「忽略 as-of」的错误实现：now=MAX ⇒ 未来事件全部涌入。
    let broken_view = recall("agent:main", None).apply(&events, i64::MAX, Budget::generous());
    let correct_view = recall("agent:main", None).apply(&events, 2_000, Budget::generous());
    assert_ne!(
        before_view.value, broken_view.value,
        "忽略 as-of 的实现必然被巩固改变——断言 1 能抓住它"
    );
    assert_eq!(before_view.value, correct_view.value, "正确实现不变");
}

/// 步骤 13 / N8：保真度下界——失真巩固被拒收，入库的 100% 达标。
#[test]
fn consolidation_below_fidelity_floor_never_enters_the_log() {
    let params = ConsolidateParams::default();
    let store = EventStore::new();
    encode(&store, "m0", 1_000, "事实 A", "semantic", 0);
    // 巩固者产出两批：一批达标、一批失真；accept 是入库前的唯一闸门。
    let candidates = [(1u32, 0.9f64), (1, 0.4)];
    for (gen, fidelity) in candidates {
        if accept(&params, gen, fidelity).is_ok() {
            store
                .append(
                    Event::pending(
                        format!("c-{gen}-{fidelity}"),
                        EVENT_MEMORY_CONSOLIDATED,
                        Entity::Memory,
                        Verb::Progressed,
                        0,
                        "agent:main",
                    )
                    .with_produced_by(0)
                    .with_ts(2_000)
                    .with_payload(serde_json::json!({ "generation": gen, "fidelity": fidelity })),
                )
                .unwrap();
        }
    }
    // N8：入库的巩固 100% 保真度达标（失真的那批从未进入 Log）。
    let consolidated: Vec<(u32, f64)> = store
        .range(crate::symbio_core::event::Seq::new(0))
        .iter()
        .filter(|e| e.kind == EVENT_MEMORY_CONSOLIDATED)
        .map(|e| {
            (
                e.payload["generation"].as_u64().unwrap() as u32,
                e.payload["fidelity"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(consolidated.len(), 1, "失真巩固被拒收");
    assert!(
        consolidated
            .iter()
            .all(|(g, f)| *g <= params.max_gen && *f >= params.min_fidelity),
        "{consolidated:?}"
    );
}

/// 步骤 11：溯源覆盖 100%——无溯源的记忆事件被 I2 抓住。
#[test]
fn memory_event_without_provenance_is_caught() {
    let bare = Event {
        produced_by: None,
        ..Event::pending(
            "m0",
            EVENT_MEMORY_ENCODED,
            Entity::Memory,
            Verb::Opened,
            0,
            "agent:main",
        )
    };
    let bad = crate::symbio_core::invariants::produced_by_coverage(std::slice::from_ref(&bare));
    assert_eq!(bad.len(), 1, "无溯源的记忆必须被看见（溯源覆盖 100%）");
    assert!(bad[0].why.contains("记忆"), "{}", bad[0].why);
}

/// 全链路：检索 Translator 产出 memory.recalled（带溯源），三条不变量仍全绿。
#[test]
fn recall_translator_produces_provenanced_event_and_invariants_stay_green() {
    let store = EventStore::new();
    encode(&store, "m0", 1_000, "用户偏好简洁答复", "semantic", 0);
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let view = recall("agent:main", None)
        .apply(&snapshot, 2_000, Budget::generous())
        .value;
    let event = RecallTranslator.recalled_event(&view, 0);
    store.append(event).unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
    let recalled = snapshot
        .iter()
        .find(|e| e.kind == EVENT_MEMORY_RECALLED)
        .unwrap();
    assert_eq!(recalled.payload["found"], 1);
}

// ── S6 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 14–15 步）：多主体与对等承诺 ──
//
// 第 14 步：principal 隔离（下方验收 1/4）+ 无直连静态扫描（scripts/no-direct-call-audit.mjs，
// 出口判据「直连调用 0」；成对性拒绝 = S08 §6 验收 3，已在 governance 测试区覆盖）。
// 第 15 步：commitment 立约 + 声誉投影（出口判据「声誉双跑一致」）。

use crate::symbio_core::governance::{Capability, PermissionMatrix, PrincipalPolicy, VisScope};
use crate::symbio_core::projection::reputation::reputation;

/// S08 §6 验收 1 + 4：thread_private 互不可见；内容可见域放宽成 public 后，
/// 同一主体、同一份事件切片立刻可见——证明「可见域是参数在生效」，不是投影写死。
#[test]
fn principals_are_isolated_until_scope_widened() {
    let store = EventStore::new();
    store
        .append(Event::pending(
            "a0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "agent:a",
        ))
        .unwrap();
    store
        .append(Event::pending(
            "b0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "agent:b",
        ))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "agent:a",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "agent:b",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("成对策略必过");

    // 验收 1：thread_private 下 A 只看见自己写的事件。
    let seen_by_a: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("agent:a", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(
        seen_by_a.len(),
        1,
        "thread_private：A 只见自己（S08 §6 验收 1）"
    );
    assert_eq!(seen_by_a[0].actor, "agent:a");

    // 验收 4（反向）：内容可见域放宽为 public ⇒ 同一 viewer、同一切片全可见。
    let seen_by_a_wide: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("agent:a", &e.actor, VisScope::Public))
        .collect();
    assert_eq!(
        seen_by_a_wide.len(),
        2,
        "public ⇒ 隔离消失：断言 1 的实现若在 public 下仍红，说明可见域没在生效"
    );
}

/// 第 15 步：立约 → 守约 / 违约 → 声誉投影。违约可观测（broken 必带 why），
/// 声誉记在承诺方（from）头上。
#[test]
fn commitment_lifecycle_feeds_reputation() {
    let keeper = CommitmentKeeper;
    let store = EventStore::new();
    // B 立约两条：一条守约、一条违约；A 立约一条守约。
    store
        .append(keeper.offer("k1", "agent:b", "agent:a", "整理周报", 0))
        .unwrap();
    store.append(keeper.release("k1", "agent:b", 0)).unwrap();
    store
        .append(keeper.offer("k2", "agent:b", "agent:a", "压测报告", 0))
        .unwrap();
    store
        .append(keeper.breach("k2", "agent:b", "上游数据没到", 0))
        .unwrap();
    // 对等宣告：违约状态告知协作方（普通事件，不是新通道）。
    store
        .append(keeper.declare("k2", "agent:b", "违约已告知 agent:a", 0))
        .unwrap();
    store
        .append(keeper.offer("k3", "agent:a", "agent:b", "评审", 0))
        .unwrap();
    store.append(keeper.release("k3", "agent:a", 0)).unwrap();

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let view = reputation()
        .apply(&snapshot, 9_999, Budget::generous())
        .value;
    let b = view.of("agent:b");
    assert_eq!((b.offered, b.kept, b.broken), (2, 1, 1));
    assert_eq!(b.score, 0, "平凡打分：守约 − 违约");
    assert_eq!(view.of("agent:a").score, 1);
    // 未立约的主体：空条目，不是错误。
    assert_eq!(view.of("agent:ghost").offered, 0);

    // 全链路不变量仍绿（承诺也是普通事件，永远逃不出审计）。
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// 出口判据「声誉双跑一致」（N1 在声誉面上的形状）：同一份切片双跑逐字节相同；
/// as-of 锚前的违约不影响当时已算出的声誉。
#[test]
fn reputation_double_run_is_identical_and_as_of_safe() {
    let keeper = CommitmentKeeper;
    let store = EventStore::new();
    store
        .append(keeper.offer("k1", "agent:b", "agent:a", "整理周报", 0))
        .unwrap();
    store
        .append(keeper.release("k1", "agent:b", 0).with_ts(1_000))
        .unwrap();
    let before = {
        let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
        reputation()
            .apply(&snapshot, 2_000, Budget::generous())
            .value
    };
    assert_eq!(before.of("agent:b").score, 1);

    // 后台追加违约（ts 在 as-of 之后）。
    store
        .append(
            keeper
                .offer("k2", "agent:b", "agent:a", "压测报告", 0)
                .with_ts(5_000),
        )
        .unwrap();
    store
        .append(
            keeper
                .breach("k2", "agent:b", "上游数据没到", 0)
                .with_ts(6_000),
        )
        .unwrap();

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let after_as_of = reputation()
        .apply(&snapshot, 2_000, Budget::generous())
        .value;
    assert_eq!(before, after_as_of, "as-of=T 的声誉不受其后追加污染");
    let now = reputation()
        .apply(&snapshot, 9_999, Budget::generous())
        .value;
    assert_eq!(now.of("agent:b").score, 0, "违约计入后声誉下降");
}
