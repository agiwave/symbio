//! `actors` 单测 —— 含 **S01 闭环彩排**（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")
//! 第 3 步：零 LLM 跑通完整闭环——应答由彩排内的定值给出，链路一条不少）。
//!
//! 彩排把 S0 的地基（store / projection / invariants）与 S1 的 ② `actors`
//! 拧成一条完整链路，验收断言逐条对应
//! [roadmap/S01 §6](../../../../docs/009-task/121-最小闭环.md "T-121-最小闭环")。

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
    let spec = ActorSpec::trivial("session/main");
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

// ── T-082 批①：收束闸判读**规格自己身上**的能力集 ─────────────────────

/// 三种主体形态一次钉住：会话主体（`reply.*` ⇒ 发言放 / 产物拒）、对等主体
/// （`produce.artifact` ⇒ 发言拒 / 产物放）、空表（平凡值 ⇒ 两臂皆拒）。
///
/// 这条的**反向用例**在末尾：清空能力集后产物臂必须拒——判定若悄悄绕回授权表
/// （不读规格），那条会绿，本判据即失效。
#[test]
fn spec_closure_judgments_read_the_specs_own_capability_set() {
    use crate::symbio_core::authz::{matrix_for, PRINCIPAL_MAIN, PRINCIPAL_PEER};

    let session = ActorSpec {
        capabilities: matrix_for(PRINCIPAL_MAIN).grant_names_of(PRINCIPAL_MAIN),
        ..ActorSpec::trivial(PRINCIPAL_MAIN)
    };
    assert!(
        session.can_land_speech(true),
        "持 reply.first ⇒ 首条收束放行"
    );
    assert!(
        session.can_land_speech(false),
        "持 reply.append ⇒ 追加收束放行"
    );
    assert!(
        session.can_land_artifact(),
        "主智能体行含 produce.artifact ⇒ 产物臂同样放行（两臂都持）"
    );

    let peer = ActorSpec {
        capabilities: matrix_for(PRINCIPAL_PEER).grant_names_of(PRINCIPAL_PEER),
        ..ActorSpec::trivial(PRINCIPAL_PEER)
    };
    assert!(
        !peer.can_land_speech(true) && !peer.can_land_speech(false),
        "对等主体无发言权（ADR-067：两条收束路各问自己那一项）"
    );
    assert!(peer.can_land_artifact(), "对等主体的权威动作 = 产生产物");

    // 反向用例：清空能力集 ⇒ 产物臂必拒。判定读的是**这个**集合，
    // 不是绕回授权表——绕回去的话这条必绿。
    let stripped = ActorSpec {
        capabilities: Vec::new(),
        ..peer.clone()
    };
    assert!(
        !stripped.can_land_artifact(),
        "清空能力集后产物臂必须拒（证明判定读的是规格自己）"
    );

    // 平凡值 `trivial`：权限取实（该主体自己的授权行）——`agent:main` 能收束。
    // 这正是订正 trivial 空表的理由：空表下平凡值连一轮对话都收束不了
    // （而 S01 §4 要求「平凡值下系统完整运行」）。
    let baseline = ActorSpec::trivial("session/main");
    assert!(
        baseline.can_land_speech(true) && baseline.can_land_artifact(),
        "平凡值权限取实：agent:main 两臂都放"
    );
    // 而只持 define.work 的自主主体照旧无发言权——窄行不因「平凡值」豁免。
    let autonomous = ActorSpec::trivial("session/autonomous");
    assert!(
        !autonomous.can_land_speech(true),
        "自主主体无发言权（平凡值不豁免窄行：自主行为不得冒充用户对话）"
    );
}

/// 规格判定与表谓词**必须一致**：同一主体的两个入口，一份口径。
///
/// 挡的是「映射分叉」：`Capability::reply_capability` 若被复制成两份（core 一份、
/// 插件一份），改一处即静默错位，而错位方向是**放行**（fail-open）。四个主体覆盖
/// 登记行（会话 / 对等 / 抢占判定者）与派生分支（未登记 `agent:<id>`）。
#[test]
fn spec_judgment_agrees_with_matrix_predicate_for_every_principal_shape() {
    use crate::symbio_core::authz::{matrix_for, PRINCIPAL_MAIN, PRINCIPAL_PEER, PRINCIPAL_REFLEX};
    for p in [
        PRINCIPAL_MAIN,
        PRINCIPAL_PEER,
        PRINCIPAL_REFLEX,
        "session/unregistered",
    ] {
        let matrix = matrix_for(p);
        let spec = ActorSpec {
            capabilities: matrix.grant_names_of(p),
            ..ActorSpec::trivial(p)
        };
        assert_eq!(
            spec.can_land_speech(true),
            matrix.can_reply(p, true),
            "{p}: 首条收束两入口必须一致"
        );
        assert_eq!(
            spec.can_land_speech(false),
            matrix.can_reply(p, false),
            "{p}: 追加收束两入口必须一致"
        );
        assert_eq!(
            spec.can_land_artifact(),
            matrix.can_produce_artifact(p),
            "{p}: 产物臂两入口必须一致"
        );
    }
}

