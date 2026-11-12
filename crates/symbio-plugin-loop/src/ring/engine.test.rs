use super::*;
use crate::ring::facts::{DebtKey, Fact, FactKind, FactView};
use crate::ring::table::{DebtRow, DebtSource, DeclInput, Table};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use symbio_kernel::decl::ring::{Decl, Pattern};
use symbio_kernel::decl::{ActorDecl, Builder, FactDecl, Frozen, PluginDeclarations, ViewDecl};
use symbio_kernel::event::{Entity, Event as KernelEvent, EventDecl, Verb};
use symbio_kernel::LatencyTier;
use symbio_plugin_api::debt::{FailureProjection, ReplyRole};
use symbio_plugin_api::drive::DRIVE_SENT;
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortFactWriter, PortInput,
    PortOutcome,
};
use symbio_plugin_api::rows::PLUGIN_ROW;

/// 测试宿主：事实在内存里，债表与执行体由测试给定。
///
/// 克隆的是句柄：后台执行体经 `shared()` 拿到同一份账（`D-376`）。
#[derive(Clone)]
struct TestCtx {
    facts: Arc<Mutex<Vec<Fact>>>,
    rows: Arc<dyn DebtSource + Send + Sync>,
    executors: Arc<HashMap<&'static str, Arc<dyn PortExecutor>>>,
    now: u64,
    outlet: PortFactEmit,
    concurrency: usize,
    step_budget: usize,
    /// 模拟「写口把投递全吸收了」：用于覆盖无进展出口。
    writes_ignored: bool,
}

impl TestCtx {
    fn new(rows: Vec<DebtRow>) -> Self {
        Self::with_source(Box::new(Table::new(rows)))
    }

    fn with_source(source: Box<dyn DebtSource + Send + Sync>) -> Self {
        Self {
            facts: Arc::new(Mutex::new(Vec::new())),
            rows: Arc::from(source),
            executors: Arc::new(HashMap::new()),
            now: 0,
            outlet: PortFactEmit::silent(),
            concurrency: 1,
            step_budget: 128,
            writes_ignored: false,
        }
    }

    fn with_step_budget(mut self, step_budget: usize) -> Self {
        self.step_budget = step_budget;
        self
    }

    fn ignoring_writes(mut self) -> Self {
        self.writes_ignored = true;
        self
    }

    fn with_concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    fn with_executor(mut self, port: &'static str, executor: Arc<dyn PortExecutor>) -> Self {
        Arc::make_mut(&mut self.executors).insert(port, executor);
        self
    }

    fn with_facts(mut self, facts: Vec<Fact>) -> Self {
        self.facts = Arc::new(Mutex::new(facts));
        self
    }

    fn with_outlet(mut self, outlet: PortFactEmit) -> Self {
        self.outlet = outlet;
        self
    }

    fn at(mut self, now: u64) -> Self {
        self.now = now;
        self
    }

    fn snapshot(&self) -> Vec<Fact> {
        self.facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl DriveCtx for TestCtx {
    fn facts(&self) -> Vec<Fact> {
        // 环的 now 只取自事实（`D-376`）：`.at(now)` 把时间推到 now = 在账上落一条
        // 已收束的时钟事实，测试不靠墙钟也能模拟「时间过去了」。
        let mut facts = self.snapshot();
        if self.now > facts.iter().map(|f| f.at_ms).max().unwrap_or(0) {
            facts.push(Fact::new(
                FactKind::Response,
                DebtKey::new("__clock", "__tick", 1),
                json!({}),
                self.now,
            ));
        }
        facts
    }

    fn append(&self, fact: Fact) {
        if self.writes_ignored {
            return;
        }
        let mut facts = self
            .facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if facts
            .iter()
            .any(|f| f.kind == fact.kind && f.key == fact.key && shard_of(f) == shard_of(&fact))
        {
            return;
        }
        facts.push(fact);
    }

    fn source(&self) -> &dyn DebtSource {
        self.rows.as_ref()
    }

    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        self.executors.get(port).cloned()
    }

    fn shared(&self) -> Arc<dyn DriveCtx> {
        Arc::new(self.clone())
    }

    fn concurrency(&self) -> usize {
        self.concurrency
    }

    fn step_budget(&self) -> usize {
        self.step_budget
    }

    fn outlet(&self) -> PortFactEmit {
        self.outlet.clone()
    }
}

#[derive(Default)]
struct Seen {
    inputs: Vec<(String, Value, String)>,
}

struct Stub {
    seen: Mutex<Seen>,
    retry_until: u32,
    non_retryable: bool,
    needs_human: bool,
}

impl Stub {
    fn ok() -> Self {
        Self {
            seen: Mutex::new(Seen::default()),
            retry_until: 1,
            non_retryable: false,
            needs_human: false,
        }
    }

    fn retry_once_then_ok() -> Self {
        Self {
            retry_until: 2,
            ..Self::ok()
        }
    }

    /// 永远可重试：用来把步数预算真的走完。
    fn always_retryable() -> Self {
        Self {
            retry_until: u32::MAX,
            ..Self::ok()
        }
    }

    fn non_retryable() -> Self {
        Self {
            non_retryable: true,
            ..Self::ok()
        }
    }

    /// 第三态「挂起」：`sent{deadline_ms:null}`，债继续开着。
    fn needs_human() -> Self {
        Self {
            needs_human: true,
            ..Self::ok()
        }
    }

    fn seen(&self) -> Vec<(String, Value, String)> {
        let guard = self
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.inputs.clone()
    }
}

fn attempt_of(key: &str) -> u32 {
    key.rsplit(':')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1)
}

#[async_trait::async_trait]
impl PortExecutor for Stub {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let attempt = attempt_of(&ctx.idempotency_key);
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .inputs
            .push((
                input.port.to_string(),
                input.payload.clone(),
                ctx.idempotency_key.clone(),
            ));
        if self.non_retryable {
            return PortOutcome::failed(PortErrorClass::NonRetryable, "nope");
        }
        if self.needs_human {
            return PortOutcome::failed(PortErrorClass::NeedsHuman, "等待用户");
        }
        if attempt < self.retry_until {
            return PortOutcome::failed(PortErrorClass::Retryable, "again");
        }
        PortOutcome {
            facts: vec![json!({"answer": input.payload})],
            ..PortOutcome::ok()
        }
    }
}

struct Emitting {
    world: Arc<RecordSink>,
}

#[async_trait::async_trait]
impl PortExecutor for Emitting {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        self.world.drafts.lock().unwrap().push(FactDraft {
            key: "w".into(),
            kind: "note".into(),
            entity: symbio_plugin_api::Entity::Domain,
            verb: symbio_plugin_api::Verb::Opened,
            payload: json!({"text": "world"}),
            produced_by: None,
        });
        PortOutcome::ok()
    }
}

#[derive(Default)]
struct RecordSink {
    drafts: Mutex<Vec<FactDraft>>,
}

/// 测试宿主的吸收与真宿主同口径：分片号是附加身份位。
fn shard_of(fact: &Fact) -> Option<u64> {
    fact.payload.get("shard").and_then(|v| v.as_u64())
}

