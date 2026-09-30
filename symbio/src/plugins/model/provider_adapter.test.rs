//! ProviderLlmAdapter 全链路彩排 —— **真实传输层**（真实 TCP + HTTP + SSE 解析）。
//!
//! 与 e2e 的 mock-llm 同形态（OpenAI Chat SSE 方言），但在 Rust 集成测试里
//! 闭环：本地 TcpListener 一次性 mock → `BoundProvider`（真实 `execute_turn`
//! 五态机 + `openai_chat` 流解析）→ ⑤ `ProviderLlmAdapter` → core `Reasoner`
//! （持 `FullModel` 令牌）→ 事件网格落 final → 不变量全绿。
//!
//! 闸门的编译期反向用例（无法在运行期测试，注释为证）：
//! `TokenIssuer::issue_rule_only()` 拿到的令牌**不实现 `CanGenerate`**，
//! 对它调用 `generate` 是编译错误——反射层调模型不是被检测到，是编译不过。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::ProviderLlmAdapter;
use crate::plugins::model::bound_provider::BoundProvider;
use crate::plugins::model::model_providers::ModelProviderConfig;
use crate::plugins::model::protocols::openai_chat::OpenaiChatProtocol;
use crate::symbio_core::adapters::{LlmAdapter as _, TokenIssuer};
use crate::symbio_core::{
    check_all, cost_ledger, turnstate, Budget, Entity, Event, EventStore, Reasoner, Seq, Store,
    Verb, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};

/// 一次性 OpenAI-SSE mock：收一个请求、回一段流、把收到的原始请求存档。
fn spawn_mock(response: &'static str) -> (u16, Arc<Mutex<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let captured = Arc::new(Mutex::new(String::new()));
    let cap = captured.clone();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut raw = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    // 头结束且 body 读满 Content-Length ⇒ 请求完整。
                    if let Some(pos) = find_header_end(&raw) {
                        let cl = content_length(&raw[..pos]);
                        if raw.len() >= pos + 4 + cl {
                            break;
                        }
                    }
                }
            }
        }
        *cap.lock().unwrap() = String::from_utf8_lossy(&raw).to_string();
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        // Connection: close + 落地 drop ⇒ 对端读到 EOF（流结束）。
    });
    (port, captured)
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(headers: &[u8]) -> usize {
    String::from_utf8_lossy(headers)
        .to_lowercase()
        .lines()
        .find_map(|l| {
            l.strip_prefix("content-length:")
                .map(|v| v.trim().parse().unwrap_or(0))
        })
        .unwrap_or(0)
}

const SSE_OK: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"秋天是 \"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"一封写给大地的信\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// 空流 mock：200 但没有任何 content 增量——模型"答了"却什么都没说。
const SSE_EMPTY: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

fn mock_provider(port: u16) -> Arc<BoundProvider> {
    let cfg: ModelProviderConfig = serde_json::from_value(json!({
        "id": "mock-provider",
        "provider": "openai",
        "api_base": format!("http://127.0.0.1:{port}"),
        "api_key": "test-key",
        "model": "mock-model",
        "rate_limit_ms": 0
    }))
    .expect("配置合法");
    Arc::new(BoundProvider::new(cfg, Arc::new(OpenaiChatProtocol)))
}

/// 全链路：事件网格 → Reasoner（持令牌）→ **真实 HTTP/SSE** → final 落事件
/// → 不变量全绿 → mock 收到的请求体里确实带着我们的 prompt。
#[tokio::test]
async fn provider_adapter_drives_full_chain_over_real_http() {
    let (port, captured) = spawn_mock(SSE_OK);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));

    // 彩排链路（与 S2 同形，只换适配器）：用户消息入库 → Reasoner 生成。
    let tok = TokenIssuer::issue_deep();
    let store = EventStore::new();
    store
        .append(
            Event::pending(
                "u0",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_payload(json!({ "text": "写一句关于秋天的诗" })),
        )
        .unwrap();
    let snapshot = store.range(Seq::new(0));
    // 埋点路径：reply_timed 返回 adapter 边界实测耗时（SLO 校准的数据来源）。
    let (reply, cost_ms) = Reasoner
        .reply_timed(&adapter, &tok, &snapshot)
        .await
        .expect("真实传输层必答");
    assert!(reply.contains("秋天"), "SSE 聚合文本：{reply}");
    assert!(cost_ms > 0, "实测耗时必须为正（真实网络往返）");

    // final 落事件（溯源 + 模型可观测 + **实测成本**）——不变量与收束投影照常工作。
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "agent:main",
            )
            .with_produced_by(0)
            .with_cost_ms(cost_ms)
            .with_payload(json!({ "text": reply, "model": adapter.model_id() })),
        )
        .unwrap();
    let snapshot = store.range(Seq::new(0));
    assert!(
        check_all(&snapshot).is_empty(),
        "{:?}",
        check_all(&snapshot)
    );
    let view = turnstate().apply(&snapshot, 0, Budget::generous());
    assert!(view.value.settled() && view.value.final_text.is_some());

    // 成本台账闭环（G3）：熔断判据「已耗多少」现在读的是**实测值**——
    // final 事件带的 cost_ms 经 ③ cost_ledger 累计，同一份事实源。
    let ledger = cost_ledger().apply(&snapshot, i64::MAX, Budget::generous());
    let spent = ledger
        .value
        .by_principal
        .get("agent:main")
        .expect("台账里有发言主体");
    assert_eq!(spent.spent_ms, cost_ms, "台账累计 == 实测耗时");

    // 真实边界证据：mock 收到的 HTTP 请求走了 /chat/completions，且带着 prompt。
    let request = captured.lock().unwrap().clone();
    assert!(request.contains("POST /chat/completions"), "{request}");
    assert!(request.contains("写一句关于秋天的诗"), "{request}");
    // HTTP/1.1 头名大小写不敏感（reqwest 发小写头名）——只认值。
    assert!(request.to_lowercase().contains("bearer test-key"));
}

