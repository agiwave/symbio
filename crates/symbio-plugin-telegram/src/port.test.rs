//! `notify` 端口的注册面与执行面（DE-048 D-302）：真实插件端口，不是夹具。
//!
//! 两条断言各管一半：
//! 1. **注册面**——提供者经 `register_ports` 写进本次装配的绑定容器，按端口名取回
//!    绑定、执行面可取；
//! 2. **执行面**——执行体对一个本地 HTTP 靶子发出**真实**的 Bot API 请求。

use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use symbio_plugin_api::port::{PortBindings, PORT_BINDINGS};
use symbio_plugin_api::{
    ExecAbortSignal, ExecEventSink, PluginInvokeRequest, PluginInvokeRequestExt as _,
    PluginSimpleRequest,
};
use tokio::sync::RwLock;

fn provider(api_base: &str, token: &str, chat_id: Option<&str>) -> Arc<dyn PortProvider> {
    let cfg = TelegramConfig {
        bot_token: token.to_string(),
        api_base: api_base.to_string(),
        chat_id: chat_id.map(str::to_owned),
        ..TelegramConfig::default()
    };
    Arc::new(NotifyPort::new(
        reqwest::Client::builder().no_proxy().build().unwrap(),
        Arc::new(RwLock::new(cfg)),
    ))
}

/// 本地 HTTP 靶子：收一条请求，回 `200 {"ok":true}`，把原始请求留在 `seen`。
fn spawn_stub() -> (
    std::net::SocketAddr,
    Arc<Mutex<String>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(String::new()));
    let seen_in = Arc::clone(&seen);

    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let hdr_end = loop {
            let n = stream.read(&mut tmp).unwrap();
            assert!(n > 0, "客户端在请求头写完之前断开");
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
        };
        let content_len = buf[..hdr_end]
            .split(|&b| b == b'\n')
            .find_map(|line| {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                line.strip_prefix(&b"Content-Length:"[..])
            })
            .map(|v| {
                v.iter()
                    .filter(|b| b.is_ascii_digit())
                    .map(|b| b.to_string())
                    .collect::<String>()
                    .parse::<usize>()
                    .unwrap()
            })
            .unwrap_or(0);
        let need = hdr_end + 4 + content_len;
        while buf.len() < need {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        *seen_in.lock().unwrap() = String::from_utf8_lossy(&buf).to_string();

        let body = b"{\"ok\":true}";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            String::from_utf8_lossy(body)
        );
        stream.write_all(response.as_bytes()).unwrap();
    });

    (addr, seen, handle)
}

#[tokio::test]
async fn notify_port_registers_and_executes_for_real() {
    let (addr, seen, handle) = spawn_stub();
    let provider = provider(&format!("http://{addr}"), "TEST", Some("42"));

    // ── 注册面 ──
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let bindings = Arc::new(PortBindings::default());
    ctx.set(PORT_BINDINGS, bindings.clone());
    symbio_plugin_api::port::register_ports(&ctx, provider.as_ref()).await;

    assert_eq!(
        bindings.bound(),
        vec![NOTIFY.to_string()],
        "一个端口一条绑定"
    );
    let (owner, executor) = bindings
        .get(NOTIFY)
        .expect("无冲突")
        .expect("端口 `notify` 应就绪");
    assert_eq!(owner, "telegram", "绑定记下提供者身份（DE-047 I-6.4）");

    // ── 执行面：真实 HTTP 发送 ──
    let outcome = executor
        .execute(
            PortExecContext {
                emit: symbio_plugin_api::port::PortFactEmit::silent(),
                abort: ExecAbortSignal::new(),
                idempotency_key: "request-notify-1".to_string(),
                sink: ExecEventSink::Null,
                ctx: None,
                subject: String::new(),
                session_id: "s".to_string(),
                group: String::new(),
                origin: None,
                scratch_dir: None,
                source: String::new(),
                reader: None,
            },
            PortInput {
                port: NOTIFY.to_string(),
                // 协议键 + 端口自己的行契约（`text`；`chat_id` 走配置默认值）
                payload: serde_json::json!({
                    "port": NOTIFY,
                    "id": "n1",
                    "attempt": 1,
                    "text": "hello from the notify port",
                }),
            },
        )
        .await;
    assert!(outcome.success(), "真实发送应成功：{outcome:?}");

    handle.join().unwrap();
    let seen = seen.lock().unwrap();
    assert!(seen.contains("POST /botTEST/sendMessage"), "{seen}");
    assert!(seen.contains("hello from the notify port"), "{seen}");
    assert!(seen.contains("\"chat_id\":\"42\""), "{seen}");
}

#[tokio::test]
async fn notify_port_without_token_fails_without_network() {
    let provider = provider("http://127.0.0.1:9", "", None);
    let outcome = provider
        .executor()
        .expect("执行面与配置无关，始终交出")
        .execute(
            PortExecContext {
                emit: symbio_plugin_api::port::PortFactEmit::silent(),
                abort: ExecAbortSignal::new(),
                idempotency_key: "request-notify-2".to_string(),
                sink: ExecEventSink::Null,
                ctx: None,
                subject: String::new(),
                session_id: "s".to_string(),
                group: String::new(),
                origin: None,
                scratch_dir: None,
                source: String::new(),
                reader: None,
            },
            PortInput {
                port: NOTIFY.to_string(),
                payload: serde_json::json!({ "text": "hi", "chat_id": "42" }),
            },
        )
        .await;
    let (class, why) = outcome.error.expect("未配置 token 必须失败");
    assert_eq!(class, PortErrorClass::NonRetryable);
    assert!(why.contains("bot_token"), "{why}");
}