// ── S01 闭环彩排 ────────────────────────────────────────────────────────
/// 一轮完整对话：用户消息入库 → 应答 → final/fallback 入库 → 三查。
///
/// 返回 (store, 收束事件 seq)；`reply = None` 模拟**无应答**（走兜底那一支）。
/// 应答是彩排内的定值——彩排验的是**链路形状**（收到 → 发出 → 收束 → 三查），
/// 规则语义本身归 [`classify`](../../../../docs/archive/06-会话响应性落地.md) 的规则表。
fn rehearse_turn(utterance: &str, reply: Option<&str>) -> (EventStore, Result<u64, u64>) {
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
    // 「思考 + 发出」：应答与兜底落**同一个** turn × closed 格子（收束语义不变）。
    let outcome = match reply {
        Some(reply) => {
            let s = store
                .append(
                    Event::pending(
                        "f0",
                        EVENT_ASSISTANT_FINAL,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "session/main",
                    )
                    .with_produced_by(seq.value())
                    .with_payload(serde_json::json!({ "text": reply })),
                )
                .expect("final 必入库");
            Ok(s.value())
        }
        None => {
            // 「兜底」：也是一条普通事件，落同一个 turn × closed 格子，同样带溯源。
            let s = store
                .append(
                    Event::pending(
                        "fb0",
                        EVENT_ASSISTANT_FALLBACK,
                        Entity::Turn,
                        Verb::Closed,
                        0,
                        "session/main",
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
    let (store, _) = rehearse_turn("你好，帮我看看", Some("你好，我能做什么？"));
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

/// 验收 2：强制无应答 → 必须存在 fallback，且它的 `produced_by` 非空。
#[test]
fn forced_failure_produces_fallback_with_provenance() {
    let (store, outcome) = rehearse_turn("这是规则表外的一句话", None);
    assert!(
        outcome.is_err(),
        "无应答 ⇒ 走兜底（兜底是触发条件，不是错误）"
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
    let (store, _) = rehearse_turn("你好", Some("你好，我能做什么？"));
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let p = turnstate();
    let v1 = p.apply(&snapshot, 1_000, Budget::generous());
    let v2 = p.apply(&snapshot, 1_000, Budget::generous());
    assert_eq!(v1, v2, "N1：同一事件序列 + 同一参数 ⇒ 逐字节相同");
    assert!(
        v1.value.opened && v1.value.settled(),
        "当前 turn 已开且已收束"
    );
    assert!(v1.value.final_text.is_some(), "有应答 ⇒ final 态");
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
                "session/main",
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
                "session/main",
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

// ── S2 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 5–6 步）：真实执行形状 ──
//
// 步骤 5：接入真实模型（pattern = reasoner）→ 真实对话 → N1/N3/N5 仍全绿；
// 步骤 6：兜底埋点 + 四层 budget_ms 就位 → 时延埋点可测。
// 这里用 ⑤ adapters 的确定性桩（StubLlmAdapter）代替真实模型——闭环形状与
// 真实接线完全一致，只换适配器。

use crate::symbio_core::adapters::{LatencyTier, LlmAdapter as _, StubLlmAdapter, TokenIssuer};
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
    let outcome = llm.generate(&tok, &render_messages(&snapshot)).await;
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
                        "session/main",
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
                    Event::pending("fb0", EVENT_ASSISTANT_FALLBACK, Entity::Turn, Verb::Closed, 0, "session/main")
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
    assert!(unresolved_turns(&snapshot, false).is_empty());
    // turnstate 投影照常工作（N1：双跑一致）。
    let p = turnstate();
    let v1 = p.apply(&snapshot, 0, Budget::generous());
    let v2 = p.apply(&snapshot, 0, Budget::generous());
    assert_eq!(v1, v2);
    assert!(v1.value.settled() && v1.value.final_text.is_some());
}

/// 输入契约：`Reasoner` **只看事件切片**——prompt 由事件渲染，
/// 桩回显 prompt ⇒ 回显里能看到事件里的那条用户消息（零 LLM 侧的同一契约见
/// `rehearse_turn`：应答是彩排内定值，形状一条不少）。
#[tokio::test]
async fn reasoner_reply_reads_the_user_message_from_events() {
    use crate::symbio_core::adapters::{StubLlmAdapter, TokenIssuer};

    let store = EventStore::new();
    store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "帮我看看这份计划" })),
        )
        .expect("用户消息必入库");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let llm = StubLlmAdapter::succeed("stub-model");
    let reply = llm
        .generate(&TokenIssuer::issue_deep(), &render_messages(&snapshot))
        .await
        .expect("成功桩必答");
    assert!(
        reply.contains("帮我看看这份计划"),
        "prompt 必须来自事件切片：{}",
        reply
    );
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
        unresolved_turns(&snapshot, false).is_empty(),
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
        !budget_exceeded(&snapshot, Some(LatencyTier::Reflex.budget_ms())).is_empty(),
        "反射档 {}/80ms 必须被看见超预算",
        cost
    );
    assert!(
        budget_exceeded(&snapshot, Some(LatencyTier::Deep.budget_ms())).is_empty(),
        "同一时延在深度档预算内"
    );
    // 反向锚：预算表口径（01 §10）在 adapters 域的单测里逐值钉死。
    assert_eq!(LatencyTier::Reflex.budget_ms(), 80);
}

// ── S5 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 11–13 步）：记忆链路 ──
//
// 编码（memory.encoded）→ 检索（recall 投影 + RecallTranslator → memory.recalled）
// → 巩固 / 遗忘（排除式 + 保真度下界）。S06 §6 四条验收逐条落地。

use crate::symbio_core::event::{EVENT_MEMORY_CALIBRATED, EVENT_MEMORY_RECALLED};
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
                "session/main",
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
    let p = recall("session/main", None, None);
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
                "session/main",
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
                "session/main",
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
    let p = recall("session/main", None, None);
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
                "session/b",
            )
            .with_produced_by(0)
            .with_ts(1_100)
            .with_payload(
                serde_json::json!({ "content": "B 的记忆", "tag": "semantic", "vec": [] }),
            ),
        )
        .unwrap();
    let events = || store.range(crate::symbio_core::event::Seq::new(0));
    let b_view = recall("session/b", None, None).apply(&events(), 9_999, Budget::generous());
    assert!(
        !b_view.value.contains_content("A 的私有记忆"),
        "跨主体默认不可见（C13）"
    );
    assert!(b_view.value.contains_content("B 的记忆"));
    let a_view = recall("session/main", None, None).apply(&events(), 9_999, Budget::generous());
    assert!(a_view.value.contains_content("A 的私有记忆"));
    assert!(!a_view.value.contains_content("B 的记忆"));
}

