use super::*;
use crate::ring::facts::{DebtKey, Fact};
use crate::ring::ledger::Ledger;
use crate::ring::table::Table;
use serde_json::json;
use std::sync::{Arc, Mutex};
use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortFactEmit, PortFactWriter, PortInput,
    PortOutcome,
};

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
    key.rsplit(':').next().and_then(|s| s.parse().ok()).unwrap_or(1)
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

#[async_trait::async_trait]
impl PortFactWriter for RecordSink {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.drafts.lock().unwrap().extend(facts);
    }
}

fn ring_with(stub: Arc<Stub>) -> Ring {
    let mut registry = Registry::new();
    registry.register("p1", stub);
    Ring::new(registry, Limits::default())
}

#[tokio::test]
async fn one_round_writes_request_sent_response() {
    let stub = Arc::new(Stub::ok());
    let mut ring = ring_with(stub.clone());
    let table = Table::new(vec![DebtRow::new("a", "p1", "d", json!({"q": 1}))]);
    let report = ring.run(&table, 0).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt(StopReason::Settled));
    let kinds: Vec<_> = ring.ledger().facts().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Request, FactKind::Sent, FactKind::Response]
    );
    let seen = stub.seen();
    assert_eq!(seen[0].0, "p1");
    assert_eq!(seen[0].1, json!({"q": 1}));
    assert_eq!(seen[0].2, "p1:d:1");
    assert_eq!(ring.ledger().facts()[2].payload, json!({"answer": {"q": 1}}));
}

#[tokio::test]
async fn retryable_error_advances_the_lease() {
    let stub = Arc::new(Stub::retry_once_then_ok());
    let mut ring = ring_with(stub.clone());
    let table = Table::new(vec![
        DebtRow::new("a", "p1", "d", json!({})).lease(60_000, 2)
    ]);
    let report = ring.run(&table, 0).await;

    assert_eq!(report.dispatched, 2);
    let kinds: Vec<_> = ring.ledger().facts().iter().map(|f| f.kind).collect();
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
    assert_eq!(ring.ledger().attempt("p1", "d"), 2);
    assert_eq!(stub.seen().len(), 2);
}

#[tokio::test]
async fn non_retryable_error_lands_failed() {
    let stub = Arc::new(Stub::non_retryable());
    let mut ring = ring_with(stub);
    let table = Table::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let _ = ring.run(&table, 0).await;

    let last = ring.ledger().facts().last().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert_eq!(last.payload["class"], "non_retryable");
}

#[tokio::test]
async fn lease_expiry_replays_an_existing_ledger() {
    let stub = Arc::new(Stub::ok());
    let seed = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new("p1", "d", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 10}),
            0,
        ),
    ];
    let mut registry = Registry::new();
    registry.register("p1", stub.clone());
    let mut ring = Ring::with_ledger(
        Ledger::from_facts(seed),
        registry,
        Limits::default(),
    );
    let table = Table::new(vec![
        DebtRow::new("a", "p1", "d", json!({})).lease(10, 2)
    ]);
    let report = ring.run(&table, 100).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(stub.seen().len(), 1);
    assert_eq!(stub.seen()[0].2, "p1:d:2");
    let response = ring
        .ledger()
        .facts()
        .iter()
        .find(|f| f.kind == FactKind::Response)
        .unwrap();
    assert_eq!(response.key.attempt, 2);
}

#[tokio::test]
async fn a_fresh_lease_is_waited_not_expired() {
    let mut registry = Registry::new();
    registry.register("p1", Arc::new(Stub::ok()));
    let seed = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new("p1", "d", 1),
            json!({}),
            0,
        ),
        Fact::new(
            FactKind::Sent,
            DebtKey::new("p1", "d", 1),
            json!({"deadline_ms": 100}),
            0,
        ),
    ];
    let mut ring = Ring::with_ledger(Ledger::from_facts(seed), registry, Limits::default());
    let table = Table::new(vec![
        DebtRow::new("a", "p1", "d", json!({})).lease(100, 2)
    ]);
    let report = ring.run(&table, 50).await;

    assert_eq!(report.dispatched, 0);
    assert_eq!(
        report.outcome,
        DriveOutcome::Waiting(vec![DebtKey::new("p1", "d", 1)])
    );
}

#[tokio::test]
async fn budget_stops_before_the_second_dispatch() {
    let stub = Arc::new(Stub::ok());
    let mut registry = Registry::new();
    registry.register("p1", stub.clone());
    let mut ring = Ring::new(registry, Limits { max_dispatches: 1 });
    let table = Table::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p1", "b", json!({})),
    ]);
    let report = ring.run(&table, 0).await;

    assert_eq!(report.dispatched, 1);
    assert_eq!(report.outcome, DriveOutcome::NoDebt(StopReason::Budget));
    assert_eq!(stub.seen().len(), 1);
}

#[tokio::test]
async fn after_gate_orders_two_ports() {
    let first = Arc::new(Stub::ok());
    let second = Arc::new(Stub::ok());
    let mut registry = Registry::new();
    registry.register("p1", first.clone());
    registry.register("p2", second.clone());
    let mut ring = Ring::new(registry, Limits::default());
    let table = Table::new(vec![
        DebtRow::new("a", "p1", "a", json!({})),
        DebtRow::new("b", "p2", "b", json!({})).after(DebtKey::new("p1", "a", 1)),
    ]);
    let report = ring.run(&table, 0).await;

    assert_eq!(report.dispatched, 2);
    assert_eq!(first.seen().len(), 1);
    assert_eq!(second.seen().len(), 1);
}

#[tokio::test]
async fn absent_executor_lands_failed_and_names_the_port() {
    let mut ring = Ring::new(Registry::new(), Limits::default());
    let table = Table::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let report = ring.run(&table, 0).await;

    assert_eq!(report.dispatched, 1);
    let last = ring.ledger().facts().last().unwrap();
    assert_eq!(last.kind, FactKind::Failed);
    assert!(last.payload["reason"].as_str().unwrap().contains("p1"));
}

#[tokio::test]
async fn protocol_drafts_pair_and_world_drafts_leave_the_ring() {
    let sink = Arc::new(RecordSink::default());
    let stub = Arc::new(Emitting { world: sink.clone() });
    let mut registry = Registry::new();
    registry.register("p1", stub);
    let mut ring = Ring::new(registry, Limits::default())
        .with_outlet(PortFactEmit::new(sink.clone()));
    let table = Table::new(vec![DebtRow::new("a", "p1", "d", json!({}))]);
    let _ = ring.run(&table, 0).await;

    let kinds: Vec<_> = ring.ledger().facts().iter().map(|f| f.kind).collect();
    assert_eq!(
        kinds,
        vec![FactKind::Request, FactKind::Sent, FactKind::Response]
    );
    let drafts = sink.drafts.lock().unwrap();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].kind, "note");
}
