//! `port.rs` 的单元测试：`port=session` 的四条行契约与 fail-closed 面。
//!
//! 三条判据（`DE-048` D-315）：
//! - 正常读取：`session-config` 回八个纯值字段，值来自交付包而不是第二份读法；
//! - 端口提供者不在：`upgrade` 失败 ⇒ `NonRetryable` 失败，不 panic、不重建；
//! - 载荷缺字段：`row` 缺 ⇒ 失败；`session-call` 缺 `op` ⇒ 失败。

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use serde_json::json;

use symbio_kernel::actors::Scope;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink, ExecTranscriptWriter};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginSimpleRequest};
use symbio_kernel::store::{EventStore, Store};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_SESSION_CONFIG,
    EVENT_USER_MESSAGE,
};
use symbio_plugin_api::port::{PortErrorClass, PortExecContext, PortFactEmit, PortFactWriter};
use symbio_plugin_api::FactDraft;
use symbio_sdk::turn::dispatch::{TurnSettings, TurnSpec, TURN_DISPATCH};
use symbio_sdk::turn::node_build::PresentationTree;

use super::*;

struct NoopWriter;

#[async_trait]
impl ExecTranscriptWriter for NoopWriter {
    async fn apply(&self, _message: symbio_sdk::chat::chat_node::ChatNode) {}
}

fn spec() -> TurnSpec {
    TurnSpec {
        session_id: "sid".to_string(),
        tool_archive_dir: std::env::temp_dir().join("tool-archives"),
        transcript_archive_dir: std::env::temp_dir().join("transcripts"),
        principal: "session/test".to_string(),
        scope: Scope::Root,
        persistent: true,
        context_window: 20,
        settings: TurnSettings {
            auto_compress: false,
            classify_enabled: false,
            compose_enabled: false,
            opening_enabled: false,
            progress_enabled: false,
            progress_interval_ms: 0,
            progress_max_per_turn: 0,
            presentation_tree: PresentationTree::Grouped,
            skill_compile_enabled: false,
            skill_fast_path: false,
            worker_force_prefix: String::new(),
            worker_keywords: String::new(),
            worker_min_chars: 0,
            task_cap: symbio_sdk::turn::dispatch::TASK_CAP,
        },
    }
}

fn dispatch(facts: Option<Arc<EventStore>>) -> Arc<TurnDispatch> {
    Arc::new(TurnDispatch {
        spec: spec(),
        facts: facts.map(|f| f as Arc<dyn Store<Event = Event>>),
        sink: ExecEventSink::silent(),
        writer: Arc::new(NoopWriter),
        abort: ExecAbortSignal::new(),
        parent: None,
        context_limit: 8192,
    })
}

/// 宿主 ctx：交付包挂在这一格（生产由编排入口设置）。
fn host(with_dispatch: bool) -> Arc<dyn PluginInvokeRequest> {
    host_with_facts(with_dispatch, None)
}

fn host_with_facts(
    with_dispatch: bool,
    facts: Option<Arc<EventStore>>,
) -> Arc<dyn PluginInvokeRequest> {
    let host = Arc::new(PluginSimpleRequest::new(None, None)) as Arc<dyn PluginInvokeRequest>;
    if with_dispatch {
        host.set(TURN_DISPATCH, dispatch(facts));
    }
    host
}

fn exec_context(with_dispatch: bool) -> PortExecContext {
    PortExecContext {
        deadline_ms: None,
        abort: ExecAbortSignal::new(),
        idempotency_key: "session-1-1".to_string(),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::silent(),
        ctx: Some(host(with_dispatch)),
        subject: 0,
        group: String::new(),
        origin: None,
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    }
}

fn executor() -> Arc<dyn PortExecutor> {
    SessionPortProvider::new(Weak::new())
        .executor()
        .expect("session 端口必须交执行面")
}

fn input(payload: serde_json::Value) -> PortInput {
    PortInput {
        port: SESSION,
        payload,
    }
}

