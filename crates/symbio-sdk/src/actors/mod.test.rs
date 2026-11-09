//! `actors` 域的用例（随实现一起从旧 core 迁入，DE-023 D-276 / D-279）。
//!
//! 覆盖四块：`ActorSpec` 的契约形状、收束闸判读规格自己的能力集、多主体隔离与
//! 对等承诺的立约→守约→声誉链路，以及**闭环彩排**。
//!
//! 闭环彩排原先留在旧 core，理由是「它验的是轮次地基，不是主体策略」。D-279 复核
//! 后改判：彩排用的每一件东西（`store` / `projection` / `invariants` / `test_fixtures`）
//! 都在 `symbio-kernel` 与 `symbio-sdk` 里，它当初住在 core **只是因为它用 `crate::` 前缀
//! 写路径，而 core 的 21 个转发桩让那些路径解析得通**。桩删掉之后，这些用例的真实住址
//! 就是这里。

use symbio_kernel::actors::{ActorSpec, Pattern, Scope};
use symbio_kernel::event::{Entity, Event, EventDecl, FactRole, Verb};
use symbio_kernel::store::{EventStore, Store};

use crate::actors::*;
use crate::governance::{Capability, PermissionMatrix, PrincipalPolicy, VisScope};
use crate::invariants::check_all;
use crate::test_fixtures::{
    CHAT, COMMITMENT, EVENTS, EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL,
    EVENT_USER_MESSAGE, EVENT_VERDICT_ASSERTED, MESSAGE,
};

// ── ActorSpec / Pattern（DE-001 §4 契约形状） ──────────────────────────────

#[test]
fn trivial_spec_is_the_s01_baseline() {
    let spec = trivial_actor_spec("session/main");
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

// ── 授权读法：规格判定与表谓词必须一份口径 ─────────────────────

/// 规格判定与表谓词**必须一致**：同一主体的两个入口，一份口径。
///
/// 挡的是「口径分叉」：能力名（`Capability::AssignWork`）若在插件侧再抄一份表，
/// 改一处即静默错位，而错位方向是**放行**（fail-open）。四个主体覆盖
/// 登记行（会话 / 对等 / 抢占判定者）与未登记的 `session/<id>`（表里没有行 ⇒ 全拒）。
#[test]
fn spec_judgment_agrees_with_matrix_predicate_for_every_principal_shape() {
    use crate::authz::{production_matrix, PRINCIPAL_MAIN, PRINCIPAL_PEER, PRINCIPAL_REFLEX};
    for p in [
        PRINCIPAL_MAIN,
        PRINCIPAL_PEER,
        PRINCIPAL_REFLEX,
        "session/unregistered",
    ] {
        let matrix = production_matrix();
        let spec = ActorSpec {
            capabilities: matrix.grant_names_of(p),
            ..trivial_actor_spec(p)
        };
        assert_eq!(
            actor_can_assign_work(&spec),
            matrix.can_write_name(p, Capability::AssignWork.name()),
            "{p}: 规格判定与表谓词两个入口必须一致"
        );
    }
}

/// 事件切片 → 请求包消息：转写条目按 `exclude_from_context` 过滤后投影。
///
/// 旧 `TranscriptView::to_messages` 的唯一读法（出口已删）；这里按同一条折一遍，
/// 让「prompt 来自事实切片」这条断言继续走真实的转写投影。
fn request_messages(events: &[Event]) -> Vec<crate::projection::transcript::PromptMessage> {
    crate::projection::transcript::transcript(CHAT, MESSAGE)
        .read_all(events)
        .value
        .entries
        .iter()
        .filter(|e| !e.exclude_from_context)
        .map(crate::projection::transcript::PromptMessage::from)
        .collect()
}

// ── 彩排：多主体与对等承诺 ──
//
// principal 隔离（下方验收 1/4）+ 无直连静态扫描（scripts/no-direct-call-audit.mjs，
// 出口判据「直连调用 0」；成对性拒绝 = DE-038 §6 验收 3，已在 governance 测试区覆盖）。
// commitment 立约成格与配对判在这里；声誉台账的读数与「双跑一致」随折叠归它的 owner 包
// （`crates/symbio-plugin-reputation/src/decl.test.rs`，DE-045 D-288），本区只留淘汰裁决那一侧。

/// DE-038 §6 验收 1 + 4：thread_private 互不可见；内容可见域放宽成 public 后，
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
            "session/a",
        ))
        .unwrap();
    store
        .append(Event::pending(
            "b0",
            "task.progress",
            Entity::Task,
            Verb::Progressed,
            "session/b",
        ))
        .unwrap();
    let snapshot = store.range(crate::event::Seq::new(0));
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
        "thread_private：A 只见自己（DE-038 §6 验收 1）"
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

