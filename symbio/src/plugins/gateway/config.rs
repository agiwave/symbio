//! 网关配置：入站（对外提供服务）与出站（本应用连向何处）
//!
//! 字段刻意**扁平化**（`inbound_*` / `outbound_*`），以直接契合设置表单的 flat key 绑定：
//! 表单按 key 读写 `config[key]`，`config/set` 整体替换，无需嵌套路径解析。

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

    // ---- 出站：本应用的前端连向何处 ----
    /// 出站协议：`native`（进程内直连本机后端）| `http`
    pub outbound_protocol: String,
    /// 目标地址（http 协议下为 `http://host:port`）
    pub outbound_endpoint: String,
    /// 访问令牌
    pub outbound_token: String,
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
            outbound_protocol: "native".to_string(),
            outbound_endpoint: "http://127.0.0.1:9231".to_string(),
            outbound_token: String::new(),
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
    fn default_outbound_is_native_and_inbound_off() {
        let c = GatewayConfig::default();
        assert!(!c.inbound_enabled);
        assert_eq!(c.inbound_protocol, "native");
        assert_eq!(c.inbound_port, 9231);
        assert_eq!(c.inbound_bind, "127.0.0.1");
        assert_eq!(c.outbound_protocol, "native");
        assert_eq!(c.outbound_endpoint, "http://127.0.0.1:9231");
    }

    /// 关键契约：前端 boot 读取 `gateway/config/get` 后按**扁平键**
    /// `outbound_protocol` / `outbound_endpoint` / `outbound_token` 解析。
    /// 若结构变回嵌套 `outbound: { protocol }`，前端出站 http 将永远不激活。
    #[test]
    fn serde_uses_flat_keys_not_nested() {
        let c = GatewayConfig {
            outbound_protocol: "http".into(),
            outbound_endpoint: "http://remote:9231".into(),
            outbound_token: "secret".into(),
            ..GatewayConfig::default()
        };
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["outbound_protocol"], "http");
        assert_eq!(v["outbound_endpoint"], "http://remote:9231");
        assert_eq!(v["outbound_token"], "secret");
        // 绝不能是嵌套对象
        assert!(v.get("outbound").is_none(), "配置被错误地嵌套为 outbound.*");

        // 反序列化回上层结构也应保留
        let back: GatewayConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back.outbound_protocol, "http");
        assert_eq!(back.outbound_endpoint, "http://remote:9231");
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

