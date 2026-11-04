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
        }
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
}

impl Stub {
    fn ok() -> Self {
        Self {
            seen: Mutex::new(Seen::default()),
            retry_until: 1,
            non_retryable: false,
        }
    }

    fn retry_once_then_ok() -> Self {
        Self {
            retry_until: 2,
            ..Self::ok()
        }
    }

    fn non_retryable() -> Self {
        Self {
            non_retryable: true,
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

const S: u64 = 7;

#[tokio::test]
async fn one_round_writes_request_sent_response() {
    let stub = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({"q": 1}))])
        .with_executor("p1", stub.clone());
    let report = drive(&ctx, S).await;

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
    assert_eq!(seen[0].2, format!("{S}:p1:d:1"));
    let response = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Response)
        .unwrap();
    assert_eq!(response.key.scope, S);
    assert_eq!(response.payload, json!({"answer": {"q": 1}}));
}

#[tokio::test]
async fn retryable_error_advances_the_lease() {
    let stub = Arc::new(Stub::retry_once_then_ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "d", json!({})).lease(60_000, 2)
    ])
    .with_executor("p1", stub.clone());
    let report = drive(&ctx, S).await;

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
    assert_eq!(view.attempt(S, "p1", "d"), 2);
    assert_eq!(stub.seen().len(), 2);
}

#[tokio::test]
async fn non_retryable_error_lands_failed() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Stub::non_retryable()));
    let _ = drive(&ctx, S).await;

    let last = ctx.snapshot().last().cloned().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["class"], "non_retryable");
}

#[tokio::test]
async fn lease_expiry_replays_an_existing_ledger() {
    let stub = Arc::new(Stub::ok());
    let seed = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new(S, "p1", "d", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new(S, "p1", "d", 1),
            json!({"deadline_ms": 10}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({})).lease(10, 2)])
        .with_executor("p1", stub.clone())
        .with_facts(seed)
        .at(100);
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(stub.seen().len(), 1);
    assert_eq!(stub.seen()[0].2, format!("{S}:p1:d:2"));
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
        Fact::new(
            FactKind::Request,
            DebtKey::new(S, "p1", "d", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new(S, "p1", "d", 1),
            json!({"deadline_ms": 100}),
            0,
        ),
    ];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({})).lease(100, 2)])
        .with_executor("p1", Arc::new(Stub::ok()))
        .with_facts(seed)
        .at(50);
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::Waiting(vec![DebtKey::new(S, "p1", "d", 1)])
    );
}

#[tokio::test]
async fn after_gate_orders_two_ports() {
    let first = Arc::new(Stub::ok());
    let second = Arc::new(Stub::ok());
    let ctx = TestCtx::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p2", "b", json!({})).after(DebtKey::new(S, "p1", "a", 1)),
    ])
    .with_executor("p1", first.clone())
    .with_executor("p2", second.clone());
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 2);
    assert_eq!(first.seen().len(), 1);
    assert_eq!(second.seen().len(), 1);
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
        fn rows(&self, _facts: &FactView<'_>, _scope: u64) -> Vec<DebtRow> {
            self.rows.clone()
        }

        fn exists(&self, kind: &str, _scope: u64) -> bool {
            self.kinds.contains(&kind)
        }

        fn exists_id(&self, kind: &str, id: &str, _scope: u64) -> bool {
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
    assert_eq!(drive(&ctx, S).await.dispatched, 2);

    // 种类事实存在 ⇒ 排除行不欠（非五类事实也算，「同一条读面」）。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: vec!["turn.done"],
        ids: Vec::new(),
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx, S).await.dispatched, 1);

    // 身份级：只有该种类、该身份的事实命中；环账上同 id 的 request / sent / retry
    // 因种类不同不命中（可重试失败后的重试轮不会被提前跳过）。
    let rows = vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})).unless_id("turn.done", "turn.done:b"),
    ];
    let ledger = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new(S, "p1", "b", 1),
            json!({}),
            0,
        ),
        Fact::new(FactKind::Sent, DebtKey::new(S, "p1", "b", 1), json!({}), 0),
        Fact::new(FactKind::Retry, DebtKey::new(S, "p1", "b", 2), json!({}), 0),
    ];
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: Vec::new(),
        ids: Vec::new(),
    }))
    .with_executor("p1", Arc::new(Stub::ok()))
    .with_facts(ledger);
    assert_eq!(
        drive(&ctx, S).await.dispatched,
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
    assert_eq!(drive(&ctx, S).await.dispatched, 1);
    assert_eq!(drive(&ctx, S).await.outcome, DriveOutcome::NoDebt);
}

