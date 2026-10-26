//! `StopSignal` 的**恰好一次**契约——走真实 `fire_hook` 链路（自建 recorder 插件，
//! 不 mock 内部函数），验证显式触发 / 生命周期兜底 / 二者叠加时的幂等性。

use super::*;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Mutex as StdMutex;
use symbio_core::{PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest};

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
        if ctx.get(symbio_core::PATH).as_deref() != Some("hook/fire") {
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
                .and_then(|e| e.get("data"))
                .and_then(|d| d.get("last_message"))
                .and_then(|v| v.as_str())
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

fn recorder_signal() -> (Arc<StopSignal>, Arc<HookRecorder>) {
    let recorder = Arc::new(HookRecorder::default());
    let parent = Some(recorder.clone() as Arc<dyn Plugin>);
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    (Arc::new(StopSignal::new(parent, ctx)), recorder)
}

/// 显式触发：携带驱动器交回的末条正文，且第二次调用不再外发。
#[tokio::test]
async fn explicit_fire_is_exactly_once_and_carries_last_message() {
    let (stop, recorder) = recorder_signal();
    assert!(!stop.fired());

    assert!(
        stop.fire_text("assistant turn".to_string()).await,
        "首次显式触发应生效"
    );
    assert!(
        !stop.fire_text("assistant turn".to_string()).await,
        "第二次显式触发应被幂等吞掉"
    );
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
    assert!(stop.fire_text(String::new()).await);
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
    assert!(!stop.fire_text("late".to_string()).await);
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

/// 最后防线：`StopSignal` 被 Drop 时补发（模拟消费循环提前 return）。
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
    assert!(stop.fire_text("x".to_string()).await);
    assert!(stop.fired());
    stop.fire_fallback();
    drop(stop);
    tokio::task::yield_now().await;
}
