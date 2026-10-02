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

// ── S7 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 16–18 步）：任务树与返工 ──
//
// 16 DAG 调度 + readyset（C14 无环）；17 独立验证者（C15 不自验，构造期拒绝）；
// 18 返工闭环（rework_created，返工次数有界 = 终止性前提 2）。

use crate::symbio_core::event::{
    EVENT_TASK_ASSERTED, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS, EVENT_TASK_REWORK_CREATED,
};
use crate::symbio_core::invariants::{acyclic_deps, rework_bounded};
use crate::symbio_core::projection::readyset::readyset;

/// 开一个任务（载荷 `{ task_id, depends_on, goal }`）。
fn open_task(store: &EventStore, id: &str, depends_on: &[&str], actor: &str) {
    store
        .append(
            Event::pending(
                format!("o-{id}"),
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                0,
                actor,
            )
            .with_payload(
                serde_json::json!({ "task_id": id, "depends_on": depends_on, "goal": "goal" }),
            ),
        )
        .expect("开任务必入库");
}

/// 终态：任务验证通过。
fn assert_task(store: &EventStore, id: &str, actor: &str) {
    store
        .append(
            Event::pending(
                format!("done-{id}"),
                EVENT_TASK_ASSERTED,
                Entity::Task,
                Verb::Asserted,
                0,
                actor,
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": id })),
        )
        .expect("终态必入库");
}

/// S03 §6 验收 1：构造含环依赖图（a→b→c→a）→ 无环检测必须报错。
#[test]
fn cyclic_dependency_graph_is_rejected() {
    let store = EventStore::new();
    open_task(&store, "a", &["c"], "agent:main");
    open_task(&store, "b", &["a"], "agent:main");
    open_task(&store, "c", &["b"], "agent:main");
    let bad = acyclic_deps(&store.range(crate::symbio_core::event::Seq::new(0)));
    assert_eq!(bad.len(), 3, "环上三个节点都要被点名：{bad:?}");
    assert!(bad[0].why.contains("依赖环"), "{}", bad[0].why);
}

/// S03 §6 验收 4（反向）：把环去掉一个依赖 → 断言 1 必须通过（检测在算，不是常量 false）。
#[test]
fn breaking_one_edge_makes_graph_pass() {
    let store = EventStore::new();
    // 同一张图，只去掉 c→b 这条边。
    open_task(&store, "a", &["c"], "agent:main");
    open_task(&store, "b", &["a"], "agent:main");
    open_task(&store, "c", &[], "agent:main");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        acyclic_deps(&snapshot).is_empty(),
        "去边后必须过：{:?}",
        acyclic_deps(&snapshot)
    );
}

/// 悬空依赖：依赖不存在的任务 ⇒ 违规（termination.rs 前提 3 的另一半）。
#[test]
fn dangling_dependency_is_rejected() {
    let store = EventStore::new();
    open_task(&store, "a", &["ghost"], "agent:main");
    let bad = acyclic_deps(&store.range(crate::symbio_core::event::Seq::new(0)));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].why.contains("悬空"), "{}", bad[0].why);
}

/// S03 §6 验收 2（C15）：同一主体同时持 define.work 与 assert.verification → 拒绝。
#[test]
fn self_verifier_is_rejected_at_construction() {
    let both = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:greedy",
        vec![Capability::DefineWork, Capability::AssertVerification],
        VisScope::ThreadPrivate,
    )]);
    assert!(
        matches!(
            &both,
            Err(crate::symbio_core::governance::PairingViolation::SelfVerifier { .. })
        ),
        "双持必须被构造期拒绝：{both:?}"
    );
    // 反向：拆成两个主体 → 通过（验证者 ≠ 产出者）。
    let split = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "agent:planner",
            vec![Capability::DefineWork],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "agent:verifier",
            vec![Capability::AssertVerification],
            VisScope::ThreadPrivate,
        ),
    ]);
    assert!(split.is_ok(), "分工后必须通过：{split:?}");
}