#[async_trait::async_trait]
impl PortFactWriter for RecordSink {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.drafts.lock().unwrap().extend(facts);
    }
}

#[tokio::test]
async fn one_round_writes_request_sent_response() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({"q": 1}))])
        .with_executor("p1", stub.clone());
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    let kinds: Vec<_> = ctx.snapshot().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Sent, FactKind::Request, FactKind::Response]
    );
    let seen = stub.seen();
    assert_eq!(seen[0].0, "p1");
    assert_eq!(seen[0].1, json!({"q": 1}));
    assert_eq!(seen[0].2, "p1:d:1");
    let response = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Response)
        .unwrap();
    // 环把端口名与语义章补进收束载荷（执行体不自报）；读侧按章判这次往返是否
    // 对读者说话、是否进台账、失败是否补卡，缺了它们这些节点会全部静默消失。
    assert_eq!(
        response.payload,
        json!({
            "answer": {"q": 1},
            "port": "p1",
            "stamp": {"role": "internal"},
        })
    );
}

#[tokio::test]
async fn settlement_stamps_carry_the_row_declared_semantics() {
    let stub = Arc::new(Stub::ok());
    let mut row = DebtRow::new("a", "p1", "d", json!({}));
    row.role = ReplyRole::Reader;
    row.accounts_usage = true;
    row.accounts_spend = true;
    row.failure = FailureProjection::ToolCall;
    let ctx = TestCtx::new(vec![row]).with_executor("p1", stub);

    let _ = drive(&ctx).await;

    let response = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Response)
        .unwrap();
    assert_eq!(
        response.payload["stamp"],
        json!({
            "role": "reader",
            "usage": true,
            "spend": true,
            "failure": "tool_call",
        })
    );
}

#[tokio::test]
async fn retryable_error_advances_the_lease() {
    let stub = Arc::new(Stub::retry_once_then_ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "d", json!({})).lease(60_000, 2)
    ])
    .with_executor("p1", stub.clone());
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 2);
    let kinds: Vec<_> = ctx.snapshot().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![
            FactKind::Sent,
            FactKind::Request,
            FactKind::Retry,
            FactKind::Sent,
            FactKind::Request,
            FactKind::Response
        ]
    );
    let facts = ctx.snapshot();
    let view = FactView::new(&facts);
    assert_eq!(view.attempt("p1", "d"), 2);
    assert_eq!(stub.seen().len(), 2);
}

/// 重试归属（D-442 / D-351）：多行共用同一个 `(port, id)` 时（工具端口的首派行
/// 与恢复行），attempt 自增后由 `retry` 载荷的 `row` 指认谁来重跑——不是表序
/// 先到先得。这里被指认的行排在表序之后 ⇒ 新 attempt 上重跑的是它，表序第一行
/// 不再跟着重来。
#[tokio::test]
async fn retry_follows_the_row_that_ran_not_table_order() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("first", "p1", "d", json!({"q": "first"})),
        DebtRow::new("resume", "p1", "d", json!({"q": "resume"})),
    ])
    .with_executor("p1", stub.clone())
    .with_facts(vec![
        // attempt 1 由恢复行跑过、失败可重试：派发时 `sent` 记了行名，租约推进
        // 据此把归属写进 `retry`——下一圈只有被指认的行能认领新 attempt。
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 60_000, "row": "resume"}),
            100,
        ),
        Fact::new(
            FactKind::Retry,
            DebtKey::new("p1", "d", 2),
            json!({"from": 1, "class": "retryable", "reason": "again", "row": "resume"}),
            200,
        ),
    ]);
    let _ = drive(&ctx).await;

    let seen = stub.seen();
    assert_eq!(seen.len(), 1);
    // 重跑的是被指认的行（载荷 `resume`），不是表序第一行（载荷 `first`）。
    assert_eq!(seen[0].1, json!({"q": "resume"}));
    assert_eq!(seen[0].2, "p1:d:2");
}

/// 孤儿判死（D-442）：`retry` 指认的行这一圈没有任何源申报（行被删 / 触发事实
/// 消失）。没有 `sent` 就没有租约，这笔债留着只会悬挂——判死是唯一诚实的收束：
/// 可见的失败优于不可见的停摆。
#[tokio::test]
async fn retry_owner_row_gone_lands_failed() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![DebtRow::new(
        "first",
        "p1",
        "d",
        json!({"q": "first"}),
    )])
    .with_executor("p1", stub.clone())
    .with_facts(vec![
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 60_000, "row": "gone"}),
            100,
        ),
        Fact::new(
            FactKind::Retry,
            DebtKey::new("p1", "d", 2),
            json!({"from": 1, "class": "retryable", "reason": "again", "row": "gone"}),
            200,
        ),
    ]);
    let _ = drive(&ctx).await;

    // 没有任何执行体被调用；这笔债以可见的失败收束，而不是悬挂。
    assert!(stub.seen().is_empty());
    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.key.attempt, 2);
    assert_eq!(last.payload["class"], "non_retryable");
    assert_eq!(last.payload["reason"], "retry owner row gone");
}

#[tokio::test]
async fn non_retryable_error_lands_failed() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Stub::non_retryable()));
    let _ = drive(&ctx).await;

    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["class"], "non_retryable");
    // 失败与成功同一条身份位与语义章：端口与角色都由环补，读侧按章判
    // 「这次往返对读者说话」。
    assert_eq!(last.payload["port"], "p1");
    assert_eq!(last.payload["stamp"], json!({"role": "internal"}));
}

/// 授权拒绝（`D-385`）：执行体交 `Denied` ⇒ 它这一笔的草稿**一概不落**，落
/// `Failed(non_retryable)` 且理由非空。
///
/// 这是 `D-385` 判据 3 的反例：类型化结论不是装饰——`Denied` 必须能在环里断掉
/// 产物通路，否则「忘了判」与「判了通过」在行为上没有区别。
struct Denying;

#[async_trait::async_trait]
impl PortExecutor for Denying {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        PortOutcome {
            facts: vec![json!({"answer": "must not land"})],
            ..PortOutcome::denied("禁止写入")
        }
    }
}

#[tokio::test]
async fn denied_outcome_drops_the_draft_and_lands_failed() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Denying));
    let _ = drive(&ctx).await;

    let kinds: Vec<_> = ctx.snapshot().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Sent, FactKind::Request, FactKind::Failed],
        "拒绝不产生 Response（草稿被丢弃）"
    );
    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["class"], "non_retryable");
    assert_eq!(last.payload["reason"], "禁止写入");
    assert_eq!(
        last.payload.get("answer"),
        None,
        "被拒的草稿不许混进失败载荷"
    );
}

/// 失败但交回产出（截断轮的形状）：产出在**顶层**，端口同成功路径。
///
/// 读侧（`rebuild::output_nodes`）只认顶层 `port` + `reasoning`；把产出裹进
/// `facts` 数组等于让它随失败一起消失（t77）。
struct DraftFailure;