/// 验收 4（反向）：忽略 as-of 的投影实现 → 断言 1 必须失败（可捕获）。
#[test]
fn projection_that_ignores_as_of_would_be_caught() {
    let store = EventStore::new();
    encode(&store, "m0", 1_000, "旧记忆", "semantic", 0);
    let before_view = recall("session/main", None, None).apply(
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
                "session/main",
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
                "session/main",
            )
            .with_produced_by(0)
            .with_ts(5_000),
        )
        .unwrap();
    let events = store.range(crate::symbio_core::event::Seq::new(0));
    // 一个「忽略 as-of」的错误实现：now=MAX ⇒ 未来事件全部涌入。
    let broken_view = recall("session/main", None, None).apply(&events, i64::MAX, Budget::generous());
    let correct_view = recall("session/main", None, None).apply(&events, 2_000, Budget::generous());
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
                        "session/main",
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
            "session/main",
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
    let view = recall("session/main", None, None)
        .apply(&snapshot, 2_000, Budget::generous())
        .value;
    let event = RecallTranslator.recalled_event(&view, 0, "session/main");
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

/// 降级也要有**生产读数口**（[T-104](../../../../docs/009-task/104-检索降级可见性批.md "T-104-检索降级可见性批")）：
/// `degraded` 从 `memory.recalled` 的载荷读出——不只在投影外壳或提示词里活着。
/// 反向：不降级的检索不标降级（不谎称）。
#[test]
fn recalled_event_records_degradation_instead_of_dropping_it() {
    let store = EventStore::new();
    for i in 0..4i64 {
        encode(
            &store,
            &format!("g{i}"),
            100 + i,
            &format!("片段 {i}"),
            "experience",
            0,
        );
    }
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    let degraded = recall("session/main", None, None)
        .apply(&snapshot, 2_000, Budget::new(0, 1))
        .value;
    let event = RecallTranslator.recalled_event(&degraded, 0, "session/main");
    assert_eq!(
        event.payload.get("degraded").and_then(|v| v.as_bool()),
        Some(true),
        "部分结果的检索必须把 degraded 写进事实——读数口靠的是这条事件"
    );

    let full = recall("session/main", None, None)
        .apply(&snapshot, 2_000, Budget::generous())
        .value;
    let ok = RecallTranslator.recalled_event(&full, 0, "session/main");
    assert_eq!(
        ok.payload.get("degraded").and_then(|v| v.as_bool()),
        Some(false),
        "不降级不标降级"
    );
}