/// 第 16 步：readyset 就绪集 = 非终态 ∧ 依赖闭合；双跑一致（N1）。
#[test]
fn readyset_contains_only_dependency_closed_tasks() {
    let store = EventStore::new();
    open_task(&store, "a", &[], "agent:main");
    open_task(&store, "b", &["a"], "agent:main");
    open_task(&store, "c", &["b"], "agent:main");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    let rs = readyset().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(rs.value.ready.len(), 1, "只有 a 就绪：{:?}", rs.value.ready);
    assert_eq!(rs.value.ready[0].task_id, "a");

    // a 终态 → 就绪集挪到 b；链式推进，调度器零内部状态。
    assert_task(&store, "a", "agent:verifier");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let rs = readyset().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(
        rs.value
            .ready
            .iter()
            .map(|t| t.task_id.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );

    // N1：双跑逐字节一致。
    let again = readyset().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(rs.value, again.value);
}

/// 第 18 步：返工闭环——验证判不合格 ⇒ 新增返工节点（不是回滚）⇒ 旧节点不再就绪。
#[test]
fn rework_creates_new_node_instead_of_rollback() {
    let store = EventStore::new();
    open_task(&store, "a", &[], "agent:planner");
    open_task(&store, "b", &["a"], "agent:planner");
    // 验证者判 a 不合格 → 返工（新增一条事件，append-only）。
    store
        .append(
            Event::pending(
                "rw-a1",
                EVENT_TASK_REWORK_CREATED,
                Entity::Task,
                Verb::Asserted,
                0,
                "agent:verifier",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": "a-r1", "replaces": "a", "round": 1 })),
        )
        .unwrap();
    open_task(&store, "a-r1", &[], "agent:planner");

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let rs = readyset().apply(&snapshot, 9_999, Budget::generous()).value;
    let ids: Vec<_> = rs.ready.iter().map(|t| t.task_id.as_str()).collect();
    assert!(ids.contains(&"a-r1"), "返工节点就绪：{ids:?}");
    assert!(
        ids.contains(&"a"),
        "旧节点仍未终态（Log 永不改——被取代靠新节点表达）"
    );

    // 返工节点过验 → b 的依赖由终态的 a-r1 之外仍卡住（a 未终态）→ b 不就绪。
    assert_task(&store, "a-r1", "agent:verifier");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let rs = readyset().apply(&snapshot, 9_999, Budget::generous()).value;
    let ids: Vec<_> = rs.ready.iter().map(|t| t.task_id.as_str()).collect();
    assert!(
        !ids.contains(&"b"),
        "b 依赖 a（不是 a-r1），依旧不就绪：{ids:?}"
    );

    // 终止性：返工 1 轮在上界内；第 3 轮超上界（max=2）→ 被看见。
    store
        .append(
            Event::pending(
                "rw-a2",
                EVENT_TASK_REWORK_CREATED,
                Entity::Task,
                Verb::Asserted,
                0,
                "agent:verifier",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": "a-r2", "replaces": "a", "round": 2 })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "rw-a3",
                EVENT_TASK_REWORK_CREATED,
                Entity::Task,
                Verb::Asserted,
                0,
                "agent:verifier",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": "a-r3", "replaces": "a", "round": 3 })),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        rework_bounded(&snapshot, 3).is_empty(),
        "3 轮对上界 3 恰好合规"
    );
    let bad = rework_bounded(&snapshot, 2);
    assert_eq!(bad.len(), 1, "上界 2 时第 3 轮违规：{bad:?}");
    assert!(bad[0].why.contains("超过上界"), "{}", bad[0].why);
    let bad = rework_bounded(&snapshot, 1);
    assert_eq!(bad.len(), 2, "上界 1 时第 2、3 轮都违规：{bad:?}");
}

/// S03 §6 验收 3：注入 N 步任务链 → 在 rework 上界内终止（全链跑完、检查全绿）。
#[test]
fn n_step_chain_terminates_within_rework_bound() {
    let store = EventStore::new();
    let n = 5;
    // 链：t0 → t1 → … → t4；每个任务至多返工 1 轮。
    for i in 0..n {
        let deps: Vec<String> = if i == 0 {
            vec![]
        } else {
            vec![format!("t{}", i - 1)]
        };
        open_task(
            &store,
            &format!("t{i}"),
            &deps.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            "agent:planner",
        );
    }
    // 全链推进：一次就绪一个，终态后下一个就绪（终止性前提 1/3 由 store 与 C14 保证）。
    for i in 0..n {
        let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
        let rs = readyset().apply(&snapshot, 9_999, Budget::generous()).value;
        assert_eq!(
            rs.ready.len(),
            1,
            "链式调度任意时刻恰一个就绪：{:?}",
            rs.ready
        );
        assert_eq!(rs.ready[0].task_id, format!("t{i}"));
        store
            .append(
                Event::pending(
                    format!("p-{i}"),
                    EVENT_TASK_PROGRESS,
                    Entity::Task,
                    Verb::Progressed,
                    0,
                    "agent:main",
                )
                .with_payload(serde_json::json!({ "task_id": format!("t{i}") })),
            )
            .unwrap();
        assert_task(&store, &format!("t{i}"), "agent:verifier");
    }
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        readyset()
            .apply(&snapshot, 9_999, Budget::generous())
            .value
            .ready
            .is_empty(),
        "全链终态"
    );
    assert!(acyclic_deps(&snapshot).is_empty());
    assert!(rework_bounded(&snapshot, 1).is_empty());
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

// ── S8 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 19–20 步）：时延与插话 ──
//
// 19 抢占判定者（80ms 反射档）+ 挂起/恢复（出口判据「二次调度 0」，roadmap/S07）；
// 20 熔断（外部执行闸门，出口判据「熔断后仍执行 0」，roadmap/S09）。

use crate::symbio_core::event::EVENT_CONTROL_OPENED;
use crate::symbio_core::projection::cost::cost_ledger;

/// 造一个在跑任务（供插话/熔断场景使用）。
fn running_task(store: &EventStore, id: &str) {
    open_task(store, id, &[], "agent:main");
}

/// S07 §6 验收 1：注入打断事件 → 80ms 内必须产出处置（Suspend ⇒ task.controlled 落库）。
#[test]
fn interruption_within_reflex_budget_produces_control_event() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let d = PreemptionDecider.decide(&snapshot, 10, 80);
    match d {
        Preemption::Suspend { task_id, as_of_seq } => {
            assert_eq!(task_id, "t1");
            // 挂起就是一条事件 + 一条控制事实（判定者只产控制事件，不发言）。
            store
                .append(PreemptionDecider.held_event(&task_id, as_of_seq))
                .unwrap();
            store
                .append(PreemptionDecider.control_event("interrupt-suspend", as_of_seq))
                .unwrap();
            let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
            assert!(
                snapshot.iter().any(|e| e.kind == EVENT_CONTROL_OPENED),
                "80ms 内必须有 task.controlled"
            );
            assert!(
                check_all(&snapshot).is_empty(),
                "{:?}",
                check_all(&snapshot)
            );
        }
        other => panic!("预算内应实质判定，得到 {other:?}"),
    }
}

/// S07 §6 验收 4（反向）：同一场景，预算放宽 → 判定结果必须改变（预算参数在生效）。
#[test]
fn widening_budget_changes_preemption_outcome() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    // 判定耗时 100ms：反射档（80ms）超时 → 默认继续（不挂起）。
    assert_eq!(
        PreemptionDecider.decide(&snapshot, 100, 80),
        Preemption::TimeoutDefaultContinue
    );
    // 同样的耗时，预算放宽到 60000 → 实质判定成立（挂起）。
    assert_eq!(
        PreemptionDecider.decide(&snapshot, 100, 60_000),
        Preemption::Suspend {
            task_id: "t1".into(),
            as_of_seq: 0
        }
    );
}

/// 04 §2.3 边界：final 已发出后到达插话 → 不抢占，排队（已发出的发言不可撤回）。
#[test]
fn interruption_after_final_is_queued_not_preempted() {
    let store = EventStore::new();
    running_task(&store, "t1");
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
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert_eq!(
        PreemptionDecider.decide(&snapshot, 10, 80),
        Preemption::Queue
    );
}

/// S07 §6 验收 2 + 3：挂起期间任务不在 readyset（二次调度 0）；以挂起锚 as-of 重放
/// 与挂起前一致（恢复零状态迁移）；恢复事件让任务重新就绪。
#[test]
fn suspended_task_leaves_readyset_and_restores() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let before = readyset().apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        9_999,
        Budget::generous(),
    );
    assert_eq!(before.value.ready.len(), 1);

    // 挂起（锚 T = 挂起前 head；held 事件 ts 在未来——as-of 语义下可被锚排除）。
    let t = store.head().value();
    store
        .append(PreemptionDecider.held_event("t1", t).with_ts(5_000))
        .unwrap();
    let events = store.range(crate::symbio_core::event::Seq::new(0));
    let suspended = readyset().apply(&events, 9_999, Budget::generous());
    assert!(
        suspended.value.ready.is_empty(),
        "挂起期间不得二次调度（S07 §6 验收 2）"
    );

    // 验收 3：以挂起锚（as-of 排除 held 事件）重放 → 与挂起前一致。
    // 恢复零状态迁移——投影是纯函数，给同一个 T 就得到同一个视图（04 §2.2）。
    let at_t = readyset().apply(&events, 1_000, Budget::generous());
    assert_eq!(at_t.value, before.value, "as-of=T 重放与挂起前一致");

    // 恢复：一条 progressed 事件 → 重新就绪（零状态迁移代码）。
    store
        .append(
            Event::pending(
                "resume-t1",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                "agent:main",
            )
            .with_payload(serde_json::json!({ "task_id": "t1" })),
        )
        .unwrap();
    let restored = readyset().apply(
        &store.range(crate::symbio_core::event::Seq::new(0)),
        9_999,
        Budget::generous(),
    );
    assert_eq!(restored.value.ready.len(), 1, "恢复后重新进入 readyset");
    assert_eq!(restored.value, before.value, "恢复后视图与挂起前一致");
}