#[async_trait::async_trait]
impl PortExecutor for DraftFailure {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        PortOutcome {
            facts: vec![json!({
                "root_id": "turn-1",
                "text": "",
                "reasoning": "先读三处，再逐段列数字",
                "reasoning_id": "turn-1-reasoning",
                "answer": "",
                "truncated": true,
            })],
            error: Some((PortErrorClass::NonRetryable, "长度截断".to_string())),
            ..PortOutcome::ok()
        }
    }
}

#[tokio::test]
async fn failed_carries_the_draft_at_top_level() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "model", "d", json!({}))])
        .with_executor("model", Arc::new(DraftFailure));
    let _ = drive(&ctx).await;

    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["port"], "model");
    assert_eq!(last.payload["reasoning"], "先读三处，再逐段列数字");
    assert_eq!(last.payload["reason"], "长度截断");
    assert!(
        last.payload.get("facts").is_none(),
        "本轮产出在顶层，不裹 facts：{last:?}"
    );
}

#[tokio::test]
async fn lease_expiry_replays_an_existing_ledger() {
    let stub = Arc::new(Stub::ok());
    let seed = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "d", 1), json!({}), 0),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 10}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({})).lease(10, 2)])
        .with_executor("p1", stub.clone())
        .with_facts(seed)
        .at(100);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(stub.seen().len(), 1);
    assert_eq!(stub.seen()[0].2, "p1:d:2");
    let response = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Response)
        .unwrap();
    assert_eq!(response.key.attempt, 2);
}

#[tokio::test]
async fn a_fresh_lease_is_waited_not_expired() {
    let seed = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "d", 1), json!({}), 0),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 100}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({})).lease(100, 2)])
        .with_executor("p1", Arc::new(Stub::ok()))
        .with_facts(seed)
        .at(50);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::Waiting(vec![DebtKey::new("p1", "d", 1)])
    );
}

/// 未配对的 `request` 不会永久悬挂：租约到期且没有下一笔 ⇒ 环自己落 `failed`。
///
/// 开轮/终答的配对由环的五事实机制记账（这是它替代 SDK 按轮分组判据的机制证明）。
#[tokio::test]
async fn an_unpaired_request_is_closed_by_lease_not_left_pending() {
    let seed = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "d", 1), json!({}), 0),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 10}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({})).lease(10, 1)])
        .with_executor("p1", Arc::new(Stub::ok()))
        .with_facts(seed)
        .at(100);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 0, "到点后不再重复派发");
    let failed = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Failed)
        .expect("未配对的 request 必须被租约收束为 failed，而不是永久悬挂");
    assert_eq!(failed.key.attempt, 1);
    assert_eq!(failed.payload["reason"], "lease-expired");
}

#[tokio::test]
async fn after_gate_orders_two_ports() {
    let first = Arc::new(Stub::ok());
    let second = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p2", "b", json!({})).after(DebtKey::new("p1", "a", 1)),
    ])
    .with_executor("p1", first.clone())
    .with_executor("p2", second.clone());
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 2);
    assert_eq!(first.seen().len(), 1);
    assert_eq!(second.seen().len(), 1);
}

#[tokio::test]
async fn after_port_waits_for_a_claim_and_treats_absence_as_settled() {
    // 前提端口这一圈有人认领 ⇒ 依赖行等它收束后的下一圈才派发。
    let gate = Arc::new(Stub::ok());
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("gate", "p1", "g", json!({})),
        DebtRow::new("dep", "p2", "d", json!({})).after_port("p1"),
    ])
    .with_executor("p1", gate.clone())
    .with_executor("p2", dependent.clone());
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 2);
    assert_eq!(gate.seen().len(), 1);
    assert_eq!(dependent.seen().len(), 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    // 前提端口这一圈**没人认领**（能力未挂载 / 开关关闭的形状）⇒ 缺席即收束，直接可还。
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("dep", "p2", "d", json!({})).after_port("p1")
    ])
    .with_executor("p2", dependent.clone());
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 1, "没人认领的前提不许挡住依赖方");
    assert_eq!(dependent.seen().len(), 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
}

/// 本圈计划快照（D-343）还没看见回执，但债本身已派发未收束：这仍然是「有人在做
/// 的活」，依赖方不许因为 `when` 的判定滞后就抢跑。
#[tokio::test]
async fn a_reported_open_debt_claims_its_port_even_when_when_looks_stale() {
    struct Stale {
        rows: Vec<DebtRow>,
    }
    impl DebtSource for Stale {
        fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
            self.rows.clone()
        }
        // 快照里还没有 `response`：`when` 判定会滞后。
        fn exists(&self, _kind: &str) -> bool {
            false
        }
    }

    let seed = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "g", 1), json!({}), 0),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "g", 1),
            json!({"deadline_ms": null}),
            0,
        ),
    ];
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::with_source(Box::new(Stale {
        rows: vec![
            DebtRow::new("gate", "p1", "g", json!({})).when("response"),
            DebtRow::new("dep", "p2", "d", json!({})).after_port("p1"),
        ],
    }))
    .with_executor("p2", dependent.clone())
    .with_facts(seed);

    let report = drive(&ctx).await;

    assert_eq!(dependent.seen().len(), 0, "回执还没到，模型不许抢跑");
    assert_eq!(
        report.outcome,
        DriveOutcome::NeedsHuman(vec![DebtKey::new("p1", "g", 1)]),
        "这笔已派发未收束的债必须交回调用方，由它等回执后再踢"
    );
}

/// 门认领了却永远收不了束（写口把投递全吸收）：依赖方不许被静默放行，
/// 也不能变成 `NoDebt`——必须停在可见的 `Stalled`。
#[tokio::test]
async fn a_gate_that_never_settles_surfaces_as_stalled_not_silence() {
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("gate", "p1", "g", json!({})),
        DebtRow::new("dep", "p2", "d", json!({})).after_port("p1"),
    ])
    .with_executor("p1", Arc::new(Stub::ok()))
    .with_executor("p2", dependent.clone())
    .ignoring_writes();

    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1, "门被认领并派发了一次");
    assert_eq!(dependent.seen().len(), 0, "门没收束，依赖方不许抢跑");
    match report.outcome {
        DriveOutcome::Stalled(keys) => {
            assert!(
                keys.contains(&DebtKey::new("p2", "d", 1)),
                "被前提挡住的依赖方必须出现在 Stalled 里：{keys:?}"
            );
        }
        other => panic!("永不收束的门债必须走到 Stalled，不能静默：{other:?}"),
    }
}

#[tokio::test]
async fn a_suspended_gate_keeps_the_dependent_waiting_visibly() {
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("gate", "p1", "g", json!({})),
        DebtRow::new("dep", "p2", "d", json!({})).after_port("p1"),
    ])
    .with_executor("p1", Arc::new(Stub::needs_human()))
    .with_executor("p2", dependent.clone());

    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(dependent.seen().len(), 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::NeedsHuman(vec![DebtKey::new("p1", "g", 1)]),
        "挂起的门不是静默：依赖方的等待必须交回调用方"
    );
}

