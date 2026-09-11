//! 网关配置：仅保留「入站」（本应用对外提供服务的 HTTP/WebSocket 网关）。
//!
//! 字段刻意**扁平化**（`inbound_*`），以直接契合设置表单的 flat key 绑定：
//! `config/set` 整体替换，无需嵌套路径解析。
//!
//! **出站（前端连向何处）已不再由本插件持有**：连接目标由前端「系统目录」切换器
//! 统一管理（localStorage 为权威），经 `initGatewayTransport` 决定 native / http 出站。
//! 因此本配置只描述「本实例如何被调用（入站）」，不再描述「前端连向何处」。

use serde::{Deserialize, Serialize};

/// 网关配置（整体作为插件配置持久化于 `symbio.plugins.gateway`）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GatewayConfig {
    // ---- 入站：本应用对外提供服务 ----
    /// 是否开放入站服务
    pub inbound_enabled: bool,
    /// 入站协议：`native`（仅 Tauri IPC，不监听）| `http`
    pub inbound_protocol: String,
    /// 监听地址（默认 `127.0.0.1`；`0.0.0.0` 对外暴露需谨慎）
    pub inbound_bind: String,
    /// 监听端口
    pub inbound_port: u16,
    /// 访问令牌（Bearer Token）；为空表示不校验（仅限回环地址时允许）
    pub inbound_token: String,
    /// 只读模式：仅放行查询类路径
    pub inbound_readonly: bool,
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
        }
    }
}

/// 只读模式放行的路径前缀白名单
///
/// 只读并非完整安全边界，而是**兜底**：即便令牌泄露到可信内网，也只能读取
/// 而无法触发写操作与命令执行。
///
/// **网关自身配置不在放行范围内**：`gateway/*` 接口恒走 native（前端不经 HTTP
/// 访问本插件），且 `gateway/config/get` 会返回 `inbound_token`，一旦放行等于
/// 只读模式下也能把令牌读走——故显式拒绝，与 `gateway/config/set` 一致。
pub fn is_readonly_allowed(path: &str) -> bool {
    let p = path.trim_start_matches('/');
    matches!(
        p,
        "entities/list"
            | "entities/get"
            | "entities/status"
            | "entities/detail"
            | "entities/providers"
            | "session/get_messages"
            | "config/get"
            | "home/get_homedir"
            | "work/get_workspace"
    ) || p.starts_with("config/get")
        || p.starts_with("entities/list")
        || p.starts_with("entities/get")
        || p.starts_with("entities/detail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_inbound_off_and_native() {
        let c = GatewayConfig::default();
        assert!(!c.inbound_enabled);
        assert_eq!(c.inbound_protocol, "native");
        assert_eq!(c.inbound_port, 9231);
        assert_eq!(c.inbound_bind, "127.0.0.1");
        // 配置仅描述入站；不再含出站字段
        let v = serde_json::to_value(&c).unwrap();
        assert!(v.get("outbound_protocol").is_none());
        assert!(v.get("outbound_endpoint").is_none());
        assert!(v.get("outbound_token").is_none());
    }

    #[test]
    fn readonly_allowlist() {
        // 放行：查询类 / config/get 及其子路径
        assert!(is_readonly_allowed("config/get"));
        assert!(is_readonly_allowed("/config/get"));
        assert!(is_readonly_allowed("entities/list"));
        assert!(is_readonly_allowed("entities/get"));
        assert!(is_readonly_allowed("entities/detail"));
        assert!(is_readonly_allowed("entities/providers"));
        assert!(is_readonly_allowed("home/get_homedir"));
        assert!(is_readonly_allowed("work/get_workspace"));

        // 拒绝：写操作 / 命令执行 / 未知
        assert!(!is_readonly_allowed("config/set"));
        assert!(!is_readonly_allowed("session/chat/send"));
        assert!(!is_readonly_allowed("entities/upload"));
        assert!(!is_readonly_allowed("bogus/path"));

        // 拒绝：网关自身配置——`gateway/*` 恒走 native，且 config/get 含 inbound_token。
        // 只读模式下放行等于把令牌读走，故与 config/set 同等对待（拒绝）。
        assert!(!is_readonly_allowed("gateway/config/get"));
        assert!(!is_readonly_allowed("gateway/config/set"));
    }
}