/// 读侧翻译（`commitment_records`）：
/// 一次立约读出**一条记录**，状态、原因、锚都在同一条上。
///
/// ## 这个用例挡的是什么
///
/// 出口表里「承诺违约率」那一格要指得到一列读数，而读数的形状由本函数定。三件事
/// 各钉一条：① `asserted`（违约时告知协作方的第三格）**不另计一条约**——按格子数
/// 折算的写法在这里就会多算；② 只有 `opened` 的在途约是 `open`，不是违约；
/// ③ 找不到立约的了结事件**跳过**（fail-closed，与 `verdict_grants_changes` 同一条）。
/// 生产写侧只走 `commitment_events`，故夹具也用它——夹具与生产共用同一个成格口，
/// 改名时两边一起红。
#[test]
fn commitment_records_pair_by_payload_id() {
    let store = EventStore::new();
    // 守约一条、违约一条、在途一条（id 故意乱序写入，读出必须按 id 字典序）。
    for e in commitment_events(
        COMMITMENT,
        "v2c-t2-bbb",
        "session/b",
        "agent:reviewer",
        "复查这段",
        Ok(()),
        7,
    ) {
        store.append(e).unwrap();
    }
    for e in commitment_events(
        COMMITMENT,
        "v2c-t1-aaa",
        "session/b",
        "agent:worker",
        "把周报整理完",
        Err("子智能体没起来"),
        4,
    ) {
        store.append(e).unwrap();
    }
    store
        .append(CommitmentKeeper::new(COMMITMENT).offer(
            "v2c-t3-ccc",
            "session/a",
            "agent:peer",
            "评审设计",
            9,
        ))
        .unwrap();
    // 找不到立约的了结 ⇒ 不猜（这一条不进记录）。
    store
        .append(
            Event::pending(
                "ghost-close",
                crate::test_fixtures::EVENT_COMMITMENT_BROKEN,
                Entity::Commitment,
                Verb::Closed,
                "session/x",
            )
            .with_payload(serde_json::json!({ "id": "never-offered", "why": "幽灵" })),
        )
        .unwrap();

    let recs = commitment_records(&store.range(crate::event::Seq::new(0)), &COMMITMENT);
    assert_eq!(
        recs.iter()
            .map(|r| (r.id.as_str(), r.state))
            .collect::<Vec<_>>(),
        vec![
            ("v2c-t1-aaa", "broken"),
            ("v2c-t2-bbb", "kept"),
            ("v2c-t3-ccc", "open"),
        ],
        "一条约一条记录，`asserted` 不另计；在途的是 open 不是违约"
    );
    assert_eq!(
        recs[0].why, "子智能体没起来",
        "违约原因跟着记录走（DE-038 §5）"
    );
    assert_eq!(recs[0].promise, "把周报整理完");
    assert_eq!(recs[0].anchor_seq, 4, "锚 = 立约那一轮开口的 seq（I2）");
    assert_eq!(recs[2].anchor_seq, 9, "在途那条也带着自己的锚");
    assert_eq!(recs[1].why, "", "守约的记录不该带原因");
}

