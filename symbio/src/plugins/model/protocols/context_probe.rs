//! OpenAI 兼容网关的最大上下文探测（尽力而为）。
//!
//! 云端 OpenAI API 不暴露模型上下文窗口，但常见 OpenAI 兼容运行时（本地推理
//! 服务为主）会主动上报：
//! - **LM Studio**：`GET {root}/api/v0/models` → `data[].max_context_length`
//! - **Ollama（已加载）**：`GET {root}/api/ps` → `models[].context_length`
//!   （实际生效的 `num_ctx`，比模型训练窗口更真实）
//! - **Ollama（模型元信息）**：`POST {root}/api/show` → `model_info` 中
//!   `<arch>.context_length`（模型训练窗口上限）
//! - **vLLM 等**：`GET {api_base}/models` → `data[].max_model_len`
//!
//! 多个探测并发执行、取成功值的最小者；结果带 TTL 缓存（上下文窗口不常变，
//! 避免每次会话发起都付出探测往返）。全部失败返回 `None`，调用方仅用用户设置。

use std::collections::HashMap;
use std::future::Future;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::symbio_core::get_http_client;
use crate::symbio_core::ModelProvider;

/// 单个探测结果：(采集时间, 上限值)。`None` 表示服务未上报。
type ProbeEntry = (Instant, Option<u32>);

/// 探测结果缓存：`(api_base|model)` → ProbeEntry。
/// `None` 也会缓存——避免对不支持探测的云端网关每次会话都白打请求。
static PROBE_CACHE: LazyLock<RwLock<HashMap<String, ProbeEntry>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// 缓存有效期：服务端重启 / 配置调整后最迟 10 分钟生效
const CACHE_TTL: Duration = Duration::from_secs(600);

/// 单次探测请求超时：本地服务毫秒级响应，云端网关快速 404；
/// 各探测并发执行，不致阻塞会话启动太久
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// 带缓存的探测执行器：命中未过期缓存直接返回，否则执行 `probe` 并写入缓存
/// （含 `None` 结果）。
pub(crate) async fn cached_probe<F>(key: &str, probe: F) -> Option<u32>
where
    F: Future<Output = Option<u32>>,
{
    {
        let cache = PROBE_CACHE.read().await;
        if let Some((at, v)) = cache.get(key) {
            if at.elapsed() < CACHE_TTL {
                return *v;
            }
        }
    }
    let result = probe.await;
    PROBE_CACHE
        .write()
        .await
        .insert(key.to_string(), (Instant::now(), result));
    result
}

/// 服务根路径：`api_base` 通常以 `/v1` 结尾（OpenAI 风格），
/// Ollama / LM Studio 的原生端点挂在服务根上。
fn server_root(api_base: &str) -> &str {
    let trimmed = api_base.trim_end_matches('/');
    trimmed.strip_suffix("/v1").unwrap_or(trimmed)
}

fn as_ctx_u32(v: &serde_json::Value) -> Option<u32> {
    v.as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0)
}

/// Ollama 的模型标识带 `:latest` 等标签；比较时剥离标签宽松匹配
fn strip_latest(s: &str) -> &str {
    s.strip_suffix(":latest").unwrap_or(s)
}

fn ollama_name_matches(candidate: &str, model: &str) -> bool {
    strip_latest(candidate) == strip_latest(model)
}

/// LM Studio：`GET {root}/api/v0/models` → 命中模型的 `max_context_length`
async fn probe_lmstudio(root: &str, model: &str) -> Option<u32> {
    let url = format!("{root}/api/v0/models");
    let resp = get_http_client()
        .get(&url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    v.get("data")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(|i| i.as_str()) == Some(model))
        .and_then(|m| m.get("max_context_length"))
        .and_then(as_ctx_u32)
}

/// Ollama（已加载模型）：`GET {root}/api/ps` → 实际生效的 `context_length`
async fn probe_ollama_ps(root: &str, model: &str) -> Option<u32> {
    let url = format!("{root}/api/ps");
    let resp = get_http_client()
        .get(&url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    v.get("models")?
        .as_array()?
        .iter()
        .filter(|m| {
            [m.get("name"), m.get("model")]
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str())
                .any(|n| ollama_name_matches(n, model))
        })
        .filter_map(|m| m.get("context_length").and_then(as_ctx_u32))
        .min()
}