/// S07 §5：抢占判定者只持 JudgeIntent——无 reply.* 写权（判定者不得直接发言）。
#[test]
fn preemption_decider_cannot_speak() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:reflex",
        vec![Capability::JudgeIntent],
        VisScope::ThreadPrivate,
    )])
    .expect("判定者策略合法");
    assert!(matrix.can_write("agent:reflex", Capability::JudgeIntent));
    assert!(
        !matrix.can_write("agent:reflex", Capability::ReplyFirst),
        "无首响写权"
    );
    assert!(
        !matrix.can_write("agent:reflex", Capability::ReplyAppend),
        "无追加写权"
    );
}

/// S09 §6 验收 1：未持外部执行能力的主体 → 拒绝且**不产生事件**。
#[test]
fn unauthorized_external_write_is_refused_without_event() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:talker",
        vec![Capability::ReplyAppend],
        VisScope::ThreadPrivate,
    )])
    .unwrap();
    let gate = CircuitBreaker;
    let authorized = matrix.can_write("agent:talker", Capability::ProduceArtifact);
    let store = EventStore::new();
    let head_before = store.head().value();
    match gate.gate(authorized, 0, 500, 1_000, 10) {
        GateDecision::Refuse => {
            assert_eq!(
                store.head().value(),
                head_before,
                "拒绝且零事件（不进 Log）"
            );
        }
        other => panic!("未授权必须 Refuse，得到 {other:?}"),
    }
}

/// S09 §6 验收 2 + 4：预算耗尽 → 必须产出熔断事件（不静默）；预算放宽 → 判定改变。
#[test]
fn budget_exhaustion_breaks_the_circuit() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:doer",
        vec![Capability::ProduceArtifact],
        VisScope::ThreadPrivate,
    )])
    .unwrap();
    let authorized = matrix.can_write("agent:doer", Capability::ProduceArtifact);
    let gate = CircuitBreaker;
    // 已耗 900 / 预算 1000，申请 500 ⇒ 超支 → 熔断（必须写事件）。
    let d = gate.gate(authorized, 900, 500, 1_000, 10);
    assert_eq!(
        d,
        GateDecision::Break {
            reason: "budget-exhausted"
        }
    );
    let store = EventStore::new();
    store
        .append(gate.break_event("budget-exhausted", 0))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        snapshot.iter().any(|e| e.kind == EVENT_CONTROL_OPENED),
        "熔断不允许静默继续"
    );
    // 验收 4（反向）：预算放宽到 10_000 → 同样的申请放行。
    assert_eq!(
        gate.gate(authorized, 900, 500, 10_000, 10),
        GateDecision::Allow
    );
    // 闸门自身超时（>80ms 反射档）→ 也是熔断（有事件的超时，不是静默失效）。
    assert_eq!(
        gate.gate(authorized, 0, 500, 10_000, 200),
        GateDecision::Break {
            reason: "gate-timeout"
        }
    );
}

/// S09 §6 验收 3：grants 表高风险组合（外部执行 + 自验）→ 构造期拒绝。
#[test]
fn high_risk_grant_combo_is_rejected() {
    let risky = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:risky",
        vec![Capability::ProduceArtifact, Capability::AssertVerification],
        VisScope::ThreadPrivate,
    )]);
    assert!(matches!(
        risky,
        Err(crate::symbio_core::governance::PairingViolation::SelfVerifier { .. })
    ));
}