// ── S6 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 14–15 步）：多主体与对等承诺 ──
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
            "session/a",
        ))
        .unwrap();
    store
        .append(Event::pending(
            "b0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            0,
            "session/b",
        ))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let matrix = PermissionMatrix::new(vec![
        PrincipalPolicy::paired(
            "session/a",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "session/b",
            vec![Capability::ProduceArtifact],
            VisScope::ThreadPrivate,
        ),
    ])
    .expect("成对策略必过");

    // 验收 1：thread_private 下 A 只看见自己写的事件。
    let seen_by_a: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("session/a", &e.actor, VisScope::ThreadPrivate))
        .collect();
    assert_eq!(
        seen_by_a.len(),
        1,
        "thread_private：A 只见自己（S08 §6 验收 1）"
    );
    assert_eq!(seen_by_a[0].actor, "session/a");

    // 验收 4（反向）：内容可见域放宽为 public ⇒ 同一 viewer、同一切片全可见。
    let seen_by_a_wide: Vec<_> = snapshot
        .iter()
        .filter(|e| matrix.can_see("session/a", &e.actor, VisScope::Public))
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
        .append(keeper.offer("k1", "session/b", "session/a", "整理周报", 0))
        .unwrap();
    store.append(keeper.release("k1", "session/b", 0)).unwrap();
    store
        .append(keeper.offer("k2", "session/b", "session/a", "压测报告", 0))
        .unwrap();
    store
        .append(keeper.breach("k2", "session/b", "上游数据没到", 0))
        .unwrap();
    // 对等宣告：违约状态告知协作方（普通事件，不是新通道）。
    store
        .append(keeper.declare("k2", "session/b", "违约已告知 agent:a", 0))
        .unwrap();
    store
        .append(keeper.offer("k3", "session/a", "session/b", "评审", 0))
        .unwrap();
    store.append(keeper.release("k3", "session/a", 0)).unwrap();

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let view = reputation()
        .apply(&snapshot, 9_999, Budget::generous())
        .value;
    let b = view.of("session/b");
    assert_eq!((b.offered, b.kept, b.broken), (2, 1, 1));
    assert_eq!(b.score, 0, "平凡打分：守约 − 违约");
    assert_eq!(view.of("session/a").score, 1);
    // 未立约的主体：空条目，不是错误。
    assert_eq!(view.of("session/ghost").offered, 0);

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
        .append(keeper.offer("k1", "session/b", "session/a", "整理周报", 0))
        .unwrap();
    store
        .append(keeper.release("k1", "session/b", 0).with_ts(1_000))
        .unwrap();
    let before = {
        let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
        reputation()
            .apply(&snapshot, 2_000, Budget::generous())
            .value
    };
    assert_eq!(before.of("session/b").score, 1);

    // 后台追加违约（ts 在 as-of 之后）。
    store
        .append(
            keeper
                .offer("k2", "session/b", "session/a", "压测报告", 0)
                .with_ts(5_000),
        )
        .unwrap();
    store
        .append(
            keeper
                .breach("k2", "session/b", "上游数据没到", 0)
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
    assert_eq!(now.of("session/b").score, 0, "违约计入后声誉下降");
}

// ── S13 淘汰回路（[roadmap/S13 §6](../../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会") 断言 2/3/4，T-073）──

use crate::symbio_core::actors::EliminationDecider;
use crate::symbio_core::authz::PRINCIPAL_ARBITER;
use crate::symbio_core::event::{Entity as VEntity, EVENT_VERDICT_ASSERTED};
use crate::symbio_core::governance::{GrantsChange, GrantsOverlay};
use crate::symbio_core::projection::reputation::{plain_score, ReputationEntry};

/// S13 §6 断言 2 + 4（反向用例）：主体**自报**的声誉事件进不了投影。
///
/// 投影只认 `commitment` 实体的事件派生（`from` = 立约方），一条自称
/// 「我声誉 99」的事件（无论落在哪个格子上）对 `reputation()` 的输出毫无影响——
/// 「自己报个好分数免于淘汰」在构造上不成立，不是靠纪律拦住。
#[test]
fn self_reported_reputation_never_enters_the_projection() {
    let store = EventStore::new();
    // 真实事实：一次违约 ⇒ 净分 −1。
    store
        .append(
            Event::pending(
                "k1",
                crate::symbio_core::event::EVENT_COMMITMENT_OFFERED,
                Entity::Commitment,
                Verb::Opened,
                0,
                "session/b",
            )
            .with_payload(serde_json::json!({ "id": "k1", "from": "session/b", "to": "session/a", "promise": "x" })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "k1c",
                crate::symbio_core::event::EVENT_COMMITMENT_BROKEN,
                Entity::Commitment,
                Verb::Closed,
                0,
                "session/b",
            )
            .with_payload(serde_json::json!({ "id": "k1", "why": "没交" })),
        )
        .unwrap();
    // 自报：换个格子、换个实体都没用——投影的输入闭集是 commitment 事件。
    store
        .append(
            Event::pending(
                "self1",
                "reputation.reported",
                Entity::Verdict,
                Verb::Asserted,
                0,
                "session/b",
            )
            .with_payload(serde_json::json!({ "principal": "session/b", "score": 99 })),
        )
        .unwrap();
    store
        .append(
            Event::pending(
                "self2",
                crate::symbio_core::event::EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                0,
                "session/b",
            )
            .with_payload(serde_json::json!({ "content": "我声誉 99", "tag": "声誉" })),
        )
        .unwrap();

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let view = reputation()
        .apply(&snapshot, 9_999, Budget::generous())
        .value;
    assert_eq!(
        view.of("session/b").score,
        -1,
        "只认派生：自报的 99 分一丝不进投影（断言 2/4）"
    );
}

/// 仲裁者行（T-073 exit ① 的表半边）：`spec()` 五字段全部**取实**——
/// principal / capabilities 同源于授权表窄行，pattern 按 `for_principal` 派生。
///
/// 三新角色（市场 / 仲裁 / 守门）都派生 `Decider`：模式闭集**不变宽**
/// （exit ① 的另一半——加角色行不许顺手加模式）。
#[test]
fn the_arbiter_row_is_the_documented_five_fields() {
    let spec = EliminationDecider.spec();
    assert_eq!(spec.principal, PRINCIPAL_ARBITER);
    assert_eq!(
        spec.pattern,
        crate::symbio_core::actors::Pattern::Decider,
        "仲裁者无 reply.* 也无 produce.artifact ⇒ Decider（闭集不变宽）"
    );
    assert_eq!(
        spec.capabilities,
        vec!["assert.verification".to_string()],
        "能力从授权表那一行取数，不在这里硬写"
    );
    assert_eq!(
        spec.budget_ms,
        crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
        "淘汰判定是规则折叠：反射档"
    );
    assert_eq!(spec.scope, crate::symbio_core::actors::Scope::Root);
}

