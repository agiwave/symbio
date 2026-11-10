//! 升格执行面的单测：产物形状、溯源回读、拒绝路径。
//!
//! 授权那一格在一个正常装配里恒真，没有「拒绝」用例——它的反例住 `loop` 的
//! `engine.test.rs`（`PortOutcome::Denied` ⇒ 不落草稿、落 `Failed`），因为丢弃草稿的是环。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::EVENT_CONATION_EXPRESSED;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortFact, PortFactEmit, PortFactReader, PortFactWriter,
    PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::{Entity, Event, ExecAbortSignal, ExecEventSink, LatencyTier, Seq, Verb};

use super::ConationPort;

#[derive(Default)]
struct Captured(Arc<Mutex<Vec<FactDraft>>>);

#[async_trait]
impl PortFactWriter for Captured {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.0.lock().unwrap().extend(facts);
    }
}

struct Events(Vec<Event>);

impl PortFactReader for Events {
    fn facts(&self) -> Vec<PortFact> {
        Vec::new()
    }

    fn events(&self) -> Vec<Event> {
        self.0.clone()
    }
}

fn want(id: &str, seq: u64, produced_by: Option<u64>) -> Event {
    let mut e = Event::pending(
        id,
        EVENT_CONATION_EXPRESSED,
        Entity::Conation,
        Verb::Opened,
        "session/autonomous",
    )
    .with_payload(serde_json::json!({ "goal": "把周报写了", "source_seq": 5 }));
    e.seq = Some(Seq::new(seq));
    e.produced_by = produced_by;
    e
}

fn call(events: Vec<Event>) -> (PortExecContext, PortInput, Arc<Mutex<Vec<FactDraft>>>) {
    let captured: Arc<Mutex<Vec<FactDraft>>> = Arc::new(Mutex::new(Vec::new()));
    let ctx = PortExecContext {
        abort: ExecAbortSignal::new(),
        idempotency_key: "conation:want-1:1".to_string(),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::new(Arc::new(Captured(captured.clone()))),
        ctx: None,
        subject: "conation:want-1:1".to_string(),
        group: String::new(),
        session_id: "s-1".to_string(),
        origin: None,
        scratch_dir: None,
        reader: Some(Arc::new(Events(events))),
        source: "autonomous".to_string(),
    };
    let input = PortInput {
        port: "conation".to_string(),
        payload: serde_json::json!({
            "intent_id": "want-1",
            "goal": "把周报写了",
            "source_seq": 5,
        }),
    };
    (ctx, input, captured)
}

async fn run(events: Vec<Event>) -> (PortOutcome, Arc<Mutex<Vec<FactDraft>>>) {
    let provider = ConationPort;
    let executor = provider.executor().expect("本提供者必须有执行面");
    let (ctx, input, captured) = call(events);
    let outcome = executor.execute(ctx, input).await;
    (outcome, captured)
}

#[tokio::test]
async fn the_executor_opens_one_long_task_anchored_to_the_intent() {
    let (outcome, captured) = run(vec![want("want-1", 42, Some(1))]).await;
    assert!(
        !outcome.authz.is_denied(),
        "正常装配里授权通过：{:?}",
        outcome.authz
    );
    assert!(outcome.error.is_none(), "{:?}", outcome.error);

    let drafts = captured.lock().unwrap();
    assert_eq!(drafts.len(), 1, "一条欲只升格一次");
    let draft = &drafts[0];
    assert_eq!(draft.key, "o-hb-5", "任务 id 由触发 seq 派生");
    assert_eq!(draft.kind, "task.opened");
    assert_eq!((draft.entity, draft.verb), (Entity::Task, Verb::Opened));
    assert_eq!(draft.payload["task_id"], "hb-5");
    assert_eq!(draft.payload["goal"], "把周报写了");
    assert_eq!(
        draft.payload["budget_ms"],
        LatencyTier::Autonomic.budget_ms()
    );
    assert_eq!(
        draft.produced_by,
        Some(42),
        "produced_by 指回那条欲的 seq（`conation.intents` 靠它认升格）"
    );
}

#[tokio::test]
async fn a_read_failure_is_non_retryable_and_leaves_no_draft() {
    let (outcome, captured) = run(Vec::new()).await;
    let (class, reason) = outcome.error.expect("回读不到欲事件必须失败");
    assert_eq!(class, PortErrorClass::NonRetryable);
    assert!(reason.contains("want-1"), "{reason}");
    assert!(captured.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_orphan_intent_never_promotes() {
    let (outcome, captured) = run(vec![want("want-1", 42, None)]).await;
    let (class, _) = outcome.error.expect("无溯源的欲不升格（I2）");
    assert_eq!(class, PortErrorClass::NonRetryable);
    assert!(captured.lock().unwrap().is_empty());
}

#[test]
fn the_provider_declares_the_port_and_its_executor() {
    let provider = ConationPort;
    assert_eq!(provider.ports(), &["conation"]);
    assert_eq!(provider.provider_id(), "conation");
    assert!(provider.capabilities().idempotent);
    assert!(provider.executor().is_some());
}
