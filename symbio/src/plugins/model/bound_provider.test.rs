//! `BoundProvider` 全链路彩排 —— **真实传输层**（真实 TCP + HTTP + SSE 解析）。
//!
//! 与 e2e 的 mock-llm 同形态（OpenAI Chat SSE 方言），但在 Rust 集成测试里
//! 闭环：本地 TcpListener 一次性 mock → `BoundProvider`（真实 `execute_turn`
//! 五态机 + `openai_chat` 流解析）→ ⑤ `ProviderLlmAdapter`（core 侧适配器，
//! 生产代码不认识插件，故彩排挂在本文件的宿主上）→ core `Reasoner`
//! （持 `FullModel` 令牌）→ 事件网格落 final → 不变量全绿。
//!
//! 闸门的编译期反向用例（无法在运行期测试，注释为证）：
//! `TokenIssuer::issue_reflex()` 拿到的 `RuleOnly` 传给 `generate` 是编译错误——
//! 反射层调模型不是被检测到，是编译不过（`generate` 只收 `&FullModel`）。
//! 快速档（`LatencyTier::Fast`）**没有令牌**：[D-096](README.md "D-096")
//! 把它退役了，那条「只分类不生成」的约束由 classify 插件的返回类型与工具集判据承载。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::plugins::model::bound_provider::BoundProvider;
use crate::plugins::model::model_providers::ModelProviderConfig;
use crate::plugins::model::protocols::openai_chat::OpenaiChatProtocol;
use crate::symbio_core::ProviderLlmAdapter;
use crate::symbio_core::{
    check_all, cost_ledger, render_messages, turnstate, Budget, Entity, Event, EventStore,
    PromptMessage, Seq, Store, Verb, EVENT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use crate::symbio_core::{LlmAdapter as _, TokenIssuer};

/// 走**生产同一条路径**：事件切片 → `render_messages` → `generate_timed`。
///
/// 此前这里调 `Reasoner::reply_timed`——那条链已无生产调用方（生成路径下沉进
/// `TurnRunner`），只被本文件撑着。删掉主体后测试直调端口：**覆盖面不变**，
/// 断言（真实 SSE 聚合文本 / 实测耗时为正 / final 落事件后不变量全绿）一字未改。
async fn generate_via_port(
    adapter: &ProviderLlmAdapter,
    tok: &crate::symbio_core::FullModel,
    snapshot: &[Event],
) -> (String, u64) {
    adapter
        .generate_timed(tok, &render_messages(snapshot))
        .await
        .expect("真实传输层必答")
}

/// 一次性 OpenAI-SSE mock：收一个请求、回一段流、把收到的原始请求存档。
fn spawn_mock(response: &'static str) -> (u16, Arc<Mutex<String>>) {
    spawn_mock_after(response, std::time::Duration::ZERO)
}

/// 带应答延迟的一次性 mock。延迟只给在 mock 侧、实测口径不动：`cost_ms` 以
/// 整毫秒计（`as_millis`），即答的回环往返可不足 1ms ⇒ 实测合法地落 0——
/// 「实测耗时必须为正」要成立，前提是真实往返 ≥1ms（应答延迟 5ms）。
/// SLO 校准口径（「mock 即答量的是系统自身开销」）不受影响：那条用例仍走
/// 无延迟的 [`spawn_mock`]。
fn spawn_mock_after(
    response: &'static str,
    delay: std::time::Duration,
) -> (u16, Arc<Mutex<String>>) {
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
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
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

/// 带推理增量的流：`reasoning_content` 先来、正文后到（真实模型的输出次序）。
const SSE_REASONING: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"先想\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"一下\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"答案\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// 只有推理、**被长度掐断**的流：正文 0 字符 + `finish_reason=length`。
/// 本机 24B 模型在 `max_tokens: 4096` 那一档的真实现场。
const SSE_REASONING_TRUNCATED: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"打算这样写：先读三处再列数字\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// 与上一条**只差结束原因**（`stop`）：同样只有推理，但 provider 说它说完了。
const SSE_REASONING_ONLY: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"答案是 42。\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// 工具调用流：身份 + 名字先到，参数分两片（逼出「快照 → 窄增量」的帧序）。
const SSE_TOOL: &str = concat!(
    "HTTP/1.1 200 OK\r\n",
    "Content-Type: text/event-stream\r\n",
    "Connection: close\r\n",
    "\r\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_t\",\"type\":\"function\",\"function\":{\"name\":\"vdfs_write\",\"arguments\":\"\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.md\\\"}\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"好的\"}}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n",
);

/// 收集口：**三条流分开记**——合成一条就验不出「推理 / 工具有没有走错通道」。
struct Collecting {
    text: Mutex<Vec<String>>,
    reasoning: Mutex<Vec<String>>,
    /// 工具帧按**到达序**记录——「身份帧先于增量」是 t10 判据的一半。
    tools: Mutex<Vec<ToolFrameRec>>,
}

/// 一条工具帧的断言视图（`snapshot` = 是否身份帧）。
#[derive(Debug)]
struct ToolFrameRec {
    snapshot: bool,
    id: String,
    name: Option<String>,
    text: String,
}

impl Collecting {
    fn new() -> Self {
        Collecting {
            text: Mutex::new(Vec::new()),
            reasoning: Mutex::new(Vec::new()),
            tools: Mutex::new(Vec::new()),
        }
    }
}

impl crate::symbio_core::DeltaSink for Collecting {
    fn on_delta(&self, text: &str) {
        self.text.lock().unwrap().push(text.to_string());
    }
    fn on_reasoning(&self, text: &str) {
        self.reasoning.lock().unwrap().push(text.to_string());
    }
    fn on_tool_frame(&self, frame: &crate::symbio_core::chat_node::ChatNode) {
        use crate::symbio_core::chat_node::MessageContent;
        let text = match &frame.content {
            Some(MessageContent::Text(t)) => t.clone(),
            _ => String::new(),
        };
        // 身份帧的判据用谓词（`is_tool_call`），不写 `msg_type` 变体名——工具骨架的
        // wire 形状（旧 `ToolCall` / 新 `Skeleton` + tag `tool`）两种都该认，
        // 而这里要验的是「桥只递帧不改写身份」，不是某一种写法。
        let rec = if let Some(d) = &frame.delta {
            ToolFrameRec {
                snapshot: false,
                id: frame.id.clone(),
                name: frame.tool_name().map(str::to_string),
                text: d.clone(),
            }
        } else if frame.is_tool_call() {
            ToolFrameRec {
                snapshot: true,
                id: frame.id.clone(),
                name: frame.tool_name().map(str::to_string),
                text,
            }
        } else {
            ToolFrameRec {
                snapshot: false,
                id: frame.id.clone(),
                name: frame.tool_name().map(str::to_string),
                text,
            }
        };
        self.tools.lock().unwrap().push(rec);
    }
}

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

/// 流式帧桥：SSE 正文增量逐片经 `DeltaSink` 送达；快照 + 窄帧拼起来与
/// 聚合全文一致；generate()（委托静默口）与流式路径同源（同一执行路径）。
#[tokio::test]
async fn provider_adapter_streams_text_deltas_through_bridge() {
    use crate::symbio_core::DeltaSink;

    let (port, _captured) = spawn_mock_after(SSE_OK, std::time::Duration::from_millis(5));
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let tok = TokenIssuer::issue_deep();

    let got = Arc::new(Collecting::new());
    let ask = one("写一句关于秋天的诗");
    let (text, cost_ms) = adapter
        .generate_streaming(&tok, &ask, got.clone() as Arc<dyn DeltaSink>)
        .await
        .expect("真实传输层必答");

    let frames = got.text.lock().unwrap();
    assert_eq!(
        frames.concat(),
        text,
        "增量拼接 = 聚合全文（桥不丢帧、不重复）"
    );
    assert!(
        frames.iter().any(|f| f.contains("秋天")),
        "首片快照帧的全文要过桥：{frames:?}"
    );
    assert!(
        cost_ms > 0,
        "实测耗时必须为正（mock 应答延迟 5ms ⇒ 真实往返必 ≥1ms，写死 0 即红）"
    );
    // 注：mock 每实例只收一次连接，「generate() 与流式同源」由实现保证
    // （generate = 流式路径 + SilentDeltas，见 provider_adapter.rs）。
}

/// 推理增量走**另一条通道**：`reasoning_content` 的片落 `on_reasoning`，正文落
/// `on_delta`，两条流不混。
///
/// 反向判据（改坏即红）：把 `DeltaBridge` 的推理分支去掉或误接到 `on_delta`，
/// `text` 里就会多出「先想一下」而 `reasoning` 为空——两条断言各拦一个方向。
#[tokio::test]
async fn provider_adapter_routes_reasoning_deltas_to_the_reasoning_channel() {
    use crate::symbio_core::DeltaSink;

    let (port, _captured) = spawn_mock(SSE_REASONING);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let tok = TokenIssuer::issue_deep();

    let got = Arc::new(Collecting::new());
    let ask = one("想一下再答");
    let (text, _) = adapter
        .generate_streaming(&tok, &ask, got.clone() as Arc<dyn DeltaSink>)
        .await
        .expect("真实传输层必答");

    assert_eq!(got.text.lock().unwrap().concat(), text, "正文流只装正文");
    assert_eq!(
        got.reasoning.lock().unwrap().concat(),
        "先想一下",
        "推理增量必须走 on_reasoning"
    );
    assert!(
        !text.contains("先想"),
        "推理内容不得混进正文（走错通道的形态）：{text}"
    );
}

/// 工具调用走**消息形状**的第三条出口（缺口 8 的桥接锚）：快照帧带全量身份
/// （id / name / tool_call_id / 参数至今），参数增量是窄追加，且**身份帧先到**——
/// delta 落在没有身份的节点上，前端就没有渲染语义可挂（t10 判据的原话）。
/// 桥在这里只递帧不建节点：节点 id 由转写面给出，分发方以同一 id 定格与落库。
#[tokio::test]
async fn provider_adapter_routes_tool_frames_to_the_message_channel() {
    use crate::symbio_core::DeltaSink;

    let (port, _captured) = spawn_mock(SSE_TOOL);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let tok = TokenIssuer::issue_deep();

    let got = Arc::new(Collecting::new());
    let ask = one("写个文件");
    adapter
        .generate_streaming(&tok, &ask, got.clone() as Arc<dyn DeltaSink>)
        .await
        .expect("真实传输层必答");

    let tools = got.tools.lock().unwrap();
    let snapshots: Vec<_> = tools.iter().filter(|r| r.snapshot).collect();
    let deltas: Vec<_> = tools.iter().filter(|r| !r.snapshot).collect();
    assert_eq!(
        snapshots.len(),
        1,
        "身份帧恰好一条（首片建节点，此后窄追加）：{tools:?}"
    );
    assert_eq!(
        snapshots[0].name.as_deref(),
        Some("vdfs_write"),
        "快照帧带工具名（身份字段只随快照下发）：{:?}",
        snapshots[0]
    );
    assert!(!deltas.is_empty(), "参数应有窄增量帧（不得只发全量快照）");
    // 同一节点的全部帧必须同 id——id 漂移 = 节点被建了两份（第二张卡的形态）。
    assert!(
        tools.iter().all(|r| r.id == snapshots[0].id),
        "id 漂移 ⇒ 前端出现第二张卡：{tools:?}"
    );
    // 快照先于全部增量：身份先立，delta 才有挂靠。
    let snap_pos = tools.iter().position(|r| r.snapshot).unwrap();
    assert!(
        tools[..snap_pos].iter().all(|r| r.snapshot),
        "身份帧必须先于任何参数增量到达：{tools:?}"
    );
    // 参数不丢不重：Σ快照正文 + Σ增量拼出完整 JSON（与转写面聚合一致）；
    // 首个增量的 arguments 常为空串，所以快照正文允许为空。
    let mut joined = String::new();
    for r in tools.iter().filter(|r| r.snapshot) {
        joined.push_str(&r.text);
    }
    for r in &deltas {
        joined.push_str(&r.text);
    }
    assert_eq!(
        joined, "{\"path\":\"a.md\"}",
        "快照 + Σ增量应拼出完整参数 JSON"
    );
}

/// 全链路：事件网格 → Reasoner（持令牌）→ **真实 HTTP/SSE** → final 落事件
/// → 不变量全绿 → mock 收到的请求体里确实带着我们的 prompt。
#[tokio::test]
async fn provider_adapter_drives_full_chain_over_real_http() {
    let (port, captured) = spawn_mock_after(SSE_OK, std::time::Duration::from_millis(5));
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
    // 埋点路径：generate_timed 返回 adapter 边界实测耗时（SLO 校准的数据来源）。
    let (reply, cost_ms) = generate_via_port(&adapter, &tok, &snapshot).await;
    assert!(reply.contains("秋天"), "SSE 聚合文本：{reply}");
    assert!(
        cost_ms > 0,
        "实测耗时必须为正（mock 应答延迟 5ms ⇒ 真实往返必 ≥1ms，写死 0 即红）"
    );

    // final 落事件（溯源 + 模型可观测 + **实测成本**）——不变量与收束投影照常工作。
    store
        .append(
            Event::pending(
                "f0",
                EVENT_ASSISTANT_FINAL,
                Entity::Turn,
                Verb::Closed,
                0,
                "session/main",
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
        .get("session/main")
        .expect("台账里有发言主体");
    assert_eq!(spent.spent_ms, cost_ms, "台账累计 == 实测耗时");

    // 真实边界证据：mock 收到的 HTTP 请求走了 /chat/completions，且带着 prompt。
    let request = captured.lock().unwrap().clone();
    assert!(request.contains("POST /chat/completions"), "{request}");
    assert!(request.contains("写一句关于秋天的诗"), "{request}");
    // HTTP/1.1 头名大小写不敏感（reqwest 发小写头名）——只认值。
    assert!(request.to_lowercase().contains("bearer test-key"));
}

/// 一条 user 消息的请求包（结构化入参，D-048a）。
///
/// 本文件测的是**传输层**，不是 prompt 组装，所以这里用最短形态即可——但它
/// 必须**真的**走消息数组入参，否则这个测试在签名回退到 `&str` 时仍会绿，
/// 那就等于没测到边界。
fn one(text: &str) -> Vec<PromptMessage> {
    vec![PromptMessage {
        role: "user".into(),
        text: text.into(),
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}

/// 空流：HTTP 200 但零 content ⇒ `GenerationFailed`（调用方走兜底，I3：
/// 「模型没答」不允许被当成「答了空话」落成 final）。
#[tokio::test]
async fn empty_stream_is_a_failure_not_an_empty_answer() {
    let (port, _) = spawn_mock(SSE_EMPTY);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let tok = TokenIssuer::issue_deep();
    let anything = one("任何话");
    let err = adapter
        .generate(&tok, &anything)
        .await
        .expect_err("空流必须按失败返回");
    assert!(format!("{err:?}").contains("empty"), "{err:?}");
}

/// 「思考即正文」的**这一格**：被 `max_tokens` 掐断的半段思考不是答话。
///
/// 无工具的请求（摘要 / 措辞这类静默往返）也走同一条回退，因此判据在这一层要单独立一次：
/// 两条断言是一对**只差结束原因**的流——把「一刀切关掉回退」当成修法时第二条必红，
/// 把截断当完成交付时第一条必红。会话轮那一侧的对偶判据在
/// `turn_runner.test.rs` 与 e2e `t77-length-truncation`。
#[tokio::test]
async fn truncated_reasoning_only_stream_is_not_returned_as_answer() {
    use crate::symbio_core::SilentDeltas;

    let ask = one("写一份报告");
    let tok = TokenIssuer::issue_deep();

    // ① 截断 ⇒ 失败（不是「答了半段草稿」）。
    let (port, _) = spawn_mock(SSE_REASONING_TRUNCATED);
    let adapter = ProviderLlmAdapter::new(mock_provider(port));
    let err = adapter
        .generate_streaming(&tok, &ask, Arc::new(SilentDeltas))
        .await
        .expect_err("被长度截断的思考不是答话");
    assert!(
        format!("{err:?}").contains("长度上限"),
        "失败原因要说得出是长度截断：{err:?}"
    );

    // ② 未截断 ⇒ 那段思考就是回答（回退照旧生效）。
    let (port2, _) = spawn_mock(SSE_REASONING_ONLY);
    let adapter2 = ProviderLlmAdapter::new(mock_provider(port2));
    let (text, _) = adapter2
        .generate_streaming(&tok, &ask, Arc::new(SilentDeltas))
        .await
        .expect("写完的思考就是回答");
    assert_eq!(text, "答案是 42。", "正文空但写完的思考必须仍被回退取用");
}

/// 连不上（真实网络错误）⇒ `GenerationFailed`——兜底路径的演练素材。
#[tokio::test]
async fn unreachable_endpoint_maps_to_generation_failed() {
    // 端口 1（tcpmux）几乎必然无人监听；即便个别环境有，也是真实边界行为。
    let adapter = ProviderLlmAdapter::new(mock_provider(1));
    let tok = TokenIssuer::issue_deep();
    let anything = one("任何话");
    let result = adapter.generate(&tok, &anything).await;
    assert!(result.is_err(), "不可达端点必须失败");
}

/// SLO 校准实测（G2，[plan/04 §1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证") §1.2）：
/// **系统自身开销**（不含模型推理——mock 即答）在 N 轮真实 HTTP/SSE 全链路上的
/// 分布。这个数字回答 G2 的缺口「只有预算断言，没有实测」：
///
/// - 反射档 80ms 预算里，传输 + 五态机 + SSE 解析 + 事件网格自身占多少；
/// - 断言取两档下界：**P50 ≤ 80ms**（反射档——系统开销中位数必须塞进反射档，
///   给模型留出主导份额）、**max ≤ 300ms**（快速档——最坏情况不得溢出快速档；
///   溢出说明链路自身失控，与模型无关）。
///
/// 实测值记录在 [docs/004-ref/004-slo-calibration.md](../../../../docs/004-ref/004-slo-calibration.md "RE-004-slo-calibration")。
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
        let (_, cost_ms) = generate_via_port(&adapter, &tok, &snapshot).await;
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

/// 真实端点校准（**opt-in**，默认 `#[ignore]`——CI 无凭据不跑）：
/// 完整校准 = 系统自身开销 + **模型延迟**。给一个真实 OpenAI 兼容端点即可手测：
///
/// ```text
/// SYMBIO_REHEARSE_API_BASE=https://...  \
/// SYMBIO_REHEARSE_API_KEY=sk-...        \
/// SYMBIO_REHEARSE_MODEL=...             \
/// cargo test -p symbio --lib provider_adapter -- --ignored --nocapture
/// ```
///
/// 链路与 CI 侧校准（[`reflex_tier_system_overhead_is_measured_and_bounded`]）
/// **完全同形**：事件网格（`EventStore`）→ `Reasoner`（持 `FullModel` 令牌）→
/// `ProviderLlmAdapter` → 真实端点。差异只在端点——mock 即答量的是**系统自身开销**，
/// 真实模型量的是**主导项延迟**。产出可抄进 `docs/004-ref/004-slo-calibration.md` 的报告行。
///
/// 守卫：每轮落在**深度档**（60s）内——真实模型延迟的主导项必须被四层预算接住。
///
/// 边界（2026-10-09）：本测只到**适配器边界**（`generate_timed` 的实测耗时），
/// 不驱动会话侧执行链（用户消息入格 / final 收束 / 成本入账 / 不变量校验）——
/// 那条链是 `session` 的 `TurnRunner` 职责。插件互不可见（见 `plugins/mod.rs`
/// 架构原则），模型层测试不越界去够会话层符号；执行链本身的覆盖在
/// `session/turn_runner.test.rs`（stub 适配器全链路）。
#[tokio::test]
#[ignore = "需要真实端点凭据：SYMBIO_REHEARSE_API_BASE / _API_KEY / _MODEL"]
async fn real_provider_full_calibration() {
    let api_base = std::env::var("SYMBIO_REHEARSE_API_BASE").expect("缺 SYMBIO_REHEARSE_API_BASE");
    let api_key = std::env::var("SYMBIO_REHEARSE_API_KEY").expect("缺 SYMBIO_REHEARSE_API_KEY");
    let model = std::env::var("SYMBIO_REHEARSE_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
    let rounds: usize = std::env::var("SYMBIO_REHEARSE_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);

    let cfg: ModelProviderConfig = serde_json::from_value(json!({
        "id": "rehearse-real",
        "provider": "openai",
        "api_base": api_base,
        "api_key": api_key,
        "model": model,
        "rate_limit_ms": 0
    }))
    .expect("配置合法");
    let adapter = ProviderLlmAdapter::new(Arc::new(BoundProvider::new(
        cfg,
        Arc::new(OpenaiChatProtocol),
    )));

    // 每轮新建 store（一轮一格），与 CI 侧校准同形——只把 mock 端点换成真实端点。
    let mut samples: Vec<u64> = Vec::new();
    for i in 0..rounds {
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
                .with_payload(json!({ "text": "用一句话说明什么是事件溯源。" })),
            )
            .unwrap();
        let snapshot = store.range(Seq::new(0));
        let (text, cost_ms) = generate_via_port(&adapter, &tok, &snapshot).await;
        assert!(!text.trim().is_empty(), "第 {i} 轮空回复");
        assert!(cost_ms <= 60_000, "第 {i} 轮 {cost_ms}ms 超深度档预算");
        samples.push(cost_ms);
        println!("[{i:>2}/{rounds}] {cost_ms:>6}ms  {text}");
    }

    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    let max = samples[samples.len() - 1];
    let total: u64 = samples.iter().sum();
    println!("═══ 真实端点校准（{rounds} 轮，{model}，适配器边界实测）═══");
    println!("样本：{samples:?}");
    println!("P50 = {p50}ms · P95 = {p95}ms · max = {max}ms · 总耗 = {total}ms");
    println!("（系统自身开销基线：P50 = 1ms / max = 3ms，见 docs/004-ref/004-slo-calibration.md）");
}