/// 判定半边：低于阈值才淘汰；**恰好等于阈值不淘汰**（边界值）。
///
/// 平凡打分 = 守约 − 违约、阈值 0 ⇒ 任何未配平的违约即淘汰；从没立约的主体
/// （空条目 score 0）**不**被淘汰——没承诺过就没有可违约的东西。
#[test]
fn should_eliminate_flips_only_below_the_threshold() {
    let entry = |score: i64| ReputationEntry {
        principal: "session/b".to_string(),
        offered: 3,
        kept: 1,
        broken: 2,
        score,
    };
    let d = EliminationDecider;
    assert!(d.should_eliminate(&entry(-1)), "净违约 ⇒ 淘汰");
    assert!(!d.should_eliminate(&entry(0)), "恰好抵平 ⇒ 不淘汰（边界）");
    assert!(!d.should_eliminate(&entry(1)), "净守约 ⇒ 不淘汰");
    let fresh = ReputationEntry {
        principal: "session/ghost".to_string(),
        offered: 0,
        kept: 0,
        broken: 0,
        score: plain_score(0, 0, 0),
    };
    assert!(
        !d.should_eliminate(&fresh),
        "从没立约的主体（空条目）不被淘汰"
    );
}

/// S13 §6 断言 3：淘汰 = 一条**可审计、可撤销**的 grants 变更事件。
///
/// 三条判据各钉一边：① 事件落在 `verdict × asserted` 格、actor 是仲裁者
/// （不是被淘汰主体——「谁做的裁决」在事实源里说得出来）；② 载荷六字段齐全
/// （score / threshold 让审计方能复算）；③ `verdict_grants_changes` 把事件流水
/// 折成覆盖层后，`is_revoked` 翻真、恢复事件再翻假——回路闭合。
#[test]
fn elimination_is_an_auditable_revocable_grants_change() {
    let d = EliminationDecider;
    let ev = d.elimination_event("session/b", -1, 42);
    // ① 格子与作者。
    assert_eq!(ev.kind, EVENT_VERDICT_ASSERTED);
    assert_eq!(ev.entity, VEntity::Verdict);
    assert_eq!(ev.verb, Verb::Asserted);
    assert_eq!(
        ev.actor, PRINCIPAL_ARBITER,
        "裁决者是仲裁者，不是被淘汰主体"
    );
    assert_eq!(ev.produced_by, Some(42), "I2：断言类事件必须带溯源");
    // ② 载荷六字段。
    let p = &ev.payload;
    assert_eq!(p["subject"], "session/b");
    assert_eq!(p["capability"], "assign.work");
    assert_eq!(p["change"], "revoke");
    assert_eq!(p["score"], -1);
    assert_eq!(p["threshold"], EliminationDecider::THRESHOLD);
    assert!(p["why"].as_str().is_some_and(|w| !w.is_empty()), "人话理由");

    // ③ 流水 → 覆盖层：撤销生效、恢复可逆、再收回又生效（last-wins）。
    let restored = d.restoration_event("session/b", "申诉受理：上游数据延迟有凭证", 77);
    assert_eq!(restored.kind, EVENT_VERDICT_ASSERTED);
    assert_eq!(restored.payload["change"], "restore");
    let changes = EliminationDecider::verdict_grants_changes(&[ev.clone(), restored.clone()]);
    assert_eq!(
        changes,
        vec![
            GrantsChange::Revoke {
                principal: "session/b".to_string(),
                capability: "assign.work".to_string()
            },
            GrantsChange::Restore {
                principal: "session/b".to_string(),
                capability: "assign.work".to_string()
            },
        ]
    );
    assert!(
        !GrantsOverlay::fold(changes).is_revoked("session/b", "assign.work"),
        "申诉成立 ⇒ 恢复（可撤销）"
    );
    assert!(
        GrantsOverlay::fold(EliminationDecider::verdict_grants_changes(&[
            ev.clone(),
            restored,
            ev,
        ]))
        .is_revoked("session/b", "assign.work"),
        "恢复之后再违约 ⇒ 再淘汰（last-wins）"
    );

    // 非 verdict 事件与认不出的 change 值：跳过，不猜。
    let other = Event::pending(
        "x",
        "task.progress",
        Entity::Task,
        Verb::Progressed,
        0,
        "session/b",
    );
    let bogus = Event::pending(
        "y",
        EVENT_VERDICT_ASSERTED,
        VEntity::Verdict,
        Verb::Asserted,
        0,
        PRINCIPAL_ARBITER,
    )
    .with_payload(serde_json::json!({ "subject": "session/b", "change": "revokee" }));
    assert!(
        EliminationDecider::verdict_grants_changes(&[other, bogus]).is_empty(),
        "别的格子 / 笔误的 change ⇒ 一条都不译（不把笔误读成改判）"
    );
}

