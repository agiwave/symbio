//! HTTP 重试机器 —— `execute_turn` 的传输层（core `llm/` 契约的实现细节）。
//!
//! 只有 model 插件使用，故住在插件内而非 core `llm/`：
//! - HTTP 客户端单例（连接池 + 双层空闲超时）
//! - 支持中止的 POST 重试机器（[`execute_post_with_abort`] → 五态
//!   [`PostResult`]，指数退避 + `Retry-After`）
//!
//! 上层（`bound_provider::execute_turn`）负责请求体构建与序列化；本模块
//! 只管「把字节安全送到端点、把响应安全拿回来」。

use crate::plugin_error;
use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::ExecAbortSignal;
use std::sync::OnceLock;

/// SSE 流空闲超时：两次数据块之间的最大间隔。
///
/// 整体 1800s 超时覆盖不了 provider 半途挂起（连接不断、但不发任何字节）：
/// 此类请求会静默挂满 30 分钟，期间无任何日志，表现为「Turn 开始后卡死」。
/// 180s 无任何字节即判定流已死，显式报错终止，让上层走 Failed 收尾而非无限等待。
pub const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

pub fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .connect_timeout(std::time::Duration::from_secs(10))
            // 读空闲超时：与 STREAM_IDLE_TIMEOUT 双保险（连接层 + 应用层）。
            .read_timeout(STREAM_IDLE_TIMEOUT)
            .timeout(std::time::Duration::from_secs(1800)) // 整体流式请求超时
            .build()
            .expect("Failed to build shared reqwest Client")
    })
}

/// 等到中止（`abort` 已置位则立即返回）。
///
/// 收口前这里是一个 `select!` 三臂：100ms 轮询标志位 | 通道取消 | 收 Abort 帧。
/// 现在只剩一条——`ExecAbortSignal::abort` 置位的同时就唤醒等待者，**无需轮询**；
/// 而「通道关闭 ⇒ 中止」的语义改由发起方在退出时显式调用 `abort()` 承担
/// （隐式的 sender drop 换成一次命名调用，行为不变、意图更清楚）。
async fn wait_for_abort_signal(abort: &ExecAbortSignal) {
    abort.cancelled().await;
}

/// POST 结果五态（`execute_turn` 的匹配面，语义见 `bound_provider` 模块文档）。
pub enum PostResult {
    Ok(reqwest::Response),
    RetryWithoutContextId,
    Err(String),
    /// 命中限流/服务端过载且重试耗尽：携带面向用户的可读提示（不回显原始 API JSON）。
    RateLimited(String),
    Aborted,
}

/// 可瞬时恢复、值得退避重试的 HTTP 状态。
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// 计算退避时长：优先尊重服务端 `Retry-After`，否则指数退避（500ms 起，封顶 8s）。
fn backoff_delay(attempt: u32, retry_after: Option<std::time::Duration>) -> std::time::Duration {
    if let Some(d) = retry_after {
        // 服务端给的等待时间通常已合理，仍封顶到 30s 避免极端值卡死。
        return d.min(std::time::Duration::from_secs(30));
    }
    let base_ms: u64 = 500;
    let exp = base_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    std::time::Duration::from_millis(exp.min(8000))
}

/// 解析 `Retry-After` 头（仅支持整数秒形式，HTTP 日期形式极少用，忽略）。
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs = v.trim().parse::<u64>().ok()?;
    Some(std::time::Duration::from_secs(secs))
}

/// 退避等待期间持续响应中止信号，避免 abort 必须等满整个退避窗口。
async fn sleep_with_abort(d: std::time::Duration, abort: &ExecAbortSignal) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = abort.cancelled() => {}
    }
}

/// 日志用的端点标签：`host/path`（不含查询串）。
///
/// 只取「主机 + 路径」——排查时关心的是**打到了哪个端点**；完整 URL 会带上无意义的
/// 查询串，也可能包含 token 类参数（不该进日志）。解析失败时原样返回，绝不吞掉信息。
pub fn endpoint_label(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()))
        .unwrap_or_else(|| url.to_string())
}