// ── 闭环彩排 ────────────────────────────────────────────────────────
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
                Entity::Scope,
                Verb::Opened,
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
                        EVENT_CHAT_ASSISTANT_FINAL,
                        Entity::Scope,
                        Verb::Closed,
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
                        EVENT_CHAT_ASSISTANT_FALLBACK,
                        Entity::Scope,
                        Verb::Closed,
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
        .range(crate::event::Seq::new(0))
        .into_iter()
        .filter(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
        .collect();
    assert_eq!(finals.len(), 1, "发出端点：发言只有一条 final");
    // 三条不变量在整条链路上全绿（含 I2：final 带溯源）。
    let snapshot = store.range(crate::event::Seq::new(0));
    assert!(
        check_all(EVENTS, &snapshot).is_empty(),
        "{:?}",
        check_all(EVENTS, &snapshot)
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
    let snapshot = store.range(crate::event::Seq::new(0));
    let fallbacks: Vec<_> = snapshot
        .iter()
        .filter(|e| e.kind == EVENT_CHAT_ASSISTANT_FALLBACK)
        .collect();
    assert_eq!(fallbacks.len(), 1, "兜底必须产生事件——静默中断 = 违例");
    assert!(fallbacks[0].produced_by.is_some(), "兜底同样可被 I2 审计");
    assert!(
        check_all(EVENTS, &snapshot).is_empty(),
        "{:?}",
        check_all(EVENTS, &snapshot)
    );
}

// ── 彩排：真实执行形状 ──
//
// 接入真实模型（pattern = reasoner）→ 真实对话 → N1/N5 仍全绿；
// 兜底埋点 + 四层 budget_ms 就位 → 时延埋点可测。
// 这里用 ⑤ adapters 的确定性桩（StubLlmAdapter）代替真实模型——闭环形状与
// 真实接线完全一致，只换适配器。

use crate::adapters::{AdapterError, FullModel, LatencyTier, LlmAdapter, TokenIssuer};
use crate::projection::transcript::PromptMessage;

/// 零 LLM 桩的**本文件版**。
///
/// `StubLlmAdapter` 住 `symbio-testkit`（见该 crate 的 `stubs` 模块文档），而 core
/// **不能**反向依赖测试设施——那会成环。故这里留一份只覆盖本文件三条演练路径的
/// 最小实现：成功 / 确定性失败 / 注入时延。
///
/// 桩自身的判据（确定性）在 `symbio-testkit/src/stubs.test.rs` 守着；本文件只要求
/// **行为与那份一致**，不重复断言。
struct MinimalStub {
    model: &'static str,
    fail_with: Option<&'static str>,
    delay_ms: u64,
}

impl MinimalStub {
    fn succeed(model: &'static str) -> Self {
        Self {
            model,
            fail_with: None,
            delay_ms: 0,
        }
    }
    fn always_fail(message: &'static str) -> Self {
        Self {
            model: "stub",
            fail_with: Some(message),
            delay_ms: 0,
        }
    }
    fn with_delay(model: &'static str, delay_ms: u64) -> Self {
        Self {
            model,
            fail_with: None,
            delay_ms,
        }
    }
}

#[async_trait::async_trait]
impl LlmAdapter for MinimalStub {
    fn model_id(&self) -> &str {
        self.model
    }
    async fn generate(
        &self,
        _tok: &FullModel,
        messages: &[PromptMessage],
    ) -> Result<String, AdapterError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        match self.fail_with {
            Some(msg) => Err(AdapterError::GenerationFailed(msg.to_string())),
            // 回显末条正文：与 `StubLlmAdapter` 同口径（旧签名是单段文本）。
            None => Ok(format!(
                "[{}] {}",
                self.model,
                messages.last().map(|m| m.text.as_str()).unwrap_or("")
            )),
        }
    }
}
use crate::invariants::budget_exceeded;
use std::time::Instant;

/// 一轮**真实执行形状**的对话：Decider 换成 Reasoner + 适配器，带 cost_ms 埋点。
///
/// 返回 (store, 收束事件 id)；失败路径产出 fallback（I3 到点必答）。
async fn rehearse_reasoner_turn(
    utterance: &str,
    llm: &dyn crate::adapters::LlmAdapter,
) -> (EventStore, Result<String, String>) {
    let tok = TokenIssuer::issue_deep();
    let store = EventStore::new();
    let seq = store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Scope,
                Verb::Opened,
                "user",
            )
            .with_payload(serde_json::json!({ "text": utterance })),
        )
        .expect("用户消息必入库");
    let snapshot = store.range(crate::event::Seq::new(0));
    let started = Instant::now();
    let outcome = llm
        .generate(
            &tok,
            &request_messages(&snapshot),
        )
        .await;
    let cost_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(reply) => {
            store
                .append(
                    Event::pending(
                        "f0",
                        EVENT_CHAT_ASSISTANT_FINAL,
                        Entity::Scope,
                        Verb::Closed,
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
                    Event::pending("fb0", EVENT_CHAT_ASSISTANT_FALLBACK, Entity::Scope, Verb::Closed, "session/main")
                        .with_produced_by(seq.value())
                        .with_cost_ms(cost_ms)
                        .with_payload(serde_json::json!({ "text": "抱歉，我暂时答不上来。", "error": format!("{e:?}") })),
                )
                .expect("fallback 必入库");
            (store, Err(format!("{e:?}")))
        }
    }
}