#[tokio::test]
async fn an_expired_gate_releases_the_dependent_instead_of_waiting_forever() {
    let seed = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "g", 1), json!({}), 0),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "g", 1),
            json!({"deadline_ms": 10}),
            0,
        ),
    ];
    let dependent = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("gate", "p1", "g", json!({})).lease(10, 1),
        DebtRow::new("dep", "p2", "d", json!({})).after_port("p1"),
    ])
    .with_executor("p1", Arc::new(Stub::ok()))
    .with_executor("p2", dependent.clone())
    .with_facts(seed)
    .at(100);

    let report = drive(&ctx).await;

    // 门的租约到期、attempt 1 没有下一笔 ⇒ 判死落 `failed`；端口清空，依赖方放行。
    assert_eq!(dependent.seen().len(), 1, "门判死后依赖方必须能继续");
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
}

#[tokio::test]
async fn unless_gate_reads_the_same_surface_as_when() {
    /// 世界事实的测试读面：种类集合 + 身份集合，与环的五事实无关。
    struct Gated {
        rows: Vec<DebtRow>,
        kinds: Vec<&'static str>,
        ids: Vec<(&'static str, String)>,
    }

    impl DebtSource for Gated {
        fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
            self.rows.clone()
        }

        fn exists(&self, kind: &str) -> bool {
            self.kinds.contains(&kind)
        }

        fn exists_id(&self, kind: &str, id: &str) -> bool {
            self.ids.iter().any(|(k, i)| *k == kind && i == id)
        }
    }

    let rows = vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})).unless("turn.done"),
    ];
    // 种类事实缺席 ⇒ 两行都欠（同一圈可以两笔在飞）。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: Vec::new(),
        ids: Vec::new(),
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx).await.dispatched, 2);

    // 种类事实存在 ⇒ 排除行不欠（非五类事实也算，「同一条读面」）。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: vec!["turn.done"],
        ids: Vec::new(),
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx).await.dispatched, 1);

    // 身份级：只有该种类、该身份的事实命中；环账上同 id 的 request / sent / retry
    // 因种类不同不命中（可重试失败后的重试轮不会被提前跳过）。
    let rows = vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})).unless_id("turn.done", "turn.done:b"),
    ];
    let ledger = vec![
        Fact::new(FactKind::Request, DebtKey::new("p1", "b", 1), json!({}), 0),
        Fact::new(FactKind::Sent, DebtKey::new("p1", "b", 1), json!({}), 0),
        Fact::new(FactKind::Retry, DebtKey::new("p1", "b", 2), json!({}), 0),
    ];
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: Vec::new(),
        ids: Vec::new(),
    }))
    .with_executor("p1", Arc::new(Stub::ok()))
    .with_facts(ledger);
    assert_eq!(
        drive(&ctx).await.dispatched,
        2,
        "b 的重试轮照常派发：环账上同 id 的 request / sent / retry 不命中这本谓词"
    );

    // 该身份的世界事实已存在 ⇒ 本行不欠。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows,
        kinds: Vec::new(),
        ids: vec![("turn.done", "turn.done:b".to_string())],
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx).await.dispatched, 1);
    assert_eq!(drive(&ctx).await.outcome, DriveOutcome::NoDebt);
}

#[tokio::test]
async fn when_gate_asks_the_host_whether_the_kind_exists() {
    /// 数据源把「存在的种类」当数据；`exists` 由宿主实现，环只问。
    struct Gated {
        rows: Vec<DebtRow>,
        kinds: Vec<&'static str>,
    }

    impl DebtSource for Gated {
        fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
            self.rows.clone()
        }

        fn exists(&self, kind: &str) -> bool {
            self.kinds.iter().copied().any(|known| known == kind)
        }
    }

    let rows = vec![DebtRow::new("a", "p1", "a", json!({})).when("user.message")];
    // 种类不存在 ⇒ 本行不欠。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: vec![],
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 0);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    // 种类存在 ⇒ 本行可还。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows,
        kinds: vec!["user.message"],
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 1);
}

#[tokio::test]
async fn when_no_open_gate_reads_debt_state() {
    let rows = vec![DebtRow::new("m", "p1", "m", json!({})).when_no_open("p1")];
    // 该端口没有未还债 ⇒ 本行可还。
    let ctx = TestCtx::new(rows.clone()).with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx).await.dispatched, 1);

    // 该端口已有一笔未收束的 request ⇒ 本行不欠。
    let ctx = TestCtx::new(rows).with_executor("p1", Arc::new(Stub::ok()));
    ctx.facts.lock().unwrap().push(Fact::new(
        FactKind::Request,
        DebtKey::new("p1", "old", 1),
        json!({}),
        0,
    ));
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 0);
    // 不派发是对的；但债还开着、又没有行能还 ⇒ 出口不能报 `NoDebt`
    // （那会让宿主把 `idle` 记成正常完成）。旧用例把这一格钉成了 `NoDebt`，
    // 钉的正是本条要修的静默停摆。这一笔**没有执行体在跑** ⇒ 归「等外部事实」
    // 那一档（`NeedsHuman`），不是「在飞」。
    assert_eq!(
        report.outcome,
        DriveOutcome::NeedsHuman(vec![DebtKey::new("p1", "old", 1)])
    );
}

/// 链式源的读面：`when_id` 按「种类 + 债 id」查事实账，与宿主 `exists_id` 同口径。
struct ChainedRows {
    facts: Arc<Mutex<Vec<Fact>>>,
    rows: Vec<DebtRow>,
}

impl DebtSource for ChainedRows {
    fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
        self.rows.clone()
    }

    fn exists_id(&self, kind: &str, id: &str) -> bool {
        self.facts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|f| f.kind.as_str() == kind && f.key.id == id)
    }
}