/// Ollama（模型元信息）：`POST {root}/api/show` → `model_info` 中
/// `<arch>.context_length` 的最小值（多架构键并存时保守取小）
async fn probe_ollama_show(root: &str, model: &str) -> Option<u32> {
    let url = format!("{root}/api/show");
    let resp = get_http_client()
        .post(&url)
        .timeout(PROBE_TIMEOUT)
        .json(&serde_json::json!({ "model": model }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let mut limits: Vec<u32> = Vec::new();
    // 兼容部分版本把 context_length 放在顶层
    if let Some(n) = v.get("context_length").and_then(as_ctx_u32) {
        limits.push(n);
    }
    if let Some(info) = v.get("model_info").and_then(|i| i.as_object()) {
        for (k, val) in info {
            if k.ends_with(".context_length") {
                if let Some(n) = as_ctx_u32(val) {
                    limits.push(n);
                }
            }
        }
    }
    limits.into_iter().min()
}

/// vLLM 等网关：`GET {api_base}/models` → 命中模型的 `max_model_len`
/// （部分网关亦用 `max_context_length` / `context_length` 字段名）
async fn probe_models_endpoint(api_base: &str, model: &str) -> Option<u32> {
    let url = format!("{}/models", api_base.trim_end_matches('/'));
    let resp = get_http_client()
        .get(&url)
        .timeout(PROBE_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: serde_json::Value = resp.json().await.ok()?;
    let entry = v
        .get("data")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(|i| i.as_str()) == Some(model))?;
    ["max_model_len", "max_context_length", "context_length"]
        .iter()
        .find_map(|k| entry.get(*k).and_then(as_ctx_u32))
}

/// OpenAI 兼容协议的统一探测入口：并发探测各运行时端点，取成功值的最小者。
///
/// 供 `openai_chat` / `openai_responses` 协议的 `ModelProtocol::query_context_limit`
/// 调用；结果按 `(api_base, model)` 缓存。
pub(crate) async fn probe_openai_compat_context(provider: &ModelProvider) -> Option<u32> {
    if provider.api_base.trim().is_empty() || provider.model.trim().is_empty() {
        return None;
    }
    let key = format!("{}|{}", provider.api_base, provider.model);
    cached_probe(&key, async move {
        let root = server_root(&provider.api_base);
        let (lm, ps, show, vllm) = tokio::join!(
            probe_lmstudio(root, &provider.model),
            probe_ollama_ps(root, &provider.model),
            probe_ollama_show(root, &provider.model),
            probe_models_endpoint(&provider.api_base, &provider.model),
        );
        [lm, ps, show, vllm].into_iter().flatten().min()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_root_strips_v1_suffix() {
        assert_eq!(
            server_root("http://localhost:11434/v1"),
            "http://localhost:11434"
        );
        assert_eq!(
            server_root("http://localhost:1234/v1/"),
            "http://localhost:1234"
        );
        // 无 /v1 后缀：保持原样
        assert_eq!(
            server_root("http://localhost:8080"),
            "http://localhost:8080"
        );
    }

    #[test]
    fn ollama_name_matches_strips_latest_tag() {
        assert!(ollama_name_matches("qwen2.5:latest", "qwen2.5"));
        assert!(ollama_name_matches("qwen2.5", "qwen2.5"));
        assert!(ollama_name_matches("qwen2.5:7b", "qwen2.5:7b"));
        assert!(!ollama_name_matches("qwen2.5:7b", "qwen2.5"));
    }

    #[tokio::test]
    async fn cached_probe_caches_none_results() {
        let key = "test://none-case";
        let first = cached_probe(key, async { None }).await;
        assert_eq!(first, None);
        // 第二次命中缓存：探测 future 不应再被调用（若被调用会 panic）
        let second = cached_probe(key, async { panic!("should hit cache") }).await;
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn cached_probe_caches_value() {
        let key = "test://value-case";
        let first = cached_probe(key, async { Some(8192) }).await;
        assert_eq!(first, Some(8192));
        let second = cached_probe(key, async { panic!("should hit cache") }).await;
        assert_eq!(second, Some(8192));
    }
}
