//! MCP 客户端 transport 路由器
//!
//! ## 职责
//!
//! - 根据 `McpServerConfig.transport_type` 把 discover / call 路由到
//!   stdio 或 http transport
//! - 应用 `include_tools` / `exclude_tools` 过滤
//!
//! ## 设计：按需加载（Lazy）
//!
//! - **stdio** transport 每次调用都 **spawn 新的子进程**，完成后立即 kill
//!   - 通过 per-server `Mutex` 序列化同一 server 的并发调用，避免 stdout/stdin 串话
//! - **http** transport 通过**共享 `reqwest::Client`**（连接池）发起请求
//!   - 复用 `Mcp-Session-Id`（`McpSessionCache`）减少 handshake 开销
//! - **不**持有长连接 / 进程
//!
//! ## 缓存策略
//!
//! - `tools_cache`：缓存 `discover_tools` 结果，TTL = 5 分钟
//!   - 缓存命中避免重复 spawn 子进程 / 重新发起 initialize 握手
//!
//! ## 激活控制
//!
//! MCP 工具是否对 agent 可见，由 `McpPlugin::traverse` 在每次 `parent.traverse`
//! 时根据 `McpConfig.servers` 的 `enabled` 字段动态决定：
//! 仅当 server 存在 + enabled=true 时才注册到 `tool_manager`。
//! 因此不需要在 `McpManager` 中维护"激活集合"——遍历 + 注册即激活。

use crate::symbio_core::schemas::mcp::mcp_config::{McpServerConfig, McpTransportType};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use super::types::McpTool;

/// BUG-MR30：`test_connection` 的成功结果
///
/// 包含 tool count + 协议版本 + server 元信息（来自 initialize 响应）。
/// 用于在 UI 展示"连接成功（来自 xxx v1.2.3，10 个工具）"等丰富提示。
#[derive(Debug, Clone)]
pub struct TestConnectionResult {
    /// 发现的工具数量
    pub tool_count: usize,
    /// 协议版本（协商结果）
    pub protocol_version: String,
    /// BUG-MR30：server 报告的名称（来自 `initialize.protocolVersion`）
    pub server_name: Option<String>,
    /// BUG-MR30：server 报告的版本（来自 `initialize.serverInfo.version`）
    pub server_version: Option<String>,
    /// BUG-MR32：server 提供的使用说明
    pub instructions: Option<String>,
    /// 测试耗时（毫秒）
    pub elapsed_ms: u64,
}

/// discover 缓存有效期（5 分钟）
const DISCOVER_CACHE_TTL: Duration = Duration::from_secs(300);

/// 缓存条目
#[derive(Clone)]
struct CachedTools {
    tools: Vec<McpTool>,
    inserted_at: Instant,
}

/// per-server 并发锁：用于序列化同一 server 的 stdio 调用
///
/// stdio transport 每次都 spawn 新进程，但 MCP 规范下多并发调用
/// 同一 server 仍可能导致 server 端资源耗尽。锁可以保证公平排队。
pub type McpServerLock = Arc<Mutex<()>>;

/// session_id 缓存（HTTP transport）
///
/// key: server name, value: `Mcp-Session-Id`
#[derive(Default, Clone)]
pub struct McpSessionCache {
    inner: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl McpSessionCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, name: &str) -> Option<String> {
        let guard = self.inner.lock().await;
        guard.get(name).cloned()
    }

    pub async fn insert(&self, name: String, session_id: String) {
        let mut guard = self.inner.lock().await;
        guard.insert(name, session_id);
    }

    pub async fn remove(&self, name: &str) {
        let mut guard = self.inner.lock().await;
        guard.remove(name);
    }
}

/// BUG-MR30：HTTP initialize 协商出的协议版本缓存
///
/// 写入时机：每次 `http_initialize_full` 成功时。
/// 读取时机：`test_connection_http` 完成后返回 `TestConnectionResult.protocol_version`。
/// 这样 test 接口无需重新解析 initialize 响应。
#[derive(Default, Clone)]
pub struct McpNegotiatedProtocols {
    inner: Arc<Mutex<std::collections::HashMap<String, String>>>,
}