/// 第二笔挂起（待批），其余成功。
struct SecondSuspends {
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PortExecutor for SecondSuspends {
    async fn execute(&self, _ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let call = input
            .payload
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(call.clone());
        if call == "call-2" {
            return PortOutcome::failed(PortErrorClass::NeedsHuman, "等待用户");
        }
        PortOutcome::ok()
    }
}

/// 第一笔首次可重试失败、重试后成功。
struct FirstRetriesOnce {
    seen: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PortExecutor for FirstRetriesOnce {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let call = input
            .payload
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let attempt = attempt_of(&ctx.idempotency_key);
        self.seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(format!("{call}#{attempt}"));
        if call == "call-1" && attempt == 1 {
            return PortOutcome::failed(PortErrorClass::Retryable, "稍后再试");
        }
        PortOutcome::ok()
    }
}

/// 重试口径：`when_id(sent, 前一债 id)` 落到 `exists_id`，只比 `kind + key.id`，
/// **不比 attempt**——问的是「这一 id 有过 `sent`」，不是「attempt 1 的 `sent`」。
/// 因此这是单调判定：第 1 笔重试到 attempt 2 也不会把第 2 笔重新挂住。
#[tokio::test]
async fn retrying_the_previous_call_does_not_reblock_the_next_one() {
    let rows = vec![
        DebtRow::new("tool-call", "tool", "call-1", json!({"call_id": "call-1"})).lease(60_000, 3),
        DebtRow::new("tool-call", "tool", "call-2", json!({"call_id": "call-2"}))
            .lease(60_000, 3)
            .when_id(DRIVE_SENT, "call-1"),
    ];
    let facts = Arc::new(Mutex::new(Vec::new()));
    let executor = Arc::new(FirstRetriesOnce {
        seen: Mutex::new(Vec::new()),
    });
    let mut ctx = TestCtx::with_source(Box::new(ChainedRows {
        facts: Arc::clone(&facts),
        rows,
    }));
    ctx.facts = facts;
    let ctx = ctx.with_executor("tool", executor.clone());

    let report = drive(&ctx).await;

    let seen = executor
        .seen
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert_eq!(seen.iter().filter(|s| *s == "call-1#1").count(), 1);
    assert_eq!(
        seen.iter().filter(|s| *s == "call-1#2").count(),
        1,
        "第 1 笔重试到 attempt 2"
    );
    assert_eq!(
        seen.iter().filter(|s| *s == "call-2#1").count(),
        1,
        "第 2 笔只派发一次：重试没有把它重新挂住"
    );
    let facts = ctx.snapshot();
    assert!(
        facts
            .iter()
            .any(|f| f.kind == FactKind::Response && f.key.id == "call-1" && f.key.attempt == 2),
        "第 1 笔在 attempt 2 收束"
    );
    assert!(
        facts
            .iter()
            .any(|f| f.kind == FactKind::Response && f.key.id == "call-2" && f.key.attempt == 1),
        "第 2 笔按自己的 attempt 1 独立收束"
    );
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
}

/// 派发链不是收束链：第 2 笔挂起（`sent{deadline_ms:null}`）时，第 3 笔仍要派发并收束。
#[tokio::test]
async fn a_suspended_call_does_not_block_the_next_one_in_the_chain() {
    let rows = vec![
        DebtRow::new("tool-call", "tool", "call-1", json!({"call_id": "call-1"})).lease(600_000, 2),
        DebtRow::new("tool-call", "tool", "call-2", json!({"call_id": "call-2"}))
            .lease(600_000, 2)
            .when_id(DRIVE_SENT, "call-1"),
        DebtRow::new("tool-call", "tool", "call-3", json!({"call_id": "call-3"}))
            .lease(600_000, 2)
            .when_id(DRIVE_SENT, "call-2"),
    ];
    // 数据源与事实账共享同一份：`when_id` 读的就是环正在写的那本账。
    let facts = Arc::new(Mutex::new(Vec::new()));
    let executor = Arc::new(SecondSuspends {
        seen: Mutex::new(Vec::new()),
    });
    let mut ctx = TestCtx::with_source(Box::new(ChainedRows {
        facts: Arc::clone(&facts),
        rows,
    }));
    ctx.facts = facts;
    let ctx = ctx.with_executor("tool", executor.clone());

    let report = drive(&ctx).await;

    assert_eq!(
        executor
            .seen
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone(),
        vec!["call-1", "call-2", "call-3"],
        "挂起的那笔不挡后面：三笔都派发"
    );
    let facts = ctx.snapshot();
    for call in ["call-1", "call-3"] {
        assert_eq!(
            facts
                .iter()
                .filter(|f| f.kind == FactKind::Response && f.key.id == call)
                .count(),
            1,
            "{call} 各自独立收束一条"
        );
    }
    assert!(
        !facts.iter().any(|f| {
            (f.kind == FactKind::Response || f.kind == FactKind::Failed) && f.key.id == "call-2"
        }),
        "挂起的第 2 笔没有收束事实"
    );
    assert!(
        facts
            .iter()
            .any(|f| f.kind == FactKind::Response && f.key.id == "call-3"),
        "第 3 笔必须收束"
    );
    assert!(
        facts.iter().any(|f| {
            f.kind == FactKind::Sent
                && f.key.id == "call-2"
                && f.payload.get("deadline_ms") == Some(&Value::Null)
        }),
        "第 2 笔停在 sent{{deadline_ms:null}}"
    );
    assert_eq!(report.dispatched, 3);
    assert!(
        matches!(
            report.outcome,
            DriveOutcome::NeedsHuman(ref keys)
                if keys.len() == 1 && keys[0].port == "tool" && keys[0].id == "call-2"
        ),
        "结束时只剩挂起的那一笔等外部事实"
    );
}

/// 逐笔债串链后的同圈窗口：链上后一笔在「前一笔刚收束」的那一圈才首次可派，
/// 此时账上还没有它的任何事实，`when_no_open`（只读 `unresolved()`）看不见它。
/// `after_port` 的认领集合含本圈候选，两者并用才等于「这批调用全部收束」。
#[tokio::test]
async fn a_chained_debt_keeps_the_port_gate_closed_in_the_same_round() {
    // call-1 已收束；call-2 是链上的下一笔，尚未派发（账上无痕迹）。
    let seed = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new("tool", "call-1", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("tool", "call-1", 1),
            json!({"deadline_ms": 60_000}),
            0,
        ),
        Fact::new(
            FactKind::Response,
            DebtKey::new("tool", "call-1", 1),
            json!({}),
            0,
        ),
    ];
    let gate = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("tool-call", "tool", "call-1", json!({})).lease(600_000, 2),
        DebtRow::new("tool-call", "tool", "call-2", json!({}))
            .lease(600_000, 2)
            .after(DebtKey::new("tool", "call-1", 1)),
        DebtRow::new("model-after-tools", "model", "model:after:1", json!({}))
            .when_no_open("tool")
            .after_port("tool"),
    ])
    .with_executor("tool", Arc::new(Stub::ok()))
    .with_executor("model", gate.clone())
    .with_facts(seed);

    let report = drive(&ctx).await;

    // 同一圈里 call-2 首派；续接行必须等它收束后的下一圈。
    assert_eq!(report.dispatched, 2);
    assert_eq!(gate.seen().len(), 1, "续接行只在 call-2 收束后派发一次");
    let facts = ctx.snapshot();
    let call2_done = facts
        .iter()
        .position(|f| f.kind == FactKind::Response && f.key.port == "tool" && f.key.id == "call-2")
        .expect("call-2 应当收束");
    let model_request = facts
        .iter()
        .position(|f| f.kind == FactKind::Request && f.key.port == "model")
        .expect("续接行应当派发");
    assert!(
        call2_done < model_request,
        "续接行的派发必须晚于 call-2 的收束，不能与它同圈"
    );
}