/// ③ 成本台账：按主体累计 cost_ms；双跑一致（N1）；as-of 同口径。
#[test]
fn cost_ledger_accumulates_per_principal() {
    let store = EventStore::new();
    store
        .append(
            Event::pending(
                "e0",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                "agent:a",
            )
            .with_cost_ms(300)
            .with_payload(serde_json::json!({ "task_id": "t0" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "e1",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                "agent:a",
            )
            .with_cost_ms(200)
            .with_payload(serde_json::json!({ "task_id": "t0" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "e2",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                "agent:b",
            )
            .with_cost_ms(50)
            .with_payload(serde_json::json!({ "task_id": "t1" })),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let ledger = cost_ledger().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(ledger.value.of("agent:a").spent_ms, 500);
    assert_eq!(ledger.value.of("agent:a").entries, 2);
    assert_eq!(ledger.value.total_ms, 550);
    // N1 双跑一致。
    assert_eq!(
        ledger.value,
        cost_ledger()
            .apply(&snapshot, 9_999, Budget::generous())
            .value
    );
    // 未记账主体零账。
    assert_eq!(ledger.value.of("agent:ghost"), Default::default());
}

// ── S9 彩排（[plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 21–22 步）：自治与学习 ──
//
// 21 定时触发 + 自主层 budget_ms（出口判据「自主写入对话 0」，roadmap/S12）；
// 22 技能编译 + 校准 + 反自动化回退（出口判据「技能溯源 100%」，roadmap/S11）。

use crate::symbio_core::event::{EVENT_CONATION_EXPRESSED, EVENT_SYSTEM_TRIGGERED};
use crate::symbio_core::projection::calibration::calibration;
use std::mem::size_of;

/// S12 §6 验收 1：无用户消息时，定时触发必须产生 `system.triggered` 事件
/// （触发器产出事件，不是旁路——自主行为同样走 I1 单通道 + I2 溯源）。
#[test]
fn scheduled_trigger_produces_event_not_side_channel() {
    let store = EventStore::new();
    // 注意：此时 Log 里没有任何用户消息——纯自主场景。
    let init = AutonomousInitiator;
    store.append(init.trigger(0)).unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        snapshot.iter().any(|e| e.kind == EVENT_SYSTEM_TRIGGERED),
        "自主行为必须以事件形态存在"
    );
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

/// S12 §6 验收 2：自主发起者尝试写 `chat.assistant.final` → 授权拒绝且不产生事件。
#[test]
fn autonomous_actor_cannot_write_to_dialog() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "agent:autonomous",
        vec![Capability::DefineWork],
        VisScope::ThreadPrivate,
    )])
    .expect("自主发起者策略合法");
    let store = EventStore::new();
    let head_before = store.head().value();
    let authorized = matrix.can_write("agent:autonomous", Capability::ReplyFirst);
    assert!(!authorized, "自主层无 reply.* 写权（自主写入对话 = 0）");
    assert_eq!(store.head().value(), head_before, "拒绝 ⇒ 零事件");
}

/// S12 §6 验收 3 + 4：长目标超过自主层预算 → 必须被看见；预算改小 → 判定改变。
#[test]
fn long_goal_overrun_is_visible_and_budget_param_is_live() {
    let store = EventStore::new();
    let init = AutonomousInitiator;
    store.append(init.trigger(0)).unwrap();
    store
        .append(init.open_long_goal("goal-1", "整理全年归档", 0))
        .unwrap();
    // 任务执行事件：耗时 90M ms > 自主层预算 86.4M。
    store
        .append(
            Event::pending(
                "g1",
                EVENT_TASK_PROGRESS,
                Entity::Task,
                Verb::Progressed,
                0,
                "agent:autonomous",
            )
            .with_produced_by(0)
            .with_cost_ms(90_000_000)
            .with_payload(serde_json::json!({ "task_id": "goal-1" })),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    // 验收 3：超 86.4M 必须产出兜底（先被看见）。
    let bad = budget_exceeded(
        &snapshot,
        crate::symbio_core::adapters::LatencyTier::Autonomic.budget_ms(),
    );
    assert_eq!(bad.len(), 1, "长目标超支必须被看见：{bad:?}");
    // 验收 4（反向）：同一事件 70M ms（预算内）；把预算改成 60000 → 判定改变。
    let ok_event = Event::pending(
        "g2",
        EVENT_TASK_PROGRESS,
        Entity::Task,
        Verb::Progressed,
        0,
        "agent:autonomous",
    )
    .with_produced_by(0)
    .with_cost_ms(70_000)
    .with_payload(serde_json::json!({ "task_id": "goal-1" }));
    let ok_events = vec![ok_event];
    assert!(
        budget_exceeded(&ok_events, 86_400_000).is_empty(),
        "70M 在自主层预算内"
    );
    assert_eq!(
        budget_exceeded(&ok_events, 60_000).len(),
        1,
        "同一耗时，深挖档预算下违规"
    );
}

/// 02 §2.3 欲治理三条：(a) 候选造出来一定未批准 (b) 无溯源构造不出 (c) 关停后
/// 无任何自主任务但读侧投影照常（J2 平凡值）。ZST 令牌零开销。
#[test]
fn conation_governance_three_rules() {
    let init = AutonomousInitiator;
    let store = EventStore::new();
    store.append(init.trigger(0)).unwrap();
    store.append(init.express_intent("整理归档", 0)).unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    // (a) 唯一构造路径读出的候选，approved 恒 false——欲不得直接变成行。
    let want = snapshot
        .iter()
        .find(|e| e.kind == EVENT_CONATION_EXPRESSED)
        .unwrap();
    let mut candidate = ConationCandidate::from_event(want).expect("合法欲事件必出候选");
    assert!(!candidate.is_approved());
    // approved 字段私有：外部无法伪造 true（error[E0451]，编译期反向用例）。

    // (b) 无溯源的欲 → 构造不出候选（I2 强化）。
    let bare = Event {
        produced_by: None,
        ..Event::pending(
            "w1",
            EVENT_CONATION_EXPRESSED,
            Entity::Conation,
            Verb::Opened,
            0,
            "agent:x",
        )
    };
    assert!(
        ConationCandidate::from_event(&bare).is_none(),
        "来路不明的意图混不进来"
    );

    // (c) 关停开关：enabled=false → 评估拒绝，无自主任务；但读侧照常工作。
    let policy_off = ConationPolicy::default();
    assert_eq!(
        IntentGate::evaluate(&candidate, &policy_off),
        IntentDecision::Rejected("conation disabled")
    );
    // 打开后：评估通过 → 持令牌升格（ZST：size_of == 0）。
    let policy_on = ConationPolicy {
        enabled: true,
        max_goal_len: 200,
    };
    let warrant = IntentGate::issue_warrant();
    assert_eq!(size_of::<GateWarrant>(), 0, "ZST 零运行时开销");
    let approved = IntentGate::approve(&mut candidate, &policy_on, &warrant).expect("合法意图必批");
    assert!(candidate.is_approved());
    assert_eq!(approved.goal, "整理归档");
    assert_eq!(approved.from_seq, want.seq.map(|s| s.value()).unwrap_or(0));
}

/// S11 §6 验收 1 + 4：编译产出的技能事件 produced_by 必须非空；置空 → I2 抓住。
#[test]
fn compiled_skill_requires_provenance() {
    let compiler = SkillCompiler;
    // 源轨迹：一个成功任务的终态（seq 2）。
    let skill = compiler.compile("weekly-report", "用户问周报格式", "按模板三段式作答", 2);
    assert_eq!(skill.produced_by, Some(2), "溯源 100%：指向源轨迹");
    assert_eq!(skill.payload["tag"], "skill");
    // 正向：I2 全绿。
    assert!(
        crate::symbio_core::invariants::produced_by_coverage(std::slice::from_ref(&skill))
            .is_empty()
    );
    // 验收 4（反向）：把 produced_by 置空 → 断言 1 必须失败。
    let bare = Event {
        produced_by: None,
        ..skill
    };
    let bad = crate::symbio_core::invariants::produced_by_coverage(std::slice::from_ref(&bare));
    assert_eq!(bad.len(), 1, "无溯源的技能必须被看见");
}

/// S11 §6 验收 2 + 3：命中技能 cost 显著低于完整推理；置信度低于阈值必回退。
#[test]
fn skill_hit_is_cheaper_and_low_confidence_falls_back() {
    let store = EventStore::new();
    let compiler = SkillCompiler;
    // 编译技能（源轨迹 seq 0）。
    store
        .append(compiler.compile("weekly-report", "周报", "三段式", 0))
        .unwrap();
    // 命中：反射档快路（cost 40ms，低两个数量级）。
    store
        .append(
            Event::pending("h1", EVENT_TASK_PROGRESS, Entity::Task, Verb::Progressed, 0, "agent:main")
                .with_produced_by(0)
                .with_cost_ms(40)
                .with_payload(
                    serde_json::json!({ "task_id": "t1", "skill_id": "weekly-report", "fallback": false }),
                ),
        )
        .unwrap();
    // 未命中 / 回退：完整推理（cost 5000ms）。
    store
        .append(
            Event::pending("h2", EVENT_TASK_PROGRESS, Entity::Task, Verb::Progressed, 0, "agent:main")
                .with_produced_by(0)
                .with_cost_ms(5_000)
                .with_payload(
                    serde_json::json!({ "task_id": "t2", "skill_id": "weekly-report", "fallback": true }),
                ),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    // 验收 2：命中路径 cost 显著低于回退路径（40ms vs 5000ms）。
    let hit_cost = snapshot
        .iter()
        .find(|e| e.payload.get("fallback") == Some(&serde_json::json!(false)))
        .map(|e| e.cost_ms)
        .unwrap();
    let fallback_cost = snapshot
        .iter()
        .find(|e| e.payload.get("fallback") == Some(&serde_json::json!(true)))
        .map(|e| e.cost_ms)
        .unwrap();
    assert!(
        hit_cost * 10 < fallback_cost,
        "技能命中必须显著更便宜：{hit_cost} vs {fallback_cost}"
    );

    // 校准：1 用 1 回退 ⇒ 置信度 0.5；低于阈值 0.8 ⇒ 必须回退（不得走技能路径）。
    let cal = calibration().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(cal.value.of("weekly-report").uses, 2);
    assert_eq!(cal.value.of("weekly-report").fallbacks, 1);
    let router = SkillRouter;
    assert_eq!(
        router.route(cal.value.of("weekly-report").confidence(), 0.8),
        SkillRoute::ReasonerFallback { budget_ms: 300 },
        "校准值持续走低 ⇒ 保留反自动化回退"
    );
    // 反向：置信度高于阈值 → 走快路。
    assert_eq!(
        router.route(0.95, 0.8),
        SkillRoute::SkillFastPath { budget_ms: 80 }
    );
    // N1：校准双跑一致。
    assert_eq!(
        cal.value,
        calibration()
            .apply(&snapshot, 9_999, Budget::generous())
            .value
    );
    // 全链路不变量仍绿。
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

// ── ③ 兜底率投影（SLO §1.2 兜底率列的统计口径）──────────────────────────

/// 深度档 3 turn（2 final + 1 fallback）→ 33%；反射档 2 turn 全 final → 0%；
/// 缺 tier 的 turn 进 unspecified 桶（可观测，不静默归类）；N1 双跑一致。
#[test]
fn fallback_rate_counts_per_tier_and_is_deterministic() {
    let store = EventStore::new();
    // 深度档：t0 final、t1 fallback、t2 final。
    for (i, turn) in [0u64, 1, 2].iter().enumerate() {
        store
            .append(
                Event::pending(
                    format!("u-deep-{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    *turn,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap();
        let kind = if *turn == 1 {
            crate::symbio_core::EVENT_ASSISTANT_FALLBACK
        } else {
            crate::symbio_core::EVENT_ASSISTANT_FINAL
        };
        store
            .append(
                Event::pending(
                    format!("a-deep-{i}"),
                    kind,
                    Entity::Turn,
                    Verb::Closed,
                    *turn,
                    "agent:main",
                )
                .with_produced_by(i as u64 * 2),
            )
            .unwrap();
    }
    // 反射档：t3、t4 全 final。
    for i in 0..2u64 {
        let turn = 3 + i;
        store
            .append(
                Event::pending(
                    format!("u-reflex-{i}"),
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    turn,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "reflex" })),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    format!("a-reflex-{i}"),
                    crate::symbio_core::EVENT_ASSISTANT_FINAL,
                    Entity::Turn,
                    Verb::Closed,
                    turn,
                    "agent:main",
                )
                .with_produced_by(turn * 2),
            )
            .unwrap();
    }
    // 未声明档位：t5 fallback —— 必须落在 unspecified 桶。
    store
        .append(
            Event::pending(
                "u-x",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                5,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "问" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "a-x",
                crate::symbio_core::EVENT_ASSISTANT_FALLBACK,
                Entity::Turn,
                Verb::Closed,
                5,
                "agent:main",
            )
            .with_produced_by(10),
        )
        .unwrap();

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let view = crate::symbio_core::fallback_rate().apply(
        &snapshot,
        i64::MAX,
        crate::symbio_core::Budget::generous(),
    );

    let deep = view.value.of("deep");
    assert_eq!((deep.turns, deep.fallbacks), (3, 1));
    assert!((deep.rate() - 1.0 / 3.0).abs() < 1e-9);

    let reflex = view.value.of("reflex");
    assert_eq!((reflex.turns, reflex.fallbacks), (2, 0));
    assert_eq!(reflex.rate(), 0.0);

    let unspec = view.value.of("unspecified");
    assert_eq!((unspec.turns, unspec.fallbacks), (1, 1));
    assert_eq!(unspec.rate(), 1.0, "未声明档位的兜底必须被看见");

    // N1：双跑逐字节一致。
    let again = crate::symbio_core::fallback_rate().apply(
        &snapshot,
        i64::MAX,
        crate::symbio_core::Budget::generous(),
    );
    assert_eq!(view.value, again.value);
}

/// 档位名字往返：LatencyTier::name / from_name 互逆；未知名字不静默归类。
#[test]
fn latency_tier_name_round_trip() {
    use crate::symbio_core::adapters::LatencyTier;
    for t in [
        LatencyTier::Reflex,
        LatencyTier::Fast,
        LatencyTier::Deep,
        LatencyTier::Autonomic,
    ] {
        assert_eq!(LatencyTier::from_name(t.name()), Some(t));
    }
    assert_eq!(LatencyTier::from_name("gpu"), None);
    assert_eq!(LatencyTier::from_name(""), None);
}

// ── v2 会话运行时：TurnRunner 验收（不变量靠构造成立 + WAL 持久 + 兜底联动）──

mod turn_runner_tests {
    use crate::symbio_core::adapters::{LatencyTier, StubLlmAdapter, TokenIssuer};
    use crate::symbio_core::store::Store;
    use crate::symbio_core::{
        check_all, cost_ledger, fallback_rate, transcript, Budget, Event, EventStore, Seq,
        TurnInput, TurnRunner, WalStore, EVENT_USER_MESSAGE,
    };

    /// 成功轮：final 落格、溯源指向本轮用户消息、实测 cost_ms > 0、
    /// 不变量绿、收束投影 settled；成本台账与兜底率同账（ADR-044）。
    #[tokio::test]
    async fn turn_runner_happy_path_lands_final_with_measured_cost() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "你好", LatencyTier::Deep)
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 0);
        assert!(!out.fell_back);
        assert!(out.text.contains("你好"), "桩回显 prompt：{}", out.text);
        // 桩零延迟 ⇒ cost_ms 可为 0（诚实值）；真实链路的正数断言在真实端点彩排里。

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );

        // 成本台账：实测值入账（agent:main 一笔）。
        let ledger = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(ledger.value.of("agent:main").spent_ms, out.cost_ms);

        // 兜底率：deep 档 1 turn，0 兜底。
        let fr = fallback_rate().apply(&snapshot, i64::MAX, Budget::generous());
        let deep = fr.value.of("deep");
        assert_eq!((deep.turns, deep.fallbacks), (1, 0));
        assert_eq!(deep.rate(), 0.0);

        // turnstate：收束且 final 可见。
        let view = crate::symbio_core::turnstate().apply(&snapshot, 0, Budget::generous());
        assert!(view.value.settled() && view.value.final_text.is_some());
    }

    /// 失败轮（I3）：兜底事件落格（`turn × closed` + `why`），`fell_back = true`，
    /// 不变量照常绿——「失败也是一句话」，不是静默。
    #[tokio::test]
    async fn turn_runner_failure_lands_fallback_not_silence() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::always_fail("模型挂了");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "你好", LatencyTier::Deep)
            .await
            .expect("兜底路径仍返回 Ok（轮次本身收束了）");
        assert!(out.fell_back, "失败轮必须走兜底");
        assert!(
            out.text.contains("模型挂了"),
            "兜底话术 = 失败原因：{}",
            out.text
        );

        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );

        // 兜底率：deep 档 1 turn 1 兜底 = 100%（被看见，不是被藏）。
        let fr = fallback_rate().apply(&snapshot, i64::MAX, Budget::generous());
        let deep = fr.value.of("deep");
        assert_eq!((deep.turns, deep.fallbacks), (1, 1));
        assert_eq!(deep.rate(), 1.0);
    }

    /// WAL 持久：多轮写入 → 重开 → 逐字节一致（N2 家族：恢复 = 重放，
    /// 投影不需要状态迁移）；turn 号跨重启由调用方续排（N3 仍成立）。
    #[tokio::test]
    async fn turn_runner_over_wal_survives_reopen() {
        let dir = std::env::temp_dir().join(format!("symbio-turn-runner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录");
        let wal = dir.join("session.wal");

        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");
        {
            let store = WalStore::<Event>::open(&wal).expect("open");
            for turn in 0..3u64 {
                TurnRunner
                    .run(&store, &llm, &tok, turn, "问", LatencyTier::Deep)
                    .await
                    .expect("桩必答");
            }
        } // drop ⇒ 文件在盘上

        let reopened = WalStore::<Event>::open(&wal).expect("reopen");
        assert_eq!(reopened.head().value(), 6, "3 轮 × 2 事件");

        // 恢复后直接续排（turn 号接续）——N3 由 turn 号单调保证。
        let out = TurnRunner
            .run(&reopened, &llm, &tok, 3, "再问", LatencyTier::Deep)
            .await
            .expect("桩必答");
        assert_eq!(out.turn, 3);

        let snapshot = reopened.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(snapshot.len(), 8);

        // 与一遍直写（不经过重启）的台账逐字节一致（N1/N2 家族）。
        let fresh = crate::symbio_core::EventStore::new();
        for turn in 0..4u64 {
            TurnRunner
                .run(
                    &fresh,
                    &llm,
                    &tok,
                    turn,
                    if turn == 3 { "再问" } else { "问" },
                    LatencyTier::Deep,
                )
                .await
                .unwrap();
        }
        let a = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
        let b = cost_ledger().apply(&fresh.range(Seq::new(0)), i64::MAX, Budget::generous());
        assert_eq!(a.value.total_ms, b.value.total_ms, "恢复后的台账与直写一致");
        assert_eq!(a.value.by_principal, b.value.by_principal);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 多轮 turn 号单调：N3（每 turn ≤ 1 条 final）由构造成立——
    /// 同一 turn 号重复使用被幂等键挡住（事件 id 撞车 ⇒ Duplicate）。
    #[tokio::test]
    async fn turn_runner_reused_turn_number_is_rejected() {
        let store = crate::symbio_core::EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        TurnRunner
            .run(&store, &llm, &tok, 0, "第一轮", LatencyTier::Deep)
            .await
            .unwrap();
        let err = TurnRunner
            .run(&store, &llm, &tok, 0, "重复 turn 号", LatencyTier::Deep)
            .await;
        assert!(err.is_err(), "事件 id 撞车必须被 Store 幂等键拒绝");
        assert!(matches!(
            err,
            Err(crate::symbio_core::store::AppendError::Duplicate)
        ));
    }

    /// 多轮对话带历史：③ transcript 投影读同一事实源，第二轮 prompt 含第一轮；
    /// 单轮 prompt 保持裸文本（与无历史形态等价）。
    #[tokio::test]
    async fn multi_turn_transcript_carries_history_from_fact_source() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed("stub-model");

        // 第一轮：单轮 prompt = 裸文本。
        let r1 = TurnRunner
            .run(&store, &llm, &tok, 0, "第一轮问题", LatencyTier::Deep)
            .await
            .unwrap();
        assert!(r1.text.contains("第一轮问题"));
        assert!(
            !r1.text.contains("<对话历史>"),
            "单轮不得引入历史标记：{}",
            r1.text
        );

        // 第二轮：prompt 带历史——模型看得见自己上一轮的答复。
        let r2 = TurnRunner
            .run(&store, &llm, &tok, 1, "第二轮问题", LatencyTier::Deep)
            .await
            .unwrap();
        assert!(
            r2.text.contains("<对话历史>"),
            "多轮必须带历史：{}",
            r2.text
        );
        assert!(r2.text.contains("用户: 第一轮问题"), "历史里有上一轮提问");
        assert!(r2.text.contains("助手: "), "历史里有上一轮答复");
        assert!(r2.text.contains("第二轮问题"), "当前消息在历史之外");

        // 投影联动与 N1。
        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 4, "两轮 × (问+答)");
        assert_eq!(view.value.entries[0].role, "user");
        assert_eq!(view.value.entries[1].role, "assistant");
        let again = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value, again.value);
    }

    /// fallback 行进转写（assistant / why）——兜底话术是用户实际看到的回复。
    #[test]
    fn transcript_includes_fallback_as_assistant_line() {
        use crate::symbio_core::{Entity, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_USER_MESSAGE};
        let store = EventStore::new();
        store
            .append(
                Event::pending(
                    "u-0",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(serde_json::json!({ "text": "问", "tier": "deep" })),
            )
            .unwrap();
        store
            .append(
                Event::pending(
                    "fb-0",
                    EVENT_ASSISTANT_FALLBACK,
                    Entity::Turn,
                    Verb::Closed,
                    0,
                    "agent:main",
                )
                .with_produced_by(0)
                .with_payload(serde_json::json!({ "why": "上游 402" })),
            )
            .unwrap();
        let snapshot = store.range(Seq::new(0));
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries.len(), 2);
        assert_eq!(view.value.entries[1].role, "assistant");
        assert_eq!(view.value.entries[1].text, "上游 402");
    }

    /// 收集口：按序记增量（流式验收用）。
    struct CollectingDeltas(std::sync::Mutex<Vec<String>>);

    impl crate::symbio_core::adapters::DeltaSink for CollectingDeltas {
        fn on_delta(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_string());
        }
    }

    /// 流式运行：分片按序进 sink；落格语义与 run() 同一条路径（final 照常、
    /// 不变量照绿）；无分片的桩走默认降级（一次性全文一帧）。
    #[tokio::test]
    async fn streaming_run_forwards_deltas_and_writes_final() {
        use crate::symbio_core::adapters::DeltaSink;
        use std::sync::{Arc, Mutex};

        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::succeed_streaming("stub-model", &["你", "好", "！"]);
        let got = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));

        let out = TurnRunner
            .run_streaming(
                &store,
                &llm,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    window_turns: None,
                },
                got.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();

        assert_eq!(
            *got.0.lock().unwrap(),
            vec!["你", "好", "！"],
            "分片按序送达"
        );
        assert_eq!(out.text, "你好！", "全文 = 分片拼接");
        assert!(!out.fell_back);
        let snapshot = store.range(Seq::new(0));
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        let view = transcript().apply(&snapshot, i64::MAX, Budget::generous());
        assert_eq!(view.value.entries[1].text, "你好！", "final 落的是全文");

        // 默认降级：无分片桩经 run_streaming = 一帧全文（诚实降级，不是静默）。
        let store2 = EventStore::new();
        let llm2 = StubLlmAdapter::succeed("stub-model");
        let got2 = Arc::new(CollectingDeltas(Mutex::new(Vec::new())));
        TurnRunner
            .run_streaming(
                &store2,
                &llm2,
                &tok,
                TurnInput {
                    turn: 0,
                    text: "问".into(),
                    tier: LatencyTier::Deep,
                    window_turns: None,
                },
                got2.clone() as Arc<dyn DeltaSink>,
            )
            .await
            .unwrap();
        let frames = got2.0.lock().unwrap();
        assert_eq!(frames.len(), 1, "非流式适配器 = 一帧全文");
        assert!(frames[0].contains("问"));
    }

    /// 中止（AdapterError::Aborted）：**不落收束格**——网格只剩已入格的用户
    /// 消息（少一格是诚实的缺口，ADR-045 同源），`aborted` 为真且 `text` 为空；
    /// 不变量照绿（C4 会看见这个缺口，那是设计而非缺陷：中止不是静默中断）。
    #[tokio::test]
    async fn aborted_run_leaves_only_user_event() {
        use crate::symbio_core::invariants::unresolved_turns;

        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = StubLlmAdapter::aborting("stub-model");

        let out = TurnRunner
            .run(&store, &llm, &tok, 0, "问", LatencyTier::Deep)
            .await
            .unwrap();

        assert!(out.aborted, "中止必须在结果里可见");
        assert!(!out.fell_back, "中止不是兜底（两者互斥）");
        assert!(out.text.is_empty(), "中止没有答复文本");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(snapshot.len(), 1, "只剩用户格：{snapshot:?}");
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(
            unresolved_turns(&snapshot).len(),
            1,
            "缺口要**可被判出**（不是假装收束）——C4 看得见它"
        );
    }
}

