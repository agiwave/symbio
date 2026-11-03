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
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink, ExecTranscriptWriter};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginSimpleRequest};
use symbio_plugin_api::port::{PortErrorClass, PortExecContext, PortFactEmit};
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
            progress_min_rounds: 0,
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

fn dispatch() -> Arc<TurnDispatch> {
    Arc::new(TurnDispatch {
        spec: spec(),
        facts: None,
        sink: ExecEventSink::silent(),
        writer: Arc::new(NoopWriter),
        abort: ExecAbortSignal::new(),
        parent: None,
        context_limit: 8192,
    })
}

/// 宿主 ctx：交付包挂在这一格（生产由编排入口设置）。
fn host(with_dispatch: bool) -> Arc<dyn PluginInvokeRequest> {
    let host = Arc::new(PluginSimpleRequest::new(None, None)) as Arc<dyn PluginInvokeRequest>;
    if with_dispatch {
        host.set(TURN_DISPATCH, dispatch());
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