#[tokio::test]
async fn session_config_reads_eight_fields_from_the_delivery_packet() {
    let outcome = executor()
        .execute(exec_context(true), input(json!({ ROW_KEY: ROW_CONFIG })))
        .await;
    assert!(
        outcome.error.is_none(),
        "正常读取不应失败：{:?}",
        outcome.error
    );
    let value = outcome.facts.first().expect("回体应在 facts 的第一项");
    let obj = value.as_object().expect("回体是纯值对象");
    let mut keys = obj.keys().map(String::as_str).collect::<Vec<_>>();
    keys.sort_unstable();
    let mut expected = vec![
        "session_id",
        "principal",
        "scope",
        "context_window",
        "persistent",
        "settings",
        "tool_archive_dir",
        "transcript_archive_dir",
    ];
    expected.sort_unstable();
    assert_eq!(keys, expected, "回体的键集合必须恰好是八格");
    assert_eq!(obj["session_id"], json!("sid"));
    assert_eq!(obj["principal"]["id"], json!("session/test"));
    assert_eq!(obj["scope"], json!({ "root": true }));
    assert_eq!(obj["context_window"], json!(20));
    assert_eq!(obj["persistent"], json!(true));
    assert_eq!(obj["settings"]["presentation_tree"], json!("Grouped"));
}

#[tokio::test]
async fn missing_provider_fails_closed_instead_of_panicking() {
    // 端口行在装配期收集，插件被卸载后句柄悬空：执行体升级失败即判死（不可重试）。
    let outcome = executor()
        .execute(
            exec_context(true),
            input(json!({ ROW_KEY: ROW_CALL, "op": "append-message" })),
        )
        .await;
    let error = outcome.error.expect("插件不在 ⇒ 必须失败");
    assert_eq!(error.0, PortErrorClass::NonRetryable);
    assert!(
        error.1.contains("会话插件不在"),
        "失败原因应指向失去宿主持有者：{}",
        error.1
    );
}

#[tokio::test]
async fn malformed_payload_fails_without_panicking() {
    // 缺行判别键。
    let outcome = executor()
        .execute(exec_context(true), input(json!({})))
        .await;
    let error = outcome.error.expect("缺 `row` ⇒ 必须失败");
    assert_eq!(error.0, PortErrorClass::NonRetryable);
    assert!(error.1.contains("缺 `row` 字段"), "{}", error.1);

    // `session-call` 缺 `op`：失败在插件升级之前，载荷校验先于一切副作用。
    let outcome = executor()
        .execute(exec_context(true), input(json!({ ROW_KEY: ROW_CALL })))
        .await;
    let error = outcome.error.expect("缺 `op` ⇒ 必须失败");
    assert_eq!(error.0, PortErrorClass::NonRetryable);
    assert!(error.1.contains("缺 `op` 字段"), "{}", error.1);

    // 无宿主上下文（读不到交付包）同样失败，而不是拿 `None` 继续。
    let mut ctx = exec_context(false);
    ctx.ctx = None;
    let outcome = executor()
        .execute(ctx, input(json!({ ROW_KEY: ROW_CONFIG })))
        .await;
    let error = outcome.error.expect("无宿主 ctx ⇒ 必须失败");
    assert_eq!(error.0, PortErrorClass::NonRetryable);
}

// ── P2-6 段 1：幕开启 / 收束两行 ───────────────────────────────────────────

/// 测试写口：按债的作用域盖章，把草稿落进同一份事实源。
struct StoreWriter {
    store: Arc<EventStore>,
    round: u64,
    now: i64,
}

#[async_trait]
impl PortFactWriter for StoreWriter {
    async fn emit(&self, facts: Vec<FactDraft>) {
        for draft in facts {
            let mut event = Event::pending(
                draft.key,
                draft.kind,
                draft.entity,
                draft.verb,
                self.round,
                "system",
            )
            .with_payload(draft.payload)
            .with_ts(self.now);
            if let Some(produced_by) = draft.produced_by {
                event = event.with_produced_by(produced_by);
            }
            let _ = self.store.append(event);
        }
    }
}