// ── 工具轮（`run_with_tools`）：产物落格 + 结果回灌 + 等待用户停下 ──────────

mod tool_round_tests {
    use std::sync::{Arc, Mutex};

    use crate::symbio_core::adapters::{
        AdapterError, DeltaSink, FullModel, LatencyTier, LlmAdapter, LlmTurn, SilentDeltas,
        TokenIssuer,
    };
    use crate::symbio_core::invariants::unresolved_turns;
    use crate::symbio_core::store::Store;
    use crate::symbio_core::{
        check_all, CapabilityMeta, DispatchOutcome, DispatchPort, Entity, EventStore, Seq,
        TurnInput, TurnRunner, TurnToolCallInfo, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FINAL,
        EVENT_USER_MESSAGE,
    };

    /// 假适配器：首次请求回一个工具调用，之后回正文；记录每次收到的 prompt。
    struct ToolCallingLlm {
        prompts: Arc<Mutex<Vec<String>>>,
    }

    impl ToolCallingLlm {
        fn new() -> Self {
            Self {
                prompts: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmAdapter for ToolCallingLlm {
        fn model_id(&self) -> &str {
            "tool-mock"
        }
        async fn generate(&self, _tok: &FullModel, _prompt: &str) -> Result<String, AdapterError> {
            // 本用例只走工具通道（`generate_turn`）——留一个诚实的不支持。
            Err(AdapterError::GenerationFailed("unused".into()))
        }
        async fn generate_turn(
            &self,
            _tok: &FullModel,
            prompt: &str,
            _tools: &[CapabilityMeta],
            _sink: Arc<dyn DeltaSink>,
        ) -> Result<LlmTurn, AdapterError> {
            let round = {
                let mut p = self.prompts.lock().unwrap();
                p.push(prompt.to_string());
                p.len()
            };
            if round == 1 {
                Ok(LlmTurn {
                    text: "我先查一下。".into(),
                    tool_calls: vec![TurnToolCallInfo {
                        id: Some("tc-1".into()),
                        wire_id: Some("w-1".into()),
                        name: Some("vdfs_read".into()),
                        arguments: serde_json::json!({ "path": "a.md" }),
                        parse_error: None,
                    }],
                    cost_ms: 3,
                })
            } else {
                Ok(LlmTurn {
                    text: "读到了。".into(),
                    tool_calls: Vec::new(),
                    cost_ms: 4,
                })
            }
        }
    }

    /// 假分发方：把工具调用映射成一条结果事实；`pending` 决定是否收束于等用户。
    struct FakeDispatch {
        pending: bool,
        rounds: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl DispatchPort for FakeDispatch {
        async fn dispatch(&self, turn: &LlmTurn) -> Vec<DispatchOutcome> {
            *self.rounds.lock().unwrap() += 1;
            turn.tool_calls
                .iter()
                .map(|tc| DispatchOutcome {
                    call_id: tc.id.clone().unwrap_or_default(),
                    name: tc.name.clone().unwrap_or_default(),
                    text: "文件内容：hello".into(),
                    ok: true,
                    needs_user_action: self.pending,
                })
                .collect()
        }
    }

    fn tools() -> Vec<CapabilityMeta> {
        vec![CapabilityMeta {
            name: "vdfs_read".into(),
            ..Default::default()
        }]
    }

    fn input() -> TurnInput {
        TurnInput {
            turn: 0,
            text: "读 a.md".into(),
            tier: LatencyTier::Deep,
            window_turns: None,
        }
    }

    /// 工具轮：产物落 `artifact × asserted` 且溯源指向本轮用户格；final 只落一次；
    /// **工具结果进了下一次请求**（回读 prompt——不变量与网格都证明不了这条）。
    #[tokio::test]
    async fn tool_round_lands_artifact_and_feeds_next_prompt() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let prompts = llm.prompts.clone();
        let dispatch = FakeDispatch {
            pending: false,
            rounds: Arc::new(Mutex::new(0)),
        };

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
            )
            .await
            .expect("工具轮必答");

        assert_eq!(out.text, "读到了。");
        assert!(!out.fell_back && !out.aborted && !out.awaits_user);
        assert_eq!(out.cost_ms, 7, "跨轮累加实测耗时（3 + 4）");
        assert_eq!(*dispatch.rounds.lock().unwrap(), 1, "分发恰好一次");

        // 两次请求；第二次带上了模型请求过的工具与它的结果。
        let seen = prompts.lock().unwrap().clone();
        assert_eq!(seen.len(), 2, "工具轮 + 收尾轮：{seen:?}");
        assert!(
            !seen[0].contains("文件内容"),
            "第一次请求还没有结果：{}",
            seen[0]
        );
        assert!(
            seen[1].contains("vdfs_read") && seen[1].contains("文件内容：hello"),
            "第二次请求要带上工具调用与结果：{}",
            seen[1]
        );

        // 网格：用户格 + 产物格 + final 格（final 只一条——N3）。
        let snapshot = store.range(Seq::new(0));
        assert_eq!(snapshot.len(), 3, "{snapshot:?}");
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(snapshot[0].kind, EVENT_USER_MESSAGE);
        assert_eq!(snapshot[1].kind, EVENT_ARTIFACT_ADDED);
        assert_eq!(snapshot[1].entity, Entity::Artifact);
        assert_eq!(snapshot[1].verb, Verb::Asserted);
        assert_eq!(snapshot[1].payload["tool"], "vdfs_read");
        assert_eq!(snapshot[1].payload["text"], "文件内容：hello");
        assert_eq!(
            snapshot[1].produced_by,
            Some(0),
            "产物格溯源指向本轮用户格（S02 §3 的 caused_by 断言）"
        );
        assert_eq!(snapshot[2].kind, EVENT_ASSISTANT_FINAL);
        assert_eq!(snapshot[2].payload["text"], "读到了。");
    }

    /// 收束于等待用户：运行器**停下且不落收束格**——本轮还没了结（网格少一格是
    /// 诚实缺口，与中止同纪律），且这个缺口必须**可被判出**（C4 看得见它）。
    #[tokio::test]
    async fn pending_tool_stops_loop_without_closing_cell() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();
        let dispatch = FakeDispatch {
            pending: true,
            rounds: Arc::new(Mutex::new(0)),
        };

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                Some(&dispatch),
            )
            .await
            .expect("等待用户不是失败");