/// 淘汰事件也是普通事件：入格后全链路不变量照旧绿（逃不出审计）。
#[test]
fn elimination_event_keeps_invariants_green() {
    let store = EventStore::new();
    store
        .append(EliminationDecider.elimination_event("session/b", -1, 0))
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
}

// ── S7 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 16–18 步）：任务树与返工 ──
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
    open_task(&store, "a", &["c"], "session/main");
    open_task(&store, "b", &["a"], "session/main");
    open_task(&store, "c", &["b"], "session/main");
    let bad = acyclic_deps(&store.range(crate::symbio_core::event::Seq::new(0)));
    assert_eq!(bad.len(), 3, "环上三个节点都要被点名：{bad:?}");
    assert!(bad[0].why.contains("依赖环"), "{}", bad[0].why);
}

/// S03 §6 验收 4（反向）：把环去掉一个依赖 → 断言 1 必须通过（检测在算，不是常量 false）。
#[test]
fn breaking_one_edge_makes_graph_pass() {
    let store = EventStore::new();
    // 同一张图，只去掉 c→b 这条边。
    open_task(&store, "a", &["c"], "session/main");
    open_task(&store, "b", &["a"], "session/main");
    open_task(&store, "c", &[], "session/main");
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
    open_task(&store, "a", &["ghost"], "session/main");
    let bad = acyclic_deps(&store.range(crate::symbio_core::event::Seq::new(0)));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].why.contains("悬空"), "{}", bad[0].why);
}

/// S03 §6 验收 2（C15）：同一主体同时持 define.work 与 assert.verification → 拒绝。
#[test]
fn self_verifier_is_rejected_at_construction() {
    let both = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "session/greedy",
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
            "session/planner",
            vec![Capability::DefineWork],
            VisScope::ThreadPrivate,
        ),
        PrincipalPolicy::paired(
            "session/verifier",
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
    open_task(&store, "a", &[], "session/main");
    open_task(&store, "b", &["a"], "session/main");
    open_task(&store, "c", &["b"], "session/main");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    let rs = readyset().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(rs.value.ready.len(), 1, "只有 a 就绪：{:?}", rs.value.ready);
    assert_eq!(rs.value.ready[0].task_id, "a");

    // a 终态 → 就绪集挪到 b；链式推进，调度器零内部状态。
    assert_task(&store, "a", "session/verifier");
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
    open_task(&store, "a", &[], "session/planner");
    open_task(&store, "b", &["a"], "session/planner");
    // 验证者判 a 不合格 → 返工（新增一条事件，append-only）。
    store
        .append(
            Event::pending(
                "rw-a1",
                EVENT_TASK_REWORK_CREATED,
                Entity::Task,
                Verb::Asserted,
                0,
                "session/verifier",
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": "a-r1", "replaces": "a", "round": 1 })),
        )
        .unwrap();
    open_task(&store, "a-r1", &[], "session/planner");

    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let rs = readyset().apply(&snapshot, 9_999, Budget::generous()).value;
    let ids: Vec<_> = rs.ready.iter().map(|t| t.task_id.as_str()).collect();
    assert!(ids.contains(&"a-r1"), "返工节点就绪：{ids:?}");
    assert!(
        ids.contains(&"a"),
        "旧节点仍未终态（Log 永不改——被取代靠新节点表达）"
    );

    // 返工节点过验 → b 的依赖由终态的 a-r1 之外仍卡住（a 未终态）→ b 不就绪。
    assert_task(&store, "a-r1", "session/verifier");
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
                "session/verifier",
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
                "session/verifier",
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
            "session/planner",
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
                    "session/main",
                )
                .with_payload(serde_json::json!({ "task_id": format!("t{i}") })),
            )
            .unwrap();
        assert_task(&store, &format!("t{i}"), "session/verifier");
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

// ── S8 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 19–20 步）：时延与插话 ──
//
// 19 抢占判定者（80ms 反射档）+ 挂起/恢复（出口判据「二次调度 0」，roadmap/S07）；
// 20 熔断（外部执行闸门，出口判据「熔断后仍执行 0」，roadmap/S09）。

use crate::symbio_core::event::EVENT_CONTROL_OPENED;
use crate::symbio_core::projection::cost::cost_ledger;

/// 造一个在跑任务（供插话/熔断场景使用）。
fn running_task(store: &EventStore, id: &str) {
    open_task(store, id, &[], "session/main");
}