fn exec_context_with_facts(store: Arc<EventStore>, subject: u64) -> PortExecContext {
    let host = host_with_facts(true, Some(store.clone()));
    PortExecContext {
        deadline_ms: None,
        abort: ExecAbortSignal::new(),
        idempotency_key: format!("session-{subject}-open"),
        sink: ExecEventSink::silent(),
        emit: PortFactEmit::new(Arc::new(StoreWriter {
            store,
            round: subject,
            now: 1,
        })),
        ctx: Some(host),
        subject,
        group: String::new(),
        origin: None,
        permitted: true,
        deny_reason: String::new(),
        scratch_dir: None,
    }
}

fn seed_user_message(store: &EventStore, scope: u64, text: &str) {
    store
        .append(
            Event::pending(
                format!("u-{scope}"),
                EVENT_USER_MESSAGE,
                Entity::Scope,
                Verb::Opened,
                scope,
                "user",
            )
            .with_ts(0)
            .with_payload(json!({ "text": text, "id": format!("m-{scope}") })),
        )
        .expect("种子用户格");
}

#[tokio::test]
async fn open_new_lands_the_user_and_config_facts() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 0);
    let outcome = executor()
        .execute(
            ctx,
            input(json!({
                ROW_KEY: ROW_OPEN,
                "mode": "new",
                "round": 3,
                "text": "hi",
                "node": { "id": "m-0", "text": "hi" },
                "auto_compress": false,
            })),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "开新幕不应失败：{:?}",
        outcome.error
    );
    let value = outcome.facts.first().expect("回体");
    assert_eq!(value["scope"], json!(0));
    assert_eq!(value["user_seq"], json!(0));
    assert_eq!(value["fresh_root"], json!(true));
    assert!(value["root_id"].as_str().is_some_and(|r| !r.is_empty()));

    let events = store.range(symbio_kernel::event::Seq::new(0));
    let user = events
        .iter()
        .find(|e| e.kind == EVENT_USER_MESSAGE)
        .expect("用户格");
    assert_eq!(user.scope, 0);
    assert_eq!(user.payload["text"], json!("hi"));
    let config = events
        .iter()
        .find(|e| e.kind == EVENT_SESSION_CONFIG)
        .expect("配置格");
    assert_eq!(config.scope, 0);
    assert_eq!(config.produced_by, Some(0), "配置格溯源指向用户格");
    assert_eq!(config.payload["auto_compress"], json!(false));
    assert_eq!(config.payload["is_persistent"], json!(true));
}

#[tokio::test]
async fn open_resume_reuses_the_unclosed_turn_without_a_second_user_fact() {
    let store = Arc::new(EventStore::new());
    seed_user_message(&store, 0, "first");
    let ctx = exec_context_with_facts(store.clone(), 0);
    let outcome = executor()
        .execute(
            ctx,
            input(json!({
                ROW_KEY: ROW_OPEN,
                "mode": "resume",
                "round": 4,
                "text": "first",
                "root": "t-old",
                "replaces": "old-child",
            })),
        )
        .await;
    assert!(outcome.error.is_none(), "续写不应失败：{:?}", outcome.error);
    let value = outcome.facts.first().expect("回体");
    assert_eq!(value["scope"], json!(0));
    assert_eq!(value["user_seq"], json!(0));
    assert_eq!(value["fresh_root"], json!(false));
    assert_eq!(value["root_id"], json!("t-old"));
    assert_eq!(value["replaces"], json!("old-child"));
    assert_eq!(
        store
            .range(symbio_kernel::event::Seq::new(0))
            .iter()
            .filter(|e| e.kind == EVENT_USER_MESSAGE)
            .count(),
        1,
        "续写复用锚，不再落第二条用户格"
    );
}