/// 发出一次（可重试的）LLM 请求。
///
/// ## `body` 是**已序列化的字节**，不是 `Value`
///
/// 收 `&[u8]` 而不是 `&Value`，是为了让「一次序列化」这件事**在结构上成立**：
/// `.json(value)` 会在 reqwest 内部 `serde_json::to_vec` 一遍，而它位于**重试循环内**
/// （最多 `1 + MAX_RETRIES` 次发送）——同一份请求体被反复序列化，重试越多浪费越多，
/// 而重试恰恰发生在网络/服务端已经不健康的时候。
///
/// 现在由调用方序列化一次（它同时要用那份字节量出日志里的「体量」），
/// 重试只复用同一份 buffer（`to_vec()` 是一次 memcpy，比序列化便宜两个数量级）。
pub async fn execute_post_with_abort(
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: &[u8],
    abort: &ExecAbortSignal,
) -> PostResult {
    // 限流/瞬时 5xx/网络抖动：有界重试 + 指数退避，避免一次瞬时错误就中断整轮对话。
    // 重试在同一 turn 内进行（复用同一个 root_id），不会额外产生 Turn/文本节点，
    // 因此不会造成"错误刷屏"。重试耗尽才向上返回错误，由上层停止并展示重试入口。
    const MAX_RETRIES: u32 = 4;
    // Content-Type 的**真源是各协议适配器的 `get_headers`**（四个适配器都设了
    // `application/json`）。`.body(bytes)` 不像 `.json()` 那样会自己补，所以这里
    // 兜一次底——只为防后来的适配器漏掉，当前四条路径都不会走到。
    let mut headers = headers;
    if !headers.contains_key(reqwest::header::CONTENT_TYPE) {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    let started = std::time::Instant::now();
    // 「请求发起」锚点不在这里：它由调用方（`model::bound_provider::execute_turn`）
    // 在**请求体构建之后**统一打一条，便于在同一条里同时给出模型/规模/端点/体量。
    // 这里的进程起点紧随其后，仍在「等待响应头」之前，故阶段定位语义不变
    // （见 `stream.rs::parse_sse_stream` 的阶段对照表）。
    let mut attempt: u32 = 0;

    loop {
        if abort.is_aborted() {
            return PostResult::Aborted;
        }

        let result = tokio::select! {
            res = get_http_client().post(url).headers(headers.clone()).body(body.to_vec()).send() => {
                res
            },
            _ = wait_for_abort_signal(abort) => {
                return PostResult::Aborted;
            }
        };

        let response = match result {
            Err(e) => {
                // 网络层错误（连接中断 / DNS / 超时）：可瞬时恢复，退避后重试。
                attempt += 1;
                if attempt <= MAX_RETRIES {
                    let delay = backoff_delay(attempt, None);
                    plugin_warn!(
                        "model",
                        "网络错误({}), 第{}/{}次重试, 退避{:?}",
                        e,
                        attempt,
                        MAX_RETRIES,
                        delay
                    );
                    sleep_with_abort(delay, abort).await;
                    continue;
                }
                return PostResult::Err(format!("网络传输失败: {e}"));
            }
            Ok(r) => r,
        };

        if abort.is_aborted() {
            plugin_info!(
                "model",
                "[LLM] 请求在等待响应阶段被中止 (耗时 {:?})",
                started.elapsed()
            );
        }

        if response.status().is_success() {
            // ② 响应头到达日志：此后卡死则卡在「流已建立、SSE 无数据」阶段。
            plugin_info!(
                "model",
                "[LLM] 响应头到达 HTTP {} (等待 {:?}, attempt {})",
                response.status(),
                started.elapsed(),
                attempt + 1
            );
            return PostResult::Ok(response);
        }

        // 先读重试头（response 被 text() 消费后再也拿不到 header）。
        let retry_after = parse_retry_after(response.headers());

        let status = response.status();
        let err_text = response.text().await.unwrap_or_default();

        // 处理上下文失效重试（交给上层清 response_id 后重发整轮）。
        if status == 400 && err_text.contains("previous_response_not_found") {
            return PostResult::RetryWithoutContextId;
        }

        if is_retryable_status(status.as_u16()) && attempt < MAX_RETRIES {
            attempt += 1;
            let delay = backoff_delay(attempt, retry_after);
            plugin_warn!(
                "model",
                "HTTP {} 可重试, 第{}/{}次重试, 退避{:?}",
                status,
                attempt,
                MAX_RETRIES,
                delay
            );
            sleep_with_abort(delay, abort).await;
            continue;
        }

        // 不可重试，或重试耗尽：返回面向用户的友好提示（不回显原始 API JSON）。
        if status.as_u16() == 429 {
            plugin_error!(
                "model",
                "[LLM] 请求最终失败：429 限流 (总耗时 {:?}, 重试 {} 次)",
                started.elapsed(),
                attempt
            );
            return PostResult::RateLimited(
                "请求过于频繁（429 限流）。请稍后重试，或切换其他模型继续。".to_string(),
            );
        }
        if status.as_u16() >= 500 {
            plugin_error!(
                "model",
                "[LLM] 请求最终失败：HTTP {status} (总耗时 {:?}, 重试 {} 次)",
                started.elapsed(),
                attempt
            );
            return PostResult::Err(format!("模型服务暂时不可用（HTTP {status}），请稍后重试。"));
        }
        plugin_error!(
            "model",
            "[LLM] 请求最终失败：HTTP {status} (总耗时 {:?})",
            started.elapsed()
        );
        return PostResult::Err(format!("API Error ({status}): {err_text}"));
    }
}
