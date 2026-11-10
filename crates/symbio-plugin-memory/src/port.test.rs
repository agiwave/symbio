//! `memory` 端口执行体的判据（D-268 订正 / D-387 Q3）：合并什么、重放怎么办。
//!
//! 生产路径上没有第二处会调巩固，因此这几条就是它的**生产判据**。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use symbio_plugin_api::effects::FactDraft;
use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::port::{
    PortExecContext, PortFact, PortFactEmit, PortFactReader, PortFactWriter, PortInput,
    PortProvider,
};
use symbio_plugin_api::{Entity, Event, ExecAbortSignal, ExecEventSink, Verb, PRINCIPAL_USER};
use symbio_testkit::EventStore;
use symbio_testkit::{Seq, Store};

use super::MemoryPort;

#[derive(Default)]
struct Captured(Arc<Mutex<Vec<FactDraft>>>);

#[async_trait]
impl PortFactWriter for Captured {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.0.lock().unwrap().extend(facts);
    }
}

struct Snapshot(Vec<Event>);

impl PortFactReader for Snapshot {
    fn facts(&self) -> Vec<PortFact> {
        Vec::new()
    }

    fn events(&self) -> Vec<Event> {
        self.0.clone()
    }
}

fn encode(store: &EventStore, id: &str, content: &str, ts: i64) {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_MEMORY_ENCODED,
                Entity::Memory,
                Verb::Opened,
                PRINCIPAL_USER,
            )
            .with_ts(ts)
            .with_payload(serde_json::json!({ "content": content, "tag": "经验" })),
        )
        .expect("种子入格");
}

async fn run(events: Vec<Event>) -> (bool, Arc<Mutex<Vec<FactDraft>>>) {
    let provider = MemoryPort;
    let executor = provider.executor().expect("本提供者必须有执行面");
    let captured: Arc<Mutex<Vec<FactDraft>>> = Arc::new(Mutex::new(Vec::new()));
    let ctx = PortExecContext {
        abort: ExecAbortSignal::new(),
        idempotency_key: "memory:memory-consolidate:1".to_string(),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::new(Arc::new(Captured(captured.clone()))),
        ctx: None,
        subject: "memory:memory-consolidate:1".to_string(),
        group: String::new(),
        session_id: "s-1".to_string(),
        origin: None,
        scratch_dir: None,
        reader: Some(Arc::new(Snapshot(events))),
        source: "drive".to_string(),
    };
    let input = PortInput {
        port: "memory".to_string(),
        payload: serde_json::json!({ "tag": "经验" }),
    };
    let outcome = executor.execute(ctx, input).await;
    (outcome.success(), captured)
}

/// 活集满 4 条 ⇒ 落一笔产物 + 两条遗忘。
#[tokio::test]
async fn the_executor_merges_and_forgets_the_oldest_two() {
    let store = EventStore::new();
    encode(&store, "m1", "甲", 100);
    encode(&store, "m2", "乙", 200);
    encode(&store, "m3", "丙", 300);
    encode(&store, "m4", "丁", 400);
    let events = store.range(Seq::new(0));

    let (ok, captured) = run(events).await;
    assert!(ok);
    let drafts = captured.lock().unwrap();
    assert_eq!(drafts.len(), 3);
    assert_eq!(drafts[0].kind, EVENT_MEMORY_CONSOLIDATED);
    assert_eq!(
        drafts[0].payload.get("tag"),
        Some(&serde_json::json!("经验"))
    );
    assert!(drafts[1..].iter().all(|d| d.kind == "memory.forgotten"));
}

/// 活集不足 4 条 ⇒ 成功但什么都不写（债被还成「无需合并」，不是失败）。
#[tokio::test]
async fn the_executor_writes_nothing_below_the_threshold() {
    let store = EventStore::new();
    encode(&store, "m1", "甲", 100);
    encode(&store, "m2", "乙", 200);
    encode(&store, "m3", "丙", 300);
    let events = store.range(Seq::new(0));

    let (ok, captured) = run(events).await;
    assert!(ok);
    assert!(captured.lock().unwrap().is_empty());
}

/// 重放：同一份计划期快照再跑一次，产物键逐字相同；按事实源的键吸收，只留一笔
/// `memory.consolidated`（不会出现第二笔）。
#[tokio::test]
async fn a_replay_does_not_produce_a_second_consolidated_fact() {
    let store = EventStore::new();
    encode(&store, "m1", "甲", 100);
    encode(&store, "m2", "乙", 200);
    encode(&store, "m3", "丙", 300);
    encode(&store, "m4", "丁", 400);
    let events = store.range(Seq::new(0));

    let (_, first) = run(events.clone()).await;
    let (_, second) = run(events).await;

    // 事实源按事件键吸收重复：两跑并起来后每个键只留一份。
    let mut keys: Vec<String> = Vec::new();
    for captured in [first, second] {
        for draft in captured.lock().unwrap().iter() {
            if !keys.contains(&draft.key) {
                keys.push(draft.key.clone());
            }
        }
    }
    assert_eq!(keys.len(), 3, "两跑的键集合必须重合（1 产物 + 2 遗忘）");
    assert_eq!(
        keys.iter().filter(|key| key.starts_with("v2mc-")).count(),
        1,
        "重放不得产生第二笔 memory.consolidated"
    );
}
