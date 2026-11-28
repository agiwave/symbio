//! 网关配置：仅保留「入站」（本应用对外提供服务的 HTTP/WebSocket 网关）。
//!
//! 字段刻意**扁平化**（`inbound_*`），以直接契合配置文档的 flat key 绑定：
//! 一次 `vdfs/write` 整体替换，无需嵌套路径解析。
//!
//! **出站（前端连向何处）不由本插件持有**：连接目标由前端「系统目录」切换器
//! 统一管理（localStorage 为权威），经 `initGatewayTransport` 决定 native / http 出站。
//! 因此本配置只描述「本实例如何被调用（入站）」。

use serde::{Deserialize, Serialize};

/// 网关配置（整体作为插件配置持久化于 `symbio.plugins.gateway`）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    // ---- 入站：本应用对外提供服务 ----
    /// 是否开放入站服务
    pub inbound_enabled: bool,
    /// 入站协议：`native`（不监听）| `http`
    pub inbound_protocol: String,
    /// 监听地址（默认 `127.0.0.1`；`0.0.0.0` 对外暴露需谨慎）
    pub inbound_bind: String,
    /// 监听端口
    pub inbound_port: u16,
    /// 访问令牌（Bearer Token）；为空表示不校验（仅限回环地址时允许）
    pub inbound_token: String,
    /// 只读模式：仅放行查询类路径
    pub inbound_readonly: bool,
    /// 同时存活的连接上限（HTTP 一次调用占一个、WS 一条连接活到关闭）。
    ///
    /// 到顶后新连接留在内核 accept 队列里等——**背压优先于报错**：拒绝本身也要
    /// 一条连接，而连接洪峰正是本上限要挡的东西。`0` 按 `1` 处理（上限为零等于
    /// 服务不可用，那不该由配置的「默认值恰好为零」意外产生）。
    pub inbound_max_connections: usize,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            inbound_enabled: false,
            inbound_protocol: "native".to_string(),
            inbound_bind: "127.0.0.1".to_string(),
            inbound_port: 9231,
            inbound_token: String::new(),
            inbound_readonly: false,
            inbound_max_connections: 64,
        }
    }
}

#[cfg(test)]
#[path = "config.test.rs"]
mod tests;