        assert!(out.awaits_user, "等待用户必须在结果里可见");
        assert!(
            !out.fell_back && !out.aborted,
            "等待用户既不是失败也不是中止"
        );
        assert_eq!(*dispatch.rounds.lock().unwrap(), 1, "停下：不再发起第二轮");

        let snapshot = store.range(Seq::new(0));
        assert_eq!(
            snapshot.len(),
            2,
            "用户格 + 产物格，**没有**收束格：{snapshot:?}"
        );
        assert_eq!(snapshot[1].kind, EVENT_ARTIFACT_ADDED);
        assert!(
            check_all(&snapshot).is_empty(),
            "{:?}",
            check_all(&snapshot)
        );
        assert_eq!(
            unresolved_turns(&snapshot).len(),
            1,
            "缺口要可被判出（不是假装收束）"
        );
    }

    /// 无分发方却收到工具调用 = 配置缺口：按失败诚实回报（兜底格），
    /// 不假装成功、也不静默丢工具调用。
    #[tokio::test]
    async fn tool_call_without_dispatch_falls_back() {
        let store = EventStore::new();
        let tok = TokenIssuer::issue_deep();
        let llm = ToolCallingLlm::new();

        let out = TurnRunner
            .run_with_tools(
                &store,
                &llm,
                &tok,
                input(),
                Arc::new(SilentDeltas),
                &tools(),
                None,
            )
            .await
            .expect("失败轮仍返回 Ok（轮次收束了）");

        assert!(out.fell_back, "没有分发通道 ⇒ 兜底");
        assert!(
            out.text.contains("工具分发通道"),
            "兜底话术说明原因：{}",
            out.text
        );
        let snapshot = store.range(Seq::new(0));
        assert_eq!(snapshot.len(), 2, "用户格 + 兜底格：{snapshot:?}");
        assert_eq!(
            snapshot[1].kind,
            crate::symbio_core::EVENT_ASSISTANT_FALLBACK
        );
    }
}