/// 验收：接入「真实模型」→ 真实对话 → N1/N5 仍全绿。
#[tokio::test]
async fn reasoner_turn_keeps_all_invariants_green() {
    let llm = MinimalStub::succeed("stub-model");
    let (store, outcome) = rehearse_reasoner_turn("写一首关于秋天的诗", &llm).await;
    let reply = outcome.expect("成功桩必答");
    assert!(reply.contains("stub-model"));
    let snapshot = store.range(crate::event::Seq::new(0));
    assert!(
        check_all(EVENTS, &snapshot).is_empty(),
        "{:?}",
        check_all(EVENTS, &snapshot)
    );
}

/// 输入契约：`Reasoner` **只看事件切片**——prompt 由事件渲染，
/// 桩回显 prompt ⇒ 回显里能看到事件里的那条用户消息（零 LLM 侧的同一契约见
/// `rehearse_turn`：应答是彩排内定值，形状一条不少）。
#[tokio::test]
async fn reasoner_reply_reads_the_user_message_from_events() {
    use crate::adapters::TokenIssuer;

    let store = EventStore::new();
    store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Scope,
                Verb::Opened,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "帮我看看这份计划" })),
        )
        .expect("用户消息必入库");
    let snapshot = store.range(crate::event::Seq::new(0));
    let llm = MinimalStub::succeed("stub-model");
    let reply = llm
        .generate(
            &TokenIssuer::issue_deep(),
            &request_messages(&snapshot),
        )
        .await
        .expect("成功桩必答");
    assert!(
        reply.contains("帮我看看这份计划"),
        "prompt 必须来自事件切片：{}",
        reply
    );
}

/// 验收：生成失败 → 兜底埋点（cost_ms / produced_by / error 载荷）。
#[tokio::test]
async fn reasoner_failure_produces_fallback_with_telemetry() {
    let llm = MinimalStub::always_fail("模型不可用");
    let (store, outcome) = rehearse_reasoner_turn("任何话", &llm).await;
    assert!(outcome.is_err());
    let snapshot = store.range(crate::event::Seq::new(0));
    let fb = snapshot
        .iter()
        .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FALLBACK)
        .expect("兜底必在");
    assert!(fb.produced_by.is_some(), "兜底可被 I2 审计");
    let payload: serde_json::Value = fb.payload.clone();
    assert!(
        payload.get("error").is_some(),
        "失败原因入载荷（可观测）：{payload}"
    );
    assert!(
        check_all(EVENTS, &snapshot).is_empty(),
        "{:?}",
        check_all(EVENTS, &snapshot)
    );
}

/// 验收：四层 budget_ms 就位——同一时延在不同档位下结论不同。
#[tokio::test]
async fn four_tier_budgets_are_measurable() {
    // 注入 120ms 时延：反射档（80ms）超预算，深度档（60s）预算内。
    let llm = MinimalStub::with_delay("stub-model", 120);
    let (store, _) = rehearse_reasoner_turn("你好", &llm).await;
    let snapshot = store.range(crate::event::Seq::new(0));
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
    // 反向锚：预算表口径（DE-001 §10）在 adapters 域的单测里逐值钉死。
    assert_eq!(LatencyTier::Reflex.budget_ms(), 80);
}

// ── 彩排：记忆链路 ──
//
// 这里留的是**内核侧**的判据：编码格子的溯源覆盖（I2）。巩固入库前的保真度
// 闸门（N8）随 `consolidate` 住它的写方（`crates/symbio-plugin-loop`，DE-025 D-221）；
// 检索那半条链路（`recall` 投影的 as-of、预算降级、主体隔离）随投影住它的唯一生产
// 消费方（[DE-024](../../../../docs/003-design/024-能力横向扩充的通路.md "DE-024-能力横向扩充的通路") D-201），验收在同侧。

/// 溯源覆盖 100%——无溯源的记忆事件被 I2 抓住。
#[test]
fn memory_event_without_provenance_is_caught() {
    let bare = Event {
        produced_by: None,
        ..Event::pending(
            "m0",
            MEMORY_ENCODED,
            Entity::Memory,
            Verb::Opened,
            "session/main",
        )
    };
    let bad = crate::invariants::produced_by_coverage(std::slice::from_ref(&bare));
    assert_eq!(bad.len(), 1, "无溯源的记忆必须被看见（溯源覆盖 100%）");
    assert!(bad[0].why.contains("记忆"), "{}", bad[0].why);
}

