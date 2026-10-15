//! `chat_loop/state.rs` 的单元测试 —— 会话上下文 / 请求快照 / 单轮状态 / 闸门结果 /
//! 退出原因 / 编排器。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。
//!
//! 覆盖两部分：
//! - `TurnRequest::new` 的**请求级默认值快照**（默认值的唯一真源是 `SessionConfig`）；
//! - `StopSignal` 的**恰好一次**契约——走真实 `fire_hook` 链路（自建 recorder 插件，
//!   不 mock 内部函数），验证显式触发 / 生命周期兜底 / 二者叠加时的幂等性。

use super::*;
use crate::symbio_core::{PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex as StdMutex;

/// 记录收到的 Hook 事件（按 `fire_hook` 的 payload 协议解析）。
#[derive(Default)]
struct HookRecorder {
    stops: StdMutex<Vec<String>>,
    others: StdMutex<usize>,
}

impl HookRecorder {
    fn stop_count(&self) -> usize {
        self.stops.lock().unwrap().len()
    }
    fn last_stop_message(&self) -> String {
        self.stops
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }
    fn other_count(&self) -> usize {
        *self.others.lock().unwrap()
    }
}

#[async_trait]
impl Plugin for HookRecorder {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("test-hook-recorder", "test-hook-recorder")
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if ctx.get(crate::symbio_core::PATH).as_deref() != Some("hook/fire") {
            *self.others.lock().unwrap() += 1;
            return Ok(PluginPayload::new(&Value::Null));
        }
        let payload: Value = ctx.payload::<Value>().ok().unwrap_or(Value::Null);
        let event = payload.get("event");
        if event
            .and_then(|e: &Value| e.get("event"))
            .and_then(|v: &Value| v.as_str())
            == Some("Stop")
        {
            let msg = event
                .and_then(|e: &Value| e.get("data"))
                .and_then(|d: &Value| d.get("last_message"))
                .and_then(|v: &Value| v.as_str())
                .unwrap_or_default()
                .to_string();
            self.stops.lock().unwrap().push(msg);
        } else {
            *self.others.lock().unwrap() += 1;
        }
        Ok(PluginPayload::new(&Value::Null))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Value::Null))
    }
}

fn text_msg(id: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn recorder_signal() -> (Arc<StopSignal>, Arc<HookRecorder>) {
    let recorder = Arc::new(HookRecorder::default());
    let parent = Some(recorder.clone() as Arc<dyn Plugin>);
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    (Arc::new(StopSignal::new(parent, ctx)), recorder)
}

/// 显式触发：携带准确末条消息，且第二次调用不再外发。
#[tokio::test]
async fn explicit_fire_is_exactly_once_and_carries_last_message() {
    let (stop, recorder) = recorder_signal();
    assert!(!stop.fired());

    let msgs = vec![text_msg("1", "user turn"), text_msg("2", "assistant turn")];
    assert!(stop.fire(&msgs).await, "首次显式触发应生效");
    assert!(!stop.fire(&msgs).await, "第二次显式触发应被幂等吞掉");
    assert!(stop.fired());

    assert_eq!(recorder.stop_count(), 1);
    assert_eq!(recorder.last_stop_message(), "assistant turn");
    assert_eq!(recorder.other_count(), 0);

    drop(stop);
    assert_eq!(recorder.stop_count(), 1, "Drop 不得重复补发");
}

/// 空 transcript：仍然触发一次，`last_message` 为空串。
#[tokio::test]
async fn explicit_fire_with_empty_transcript() {
    let (stop, recorder) = recorder_signal();
    assert!(stop.fire(&[]).await);
    assert_eq!(recorder.stop_count(), 1);
    assert_eq!(recorder.last_stop_message(), "");
}

/// 兜底触发：显式触发点未执行时补发一次（异步 detached 任务）。
#[tokio::test]
async fn fallback_fire_covers_missing_explicit_call() {
    let (stop, recorder) = recorder_signal();
    stop.fire_fallback();
    // fire_fallback 投递 detached 任务，让运行时调度若干次
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(recorder.stop_count(), 1);
    assert_eq!(recorder.last_stop_message(), "");

    // 兜底之后再显式触发也不得外发
    assert!(!stop.fire(&[text_msg("1", "late")]).await);
    assert_eq!(recorder.stop_count(), 1);
}

/// 兜底幂等：多次 `fire_fallback` 只外发一次。
#[tokio::test]
async fn fallback_fire_is_idempotent() {
    let (stop, recorder) = recorder_signal();
    stop.fire_fallback();
    stop.fire_fallback();
    stop.fire_fallback();
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(recorder.stop_count(), 1);
}

/// 最后防线：`StopSignal` 被 Drop 时补发（模拟 chat_loop 提前 return）。
#[tokio::test]
async fn drop_emits_fallback_stop() {
    let (stop, recorder) = recorder_signal();
    drop(stop);
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(recorder.stop_count(), 1);
}

/// 无父插件（standalone 会话）：仍然置位 fired，且不产生任何外发/告警噪声。
#[tokio::test]
async fn signal_without_parent_stays_silent() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let stop = Arc::new(StopSignal::new(None, ctx));
    assert!(stop.fire(&[text_msg("1", "x")]).await);
    assert!(stop.fired());
    stop.fire_fallback();
    drop(stop);
    tokio::task::yield_now().await;
}

// ── 请求级默认值快照（默认值的唯一真源是 SessionConfig）────────────────