#[tokio::test]
async fn when_gate_asks_the_host_whether_the_kind_exists() {
    /// 数据源把「存在的种类」当数据；`exists` 由宿主实现，环只问。
    struct Gated {
        rows: Vec<DebtRow>,
        kinds: Vec<&'static str>,
    }

    impl DebtSource for Gated {
        fn rows(&self, _facts: &FactView<'_>, _scope: u64) -> Vec<DebtRow> {
            self.rows.clone()
        }

        fn exists(&self, kind: &str, scope: u64) -> bool {
            scope == S && self.kinds.iter().any(|known| *known == kind)
        }
    }

    let rows = vec![DebtRow::new("a", "p1", "a", json!({})).when("user.message")];
    // 种类不存在 ⇒ 本行不欠。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows: rows.clone(),
        kinds: vec![],
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    let report = drive(&ctx, S).await;
    assert_eq!(report.dispatched, 0);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    // 种类存在 ⇒ 本行可还。
    let ctx = TestCtx::with_source(Box::new(Gated {
        rows,
        kinds: vec!["user.message"],
    }))
    .with_executor("p1", Arc::new(Stub::ok()));
    let report = drive(&ctx, S).await;
    assert_eq!(report.dispatched, 1);
}

#[tokio::test]
async fn when_no_open_gate_reads_debt_state() {
    let rows = vec![DebtRow::new("m", "p1", "m", json!({})).when_no_open("p1")];
    // 该端口没有未还债 ⇒ 本行可还。
    let ctx = TestCtx::new(rows.clone()).with_executor("p1", Arc::new(Stub::ok()));
    assert_eq!(drive(&ctx, S).await.dispatched, 1);

    // 该端口已有一笔未收束的 request ⇒ 本行不欠。
    let ctx = TestCtx::new(rows).with_executor("p1", Arc::new(Stub::ok()));
    ctx.facts.lock().unwrap().push(Fact::new(
        FactKind::Request,
        DebtKey::new(S, "p1", "old", 1),
        json!({}),
        0,
    ));
    let report = drive(&ctx, S).await;
    assert_eq!(report.dispatched, 0);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);
}

#[tokio::test]
async fn absent_executor_lands_failed_and_names_the_port() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let report = drive(&ctx, S).await;

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
    let _ = drive(&ctx, S).await;

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
async fn scopes_do_not_cross() {
    let stub = Arc::new(Stub::ok());
    let seed = vec![Fact::new(
        FactKind::Response,
        DebtKey::new(9, "p1", "d", 1),
        json!({}),
        0,
    )];
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", stub.clone())
        .with_facts(seed);
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 1);
    let response = ctx
        .snapshot()
        .into_iter()
        .find(|f| f.kind == FactKind::Response && f.key.scope == S)
        .unwrap();
    assert_eq!(response.key.id, "d");
}

#[tokio::test]
async fn a_settled_scope_drives_to_no_debt_without_state() {
    let ctx = TestCtx::new(vec![DebtRow::new("a", "p1", "d", json!({}))])
        .with_executor("p1", Arc::new(Stub::ok()));
    let first = drive(&ctx, S).await;
    let second = drive(&ctx, S).await;
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
    let report = drive(&ctx, S).await;
    assert_eq!(report.dispatched, 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt);

    let responses: Vec<_> = ctx
        .snapshot()
        .into_iter()
        .filter(|f| f.kind == FactKind::Response)
        .collect();
    assert_eq!(responses.len(), 3, "分片按序各成一条事实");
    for fact in &responses {
        assert_eq!(fact.key, DebtKey::new(S, "p1", "d", 1), "配对键不含分片号");
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
    let report = tokio::time::timeout(std::time::Duration::from_secs(2), drive(&ctx, S))
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
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 1, "同一 (port, id, attempt) 只派发一次");
    assert_eq!(stub.seen().len(), 1);
}

#[tokio::test]
async fn waiting_collects_every_in_flight_key() {
    let seed = vec![
        Fact::new(
            FactKind::Sent,
            DebtKey::new(S, "p1", "a", 1),
            json!({"deadline_ms": 60_000}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new(S, "p1", "b", 1),
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
    let report = drive(&ctx, S).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::Waiting(vec![
            DebtKey::new(S, "p1", "a", 1),
            DebtKey::new(S, "p1", "b", 1),
        ])
    );
}