/// 重试之后门控不能被旧 attempt 永久挡住：`retry` 把 attempt 1 作废，
/// `when_no_open` 只该看最新 attempt。今天缺的正是「重试 + 门控」这个组合。
#[tokio::test]
async fn retry_does_not_poison_the_when_no_open_gate() {
    // 账上已经是「attempt 1 可重试失败、retry 指向 attempt 2」的形状。
    let seed = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new("tool", "call-1", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("tool", "call-1", 1),
            json!({"deadline_ms": 60_000}),
            0,
        ),
        Fact::new(
            FactKind::Retry,
            DebtKey::new("tool", "call-1", 2),
            json!({"from": 1}),
            0,
        ),
    ];
    let gate = Arc::new(Stub::needs_human());
    let ctx = TestCtx::new(vec![
        DebtRow::new("tool-calls", "tool", "call-1", json!({})).lease(600_000, 2),
        // 工具端口还开着债就不欠；attempt 1 被 retry 取代后不该再挡住它。
        DebtRow::new("model-after-tools", "model", "model:after:1", json!({})).when_no_open("tool"),
    ])
    .with_executor("tool", Arc::new(Stub::ok()))
    .with_executor("model", gate.clone())
    .with_facts(seed);

    let report = drive(&ctx).await;

    // tool 的 attempt 2 收束后，续接行仍能欠账并派发（旧口径下它永远不会被派）。
    assert_eq!(report.dispatched, 2);
    assert_eq!(gate.seen().len(), 1);
    // 续接行挂起（执行体明确说「等你」：`sent{deadline_ms: null}`，债继续开着）⇒
    // 出口必须是「等外部事实」，而不是骗人的 `NoDebt`，也不是「在飞」——没有执行体
    // 还在跑，环自己已经推不动了。
    assert_eq!(
        report.outcome,
        DriveOutcome::NeedsHuman(vec![DebtKey::new("model", "model:after:1", 1)])
    );
}

#[tokio::test]
async fn absent_executor_lands_failed_and_names_the_port() {
    // 「孤名」= 装配期没有任何插件登记的端口名。它不能静默跳过：环仍写全协议三格，
    // 终态是 `failed` 且点名端口——这是 J6 白名单退场后真正拦住孤儿端口的那一处
    // （`scripts/obligation-ring-accept.mjs` 文件头「孤名 = 0 的拦截 owner」）。
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    let facts = ctx.snapshot();
    let kinds: Vec<_> = facts.iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Sent, FactKind::Request, FactKind::Failed]
    );
    let last = facts.last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert!(last.payload["reason"]
        .as_str()
        .unwrap()
        .contains("no provider for port p1"));
}

#[tokio::test]
async fn protocol_drafts_pair_and_world_drafts_leave_the_ring() {
    let sink = Arc::new(RecordSink::default());
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor(
            "p1",
            Arc::new(Emitting {
                world: sink.clone(),
            }),
        )
        .with_outlet(PortFactEmit::new(sink.clone()));
    let _ = drive(&ctx).await;

    let kinds: Vec<_> = ctx.snapshot().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Sent, FactKind::Request, FactKind::Response]
    );
    let drafts = sink.drafts.lock().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].kind, "note");
}

#[tokio::test]
async fn a_closed_debt_is_not_dispatched_again() {
    let stub = Arc::new(Stub::ok());
    let seed = vec![Fact::new(
        FactKind::Response,
        DebtKey::new("p1", "d", 1),
        json!({}),
        0,
    )];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", stub.clone())
        .with_facts(seed);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
    assert_eq!(ctx.snapshot().len(), 1, "已收束的债不再落新事实");
}

#[tokio::test]
async fn a_settled_session_drives_to_no_debt_without_state() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Stub::ok()));
    let first = drive(&ctx).await;
    let second = drive(&ctx).await;
    assert_eq!(first.dispatched, 1);
    assert_eq!(second.dispatched, 0);
    assert_eq!(second.outcome, DriveOutcome::NoDebt);
}

/// 分片的执行体：同一笔债 emit 三条 `response`，分片号只在载荷里。
struct Sharded;

#[async_trait::async_trait]
impl PortExecutor for Sharded {
    async fn execute(&self, ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        for shard in 1..=3u64 {
            ctx.emit
                .emit(vec![FactDraft {
                    key: format!("shard-{shard}"),
                    kind: FactKind::Response.as_str().to_string(),
                    entity: symbio_plugin_api::Entity::Scope,
                    verb: symbio_plugin_api::Verb::Asserted,
                    payload: json!({
                        "shard": shard,
                        "text": format!("chunk-{shard}"),
                        "done": shard == 3,
                    }),
                    produced_by: None,
                }])
                .await;
        }
        PortOutcome::ok()
    }
}

#[tokio::test]
async fn response_shards_land_one_fact_per_draft_under_one_key() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Sharded));
    let report = drive(&ctx).await;
    assert_eq!(report.dispatched, 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    let responses: Vec<_> = ctx
        .snapshot()
        .into_iter()
        .filter(|f| f.kind == FactKind::Response)
        .collect();
    assert_eq!(responses.len(), 3, "分片按序各成一条事实");
    for fact in &responses {
        assert_eq!(fact.key, DebtKey::new("p1", "d", 1), "配对键不含分片号");
    }
    assert_eq!(responses[0].payload["shard"], json!(1));
    assert_eq!(responses[2].payload["done"], json!(true));
}

/// 会合点：两笔必须同时在场才能过——串行派发会在这里等到超时。
struct Rendezvous {
    barrier: Arc<tokio::sync::Barrier>,
}

#[async_trait::async_trait]
impl PortExecutor for Rendezvous {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        self.barrier.wait().await;
        PortOutcome::ok()
    }
}

#[tokio::test]
async fn independent_rows_run_concurrently_within_the_cap() {
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})),
    ])
    .with_executor(
        "p1",
        Arc::new(Rendezvous {
            barrier: barrier.clone(),
        }),
    )
    .with_concurrency(2);
    let report = tokio::time::timeout(std::time::Duration::from_secs(2), drive(&ctx))
        .await
        .expect("两笔同轮并发，互不依赖的债应同时飞");

    assert_eq!(report.dispatched, 2);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
}

/// 入批顺序与装配顺序无关：同一组债换个申报次序，派发与落格序列一致。
///
/// 债表来自能力收集（`IndexMap` 插入序 = 插件挂载序）；不排序时同一组能力换个
/// 装配顺序就会换一个派发顺序，进而换一份事实次序。判据落在**落格序列**上：
/// 只断言 `dispatched` 相同不足以证明行为一致。
#[tokio::test]
async fn batch_order_depends_on_name_and_id_not_assembly_order() {
    async fn land(rows: Vec<DebtRow>) -> Vec<(FactKind, String)> {
        let stub = Arc::new(Stub::ok());
        let ctx = TestCtx::new(rows)
            .with_executor("p1", stub)
            .with_concurrency(4);
        let _ = drive(&ctx).await;
        ctx.snapshot()
            .into_iter()
            .map(|f| (f.kind, format!("{}:{}", f.key.port, f.key.id)))
            .collect()
    }

    let mut rows = vec![
        DebtRow::new("z-row", "p1", "id-z", json!({})),
        DebtRow::new("a-row", "p1", "id-a", json!({})),
        DebtRow::new("m-row", "p1", "id-m", json!({})),
    ];
    let declared_in_order = land(rows.clone()).await;
    rows.reverse();
    let declared_reversed = land(rows).await;
    assert_eq!(
        declared_in_order, declared_reversed,
        "同一组债换装配顺序后，入批与落格序列必须一致（顺序只由行名与债 id 决定）"
    );
}