#[test]
fn request_defaults_come_from_session_config() {
    let defaults = SessionConfig::default();
    let r = TurnRequest::new(&model_chat::Request::default());
    assert_eq!(r.auto_compress, defaults.auto_compress);
    assert_eq!(r.enable_compact_tool, defaults.enable_compact_tool);
    assert_eq!(r.tool_context_window, defaults.tool_context_window);
    assert_eq!(r.max_tool_rounds, None, "默认 = 无上限");
    assert!(r.load_history, "该字段无配置对应项：缺省即加载历史");
    assert_eq!(r.system_prompt, None);
}

#[test]
fn explicit_request_values_win_over_defaults() {
    let defaults = SessionConfig::default();
    let r = TurnRequest::new(&model_chat::Request {
        max_tool_rounds: Some(7),
        auto_compress: Some(!defaults.auto_compress),
        enable_compact_tool: Some(!defaults.enable_compact_tool),
        tool_context_window: Some(defaults.tool_context_window + 1),
        system_prompt: Some("p".into()),
        ..Default::default()
    });
    assert_eq!(r.max_tool_rounds, Some(7));
    assert_eq!(r.auto_compress, !defaults.auto_compress);
    assert_eq!(r.enable_compact_tool, !defaults.enable_compact_tool);
    assert_eq!(r.tool_context_window, defaults.tool_context_window + 1);
    assert_eq!(r.system_prompt.as_deref(), Some("p"));
}

#[test]
fn load_history_false_is_honored() {
    // 心跳等无上下文场景：`load_history = false` 必须透传，不能被默认值覆盖。
    let r = TurnRequest::new(&model_chat::Request {
        load_history: Some(false),
        ..Default::default()
    });
    assert!(!r.load_history);
}

// ── `request_actor_spec` 的五字段真值（T-107 exit ③）─────────────────────────
//
// 会话选定哪个 agent ⇒ 规格就从授权表那一行取。三个角色主体（工人 / 规划者 / 验证者）
// 逐一钉：**能力集从登记行取**（单一真源，不是空表也不是硬编码清单）、**模式按矩阵
// 谓词派生**（工人无发言权而有产物能力 ⇒ `Translator`，与对等行同形；规划者 / 验证者
// 两者皆无 ⇒ `Decider`，与 S03 §3 的 ActorSpec 行 `pattern: decider` 一致）、**预算
// 按档位**、**作用域由委派链给**（子会话 ⇒ `Child(父主体)`）。
//
// 硬编码任何一项的后果都是「表授予 A、规格带 B」静默成立——那一行就成了没人读的死记录。

/// 带 `AGENT_ID` 的请求上下文（会话选定了哪个 agent）。
fn agent_ctx(agent_id: &str) -> std::sync::Arc<dyn crate::symbio_core::PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::AGENT_ID, agent_id.to_string());
    std::sync::Arc::new(req)
}

#[test]
fn the_worker_spec_takes_its_registered_row() {
    use crate::symbio_core::{LatencyTier, Pattern, Scope};
    let spec = request_actor_spec(agent_ctx("worker").as_ref(), LatencyTier::Deep, Scope::Root);
    assert_eq!(spec.principal, "agent:worker");
    assert_eq!(
        spec.capabilities,
        vec!["produce.artifact".to_string()],
        "能力集从授权表登记行取（单一真源）"
    );
    assert_eq!(
        spec.pattern,
        Pattern::Translator,
        "无发言权而有 produce.artifact ⇒ Translator（对等行同形，S02 行与派生不冲突）"
    );
    assert_eq!(spec.budget_ms, LatencyTier::Deep.budget_ms());
    assert_eq!(spec.scope, Scope::Root);
}

#[test]
fn the_planner_and_verifier_specs_take_their_registered_rows() {
    use crate::symbio_core::{LatencyTier, Pattern, Scope};
    let planner = request_actor_spec(
        agent_ctx("planner").as_ref(),
        LatencyTier::Deep,
        Scope::Root,
    );
    assert_eq!(planner.principal, "agent:planner");
    assert_eq!(
        planner.capabilities,
        vec!["define.work".to_string()],
        "规划者的权威动作是立任务（S03 §3 grants 行）"
    );
    assert_eq!(
        planner.pattern,
        Pattern::Decider,
        "define.work 独持 ⇒ Decider（S03 §3 ActorSpec 行 pattern: decider）"
    );

    let verifier = request_actor_spec(
        agent_ctx("verifier").as_ref(),
        LatencyTier::Fast,
        Scope::Root,
    );
    assert_eq!(verifier.principal, "agent:verifier");
    assert_eq!(
        verifier.capabilities,
        vec!["assert.verification".to_string()],
        "验证者的权威动作是独立判定（S03 §3 grants 行）"
    );
    assert_eq!(verifier.pattern, Pattern::Decider);
    assert_eq!(verifier.budget_ms, LatencyTier::Fast.budget_ms());
}

#[test]
fn a_delegated_worker_spec_is_scoped_to_its_parent() {
    use crate::symbio_core::{LatencyTier, Scope};
    // 委派链上（agent_run 派生的子会话）：作用域挂父主体——「委派不放大权威」的
    // 规格侧形态（T-105）。工人那一行不变，变的是 scope。
    let spec = request_actor_spec(
        agent_ctx("worker").as_ref(),
        LatencyTier::Deep,
        Scope::Child("agent:main".to_string()),
    );
    assert_eq!(spec.scope, Scope::Child("agent:main".to_string()));
    assert_eq!(spec.capabilities, vec!["produce.artifact".to_string()]);
}