// ── S13 淘汰回路（[DE-043 §6](../../../../docs/003-design/043-多智能体社会.md "DE-043-多智能体社会") 断言 2/3/4）──
use crate::actors::EliminationDecider;
use crate::authz::PRINCIPAL_ARBITER;
use crate::event::Entity as VEntity;
use crate::governance::{GrantsChange, GrantsOverlay};

/// 仲裁者行（exit ① 的表半边）：`spec` 五字段全部**取实**——
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
        Pattern::Decider,
        "仲裁者无 reply.* 也无 produce.artifact ⇒ Decider（闭集不变宽）"
    );
    assert_eq!(
        spec.capabilities,
        vec!["assert.verification".to_string()],
        "能力从授权表那一行取数，不在这里硬写"
    );
    assert_eq!(
        spec.budget_ms,
        crate::adapters::LatencyTier::Reflex.budget_ms(),
        "淘汰判定是规则折叠：反射档"
    );
    assert_eq!(spec.scope, Scope::Root);
}

/// DE-043 §6 断言 3：裁决 = 一条**可审计、可撤销**的 grants 变更事件。
///
/// 撤销事件的**写方**是声誉 owner（`crates/symbio-plugin-reputation/src/elimination.rs`
/// 的 `verdict_draft`，同一份载荷），本域只剩读侧翻译与恢复事件的构造。判据两条：
/// ① 载荷六字段齐全（`score` / `threshold` 让审计方能复算）；② `verdict_grants_changes`
/// 把事件流水折成覆盖层后，`is_revoked` 翻真、恢复事件再翻假——回路闭合。
#[test]
fn verdict_flow_folds_into_a_revocable_grants_overlay() {
    let revoke = Event::pending(
        "v-elim-session/b-42",
        EVENT_VERDICT_ASSERTED,
        VEntity::Verdict,
        Verb::Asserted,
        PRINCIPAL_ARBITER,
    )
    .with_produced_by(42)
    .with_payload(serde_json::json!({
        "subject": "session/b",
        "capability": "assign.work",
        "change": "revoke",
        "score": -1,
        "threshold": 0,
        "why": "声誉分 -1 低于淘汰阈值 0",
    }));
    // ① 载荷六字段（写方在声誉 owner，读侧必须按同一形状翻译）。
    let p = &revoke.payload;
    assert_eq!(p["subject"], "session/b");
    assert_eq!(p["capability"], "assign.work");
    assert_eq!(p["change"], "revoke");
    assert_eq!(p["score"], -1);
    assert_eq!(p["threshold"], 0);
    assert!(p["why"].as_str().is_some_and(|w| !w.is_empty()), "人话理由");

    // ② 流水 → 覆盖层：撤销生效、恢复可逆、再收回又生效（last-wins）。
    let restored = EliminationDecider.restoration_event(
        EVENT_VERDICT_ASSERTED,
        "session/b",
        "申诉受理：上游数据延迟有凭证",
        77,
    );
    assert_eq!(restored.kind, EVENT_VERDICT_ASSERTED);
    assert_eq!(restored.entity, VEntity::Verdict);
    assert_eq!(restored.verb, Verb::Asserted);
    assert_eq!(
        restored.actor, PRINCIPAL_ARBITER,
        "裁决者是仲裁者，不是被淘汰主体"
    );
    assert_eq!(restored.produced_by, Some(77), "I2：断言类事件必须带溯源");
    assert_eq!(restored.payload["change"], "restore");
    let changes = EliminationDecider::verdict_grants_changes(&[revoke.clone(), restored.clone()]);
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
            revoke.clone(),
            restored,
            revoke,
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
        "session/b",
    );
    let bogus = Event::pending(
        "y",
        EVENT_VERDICT_ASSERTED,
        VEntity::Verdict,
        Verb::Asserted,
        PRINCIPAL_ARBITER,
    )
    .with_payload(serde_json::json!({ "subject": "session/b", "change": "revokee" }));
    assert!(
        EliminationDecider::verdict_grants_changes(&[other, bogus]).is_empty(),
        "别的格子 / 笔误的 change ⇒ 一条都不译（不把笔误读成改判）"
    );
}

