//! 共享入站策略：只读调用白名单与请求重放防护。
//!
//! 这里只放两个 ingress 必须同形、又不依赖具体宿主的规则；路由分发与调用上下文仍由
//! [`Ingress`](crate::Ingress) 实现决定。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use symbio_plugin_api::routes::{
    ROUTE_VDFS_LIST, ROUTE_VDFS_READ, ROUTE_VDFS_ROOT, ROUTE_VDFS_SEARCH, ROUTE_VDFS_STAT,
    ROUTE_VDFS_TREE,
};
use symbio_plugin_api::PLUGIN_FILE;

use crate::{HttpRequest, HttpResponse};

const IDEMPOTENCY_TTL: Duration = Duration::from_secs(600);
const IDEMPOTENCY_CAPACITY: usize = 4096;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 200;

/// 只读模式放行的调用白名单。
///
/// 只读是令牌泄露后的兜底，不是完整安全边界：只放行 VDFS 读操作，写操作与节点动作一律拒绝。
/// `vdfs/read` 落到任一插件配置文档时也要拒绝——配置可能含访问令牌等凭据。
/// `event_bus/subscribe` 不在白名单内：它是覆盖全部在途内容的长连接广播，与「点名读一份」不同形。
pub fn is_readonly_allowed(path: &str, payload: &Value) -> bool {
    let p = path.trim_start_matches('/');
    if p == ROUTE_VDFS_READ && reads_config_document(payload) {
        return false;
    }
    matches!(
        p,
        ROUTE_VDFS_ROOT
            | ROUTE_VDFS_LIST
            | ROUTE_VDFS_TREE
            | ROUTE_VDFS_STAT
            | ROUTE_VDFS_READ
            | ROUTE_VDFS_SEARCH
    )
}

fn reads_config_document(payload: &Value) -> bool {
    payload
        .get("path")
        .and_then(Value::as_str)
        .map(|addr| {
            addr.trim_end_matches('/')
                .ends_with(&format!("/{}", PLUGIN_FILE))
        })
        .unwrap_or(false)
}

/// 按请求头登记幂等键；`Err` 是应直接返回给调用方的响应。
pub fn reserve_idempotency(
    request: &HttpRequest,
    table: &Arc<Idempotency>,
) -> Result<Option<IdempotencyTicket>, HttpResponse> {
    match request.headers.get("idempotency-key").map(|key| key.trim()) {
        Some(key) if !valid_idempotency_key(key) => Err(HttpResponse::json(
            400,
            json!({ "error": "invalid Idempotency-Key" }),
        )),
        Some(key) => table.reserve(key).map(Some).ok_or_else(|| {
            HttpResponse::json(409, json!({ "error": "duplicate Idempotency-Key" }))
        }),
        None => Ok(None),
    }
}

/// 入站调用的进程内幂等键表。
#[derive(Debug)]
pub struct Idempotency {
    seen: Mutex<HashMap<String, Instant>>,
}

/// 一条已登记的键；分派失败时释放，让失败请求可以合法重试。
#[derive(Debug)]
pub struct IdempotencyTicket {
    table: Arc<Idempotency>,
    key: String,
}

impl Idempotency {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(HashMap::new()),
        })
    }

    fn reserve(self: &Arc<Self>, key: &str) -> Option<IdempotencyTicket> {
        self.reserve_at(key, Instant::now())
    }

    fn reserve_at(self: &Arc<Self>, key: &str, now: Instant) -> Option<IdempotencyTicket> {
        let mut seen = self.seen.lock().unwrap_or_else(|error| error.into_inner());
        seen.retain(|_, stored| now.duration_since(*stored) < IDEMPOTENCY_TTL);
        if seen.contains_key(key) {
            return None;
        }
        if seen.len() >= IDEMPOTENCY_CAPACITY {
            if let Some(oldest) = seen
                .iter()
                .min_by_key(|(_, stored)| **stored)
                .map(|(key, _)| key.clone())
            {
                seen.remove(&oldest);
            }
        }
        seen.insert(key.to_string(), now);
        Some(IdempotencyTicket {
            table: self.clone(),
            key: key.to_string(),
        })
    }
}

impl IdempotencyTicket {
    pub fn release(&self) {
        let mut seen = self
            .table
            .seen
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        seen.remove(&self.key);
    }
}

fn valid_idempotency_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_IDEMPOTENCY_KEY_BYTES
        && key.bytes().all(|byte| (0x21..=0x7E).contains(&byte))
}

#[cfg(test)]
#[path = "ingress_policy.test.rs"]
mod tests;
