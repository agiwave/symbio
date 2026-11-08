//! `compact` 执行面的判据：**只压到切片边界为止**（D-401）。
//!
//! 摘要正文由 `model.assist` 交回；这里钉的是落格形状：`keep_from` / `split` 必须
//! 随快照事实一起落下，且「只压前缀」时快照里不能再补一份本轮终答（否则读面重复）。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;

use symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortFact, PortFactEmit, PortFactReader, PortFactWriter,
    PortInput, PortOutcome, PortProvider,
};
use symbio_plugin_api::{ExecAbortSignal, ExecEventSink, FactDraft, EVENT_FAILED, EVENT_RESPONSE};

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
    let (outcome, captured) = exec(payload, facts).await;
    (outcome.success(), captured)
}

async fn exec(
    payload: serde_json::Value,
    facts: Vec<PortFact>,
) -> (PortOutcome, Arc<Mutex<Vec<FactDraft>>>) {
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
    (outcome, captured)
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

/// 拿不到摘要 ⇒ **失败是产物**（D-402 ①）：原因分类、压缩节点身份、切片边界
/// 全部随失败事实交出，读面与重试都只用这一份材料。
#[tokio::test]
async fn a_missing_summary_fails_with_a_classified_fact() {
    let final_id = "chat.assistant.final:u-1";
    let after_id = "compact-summarize:u-0|1000|";
    let (outcome, captured) = exec(
        json!({
            "root_id": "u-1",
            "final_id": final_id,
            "after_port": "model.assist",
            "after_id": after_id,
            "fingerprint": "u-0|1000|",
            "from": "u-0",
            "to": final_id,
            "truncate": {},
            "keep_from": null,
            "split": false,
        }),
        vec![
            final_fact(final_id),
            fact(
                &format!("ring:model.assist:{after_id}:1:failed"),
                EVENT_FAILED,
                "model.assist",
                json!({
                    "class": "non_retryable",
                    "reason": "prompt is too long: 200000 tokens > 8192 maximum context length",
                }),
            ),
        ],
    )
    .await;

    let (class, why) = outcome.error.expect("拿不到摘要不许返回成功");
    assert_eq!(class, PortErrorClass::NonRetryable);
    assert!(why.contains("non_retryable_context_length"), "{why}");
    assert!(
        captured.lock().unwrap().is_empty(),
        "失败不落世界事实（会话快照没有变）"
    );
    assert_eq!(outcome.facts.len(), 1, "失败事实带一份可读材料");
    let payload = &outcome.facts[0];
    assert_eq!(
        payload["failure_class"],
        json!("non_retryable_context_length"),
        "超窗按已登记特征单独归类"
    );
    assert_eq!(
        payload["compression_id"],
        json!("compression:u-0|1000|"),
        "失败节点身份按前缀指纹派生（重试换纪元 ⇒ 换身份，D-402 ③）"
    );
    assert_eq!(payload["from"], json!("u-0"));
    assert_eq!(payload["to"], json!(final_id));
    assert_eq!(payload["fingerprint"], json!("u-0|1000|"));
    assert!(
        payload["text"]
            .as_str()
            .unwrap_or_default()
            .contains("non_retryable_context_length"),
        "展示面正文带分类"
    );
}

/// 没有任何上游失败、摘要就是空的 ⇒ 分类为 `non_retryable_empty_summary`，仍失败。
#[tokio::test]
async fn an_empty_summary_is_a_failure_too() {
    let final_id = "chat.assistant.final:u-1";
    let (outcome, _) = exec(
        json!({
            "root_id": "u-1",
            "final_id": final_id,
            "after_port": "model.assist",
            "after_id": "compact-summarize:u-0|1000|",
        }),
        vec![
            final_fact(final_id),
            fact(
                "summary-1",
                EVENT_RESPONSE,
                "model.assist",
                json!({ "final_id": final_id, "text": "" }),
            ),
        ],
    )
    .await;
    assert!(outcome.error.is_some(), "空摘要不许当成成功");
    assert_eq!(
        outcome.facts[0]["failure_class"],
        json!("non_retryable_empty_summary")
    );
}