// ── 彩排：任务树与返工 ──
//
// DAG 无环（C14）；独立验证者（C15 不自验，构造期拒绝）；
// 返工闭环（返工那格，返工次数有界 = 终止性前提 2）。
//
// 就绪集不在这里：那条折叠随它的 owner 进了 `crates/symbio-plugin-task`（DE-045 `D-289`），
// 本节的三条判据都是**守门**（`invariants`），不是读数。

use crate::invariants::{acyclic_deps, rework_bounded};
use crate::projection::task_vocab::TaskVocab;

/// 任务族五格的**夹具拼写**（DE-024 D-206）：名字与格位都在 owner 插件
/// （`crates/symbio-plugin-task` 的 `EventDecl` + 清单 `shared_events`），core 与内核都不认识它们。
/// core 的夹具因此自己写一遍这一族的拼写，交给落格的辅助函数——与 `REWORK_VOCAB` 同一形状：
/// 词汇是数据。
const TASK: TaskVocab<'static> = TaskVocab {
    opened: "task.opened",
    progressed: "task.progress",
    held: "task.held",
    terminal: "task.asserted",
    lease: "task.assigned",
};

/// `memory.encoded`：名字与格位都在 owner 插件（`crates/symbio-plugin-loop` 的 `EventDecl` + 清单
/// `shared_events`，DE-024 D-204），core 不认识它。core 的夹具因此自己写这条字面量——
/// 与 `REWORK` 同一形状：词汇是数据，不是 core 的常量。
const MEMORY_ENCODED: &str = "memory.encoded";

/// 返工那一格：名字与角色随 owner 插件（`crates/symbio-plugin-task::decl` 的 `EventDecl`），core 不认识它。
///
/// core 的判据只认角色（DE-024 D-203），所以这里自己声明一行当词汇面——生产环境里
/// 这张表由 owner 在装配期交来（`Frozen::vocab`）。
const REWORK: &str = "task.rework_created";
const REWORK_VOCAB: &[EventDecl] = &[EventDecl::with_roles(
    REWORK,
    (Entity::Task, Verb::Asserted),
    &[FactRole::CountsRework],
)];

/// 开一个任务（载荷 `{ task_id, depends_on, goal }`）。
fn open_task(store: &EventStore, id: &str, depends_on: &[&str], actor: &str) {
    store
        .append(
            Event::pending(
                format!("o-{id}"),
                TASK.opened,
                Entity::Task,
                Verb::Opened,
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
                TASK.terminal,
                Entity::Task,
                Verb::Asserted,
                actor,
            )
            .with_produced_by(0)
            .with_payload(serde_json::json!({ "task_id": id })),
        )
        .expect("终态必入库");
}

/// DE-033 §6 验收 1：构造含环依赖图（a→b→c→a）→ 无环检测必须报错。
#[test]
fn cyclic_dependency_graph_is_rejected() {
    let store = EventStore::new();
    open_task(&store, "a", &["c"], "session/main");
    open_task(&store, "b", &["a"], "session/main");
    open_task(&store, "c", &["b"], "session/main");
    let bad = acyclic_deps(&store.range(crate::event::Seq::new(0)));
    assert_eq!(bad.len(), 3, "环上三个节点都要被点名：{bad:?}");
    assert!(bad[0].why.contains("依赖环"), "{}", bad[0].why);
}

/// DE-033 §6 验收 4（反向）：把环去掉一个依赖 → 断言 1 必须通过（检测在算，不是常量 false）。
#[test]
fn breaking_one_edge_makes_graph_pass() {
    let store = EventStore::new();
    // 同一张图，只去掉 c→b 这条边。
    open_task(&store, "a", &["c"], "session/main");
    open_task(&store, "b", &["a"], "session/main");
    open_task(&store, "c", &[], "session/main");
    let snapshot = store.range(crate::event::Seq::new(0));
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
    let bad = acyclic_deps(&store.range(crate::event::Seq::new(0)));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].why.contains("悬空"), "{}", bad[0].why);
}

/// DE-033 §6 验收 2（C15）：同一主体同时持 define.work 与 assert.verification → 拒绝。
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
            Err(crate::governance::PairingViolation::SelfVerifier { .. })
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