#[tokio::test]
async fn finalize_lands_the_closure_fact() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 1);
    let outcome = executor()
        .execute(
            ctx,
            input(json!({
                ROW_KEY: ROW_FINALIZE,
                "scope": 1,
                "user_seq": 5,
                "reason": "final",
                "payload": { "text": "答完了" },
            })),
        )
        .await;
    assert!(outcome.error.is_none(), "收束不应失败：{:?}", outcome.error);
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let final_fact = events
        .iter()
        .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL)
        .expect("收束格");
    assert_eq!(final_fact.event_id, "f-1");
    assert_eq!(final_fact.scope, 1);
    assert_eq!(final_fact.verb, Verb::Closed);
    assert_eq!(final_fact.produced_by, Some(5));
    assert_eq!(final_fact.payload["text"], json!("答完了"));
}

#[tokio::test]
async fn finalize_fallback_lands_the_fallback_fact() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 2);
    let outcome = executor()
        .execute(
            ctx,
            input(json!({
                ROW_KEY: ROW_FINALIZE,
                "scope": 2,
                "user_seq": 9,
                "reason": "exhausted",
                "detail": "步数上界",
                "payload": {},
            })),
        )
        .await;
    assert!(outcome.error.is_none(), "兜底不应失败：{:?}", outcome.error);
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let fallback = events
        .iter()
        .find(|e| e.kind == EVENT_CHAT_ASSISTANT_FALLBACK)
        .expect("兜底格");
    assert_eq!(fallback.event_id, "fb-2");
    assert_eq!(fallback.payload["status"], json!("failed"));
    assert_eq!(fallback.payload["reason"], json!("exhausted"));
    assert_eq!(fallback.payload["detail"], json!("步数上界"));
}

#[tokio::test]
async fn finalize_denied_lands_a_denial_and_no_closure() {
    let store = Arc::new(EventStore::new());
    let mut ctx = exec_context_with_facts(store.clone(), 3);
    ctx.permitted = false;
    ctx.deny_reason = "缺 reply.first".to_string();
    let outcome = executor()
        .execute(
            ctx,
            input(json!({
                ROW_KEY: ROW_FINALIZE,
                "scope": 3,
                "user_seq": 11,
                "reason": "final",
                "payload": { "text": "不该落" },
            })),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "拒绝是正常结论：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    assert!(
        !events.iter().any(|e| e.kind == EVENT_CHAT_ASSISTANT_FINAL),
        "被拒 ⇒ 不落收束格"
    );
    let denial = events
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_AUTHZ_DENIED)
        .expect("否定事实");
    assert_eq!(denial.event_id, "deny-3-speech");
    assert_eq!(denial.payload["key"], json!(EVENT_CHAT_ASSISTANT_FINAL));
    assert_eq!(denial.payload["kind"], json!("speech"));
    assert_eq!(denial.payload["detail"], json!("缺 reply.first"));
}

// ── P2-6 段 2：轮内阶段五行（写方是会话执行体）────────────────────────────

fn stage_input(row: &str, id: &str, data: serde_json::Value) -> symbio_plugin_api::port::PortInput {
    input(json!({
        ROW_KEY: row,
        "id": id,
        "anchor": 4,
        "user_seq": 0,
        "data": data,
    }))
}

#[tokio::test]
async fn decide_lands_the_world_fact_with_the_derived_key() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 1);
    let outcome = executor()
        .execute(
            ctx,
            stage_input(
                ROW_DECIDE,
                "decided-u-1",
                json!({
                    "reason": "greeting",
                    "source": "rule",
                    "model_calls": 0,
                    "result": null,
                    "answer": null,
                    "opening": null,
                }),
            ),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "判决行不应失败：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let decided = events
        .iter()
        .find(|e| e.kind == crate::declarations::EVENT_CHAT_DECIDED)
        .expect("判决格");
    assert_eq!(
        decided.event_id, "decided-u-1",
        "幂等键由环派生、执行体原样用"
    );
    assert_eq!(decided.payload["reason"], json!("greeting"));
    assert_eq!(decided.payload["source"], json!("rule"));
    assert_eq!(decided.payload["anchor"], json!(4));
    assert_eq!(decided.produced_by, Some(0));
}