/// S07 §6 验收 1：注入打断事件 → 80ms 内必须产出处置（Suspend ⇒ task.controlled 落库）。
#[test]
fn interruption_within_reflex_budget_produces_control_event() {
    let store = EventStore::new();
    running_task(&store, "t1");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let d = PreemptionDecider.decide(&PreemptionDecider.spec(), &snapshot, 10);
    match d {
        Preemption::Suspend { task_id, as_of_seq } => {
            assert_eq!(task_id, "t1");
            // 挂起就是一条事件 + 一条控制事实（判定者只产控制事件，不发言）。
            store
                .append(PreemptionDecider.held_event(&task_id, as_of_seq, as_of_seq))
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
    // 判定耗时 100ms：判定者行（反射档，`spec().budget_ms` = 80）超时 → 默认继续（不挂起）。
    assert_eq!(
        PreemptionDecider.decide(&PreemptionDecider.spec(), &snapshot, 100),
        Preemption::TimeoutDefaultContinue
    );
    // 同一份判据数据，只把**行上的预算**放宽到 60000 → 实质判定成立（挂起）。
    // 断言「`decide` 读的是规格自己的 budget_ms」——不是某个调用方现算的数值。
    let mut generous = PreemptionDecider.spec();
    generous.budget_ms = 60_000;
    assert_eq!(
        PreemptionDecider.decide(&generous, &snapshot, 100),
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
                "session/main",
            )
            .with_produced_by(0),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    assert_eq!(
        PreemptionDecider.decide(&PreemptionDecider.spec(), &snapshot, 10),
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
        .append(PreemptionDecider.held_event("t1", t, 0).with_ts(5_000))
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
                "session/main",
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
///
/// 判据走**生产表**（`matrix_for`），不是自造矩阵：自造一份 `PrincipalPolicy::paired`
/// 只证明「我拒我」——表里少一行 `agent:reflex`，它照样绿，而生产里 `matrix_for` 的
/// 派生分支早把主智能体整套（含 `reply.*`）给了判定者（T-132 §3.1 批评的正是这个形态）。
#[test]
fn preemption_decider_cannot_speak() {
    use crate::symbio_core::authz::{matrix_for, PRINCIPAL_REFLEX};

    let matrix = matrix_for(PRINCIPAL_REFLEX);
    assert!(matrix.can_write(PRINCIPAL_REFLEX, Capability::JudgeIntent));
    assert!(
        !matrix.can_write(PRINCIPAL_REFLEX, Capability::ReplyFirst),
        "无首响写权"
    );
    assert!(
        !matrix.can_write(PRINCIPAL_REFLEX, Capability::ReplyAppend),
        "无追加写权"
    );

    // 反向判据：**未登记**的同前缀主体仍派生主智能体整套——两边的差别正是那一行。
    // 若哪天有人把这一行删掉（或改宽），上面两条断言与这条对照一起变红。
    let unregistered = matrix_for("session/unregistered");
    assert!(
        unregistered.can_write("session/unregistered", Capability::ReplyFirst),
        "未登记主体派生主智能体那一行（身份分层 ≠ 权限分层，S08 §4）——判定者的窄是**登记**带来的"
    );
}

/// T-082 批②：S07 判定者**行**（`PreemptionDecider::spec`）——五字段取实 + 能力读方。
///
/// 判据分两半：① 行的形状与 S07 §3 表逐字相符（五字段全部取实、非平凡值）；
/// ② **能力读方**——`can_judge_intent` 读规格**自己身上**的能力集，含反向用例
/// （清空能力集 ⇒ 写权拒；判定若绕回授权表则这条绿）。
#[test]
fn preemption_decider_spec_carries_its_row_and_gates_on_judge_intent() {
    use crate::symbio_core::authz::PRINCIPAL_REFLEX;

    let spec = PreemptionDecider.spec();
    assert_eq!(
        spec.principal, PRINCIPAL_REFLEX,
        "主体名 = 授权表那一行的键"
    );
    assert_eq!(spec.pattern, Pattern::Decider, "规则判定，非模型（S07 §3）");
    assert_eq!(
        spec.capabilities,
        vec!["judge.intent".to_string()],
        "能力取授权表**窄行**（T-081），不是主智能体派生整套"
    );
    assert_eq!(
        spec.budget_ms,
        crate::symbio_core::adapters::LatencyTier::Reflex.budget_ms(),
        "预算 = 反射档 80ms"
    );
    assert!(
        matches!(spec.scope, Scope::Root),
        "会话驱动的判定都是根作用域"
    );

    // 能力读方：判定者持 judge.intent ⇒ 可落控制事实（`task.held` / `control`）。
    assert!(
        spec.can_judge_intent(),
        "判定者持 judge.intent ⇒ 写侧闸放行"
    );
    // 反向用例：清空能力集 ⇒ 写权拒。判定若绕回授权表，这条会绿（假绿）——它必须红。
    let mut stripped = PreemptionDecider.spec();
    stripped.capabilities.clear();
    assert!(
        !stripped.can_judge_intent(),
        "无 judge.intent ⇒ 不得落控制事实（读的是规格自己的集合）"
    );
}

/// S09 §6 验收 1：未持外部执行能力的主体 → 拒绝且**不产生事件**。
#[test]
fn unauthorized_external_write_is_refused_without_event() {
    let matrix = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "session/talker",
        vec![Capability::ReplyAppend],
        VisScope::ThreadPrivate,
    )])
    .unwrap();
    let gate = CircuitBreaker;
    let authorized = matrix.can_write("session/talker", Capability::ProduceArtifact);
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
        "session/doer",
        vec![Capability::ProduceArtifact],
        VisScope::ThreadPrivate,
    )])
    .unwrap();
    let authorized = matrix.can_write("session/doer", Capability::ProduceArtifact);
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
        "session/risky",
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
                "session/a",
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
                "session/a",
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
                "session/b",
            )
            .with_cost_ms(50)
            .with_payload(serde_json::json!({ "task_id": "t1" })),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));
    let ledger = cost_ledger().apply(&snapshot, 9_999, Budget::generous());
    assert_eq!(ledger.value.of("session/a").spent_ms, 500);
    assert_eq!(ledger.value.of("session/a").entries, 2);
    assert_eq!(ledger.value.total_ms, 550);
    // N1 双跑一致。
    assert_eq!(
        ledger.value,
        cost_ledger()
            .apply(&snapshot, 9_999, Budget::generous())
            .value
    );
    // 未记账主体零账。
    assert_eq!(ledger.value.of("session/ghost"), Default::default());
}