/// DE-033 §6 验收 3：注入 N 步任务链 → 在 rework 上界内终止（全链跑完、检查全绿）。
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
    for i in 0..n {
        store
            .append(
                Event::pending(
                    format!("p-{i}"),
                    TASK.progressed,
                    Entity::Task,
                    Verb::Progressed,
                    "session/main",
                )
                .with_payload(serde_json::json!({ "task_id": format!("t{i}") })),
            )
            .unwrap();
        assert_task(&store, &format!("t{i}"), "session/verifier");
    }
    let snapshot = store.range(crate::event::Seq::new(0));
    assert!(acyclic_deps(&snapshot).is_empty());
    assert!(rework_bounded(REWORK_VOCAB, &snapshot, 1).is_empty());
    assert!(
        check_all(EVENTS, &snapshot).is_empty(),
        "{:?}",
        check_all(EVENTS, &snapshot)
    );
}

/// DE-039 §6 验收 3：grants 表高风险组合（外部执行 + 自验）→ 构造期拒绝。
#[test]
fn high_risk_grant_combo_is_rejected() {
    let risky = PermissionMatrix::new(vec![PrincipalPolicy::paired(
        "session/risky",
        vec![Capability::ProduceArtifact, Capability::AssertVerification],
        VisScope::ThreadPrivate,
    )]);
    assert!(matches!(
        risky,
        Err(crate::governance::PairingViolation::SelfVerifier { .. })
    ));
}

// ── 彩排：自治与学习 ──
//
// 定时触发的**授权面**（出口判据「自主写入对话 0」，DE-042）留在这里；
// 触发、长目标预算、欲治理与技能编译 / 路由的实现和用例随唯一消费方住
// `crates/symbio-plugin-session/src/transcript/autonomous` 与 `crates/symbio-plugin-skill`（D-189 / D-262）。

/// DE-042 §6 验收 2：自主发起者尝试写 `chat.assistant.final` → 授权拒绝且不产生事件。
///
/// 判据查的是**生产授权表**（`authz::production_matrix`），
/// 不是自造矩阵——自造矩阵只证明「一个我自己写的策略拒绝了我自己」，生产表改了它照样绿。
/// 三条断言缺一不可：
/// ① 自主发起者**不持** `reply`（验收 2 的结论）；
/// ② 它**持** `define.work`（否则 ① 可能只是「矩阵整个是空的」的平凡真）；
/// ③ 主智能体**持** `reply`（对照：拒绝是**针对这个主体**的，不是一律拒绝）。
///
/// 「拒绝 ⇒ 不产生事件」的**执行**在写侧闸（`crates/symbio-plugin-session` 运行器的
/// `turn_runner::closure_gate`，其判据用例见
/// `turn_runner.test.rs::closure_is_withheld_when_the_principal_lacks_the_grant`）
/// ——core 这边只钉**表里的结论**，不假装自己跑过那条闸。
#[test]
fn autonomous_actor_cannot_write_to_dialog() {
    use crate::authz::{production_matrix, PRINCIPAL_AUTONOMOUS, PRINCIPAL_MAIN};

    let initiator = production_matrix();
    assert!(
        !initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::Reply),
        "自主层无 reply 写权（自主写入对话 = 0）"
    );
    assert!(
        !initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::Reply),
        "续写同样不持——自主行为不得冒充用户对话"
    );
    assert!(
        initiator.can_write(PRINCIPAL_AUTONOMOUS, Capability::DefineWork),
        "自主层必须能定义工作（否则上一条只是『空矩阵』的平凡真）"
    );
    let main = production_matrix();
    assert!(
        main.can_write(PRINCIPAL_MAIN, Capability::Reply),
        "对照：主智能体持 reply——拒绝是针对主体的，不是矩阵一律拒绝"
    );
}

// DE-042 §6 验收 3 + 4：长目标预算判定的实现随发起者住 `crates/symbio-plugin-session/src/transcript/autonomous`
// （D-189）；core 只留授权表结论（见上一条）。

// [DE-001 §4.2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 欲治理三条：(a) 候选造出来一定未批准 (b) 无溯源构造不出 (c) 关停后
// 无任何自主任务但读侧投影照常（J2 平凡值）。实现与用例随唯一消费方住
// `crates/symbio-plugin-session/src/transcript/autonomous`（D-189）。

/// 档位名字往返：LatencyTier::name / from_name 互逆；未知名字不静默归类。
#[test]
fn latency_tier_name_round_trip() {
    use crate::adapters::LatencyTier;
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