/// 空流：HTTP 200 但零 content ⇒ `GenerationFailed`（调用方走兜底，I3：
/// 「模型没答」不允许被当成「答了空话」落成 final）。
#[tokio::test]
async fn empty_stream_is_a_failure_not_an_empty_answer() {
    let (port, _) = spawn_mock(SSE_EMPTY);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let tok = TokenIssuer::issue_deep();
    let err = adapter
        .generate(&tok, "任何话")
        .await
        .expect_err("空流必须按失败返回");
    assert!(format!("{err:?}").contains("empty"), "{err:?}");
}

/// 连不上（真实网络错误）⇒ `GenerationFailed`——兜底路径的演练素材。
#[tokio::test]
async fn unreachable_endpoint_maps_to_generation_failed() {
    // 端口 1（tcpmux）几乎必然无人监听；即便个别环境有，也是真实边界行为。
    let adapter = ProviderLlmAdapter::new(mock_provider(1));
    let tok = TokenIssuer::issue_deep();
    let result = adapter.generate(&tok, "任何话").await;
    assert!(result.is_err(), "不可达端点必须失败");
}

/// SLO 校准实测（G2，[plan/04 §1](../../../../docs/plan/04-工程落地.md) §1.2）：
/// **系统自身开销**（不含模型推理——mock 即答）在 N 轮真实 HTTP/SSE 全链路上的
/// 分布。这个数字回答 G2 的缺口「只有预算断言，没有实测」：
///
/// - 反射档 80ms 预算里，传输 + 五态机 + SSE 解析 + 事件网格自身占多少；
/// - 断言取两档下界：**P50 ≤ 80ms**（反射档——系统开销中位数必须塞进反射档，
///   给模型留出主导份额）、**max ≤ 300ms**（快速档——最坏情况不得溢出快速档；
///   溢出说明链路自身失控，与模型无关）。
///
/// 实测值记录在 [docs/plan/slo-calibration.md](../../../../docs/plan/slo-calibration.md)。
#[tokio::test]
async fn reflex_tier_system_overhead_is_measured_and_bounded() {
    let mut samples: Vec<u64> = Vec::new();
    for _ in 0..12 {
        let (port, _) = spawn_mock(SSE_OK);
        let adapter = ProviderLlmAdapter::new(mock_provider(port));
        let tok = TokenIssuer::issue_deep();
        let store = EventStore::new();
        store
            .append(
                Event::pending(
                    "u0",
                    EVENT_USER_MESSAGE,
                    Entity::Turn,
                    Verb::Opened,
                    0,
                    "user",
                )
                .with_payload(json!({ "text": "校准" })),
            )
            .unwrap();
        let snapshot = store.range(Seq::new(0));
        let (_, cost_ms) = Reasoner
            .reply_timed(&adapter, &tok, &snapshot)
            .await
            .expect("校准轮必答");
        samples.push(cost_ms);
    }
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let max = samples[samples.len() - 1];
    println!("SLO 校准样本（ms）：{samples:?}");
    println!("SLO 校准：P50 = {p50}ms，max = {max}ms（反射档预算 80 / 快速档 300）");
    assert!(
        p50 <= 80,
        "系统自身开销 P50 = {p50}ms 超反射档预算——链路自身失控，与模型无关"
    );
    assert!(
        max <= 300,
        "系统自身开销 max = {max}ms 超快速档预算——传输/解析/网格有回归"
    );
}
