//! `compact` 执行面的判据：**只压到切片边界为止**（D-401）。
//!
//! 摘要正文由 `model.assist` 交回；这里钉的是落格形状：`keep_from` / `split` 必须
//! 随快照事实一起落下，且「只压前缀」时快照里不能再补一份本轮终答（否则读面重复）。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;
use symbio_plugin_api::port::{
    PortExecContext, PortFact, PortFactEmit, PortFactReader, PortFactWriter, PortInput,
    PortProvider,
};
use symbio_plugin_api::EVENT_RESPONSE;
use symbio_plugin_api::{ExecAbortSignal, ExecEventSink, FactDraft};

use super::{CompactPort, COMPACT_PORT};

#[derive(Default)]
struct Captured(Arc<Mutex<Vec<FactDraft>>>);

#[async_trait]
impl PortFactWriter for Captured {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.0.lock().unwrap().extend(facts);
    }
}

struct Snapshot(Vec<PortFact>);

impl PortFactReader for Snapshot {
    fn facts(&self) -> Vec<PortFact> {
        self.0.clone()
    }

    fn events(&self) -> Vec<symbio_plugin_api::Event> {
        Vec::new()
    }
}

fn fact(id: &str, kind: &str, port: &str, payload: serde_json::Value) -> PortFact {
    PortFact {
        id: id.to_string(),
        kind: kind.to_string(),
        port: port.to_string(),
        payload,
    }
}

async fn run(
    payload: serde_json::Value,
    facts: Vec<PortFact>,
) -> (bool, Arc<Mutex<Vec<FactDraft>>>) {
    let executor = CompactPort.executor().expect("本提供者必须有执行面");
    let captured: Arc<Mutex<Vec<FactDraft>>> = Arc::new(Mutex::new(Vec::new()));
    let ctx = PortExecContext {
        abort: ExecAbortSignal::new(),
        idempotency_key: "compact:compact-write:1".to_string(),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::new(Arc::new(Captured(captured.clone()))),
        ctx: None,
        subject: "compact:compact-write:1".to_string(),
        group: String::new(),
        session_id: "s-1".to_string(),
        origin: None,
        scratch_dir: None,
        reader: Some(Arc::new(Snapshot(facts))),
        source: "drive".to_string(),
    };
    let input = PortInput {
        port: COMPACT_PORT,
        payload,
    };
    let outcome = executor.execute(ctx, input).await;
    (outcome.success(), captured)
}

fn final_fact(final_id: &str) -> PortFact {
    fact(
        final_id,
        EVENT_CHAT_ASSISTANT_FINAL,
        "",
        json!({ "text": "本轮答复" }),
    )
}

fn summary_fact(final_id: &str) -> PortFact {
    fact(
        "summary-1",
        EVENT_RESPONSE,
        "model.assist",
        json!({ "final_id": final_id, "text": "摘要正文" }),
    )
}

/// 只压最旧前缀：快照只放折叠叶，`keep_from` / `split` 随事实落下。
#[tokio::test]
async fn prefix_compaction_carries_keep_from_and_split() {
    let final_id = "chat.assistant.final:u-1";
    let (ok, captured) = run(
        json!({
            "root_id": "u-1",
            "final_id": final_id,
            "after_port": "model.assist",
            "keep_from": "u-2",
            "split": true,
        }),
        vec![final_fact(final_id), summary_fact(final_id)],
    )
    .await;
    assert!(ok);
    let drafts = captured.lock().unwrap();
    assert_eq!(drafts.len(), 1);
    let payload = &drafts[0].payload;
    assert_eq!(payload["keep_from"], json!("u-2"));
    assert_eq!(payload["split"], json!(true));
    let messages = payload["messages"].as_array().expect("快照节点列表");
    assert_eq!(messages.len(), 1, "后缀由读面接着折，不在快照里补终答");
    assert_eq!(messages[0]["meta"]["split"], json!(true), "展示面留痕");
}

/// 整份都压掉（无 `keep_from`）：保留旧形状——折叠叶 + 本轮终答。
#[tokio::test]
async fn full_compaction_keeps_the_legacy_shape() {
    let final_id = "chat.assistant.final:u-1";
    let (ok, captured) = run(
        json!({
            "root_id": "u-1",
            "final_id": final_id,
            "after_port": "model.assist",
            "split": false,
        }),
        vec![final_fact(final_id), summary_fact(final_id)],
    )
    .await;
    assert!(ok);
    let drafts = captured.lock().unwrap();
    let messages = drafts[0].payload["messages"]
        .as_array()
        .expect("快照节点列表");
    assert_eq!(messages.len(), 2, "折叠叶 + 本轮终答");
    assert!(drafts[0].payload.get("keep_from").is_none());
}