// ── S9 彩排（[plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 21–22 步）：自治与学习 ──
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
///
/// 判据查的是**生产授权表**（[`authz::matrix_for`](crate::symbio_core::authz::matrix_for)），
/// 不是自造矩阵——自造矩阵只证明「一个我自己写的策略拒绝了我自己」，生产表改了它照样绿。
/// 三条断言缺一不可：
/// ① 自主发起者**不持** `reply.*`（验收 2 的结论）；
/// ② 它**持** `define.work`（否则 ① 可能只是「矩阵整个是空的」的平凡真）；
/// ③ 主智能体**持** `reply.first`（对照：拒绝是**针对这个主体**的，不是一律拒绝）。
///
/// 「拒绝 ⇒ 不产生事件」的**执行**在写侧闸（`plugins/session` 运行器的
/// `turn_runner::closure_granted`，其判据用例见
/// `turn_runner.test.rs::closure_is_withheld_when_the_principal_lacks_the_grant`）
/// ——core 这边只钉**表里的结论**，不假装自己跑过那条闸。
#[test]
fn autonomous_actor_cannot_write_to_dialog() {
    use crate::symbio_core::authz::{matrix_for, PRINCIPAL_AUTONOMOUS, PRINCIPAL_MAIN};

    let initiator = matrix_for(PRINCIPAL_AUTONOMOUS);
    assert!(
        !initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::ReplyFirst),
        "自主层无 reply.first 写权（自主写入对话 = 0）"
    );
    assert!(
        !initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::ReplyAppend),
        "续写同样不持——自主行为不得冒充用户对话"
    );
    assert!(
        initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::DefineWork),
        "自主层必须能定义工作（否则上一条只是『空矩阵』的平凡真）"
    );
    let main = matrix_for(PRINCIPAL_MAIN);
    assert!(
        main.can_write(PRINCIPAL_MAIN, Capability::ReplyFirst),
        "对照：主智能体持 reply.first——拒绝是针对主体的，不是矩阵一律拒绝"
    );
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
                crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS,
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
        Some(crate::symbio_core::adapters::LatencyTier::Autonomic.budget_ms()),
    );
    assert_eq!(bad.len(), 1, "长目标超支必须被看见：{bad:?}");
    // 验收 4（反向）：同一事件 70M ms（预算内）；把预算改成 60000 → 判定改变。
    let ok_event = Event::pending(
        "g2",
        EVENT_TASK_PROGRESS,
        Entity::Task,
        Verb::Progressed,
        0,
        crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS,
    )
    .with_produced_by(0)
    .with_cost_ms(70_000)
    .with_payload(serde_json::json!({ "task_id": "goal-1" }));
    let ok_events = vec![ok_event];
    assert!(
        budget_exceeded(&ok_events, Some(86_400_000)).is_empty(),
        "70M 在自主层预算内"
    );
    assert_eq!(
        budget_exceeded(&ok_events, Some(60_000)).len(),
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
            "session/x",
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
            Event::pending(
                "h1",
                EVENT_MEMORY_CALIBRATED,
                Entity::Memory,
                Verb::Asserted,
                0,
                "session/main",
            )
            .with_produced_by(0)
            .with_cost_ms(40)
            .with_payload(serde_json::json!({ "skill_id": "weekly-report", "fallback": false })),
        )
        .unwrap();
    // 未命中 / 回退：完整推理（cost 5000ms）。
    store
        .append(
            Event::pending(
                "h2",
                EVENT_MEMORY_CALIBRATED,
                Entity::Memory,
                Verb::Asserted,
                0,
                "session/main",
            )
            .with_produced_by(0)
            .with_cost_ms(5_000)
            .with_payload(serde_json::json!({ "skill_id": "weekly-report", "fallback": true })),
        )
        .unwrap();
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    // 验收 2：命中路径 cost 显著低于回退路径（40ms vs 5000ms）。
    let hit_cost = snapshot
        .iter()
        .find(|e| {
            e.kind == EVENT_MEMORY_CALIBRATED
                && e.payload.get("fallback") == Some(&serde_json::json!(false))
        })
        .map(|e| e.cost_ms)
        .unwrap();
    let fallback_cost = snapshot
        .iter()
        .find(|e| {
            e.kind == EVENT_MEMORY_CALIBRATED
                && e.payload.get("fallback") == Some(&serde_json::json!(true))
        })
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
                    "session/main",
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
                    "session/main",
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
                "session/main",
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
