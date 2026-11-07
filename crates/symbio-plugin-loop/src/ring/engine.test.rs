use super::*;
use crate::ring::facts::{DebtKey, Fact, FactKind, FactView};
use crate::ring::table::{DebtRow, DebtSource, Table};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortFactWriter, PortInput,
    PortOutcome,
};

/// 测试宿主：事实在内存里，债表与执行体由测试给定。
struct TestCtx {
    facts: Mutex<Vec<Fact>>,
    rows: Box<dyn DebtSource + Send + Sync>,
    executors: HashMap<&'static str, Arc<dyn PortExecutor>>,
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
            facts: Mutex::new(Vec::new()),
            rows: source,
            executors: HashMap::new(),
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
        self.executors.insert(port, executor);
        self
    }

    fn with_facts(mut self, facts: Vec<Fact>) -> Self {
        self.facts = Mutex::new(facts);
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
        self.snapshot()
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

    fn now_ms(&self) -> u64 {
        self.now
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
        vec![FactKind::Request, FactKind::Sent, FactKind::Response]
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
    // 环把端口名补进收束载荷（执行体不自报「我是哪个端口」）；重建面按
    // `payload.port == "model"` 判这次往返是否对读者说话，缺了它正文 / 思考节点
    // 会全部静默消失（见 `response_payloads` 的文档）。这里钉的是「补进去」。
    assert_eq!(response.payload, json!({"answer": {"q": 1}, "port": "p1"}));
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
            FactKind::Request,
            FactKind::Sent,
            FactKind::Retry,
            FactKind::Request,
            FactKind::Sent,
            FactKind::Response
        ]
    );
    let facts = ctx.snapshot();
    let view = FactView::new(&facts);
    assert_eq!(view.attempt("p1", "d"), 2);
    assert_eq!(stub.seen().len(), 2);
}

#[tokio::test]
async fn non_retryable_error_lands_failed() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Stub::non_retryable()));
    let _ = drive(&ctx).await;

    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["class"], "non_retryable");
    // 失败与成功同一条身份位：端口由环补，读侧按它判「这次往返对读者说话」。
    assert_eq!(last.payload["port"], "p1");
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
            ..Default::default()
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
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let report = drive(&ctx).await;

    assert_eq!(report.dispatched, 1);
    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert!(last.payload["reason"].as_str().unwrap().contains("p1"));
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
        vec![FactKind::Request, FactKind::Sent, FactKind::Response]
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