impl McpNegotiatedProtocols {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self, name: &str) -> Option<String> {
        let guard = self.inner.lock().await;
        guard.get(name).cloned()
    }

    pub async fn insert(&self, name: String, protocol: String) {
        let mut guard = self.inner.lock().await;
        guard.insert(name, protocol);
    }

    pub async fn remove(&self, name: &str) {
        let mut guard = self.inner.lock().await;
        guard.remove(name);
    }
}

/// MCP transport 路由器
///
/// 持有共享的 `reqwest::Client`（连接池）+ session cache + per-server lock。
pub struct McpManager {
    /// 共享 HTTP client（连接池复用）
    pub(super) http_client: reqwest::Client,
    /// HTTP session 缓存
    pub(super) session_cache: McpSessionCache,
    /// per-server 并发锁：key = server name
    server_locks: Arc<Mutex<std::collections::HashMap<String, McpServerLock>>>,
    /// discover 结果缓存
    tools_cache: Arc<Mutex<std::collections::HashMap<String, CachedTools>>>,
    /// BUG-MR30：最近协商的协议版本（HTTP initialize 时写入）
    pub(super) negotiated_protocols: McpNegotiatedProtocols,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            // BUG-MR31：不再使用 client 级硬超时——每个请求按 McpServerConfig.timeout_secs
            // 设置超时（通过 http.rs::build_request 的 `.timeout()` 方法）。
            // 这允许不同 server 独立配置超时（大工具调用可放宽，小查询可收紧）。
            http_client: reqwest::Client::builder()
                .build()
                .expect("failed to build reqwest client"),
            session_cache: McpSessionCache::new(),
            server_locks: Arc::new(Mutex::new(std::collections::HashMap::new())),
            tools_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            negotiated_protocols: McpNegotiatedProtocols::new(),
        }
    }

    /// 获取（或创建）per-server 并发锁
    async fn get_server_lock(&self, name: &str) -> McpServerLock {
        let mut guards = self.server_locks.lock().await;
        guards
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// 通过 server 配置发现工具（带缓存 + BUG-MR24 失败时 fallback）
    ///
    /// 缓存命中直接返回；miss 时调用对应 transport，发现后写入缓存。
    /// **如果 transport 调用失败但缓存有**陈旧**条目**（即使超过 TTL），
    /// 返回陈旧条目以保证 agent 工具列表的稳定性（避免临时网络抖动导致工具消失）。
    pub async fn discover_tools(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<Vec<McpTool>, String> {
        // 1. 检查缓存
        {
            let cache = self.tools_cache.lock().await;
            if let Some(cached) = cache.get(name) {
                if cached.inserted_at.elapsed() < DISCOVER_CACHE_TTL {
                    info!(
                        server = name,
                        tools = cached.tools.len(),
                        cache_age_ms = cached.inserted_at.elapsed().as_millis() as u64,
                        "MCP discover 命中缓存"
                    );
                    return Ok(cached.tools.clone());
                }
            }
        }

        // 2. 缓存 miss，调用 transport
        let result = match config.transport_type {
            McpTransportType::Stdio => self.discover_tools_stdio(config).await,
            McpTransportType::Http | McpTransportType::Sse => {
                self.discover_tools_http(name, config).await
            }
        };

        let raw = match result {
            Ok(t) => t,
            Err(e) => {
                // BUG-MR24：transport 失败时，检查是否有**陈旧**缓存可作为 fallback
                let cache = self.tools_cache.lock().await;
                if let Some(cached) = cache.get(name) {
                    warn!(
                        server = name,
                        error = %e,
                        stale_age_ms = cached.inserted_at.elapsed().as_millis() as u64,
                        "MCP discover 失败，返回陈旧缓存作为 fallback"
                    );
                    return Ok(cached.tools.clone());
                }
                return Err(e);
            }
        };
        // BUG-MR27：过滤掉非法 tool name（避免污染 agent 工具路由）
        let (valid, invalid_count) = super::types::filter_valid_tool_names(raw);
        if invalid_count > 0 {
            warn!(
                server = name,
                invalid_count, "MCP server 返回了非法 tool name，已过滤"
            );
        }
        let filtered =
            super::types::filter_tools(valid, &config.include_tools, &config.exclude_tools);

        // 3. 写缓存
        {
            let mut cache = self.tools_cache.lock().await;
            cache.insert(
                name.to_string(),
                CachedTools {
                    tools: filtered.clone(),
                    inserted_at: Instant::now(),
                },
            );
        }

        info!(
            server = name,
            transport = ?config.transport_type,
            tools = filtered.len(),
            "MCP 工具发现完成"
        );
        Ok(filtered)
    }

    /// 失效某个 server 的 discover 缓存
    ///
    /// server 配置变更后调用，下次 discover 会重新发现。
    pub async fn invalidate_discover_cache(&self, name: &str) {
        let mut cache = self.tools_cache.lock().await;
        cache.remove(name);
    }

    /// BUG-MR29：删除 server 时清理相关所有内部状态
    ///
    /// 包括 discover 缓存、HTTP session 缓存、per-server 并发锁、协商协议版本。
    /// 由 `McpPlugin::route("servers/delete")` 调用，避免长期运行时
    /// server_locks 累积孤儿条目。
    pub async fn forget_server(&self, name: &str) {
        self.invalidate_discover_cache(name).await;
        self.session_cache.remove(name).await;
        self.negotiated_protocols.remove(name).await;
        let mut locks = self.server_locks.lock().await;
        locks.remove(name);
        debug!(
            server = name,
            "MCP 内部状态已清理（cache + session + lock + protocol）"
        );
    }

    /// 测试某个 MCP server 的连接（完整握手 + 一次 `tools/list`）
    ///
    /// ## 设计
    ///
    /// - **不**走 discover 缓存（避免缓存的 false-positive）
    /// - **不**写 discover 缓存（避免污染后续正常调用）
    /// - **不**修改 HTTP session 缓存
    /// - 仅用于"用户在 UI 上点击测试连接"时的可用性验证
    ///
    /// 返回 `TestConnectionResult`：
    /// - `Ok(result)` 包含 tool count、协议版本、server 名称/版本、instructions
    /// - `Err(e)` 表示失败
    pub async fn test_connection(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<TestConnectionResult, String> {
        let start = std::time::Instant::now();
        let result = match config.transport_type {
            McpTransportType::Stdio => self.test_connection_stdio(config).await,
            McpTransportType::Http | McpTransportType::Sse => {
                self.test_connection_http(name, config).await
            }
        };
        let elapsed_ms = start.elapsed().as_millis() as u64;
        match &result {
            Ok(r) => info!(
                server = name,
                transport = ?config.transport_type,
                elapsed_ms,
                tool_count = r.tool_count,
                protocol = %r.protocol_version,
                upstream = ?r.server_name,
                "MCP test_connection 成功"
            ),
            Err(e) => info!(
                server = name,
                transport = ?config.transport_type,
                elapsed_ms,
                error = %e,
                "MCP test_connection 失败"
            ),
        }
        result.map(|mut r| {
            r.elapsed_ms = elapsed_ms;
            r
        })
    }

    /// 调用 server 上的工具
    ///
    /// **stdio** 走 per-server Mutex 序列化并发；
    /// **http** 不需要锁（每个请求独立）。
    pub async fn call_tool(
        &self,
        name: &str,
        config: &McpServerConfig,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        info!(
            server = name,
            tool = tool_name,
            transport = ?config.transport_type,
            "MCP 工具调用"
        );
        match config.transport_type {
            McpTransportType::Stdio => {
                let lock = self.get_server_lock(name).await;
                let _guard = lock.lock().await;
                self.call_tool_stdio(config, tool_name, arguments).await
            }
            McpTransportType::Http | McpTransportType::Sse => {
                self.call_tool_http(name, config, tool_name, arguments)
                    .await
            }
        }
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