#[tokio::test]
async fn the_same_identity_is_never_dispatched_twice() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "d", json!({})),
        DebtRow::new("b", "p1", "d", json!({})),
    ])
    .with_executor("p1", stub.clone())
    .with_concurrency(4);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1, "同一 (port, id, attempt) 只派发一次");
    assert_eq!(stub.seen().len(), 1);
}

#[tokio::test]
async fn waiting_collects_every_in_flight_key() {
    let seed = vec![
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "a", 1),
            json!({"deadline_ms": 60_000}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "b", 1),
            json!({"deadline_ms": 60_000}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})),
    ])
    .with_executor("p1", Arc::new(Stub::ok()))
    .with_facts(seed);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::Waiting(vec![DebtKey::new("p1", "a", 1), DebtKey::new("p1", "b", 1),])
    );
}

#[tokio::test]
async fn step_budget_stops_a_runaway_loop_and_reports_it() {
    // 每圈都派发、都会写 Retry(attempt+1)，账一直开着；把预算压到 3 步。
    let ctx = TestCtx::new(vec![
        DebtRow::new("r", "p1", "x", json!({})).lease(1_000, u32::MAX)
    ])
    .with_executor("p1", Arc::new(Stub::always_retryable()))
    .with_step_budget(3);

    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 3);
    assert!(
        matches!(report.outcome, DriveOutcome::Exhausted { steps: 3, .. }),
        "超界必须返回可见的 Exhausted，不能静默：{:?}",
        report.outcome
    );
}

#[tokio::test]
async fn a_lap_without_new_facts_is_reported_as_stalled() {
    // 这一笔有可派发的动作，但写口把所有投递都吸收了：一圈下来账上零新增。
    let ctx = TestCtx::new(vec![DebtRow::new("r", "p1", "x", json!({}))])
        .with_executor("p1", Arc::new(Stub::ok()))
        .ignoring_writes();

    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    assert!(
        matches!(report.outcome, DriveOutcome::Stalled(_)),
        "一圈零新增必须停摆并交回账，不能继续自旋：{:?}",
        report.outcome
    );
}

/// 会真的挂住的执行体：第一次进来就停在 `release` 上，直到测试放行。
struct HungOnce {
    release: Arc<tokio::sync::Notify>,
    calls: Arc<Mutex<Vec<u32>>>,
    returned: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl PortExecutor for HungOnce {
    async fn execute(&self, ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        let attempt = attempt_of(&ctx.idempotency_key);
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(attempt);
        if attempt == 1 {
            self.release.notified().await;
        }
        self.returned.store(true, Ordering::SeqCst);
        PortOutcome::ok()
    }
}

/// 租约真超时（`D-376`）：`sent` 在执行体返回**之前**落账；执行体挂死时 `expire`
/// 照样排得上并推进环；超时之后才回来的那一笔被 `(port, id, attempt)` 整笔吸收。
#[tokio::test]
async fn a_hung_executor_lets_expire_advance_and_drops_the_late_return() {
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let returned = Arc::new(AtomicBool::new(false));
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "d", json!({"q": 1})).lease(100, 1)
    ])
    .with_executor(
        "p1",
        Arc::new(HungOnce {
            release: release.clone(),
            calls: calls.clone(),
            returned: returned.clone(),
        }),
    );

    // 第一圈：执行体已经起了活并挂住，环靠「租约未过」把它当在飞，不被它挡住。
    let first = drive(&ctx).await;
    assert_eq!(first.dispatched, 1);
    assert_eq!(
        first.outcome,
        DriveOutcome::Waiting(vec![DebtKey::new("p1", "d", 1)])
    );
    assert_eq!(*calls.lock().unwrap(), vec![1], "执行体确实起了活");
    assert!(!returned.load(Ordering::SeqCst), "执行体还挂在那儿");

    // `sent` 是 before 事实：执行体返回之前就在账上，租约从它的 `at_ms` 起算。
    let sent = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Sent)
        .expect("sent 必须在执行体返回之前落账");
    assert_eq!(sent.at_ms, 0);
    assert_eq!(sent.payload["deadline_ms"], json!(100));

    // 时间前进 = 在账上落一条更晚的事实，不读墙钟。
    ctx.append(Fact::new(
        FactKind::Response,
        DebtKey::new("__clock", "__tick", 1),
        json!({}),
        1_000,
    ));

    // 第二圈：租约已过，`expire` 判死——执行体仍挂着也不妨碍环推进。
    let second = drive(&ctx).await;
    assert_eq!(second.dispatched, 0);
    assert_eq!(second.outcome, DriveOutcome::NoDebt);
    assert!(!returned.load(Ordering::SeqCst), "expire 时执行体还没返回");
    let failed = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Failed)
        .expect("租约过期必须判死");
    assert_eq!(failed.key, DebtKey::new("p1", "d", 1));
    assert_eq!(failed.payload["reason"], "lease-expired");

    // 迟归：执行体此刻才返回。整笔被幂等键吸收，不长出新事实与新债。
    let before = format!("{:?}", ctx.snapshot());
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !returned.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("放行后执行体应当返回");
    // 让后台任务的 `finish` 跑完。
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        format!("{:?}", ctx.snapshot()),
        before,
        "迟归一整笔丢弃：账上不多一条，也不长出新债"
    );
}

/// 同一份事实日志重放两次，结果逐字节相同（`D-376` 的确定性时钟判据）。
///
/// 新事实的时间戳一律取日志里的「现在」（最晚 `at_ms`），不是进程时钟——若环
/// 读墙钟，两次重放的时间戳就会漂，进而租约判定与落格序列都会不同。
#[tokio::test]
async fn replaying_the_same_fact_log_is_byte_identical() {
    async fn replay() -> Vec<Fact> {
        let ctx = TestCtx::new(vec![
            DebtRow::new("a", "p1", "d", json!({"q": 1})).lease(100, 2)
        ])
        .with_executor("p1", Arc::new(Stub::retry_once_then_ok()))
        .with_facts(vec![Fact::new(
            FactKind::Response,
            DebtKey::new("__seed", "__seed", 1),
            json!({}),
            42,
        )]);
        let _ = drive(&ctx).await;
        ctx.snapshot()
    }

    let first = replay().await;
    let second = replay().await;
    assert_eq!(
        format!("{first:?}"),
        format!("{second:?}"),
        "同一份事实日志重放两次必须逐字节相同（环不读墙钟）"
    );
    for fact in &first[1..] {
        assert_eq!(
            fact.at_ms, 42,
            "事实时间取自日志里的现在，不是墙钟：{fact:?}"
        );
    }
}

// ── 选债口按视图切点切片（`Frozen::scan_bound` 的接线）───────────────────
//
// 视图求值有长度上界（`Frozen::eval` 超 `cost` 回 `None`），而 `owed` 把 `None`
// 折成空集——「算不出来」与「没有欠账」同形。切点由声明表回答，选债口取**尾部**；
// 下面两条一个钉接线后的行为，一个钉「不切就会零债」这个前提本身。