#[tokio::test]
async fn recall_lands_the_entries_with_tags_for_capability_rows() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 2);
    let outcome = executor()
        .execute(
            ctx,
            stage_input(
                ROW_RECALL,
                "recalled-u-2",
                json!({
                    "entries": [
                        { "seq": 7, "tag": "skill", "content": "技能正文" },
                        { "seq": 8, "tag": "memory", "content": "记忆正文" },
                    ],
                    "resumed": false,
                    "degraded": false,
                }),
            ),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "召回行不应失败：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let recalled = events
        .iter()
        .find(|e| e.kind == crate::declarations::EVENT_SESSION_RECALLED)
        .expect("召回格");
    assert_eq!(recalled.event_id, "recalled-u-2");
    assert_eq!(recalled.payload["entries"][0]["tag"], json!("skill"));
    assert_eq!(recalled.payload["entries"][1]["tag"], json!("memory"));
    assert_eq!(recalled.payload["resumed"], json!(false));
    assert_eq!(recalled.payload["anchor"], json!(4));
}

#[tokio::test]
async fn prepare_lands_the_assembly_fact() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 3);
    let outcome = executor()
        .execute(
            ctx,
            stage_input(
                ROW_PREPARE,
                "prepared-u-3",
                json!({ "kind": "inputs", "result": "ready", "replaces": null }),
            ),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "装配行不应失败：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let prepared = events
        .iter()
        .find(|e| e.kind == crate::declarations::EVENT_SESSION_PREPARED)
        .expect("装配格");
    assert_eq!(prepared.event_id, "prepared-u-3");
    assert_eq!(prepared.payload["kind"], json!("inputs"));
    assert_eq!(prepared.payload["result"], json!("ready"));
}

#[tokio::test]
async fn compress_lands_the_consolidation_with_the_model_call_count() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 4);
    let outcome = executor()
        .execute(
            ctx,
            stage_input(
                ROW_COMPRESS,
                "compressed-u-4",
                json!({ "messages": [{ "role": "user", "text": "摘要" }], "turns": 2, "model_calls": 1 }),
            ),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "压缩行不应失败：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let consolidated = events
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED)
        .expect("压缩格");
    assert_eq!(consolidated.event_id, "compressed-u-4");
    assert_eq!(consolidated.payload["model_calls"], json!(1));
    assert_eq!(consolidated.payload["turns"], json!(2));
}

#[tokio::test]
async fn report_lands_the_reported_fact_even_when_silent() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 5);
    let outcome = executor()
        .execute(
            ctx,
            stage_input(
                ROW_REPORT,
                "reported-u-5",
                json!({ "text": "", "tool_rounds": 2, "quiet_ms": 900, "node": null }),
            ),
        )
        .await;
    assert!(
        outcome.error.is_none(),
        "汇报行不应失败：{:?}",
        outcome.error
    );
    let events = store.range(symbio_kernel::event::Seq::new(0));
    let reported = events
        .iter()
        .find(|e| e.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED)
        .expect("汇报格");
    assert_eq!(reported.event_id, "reported-u-5");
    assert_eq!(
        reported.payload["text"],
        json!(""),
        "静默也落格：试过、没话"
    );
    assert_eq!(reported.payload["quiet_ms"], json!(900));
}

#[tokio::test]
async fn stage_rows_reject_a_payload_without_an_id() {
    let store = Arc::new(EventStore::new());
    let ctx = exec_context_with_facts(store.clone(), 6);
    let outcome = executor()
        .execute(
            ctx,
            input(json!({ ROW_KEY: ROW_DECIDE, "anchor": 4, "data": { "source": "rule" } })),
        )
        .await;
    let error = outcome.error.expect("缺 id ⇒ 必须失败");
    assert_eq!(error.0, PortErrorClass::NonRetryable);
    assert!(error.1.contains("id"), "{}", error.1);
}