const SLICE_SRC: &str = "slice.src";
const SLICE_EXIT: &str = "slice.exit";
const SLICE_VIEW: &str = "slice.view";
const SLICE_ROW: &str = "slice.row";

const SLICE_EVENTS: &[EventDecl] = &[
    EventDecl::new(SLICE_SRC, (Entity::Scope, Verb::Asserted)),
    EventDecl::new(SLICE_EXIT, (Entity::Scope, Verb::Asserted)),
];

const SLICE_FACTS: &[FactDecl] = &[
    FactDecl {
        name: SLICE_SRC,
        port: None,
        key: &["id"],
        expand: &[],
        fields: &["id"],
    },
    FactDecl {
        name: SLICE_EXIT,
        port: None,
        key: &["id"],
        expand: &[],
        fields: &["id"],
    },
];

/// 视图：把 `SLICE_SRC` 每条事实的 `id` 折成一条实例。
///
/// 函数体**不看条数**——超界由 `Frozen::eval` 判。于是它同时是「切对了就看得见」
/// 与「不切就整族消失」的同一份读法。
fn slice_ids(events: &[KernelEvent], _now: i64) -> Value {
    Value::Array(
        events
            .iter()
            .filter(|e| e.kind == SLICE_SRC)
            .map(|e| json!({ "id": e.payload.get("id").cloned().unwrap_or(Value::Null) }))
            .collect(),
    )
}

/// 归宿行：只回答「这个事实名有人管」，不派发（§3.4 第 7 条）。
fn cover_row(fact: &'static str) -> Decl {
    let exits: &'static [Pattern] = Box::leak(Box::new([Pattern {
        source: fact,
        key: &[],
        eq: &[],
    }]));
    Decl {
        name: fact,
        reads: &[],
        tier: LatencyTier::Reflex,
        expand: None,
        owes: exits[0],
        until: exits,
        unless: &[],
    }
}

/// 一份「视图为源头、`cost` 很小」的冻结表：事实条数可以超过视图声明。
fn declaration_world(cost: u32) -> Arc<Frozen> {
    let mut rows: Vec<Decl> = [
        symbio_kernel::event::EVENT_REQUEST,
        symbio_kernel::event::EVENT_SENT,
        symbio_kernel::event::EVENT_RESPONSE,
        symbio_kernel::event::EVENT_FAILED,
        symbio_kernel::event::EVENT_RETRY,
    ]
    .into_iter()
    .map(cover_row)
    .collect();
    let exits: &'static [Pattern] = Box::leak(Box::new([Pattern {
        source: SLICE_EXIT,
        key: &["id"],
        eq: &[],
    }]));
    rows.push(Decl {
        name: SLICE_ROW,
        reads: &[],
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: SLICE_VIEW,
            key: &["id"],
            eq: &[],
        },
        until: exits,
        unless: &[],
    });
    let views: &'static [ViewDecl] = Box::leak(Box::new([ViewDecl {
        name: SLICE_VIEW,
        ver: 1,
        reads: &[(SLICE_SRC, "id")],
        evidence: &[SLICE_SRC],
        cost,
        eval: slice_ids,
        agg: None,
    }]));
    let mut b = Builder::new();
    b.declare(
        "fixture/slice",
        PluginDeclarations {
            events: SLICE_EVENTS,
            facts: SLICE_FACTS,
            views,
            ports: &[],
            actors: vec![ActorDecl {
                name: PLUGIN_ROW,
                mode: LatencyTier::Reflex,
                writes: &[SLICE_SRC, SLICE_EXIT],
            }],
            rows: &rows,
        },
    );
    // 这一行会开账，因此必须有人跑（`D-306`：谁执行由 `owes` 与行注册表派生）。
    let row_writes: &'static [(&'static str, &'static [&'static str])] =
        Box::leak(vec![(SLICE_ROW, &[SLICE_SRC, SLICE_EXIT][..])].into_boxed_slice());
    b.seal(row_writes)
        .unwrap_or_else(|gaps| panic!("切片夹具应能冻结，实测缺口：{gaps:?}"))
}

fn slice_event(kind: &'static str, id: &str, ts: i64) -> KernelEvent {
    KernelEvent::pending(
        format!("{kind}:{id}"),
        kind,
        Entity::Scope,
        Verb::Asserted,
        "fixture",
    )
    .with_payload(json!({ "id": id }))
    .with_ts(ts)
}

/// 六条事实、视图只声明扫两条；债源 `SLICE_SRC` 压在**尾部**（最近的一条）。
fn over_cost_events() -> Vec<KernelEvent> {
    let mut events: Vec<KernelEvent> = (0..5)
        .map(|i| slice_event(SLICE_EXIT, &format!("noise-{i}"), i))
        .collect();
    events.push(slice_event(SLICE_SRC, "wanted", 5));
    events
}

/// 声明行的债源只交数据；这里给环一份带冻结表的选债输入。
struct SliceSource {
    frozen: Arc<Frozen>,
    events: Vec<KernelEvent>,
}

impl DebtSource for SliceSource {
    fn rows(&self, _facts: &FactView<'_>) -> Vec<DebtRow> {
        Vec::new()
    }

    fn declarations(&self) -> Option<DeclInput> {
        Some(DeclInput {
            frozen: self.frozen.clone(),
            events: self.events.clone(),
        })
    }
}

/// 正向：事件数超过视图 `cost` 时，视图为源头的债**仍然被选中**。
#[tokio::test]
async fn view_sourced_debt_survives_a_slice_longer_than_the_view_cost() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::with_source(Box::new(SliceSource {
        frozen: declaration_world(2),
        events: over_cost_events(),
    }))
    .with_executor(PLUGIN_ROW, stub.clone());

    let report = drive(&ctx).await;

    assert_eq!(
        report.dispatched, 1,
        "超界切片不许让视图债整族静默消失（零告警）"
    );
    let seen = stub.seen();
    assert_eq!(seen[0].0, PLUGIN_ROW);
    assert_eq!(
        seen[0].1,
        json!({"row": SLICE_ROW, "key": [["id", "wanted"]]})
    );
}

/// 反向：把**整份**切片直接交给 `owed`（不切）⇒ 视图算不出来 ⇒ 零债。
///
/// 这条钉的是缺陷形状本身，也是正向用例的前提：没有接线，正向那条也会零派发。
#[test]
fn without_the_window_the_view_yields_no_debt_at_all() {
    let frozen = declaration_world(2);
    let events = over_cost_events();

    let whole = owed(frozen.rows(), &frozen, &events, 0);
    assert!(
        whole.is_empty(),
        "整份切片超界 ⇒ eval 回 None ⇒ owed 折成空集（本缺陷的形状）"
    );

    let bound = frozen.scan_bound().expect("有视图就有切点");
    let tail = &events[events.len() - bound..];
    let windowed = owed(frozen.rows(), &frozen, tail, 0);
    assert_eq!(windowed.len(), 1, "尾部窗口内视图仍然算得出来");
    assert_eq!(windowed[0].decl.name, SLICE_ROW);
}
